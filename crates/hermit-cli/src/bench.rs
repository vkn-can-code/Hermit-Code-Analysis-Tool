//! `hermit bench`: accuracy and latency on labelled fixtures.
//!
//! Each fixture is `fixtures/<name>/{base/, head/, expected.toml}`. A throw-away git repo
//! is built (main = base, feature = head), analyzed, and compared with the labels.

use anyhow::{Context, Result, bail};
use hermit_core::FindingState;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

#[derive(Deserialize)]
struct ExpectedFile {
    expect: Expected,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Expected {
    fixed_cwes: Vec<String>,
    new_cwes: Vec<String>,
    changed: Vec<String>,
    impacted: Vec<String>,
    tests: Vec<String>,
    tier: Option<String>,
}

/// Precision/recall counts.
#[derive(Default, Serialize, Clone, Copy)]
pub struct Counts {
    tp: usize,
    fp: usize,
    fn_: usize,
}

impl Counts {
    fn add(&mut self, expected: &BTreeSet<String>, got: &BTreeSet<String>) {
        self.tp += expected.intersection(got).count();
        self.fp += got.difference(expected).count();
        self.fn_ += expected.difference(got).count();
    }
    fn precision(&self) -> f64 {
        if self.tp + self.fp == 0 {
            1.0
        } else {
            self.tp as f64 / (self.tp + self.fp) as f64
        }
    }
    fn recall(&self) -> f64 {
        if self.tp + self.fn_ == 0 {
            1.0
        } else {
            self.tp as f64 / (self.tp + self.fn_) as f64
        }
    }
}

#[derive(Serialize)]
struct FixtureResult {
    name: String,
    score: u32,
    tier: String,
    tier_ok: bool,
    security: Counts,
    changed: Counts,
    impacted: Counts,
    tests: Counts,
    tests_selected: usize,
    tests_total: usize,
    analysis_ms: u64,
    ai_ms: Option<u64>,
    ai_comments: Option<usize>,
    ai_rejected: Option<u32>,
    misses: Vec<String>,
}

#[derive(Serialize)]
struct Summary {
    fixtures: Vec<FixtureResult>,
    security_precision: f64,
    security_recall: f64,
    impact_precision: f64,
    impact_recall: f64,
    test_precision: f64,
    test_recall: f64,
    tests_selected: usize,
    tests_total: usize,
    p50_ms: u64,
    p95_ms: u64,
}

fn git(dir: &Path, args: &[&str]) -> Result<()> {
    let ok = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "hermit")
        .env("GIT_AUTHOR_EMAIL", "hermit@localhost")
        .env("GIT_COMMITTER_NAME", "hermit")
        .env("GIT_COMMITTER_EMAIL", "hermit@localhost")
        .output()?
        .status
        .success();
    if !ok {
        bail!("git {args:?} failed in {}", dir.display());
    }
    Ok(())
}

fn copy_dir(src: &Path, dst: &Path) -> Result<()> {
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            std::fs::create_dir_all(&to)?;
            copy_dir(&entry.path(), &to)?;
        } else {
            std::fs::copy(entry.path(), to)?;
        }
    }
    Ok(())
}

/// Builds the fixture repo; returns its path.
pub fn build_repo(fixture: &Path, dest: &Path) -> Result<()> {
    let _ = std::fs::remove_dir_all(dest);
    std::fs::create_dir_all(dest)?;
    git(dest, &["init", "-q", "-b", "main"])?;
    copy_dir(&fixture.join("base"), dest)?;
    git(dest, &["add", "-A"])?;
    git(dest, &["commit", "-qm", "base", "--allow-empty"])?;
    git(dest, &["checkout", "-qb", "feature"])?;
    for entry in std::fs::read_dir(dest)? {
        let entry = entry?;
        if entry.file_name() == ".git" {
            continue;
        }
        if entry.file_type()?.is_dir() {
            std::fs::remove_dir_all(entry.path())?;
        } else {
            std::fs::remove_file(entry.path())?;
        }
    }
    copy_dir(&fixture.join("head"), dest)?;
    git(dest, &["add", "-A"])?;
    git(dest, &["commit", "-qm", "head", "--allow-empty"])?;
    Ok(())
}

fn set(v: impl IntoIterator<Item = String>) -> BTreeSet<String> {
    v.into_iter().collect()
}

pub async fn run(dir: &Path, with_ai: bool, json_out: Option<&Path>) -> Result<bool> {
    let mut names: Vec<_> = std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().join("expected.toml").is_file())
        .map(|e| e.path())
        .collect();
    names.sort();
    if names.is_empty() {
        bail!("no fixtures with expected.toml in {}", dir.display());
    }
    let tmp = std::env::temp_dir().join(format!("hermit-bench-{}", std::process::id()));
    let mut results = Vec::new();
    let mut cache = crate::load_cache();

    for fixture in &names {
        let name = fixture.file_name().unwrap().to_string_lossy().to_string();
        let exp: ExpectedFile =
            toml::from_str(&std::fs::read_to_string(fixture.join("expected.toml"))?)
                .with_context(|| format!("{name}/expected.toml"))?;
        let exp = exp.expect;
        let repo = tmp.join(&name);
        build_repo(fixture, &repo)?;

        let mut cfg = hermit_engine::load_config(&repo, None)?;
        cfg.ai.enabled = with_ai;
        let start = Instant::now();
        let (r2, c2) = (repo.clone(), cfg.clone());
        let mut a = tokio::task::spawn_blocking(move || {
            hermit_engine::analyze(
                &hermit_engine::Request {
                    repo: &r2,
                    base: "main",
                    head: "feature",
                    name: None,
                },
                &c2,
            )
        })
        .await??;
        let analysis_ms = start.elapsed().as_millis() as u64;
        if with_ai && let Err(e) = hermit_engine::ai_review(&mut a, &cfg, &mut cache).await {
            eprintln!("{name}: AI review failed: {e:#}");
        }
        let stats = hermit_engine::stats(&a);
        let r = &a.report;

        let mut misses = Vec::new();
        let mut cmp = |label: &str, expected: BTreeSet<String>, got: BTreeSet<String>| {
            let mut c = Counts::default();
            c.add(&expected, &got);
            for m in expected.difference(&got) {
                misses.push(format!("missed {label}: {m}"));
            }
            for m in got.difference(&expected) {
                misses.push(format!("extra {label}: {m}"));
            }
            c
        };
        let sec_expected = set(exp
            .fixed_cwes
            .iter()
            .map(|c| format!("fixed:{c}"))
            .chain(exp.new_cwes.iter().map(|c| format!("new:{c}"))));
        let sec_got = set(r.findings.iter().filter_map(|f| {
            let cwe = f.cwe.clone()?;
            Some(match f.state {
                FindingState::New => format!("new:{cwe}"),
                FindingState::Fixed => format!("fixed:{cwe}"),
            })
        }));
        let security = cmp("finding", sec_expected, sec_got);
        let changed = cmp(
            "changed",
            set(exp.changed.clone()),
            set(r.symbols.iter().map(|s| s.symbol.qualified.clone())),
        );
        let impacted = cmp(
            "impacted",
            set(exp.impacted.clone()),
            set(r.impacted.iter().map(|i| i.qualified.clone())),
        );
        let tests = cmp(
            "test",
            set(exp.tests.clone()),
            set(r.tests.iter().map(|t| t.qualified.clone())),
        );
        let tier = hermit_engine::report::tier_label(r.risk.tier).to_lowercase();
        let tier_ok = exp
            .tier
            .as_ref()
            .is_none_or(|t| t.eq_ignore_ascii_case(&tier));
        if !tier_ok {
            misses.push(format!(
                "tier {tier}, expected {}",
                exp.tier.clone().unwrap_or_default()
            ));
        }
        results.push(FixtureResult {
            name,
            score: r.risk.score,
            tier,
            tier_ok,
            security,
            changed,
            impacted,
            tests,
            tests_selected: r.tests.len(),
            tests_total: stats["total_tests"],
            analysis_ms,
            ai_ms: r.ai.as_ref().map(|x| x.latency_ms),
            ai_comments: r.ai.as_ref().map(|x| x.comments.len()),
            ai_rejected: r.ai.as_ref().map(|x| x.rejected),
            misses,
        });
    }
    let _ = std::fs::remove_dir_all(&tmp);
    if with_ai {
        crate::save_cache(&cache);
    }

    let sum = |f: fn(&FixtureResult) -> Counts| {
        results.iter().fold(Counts::default(), |mut acc, r| {
            let c = f(r);
            acc.tp += c.tp;
            acc.fp += c.fp;
            acc.fn_ += c.fn_;
            acc
        })
    };
    let (sec, imp, tst) = (sum(|r| r.security), sum(|r| r.impacted), sum(|r| r.tests));
    let mut lat: Vec<u64> = results.iter().map(|r| r.analysis_ms).collect();
    lat.sort();
    let pct = |p: f64| lat[((lat.len() as f64 - 1.0) * p).round() as usize];
    let summary = Summary {
        security_precision: sec.precision(),
        security_recall: sec.recall(),
        impact_precision: imp.precision(),
        impact_recall: imp.recall(),
        test_precision: tst.precision(),
        test_recall: tst.recall(),
        tests_selected: results.iter().map(|r| r.tests_selected).sum(),
        tests_total: results.iter().map(|r| r.tests_total).sum(),
        p50_ms: pct(0.5),
        p95_ms: pct(0.95),
        fixtures: results,
    };

    println!("| Fixture | Risk | Security P/R | Impact P/R | Tests P/R | Tests run | Time |");
    println!("|---|---|---|---|---|---|---|");
    for r in &summary.fixtures {
        let ai = r
            .ai_ms
            .map(|ms| {
                format!(
                    " (+AI {ms} ms, {} kept / {} dropped)",
                    r.ai_comments.unwrap_or(0),
                    r.ai_rejected.unwrap_or(0)
                )
            })
            .unwrap_or_default();
        println!(
            "| {} | {} {}{} | {:.2}/{:.2} | {:.2}/{:.2} | {:.2}/{:.2} | {}/{} | {} ms{} |",
            r.name,
            r.score,
            r.tier,
            if r.tier_ok { "" } else { " ✘" },
            r.security.precision(),
            r.security.recall(),
            r.impacted.precision(),
            r.impacted.recall(),
            r.tests.precision(),
            r.tests.recall(),
            r.tests_selected,
            r.tests_total,
            r.analysis_ms,
            ai
        );
    }
    println!(
        "\n**Overall** security P={:.2} R={:.2} · blast radius P={:.2} R={:.2} · tests P={:.2} R={:.2} · ran {}/{} tests ({:.0}% fewer) · latency p50 {} ms, p95 {} ms",
        summary.security_precision,
        summary.security_recall,
        summary.impact_precision,
        summary.impact_recall,
        summary.test_precision,
        summary.test_recall,
        summary.tests_selected,
        summary.tests_total,
        100.0 * (1.0 - summary.tests_selected as f64 / summary.tests_total.max(1) as f64),
        summary.p50_ms,
        summary.p95_ms
    );
    for r in &summary.fixtures {
        for m in &r.misses {
            println!("  {}: {m}", r.name);
        }
    }
    if let Some(p) = json_out {
        std::fs::write(p, serde_json::to_string_pretty(&summary)?)?;
    }
    let perfect = summary.fixtures.iter().all(|r| r.misses.is_empty());
    Ok(perfect)
}
