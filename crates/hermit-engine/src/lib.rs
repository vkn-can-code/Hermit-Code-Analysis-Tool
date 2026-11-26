//! The Hermit analysis pipeline: diff -> AST -> security -> blast radius -> tests -> risk.
//!
//! Used identically by the CLI, the GitHub webhook worker and the benchmark.

pub mod config;
pub mod report;

pub use config::Config;

use anyhow::Result;
use hermit_core::{
    AnalysisReport, ChangeKind, ChangedFile, ChangedSymbol, Language, NewDependency, Timings,
};
use hermit_diff::ChangeSet;
use hermit_graph::{RepoIndex, SourceFile};
use hermit_security::{Scanner, Sources};
use hermit_tests::JsRunner;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Directories never indexed.
const SKIP_DIRS: &[&str] = &[
    "node_modules",
    "target",
    ".venv",
    "venv",
    "dist",
    "build",
    "vendor",
    "__pycache__",
    ".git",
    ".next",
    "coverage",
    "site-packages",
];
/// Files larger than this are skipped (generated / minified code).
const MAX_FILE_BYTES: usize = 512 * 1024;

/// What to analyze.
pub struct Request<'a> {
    pub repo: &'a Path,
    pub base: &'a str,
    pub head: &'a str,
    /// Display name (e.g. `owner/name`); defaults to the directory name.
    pub name: Option<String>,
}

/// Intermediate state kept for later stages (AI review needs the index and sources).
pub struct Analysis {
    pub report: AnalysisReport,
    pub index: RepoIndex,
    pub sources: Sources,
    /// Numbered diff (head line numbers) for the AI stage.
    pub patch: String,
}

/// Runs the deterministic pipeline (everything except the AI review).
pub fn analyze(req: &Request, cfg: &Config) -> Result<Analysis> {
    analyze_with_progress(req, cfg, &|_| {})
}

/// Pipeline stage names reported through the progress callback, in order.
pub const STAGES: &[&str] = &[
    "diff",
    "ast+security",
    "blast_radius",
    "tests",
    "risk",
    "ai",
];

/// Like [`analyze`], calling `progress(stage)` as each stage starts.
pub fn analyze_with_progress(
    req: &Request,
    cfg: &Config,
    progress: &(dyn Fn(&str) + Sync),
) -> Result<Analysis> {
    let total = Instant::now();
    progress("diff");
    let mut t = Timings::default();

    // 1. Diff
    let start = Instant::now();
    let cs = ChangeSet::open(req.repo, req.base, req.head)?;
    let files: Vec<ChangedFile> = cs
        .changed_files()?
        .into_iter()
        .filter(|f| !is_ignored(&f.path, cfg))
        .collect();
    let mut sources = Sources::default();
    for f in files.iter().filter(|f| f.language.is_some()) {
        if let Some(text) = cs.read_head(&f.path)? {
            sources.head.insert(f.path.clone(), text);
        }
        if let Some(text) = cs.read_base(f.base_path())? {
            sources.base.insert(f.base_path().to_path_buf(), text);
        }
    }
    t.diff_ms = ms(start);
    progress("ast+security");

    // 2 + 3. Security scan runs on its own thread while the repo is indexed
    // (Semgrep start-up dominates otherwise).
    let mut repo_sources = Vec::new();
    for path in cs.head_paths()? {
        let Some(language) = Language::from_path(&path) else {
            continue;
        };
        if is_ignored(&path, cfg) {
            continue;
        }
        let text = match sources.head.get(&path) {
            Some(t) => t.clone(),
            None => match cs.read_head(&path)? {
                Some(t) if t.len() <= MAX_FILE_BYTES => t,
                _ => continue,
            },
        };
        repo_sources.push(SourceFile {
            path,
            language,
            text,
        });
    }
    let scanner = if cfg.security.semgrep {
        Scanner::detect(cfg.security.extra_configs.clone())
    } else {
        Scanner::builtin_only()
    };
    let (index, symbols, base_parsed, findings, parse_ms, security_ms) =
        std::thread::scope(|scope| -> Result<_> {
            let sec = scope.spawn(|| {
                let start = Instant::now();
                let r = match scanner.scan(&files, &sources) {
                    Ok(f) => Ok(f),
                    Err(e) => {
                        tracing::warn!("semgrep failed, falling back to builtin rules: {e:#}");
                        Scanner::builtin_only().scan(&files, &sources)
                    }
                };
                (r, ms(start))
            });
            let start = Instant::now();
            let index = RepoIndex::build(repo_sources);
            let mut symbols: Vec<ChangedSymbol> = Vec::new();
            let mut base_parsed = HashMap::new();
            for f in &files {
                let Some(lang) = f.language else { continue };
                let base = sources
                    .base
                    .get(f.base_path())
                    .and_then(|s| hermit_lang::parse(lang, f.base_path(), s).ok());
                let head = index.files.get(&f.path);
                symbols.extend(hermit_lang::diff_symbols(f, base.as_ref(), head));
                if let Some(b) = base {
                    base_parsed.insert(f.path.clone(), b);
                }
            }
            let parse_ms = ms(start);
            let (findings, security_ms) = sec.join().expect("security thread panicked");
            Ok((
                index,
                symbols,
                base_parsed,
                findings?,
                parse_ms,
                security_ms,
            ))
        })?;
    t.parse_ms = parse_ms;
    t.security_ms = security_ms;

    // 4. Blast radius
    progress("blast_radius");
    let start = Instant::now();
    let roots: Vec<(String, String)> = symbols
        .iter()
        .filter(|s| !s.symbol.is_test)
        .map(|s| (s.symbol.qualified.clone(), s.symbol.name.clone()))
        .collect();
    // Tests are reported separately (test selection), not as impacted callers.
    let impacted: Vec<_> = index
        .blast_radius(&roots, cfg.graph.max_depth, &cfg.graph.sensitive)
        .into_iter()
        .filter(|i| !index.symbols.get(&i.qualified).is_some_and(|s| s.is_test))
        .collect();
    let new_dependencies = new_dependencies(&index, &symbols, &base_parsed);
    t.graph_ms = ms(start);

    // 5. Tests
    progress("tests");
    let start = Instant::now();
    let pkg = cs.read_head(Path::new("package.json")).ok().flatten();
    let js = JsRunner::detect(pkg.as_deref());
    let selection = hermit_tests::select(&index, &symbols, cfg.graph.test_depth, js);
    t.tests_ms = ms(start);

    // 6. Risk
    progress("risk");
    let start = Instant::now();
    let risk = hermit_risk::assess(
        &hermit_risk::RiskInput {
            files: &files,
            symbols: &symbols,
            findings: &findings,
            impacted: &impacted,
            untested: &selection.untested,
        },
        &cfg.risk,
    );
    t.risk_ms = ms(start);
    t.total_ms = ms(total);

    let code_paths: Vec<PathBuf> = files
        .iter()
        .filter(|f| f.language.is_some())
        .map(|f| f.path.clone())
        .collect();
    let patch = if code_paths.is_empty() {
        String::new()
    } else {
        cs.numbered_patch(&code_paths, 1200)?
    };
    let name = req.name.clone().unwrap_or_else(|| {
        cs.root()
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "repo".into())
    });
    let report = AnalysisReport {
        repo: name,
        base: cs.base.to_string(),
        head: cs.head.to_string(),
        files,
        symbols,
        findings,
        impacted,
        new_dependencies,
        tests: selection.tests,
        untested: selection.untested,
        risk,
        ai: None,
        timings: t,
    };
    Ok(Analysis {
        report,
        index,
        sources,
        patch,
    })
}

/// Runs the local-LLM review and attaches it to the report.
///
/// Failures (Ollama down, timeout, bad JSON) are returned as errors; the caller keeps
/// the deterministic report, so the pipeline never blocks on the model.
pub async fn ai_review(
    analysis: &mut Analysis,
    cfg: &Config,
    cache: &mut hermit_ai::EmbedCache,
) -> Result<()> {
    if analysis.report.symbols.is_empty() {
        return Ok(());
    }
    let start = Instant::now();
    let rc = hermit_ai::ReviewerConfig {
        ollama_url: cfg.ai.ollama_url.clone(),
        model: cfg.ai.model.clone(),
        embed_model: cfg.ai.embed_model.clone(),
        timeout: std::time::Duration::from_secs(cfg.ai.timeout_secs),
        context_chunks: cfg.ai.context_chunks,
    };
    let review = hermit_ai::review(
        &rc,
        &hermit_ai::ReviewInput {
            report: &analysis.report,
            patch: &analysis.patch,
            index: &analysis.index,
        },
        cache,
    )
    .await?;
    analysis.report.ai = Some(review);
    analysis.report.timings.ai_ms = ms(start);
    analysis.report.timings.total_ms += analysis.report.timings.ai_ms;
    Ok(())
}

/// Calls made by changed symbols at head that they did not make at base.
fn new_dependencies(
    index: &RepoIndex,
    symbols: &[ChangedSymbol],
    base: &HashMap<PathBuf, hermit_lang::ParsedFile>,
) -> Vec<NewDependency> {
    let mut out = Vec::new();
    for s in symbols
        .iter()
        .filter(|s| s.change != ChangeKind::Deleted && !s.symbol.is_test)
    {
        let local = s.symbol.qualified.rsplit_once("::").map_or("", |(_, l)| l);
        let before: Vec<&str> = base
            .get(&s.symbol.file)
            .map(|pf| {
                pf.calls
                    .iter()
                    .filter(|c| {
                        c.caller
                            .as_deref()
                            .and_then(|q| q.rsplit_once("::"))
                            .is_some_and(|(_, l)| l == local)
                    })
                    .map(|c| c.callee.as_str())
                    .collect()
            })
            .unwrap_or_default();
        for e in index.callees_of(&s.symbol.qualified) {
            let callee_name = e.callee.rsplit(['.', ':']).next().unwrap_or(&e.callee);
            if !before.contains(&callee_name)
                && !out
                    .iter()
                    .any(|d: &NewDependency| d.from == s.symbol.qualified && d.to == e.callee)
            {
                out.push(NewDependency {
                    from: s.symbol.qualified.clone(),
                    to: e.callee.clone(),
                    file: s.symbol.file.clone(),
                    line: e.line,
                });
            }
        }
    }
    out
}

/// Summary stats used by the benchmark and the dashboard.
pub fn stats(a: &Analysis) -> HashMap<&'static str, usize> {
    HashMap::from([
        ("indexed_files", a.index.files.len()),
        ("indexed_symbols", a.index.symbols.len()),
        ("call_edges", a.index.edge_count()),
        (
            "total_tests",
            a.index.symbols.values().filter(|s| s.is_test).count(),
        ),
        ("selected_tests", a.report.tests.len()),
    ])
}

fn is_ignored(path: &Path, cfg: &Config) -> bool {
    let s = path.to_string_lossy();
    path.components().any(|c| {
        let c = c.as_os_str().to_string_lossy();
        SKIP_DIRS.contains(&c.as_ref())
    }) || s.ends_with(".min.js")
        || cfg.ignore.iter().any(|p| s.starts_with(p.as_str()))
}

fn ms(since: Instant) -> u64 {
    since.elapsed().as_millis() as u64
}

/// Loads `hermit.toml` from a revision of the repository (works for bare mirrors).
pub fn load_config_at(repo: &Path, rev: &str) -> Result<Config> {
    let cs = ChangeSet::open(repo, rev, rev)?;
    match cs.read_head(Path::new("hermit.toml"))? {
        Some(text) => Config::from_toml(&text).map(Config::with_env),
        None => Ok(Config::default().with_env()),
    }
}

/// Loads `hermit.toml` from the repo's head, a path, or defaults.
pub fn load_config(repo: &Path, explicit: Option<&Path>) -> Result<Config> {
    let path: Option<PathBuf> = match explicit {
        Some(p) => Some(p.to_path_buf()),
        None => {
            let p = repo.join("hermit.toml");
            p.is_file().then_some(p)
        }
    };
    match path {
        Some(p) => Config::from_toml(&std::fs::read_to_string(&p)?).map(Config::with_env),
        None => Ok(Config::default().with_env()),
    }
}
