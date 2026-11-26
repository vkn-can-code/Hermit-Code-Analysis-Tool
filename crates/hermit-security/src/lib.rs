//! Security scanning restricted to the change set.
//!
//! Runs Semgrep (bundled Hermit rules plus optional extra configs) over the base
//! and head versions of changed files, keeps findings on changed lines, and
//! classifies them as `New` (introduced) or `Fixed` (removed by this change).
//! Built-in regex rules always detect hard-coded secrets, and stand in for the
//! main injection rules when Semgrep is not installed.

mod builtin;

use anyhow::{Context, Result, bail};
use hermit_core::{ChangedFile, FileStatus, Finding, FindingState, LineRange, Severity};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The bundled ruleset, embedded at compile time.
pub const BUNDLED_RULES: &str = include_str!("../rules/hermit.yml");

/// Scanner configuration.
#[derive(Debug, Clone, Default)]
pub struct Scanner {
    semgrep: Option<PathBuf>,
    /// Extra `--config` values (local rule files or registry packs such as `p/owasp-top-ten`).
    pub extra_configs: Vec<String>,
}

/// Source text of changed files on each side, keyed by repo-relative path.
#[derive(Default)]
pub struct Sources {
    pub base: HashMap<PathBuf, String>,
    pub head: HashMap<PathBuf, String>,
}

impl Scanner {
    /// Locates `semgrep` on `PATH`.
    pub fn detect(extra_configs: Vec<String>) -> Self {
        let semgrep = which("semgrep");
        Self {
            semgrep,
            extra_configs,
        }
    }

    /// Scanner that never calls Semgrep (built-in rules only).
    pub fn builtin_only() -> Self {
        Self::default()
    }

    pub fn engine(&self) -> &'static str {
        if self.semgrep.is_some() {
            "semgrep"
        } else {
            "builtin"
        }
    }

    /// Scans the change set. Findings are sorted by severity (highest first), then location.
    pub fn scan(&self, files: &[ChangedFile], src: &Sources) -> Result<Vec<Finding>> {
        let mut head_raw: Vec<Finding> = Vec::new();
        let mut base_raw: Vec<Finding> = Vec::new();

        if let Some(bin) = &self.semgrep {
            let (h, b) = self.run_semgrep(bin, src)?;
            head_raw.extend(h);
            base_raw.extend(b);
        }
        let fallback = self.semgrep.is_none();
        for (path, text) in &src.head {
            head_raw.extend(builtin::scan(path, text, fallback));
        }
        for (path, text) in &src.base {
            base_raw.extend(builtin::scan(path, text, fallback));
        }

        let by_path: HashMap<&Path, &ChangedFile> = files
            .iter()
            .flat_map(|f| {
                let mut v = vec![(f.path.as_path(), f)];
                if let Some(o) = &f.old_path {
                    v.push((o.as_path(), f));
                }
                v
            })
            .collect();

        let mut out = Vec::new();
        // New: on added head lines (or anywhere in a brand-new file).
        for f in &head_raw {
            let Some(cf) = by_path.get(f.file.as_path()) else {
                continue;
            };
            let r = LineRange::new(f.line, f.end_line.max(f.line));
            if cf.status == FileStatus::Added || cf.touches_head(&r) {
                out.push(f.clone());
            }
        }
        // Fixed: on removed base lines, and that rule now fires fewer times in the file.
        let mut head_count: HashMap<(PathBuf, String), usize> = HashMap::new();
        for f in &head_raw {
            *head_count
                .entry((f.file.clone(), f.rule_id.clone()))
                .or_default() += 1;
        }
        let mut base_count: HashMap<(PathBuf, String), usize> = HashMap::new();
        for f in &base_raw {
            *base_count
                .entry((f.file.clone(), f.rule_id.clone()))
                .or_default() += 1;
        }
        let mut fixed_budget: HashMap<(PathBuf, String), usize> = HashMap::new();
        for f in &base_raw {
            let Some(cf) = by_path.get(f.file.as_path()) else {
                continue;
            };
            let head_key = (cf.path.clone(), f.rule_id.clone());
            let base_key = (f.file.clone(), f.rule_id.clone());
            let budget = fixed_budget.entry(base_key.clone()).or_insert_with(|| {
                base_count[&base_key]
                    .saturating_sub(head_count.get(&head_key).copied().unwrap_or(0))
            });
            let r = LineRange::new(f.line, f.end_line.max(f.line));
            if *budget > 0 && (cf.status == FileStatus::Deleted || cf.touches_base(&r)) {
                *budget -= 1;
                let mut fixed = f.clone();
                fixed.state = FindingState::Fixed;
                out.push(fixed);
            }
        }

        dedupe(&mut out);
        out.sort_by(|a, b| {
            (a.state as u8)
                .cmp(&(b.state as u8))
                .then(b.severity.cmp(&a.severity))
                .then(a.file.cmp(&b.file))
                .then(a.line.cmp(&b.line))
        });
        Ok(out)
    }

    fn run_semgrep(&self, bin: &Path, src: &Sources) -> Result<(Vec<Finding>, Vec<Finding>)> {
        let tmp = tempfile::tempdir().context("creating semgrep workspace")?;
        let root = tmp.path();
        let rules = root.join("hermit-rules.yml");
        std::fs::write(&rules, BUNDLED_RULES)?;

        let mut targets = Vec::new();
        for (side, map) in [("head", &src.head), ("base", &src.base)] {
            if map.is_empty() {
                continue;
            }
            for (p, text) in map {
                let dest = root.join(side).join(p);
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&dest, text)?;
            }
            targets.push(side);
        }
        if targets.is_empty() {
            return Ok((Vec::new(), Vec::new()));
        }

        let mut cmd = Command::new(bin);
        cmd.current_dir(root)
            .args([
                "scan",
                "--json",
                "--metrics=off",
                "--disable-version-check",
                "--quiet",
            ])
            .args(["--timeout", "20", "--config"])
            .arg(&rules);
        for c in &self.extra_configs {
            cmd.arg("--config").arg(c);
        }
        cmd.args(&targets);
        let output = cmd.output().context("running semgrep")?;
        // Exit code 1 means "findings present"; >1 is a real error.
        if output.status.code().is_some_and(|c| c > 1) {
            bail!(
                "semgrep failed ({}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
                    .lines()
                    .last()
                    .unwrap_or_default()
            );
        }
        let parsed: SemgrepOutput =
            serde_json::from_slice(&output.stdout).context("parsing semgrep JSON")?;

        let mut head = Vec::new();
        let mut base = Vec::new();
        for r in parsed.results {
            let (side, rel) = match r.path.split_once('/') {
                Some((s, rest)) => (s.to_string(), PathBuf::from(rest)),
                None => continue,
            };
            let text = if side == "head" {
                src.head.get(&rel)
            } else {
                src.base.get(&rel)
            };
            let snippet = text
                .map(|t| lines(t, r.start.line, r.end.line))
                .unwrap_or_default();
            let rule_id = normalize_rule_id(&r.check_id);
            let meta = &r.extra.metadata;
            let severity = match r.extra.severity.as_str() {
                "ERROR" if meta.critical.unwrap_or(false) => Severity::Critical,
                "ERROR" => Severity::High,
                "WARNING" => Severity::Medium,
                "INFO" => Severity::Low,
                _ => Severity::Info,
            };
            let finding = Finding {
                title: meta.title.clone().unwrap_or_else(|| rule_id.clone()),
                rule_id,
                message: r.extra.message.trim().to_string(),
                severity,
                cwe: meta.cwe.as_ref().map(CweField::first),
                file: rel,
                line: r.start.line,
                end_line: r.end.line,
                snippet,
                state: FindingState::New,
                engine: "semgrep".into(),
            };
            if side == "head" {
                head.push(finding);
            } else {
                base.push(finding);
            }
        }
        Ok((head, base))
    }
}

/// Semgrep prefixes rule ids with the config file's directory; keep from `hermit.` on.
fn normalize_rule_id(id: &str) -> String {
    match id.find("hermit.") {
        Some(i) => id[i..].to_string(),
        None => id.to_string(),
    }
}

fn lines(text: &str, start: u32, end: u32) -> String {
    text.lines()
        .skip(start.saturating_sub(1) as usize)
        .take((end.saturating_sub(start) + 1).min(6) as usize)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Removes duplicates on the same line with the same CWE (Semgrep and builtin overlap).
fn dedupe(v: &mut Vec<Finding>) {
    let mut seen = std::collections::HashSet::new();
    v.retain(|f| {
        let key = (
            f.file.clone(),
            f.line,
            f.cwe.clone().unwrap_or_else(|| f.rule_id.clone()),
            f.state as u8,
        );
        seen.insert(key)
    });
}

fn which(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(bin))
        .find(|p| p.is_file())
}

#[derive(Deserialize)]
struct SemgrepOutput {
    results: Vec<SemgrepResult>,
}

#[derive(Deserialize)]
struct SemgrepResult {
    check_id: String,
    path: String,
    start: Pos,
    end: Pos,
    extra: Extra,
}

#[derive(Deserialize)]
struct Pos {
    line: u32,
}

#[derive(Deserialize)]
struct Extra {
    message: String,
    severity: String,
    #[serde(default)]
    metadata: Meta,
}

#[derive(Deserialize, Default)]
struct Meta {
    cwe: Option<CweField>,
    title: Option<String>,
    critical: Option<bool>,
}

/// Registry rules use a list of CWE strings, ours a single string.
#[derive(Deserialize)]
#[serde(untagged)]
enum CweField {
    One(String),
    Many(Vec<String>),
}

impl CweField {
    fn first(&self) -> String {
        let s = match self {
            Self::One(s) => s.as_str(),
            Self::Many(v) => v.first().map_or("", String::as_str),
        };
        s.split(':').next().unwrap_or(s).trim().to_string()
    }
}

#[cfg(test)]
mod tests;
