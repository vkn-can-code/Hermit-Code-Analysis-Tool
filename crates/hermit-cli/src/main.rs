//! `hermit` command-line interface.

mod bench;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use hermit_core::RiskTier;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "hermit",
    version,
    about = "Intelligent code analysis for pull requests"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Analyze the change between two revisions of a repository.
    Analyze {
        /// Repository path.
        #[arg(long, default_value = ".")]
        repo: PathBuf,
        /// Base revision (branch, tag or SHA). The merge-base with head is used.
        #[arg(long, default_value = "main")]
        base: String,
        /// Head revision.
        #[arg(long, default_value = "HEAD")]
        head: String,
        /// Output format.
        #[arg(long, value_enum, default_value_t = Format::Markdown)]
        format: Format,
        /// Write output to a file instead of stdout.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Config file (default: <repo>/hermit.toml).
        #[arg(long)]
        config: Option<PathBuf>,
        /// Skip the local LLM review.
        #[arg(long)]
        no_ai: bool,
        /// Exit with code 2 when risk reaches this tier.
        #[arg(long, value_enum)]
        fail_on: Option<Tier>,
    },
    /// Run the webhook server, REST API, dashboard and workers.
    Serve {
        /// Listen address (overrides HERMIT_BIND).
        #[arg(long)]
        bind: Option<String>,
        /// PostgreSQL URL (overrides DATABASE_URL).
        #[arg(long)]
        database_url: Option<String>,
        /// Number of analysis workers.
        #[arg(long)]
        workers: Option<usize>,
    },
    /// Measure accuracy and speed on labelled fixtures (fixtures/<name>/{base,head,expected.toml}).
    Bench {
        #[arg(default_value = "fixtures")]
        dir: PathBuf,
        /// Also run the AI review and report its latency / rejection rate.
        #[arg(long)]
        ai: bool,
        /// Write results as JSON.
        #[arg(long)]
        json: Option<PathBuf>,
    },
    /// Check that external dependencies (git, semgrep, Ollama models, PostgreSQL) are ready.
    Doctor,
}

#[derive(Clone, Copy, ValueEnum)]
enum Format {
    Markdown,
    Json,
}

#[derive(Clone, Copy, ValueEnum)]
enum Tier {
    Low,
    Medium,
    High,
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,sqlx=warn,tower_http=info".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    match rt.block_on(run()) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

/// Embedding cache file for CLI runs.
fn cache_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".hermit/embeddings.json")
}

pub(crate) fn load_cache() -> hermit_ai::EmbedCache {
    std::fs::read(cache_path())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub(crate) fn save_cache(cache: &hermit_ai::EmbedCache) {
    let p = cache_path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(bytes) = serde_json::to_vec(cache) {
        let _ = std::fs::write(p, bytes);
    }
}

async fn run() -> Result<ExitCode> {
    match Cli::parse().cmd {
        Cmd::Analyze {
            repo,
            base,
            head,
            format,
            out,
            config,
            no_ai,
            fail_on,
        } => {
            let cfg = hermit_engine::load_config(&repo, config.as_deref())?;
            let (r2, c2) = (repo.clone(), cfg.clone());
            let mut analysis = tokio::task::spawn_blocking(move || {
                hermit_engine::analyze(
                    &hermit_engine::Request {
                        repo: &r2,
                        base: &base,
                        head: &head,
                        name: None,
                    },
                    &c2,
                )
            })
            .await??;
            if cfg.ai.enabled && !no_ai {
                let mut cache = load_cache();
                eprintln!("hermit: running local AI review ({})…", cfg.ai.model);
                if let Err(e) = hermit_engine::ai_review(&mut analysis, &cfg, &mut cache).await {
                    eprintln!("hermit: AI review skipped: {e:#}");
                }
                save_cache(&cache);
            }
            let report = &analysis.report;
            let text = match format {
                Format::Markdown => hermit_engine::report::markdown(report),
                Format::Json => serde_json::to_string_pretty(report)?,
            };
            match out {
                Some(p) => std::fs::write(p, text)?,
                None => println!("{text}"),
            }
            let tripped = fail_on.is_some_and(|t| {
                let threshold = match t {
                    Tier::Low => RiskTier::Low,
                    Tier::Medium => RiskTier::Medium,
                    Tier::High => RiskTier::High,
                };
                report.risk.tier as u8 >= threshold as u8
            });
            Ok(if tripped {
                ExitCode::from(2)
            } else {
                ExitCode::SUCCESS
            })
        }
        Cmd::Serve {
            bind,
            database_url,
            workers,
        } => {
            let mut opts = hermit_server::ServeOptions::from_env()?;
            if let Some(b) = bind {
                opts.bind = b.parse().context("--bind must be host:port")?;
            }
            if let Some(u) = database_url {
                opts.database_url = u;
            }
            if let Some(w) = workers {
                opts.workers = w;
            }
            hermit_server::run(opts).await?;
            Ok(ExitCode::SUCCESS)
        }
        Cmd::Bench { dir, ai, json } => {
            let ok = bench::run(&dir, ai, json.as_deref()).await?;
            Ok(if ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
        Cmd::Doctor => doctor().await,
    }
}

async fn doctor() -> Result<ExitCode> {
    let mut ok = true;
    let mut line = |name: &str, good: bool, detail: String| {
        ok &= good;
        println!("{} {name:<12} {detail}", if good { "✔" } else { "✘" });
    };
    let git = std::process::Command::new("git").arg("--version").output();
    line(
        "git",
        git.as_ref().is_ok_and(|o| o.status.success()),
        git.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_else(|_| "not found".into()),
    );
    let sg = std::process::Command::new("semgrep")
        .arg("--version")
        .output();
    line(
        "semgrep",
        sg.as_ref().is_ok_and(|o| o.status.success()),
        sg.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_else(|_| "not found (builtin rules will be used)".into()),
    );
    let cfg = hermit_engine::Config::default();
    match hermit_ai::Ollama::new(&cfg.ai.ollama_url, std::time::Duration::from_secs(3))?
        .models()
        .await
    {
        Ok(models) => {
            let has = |m: &str| {
                models
                    .iter()
                    .any(|x| x == m || x.starts_with(&format!("{m}:")))
            };
            line(
                "ollama",
                true,
                format!("{} model(s) at {}", models.len(), cfg.ai.ollama_url),
            );
            line(
                "llm model",
                has(&cfg.ai.model),
                format!("{} (ollama pull {})", cfg.ai.model, cfg.ai.model),
            );
            line(
                "embed model",
                has(&cfg.ai.embed_model),
                format!(
                    "{} (ollama pull {})",
                    cfg.ai.embed_model, cfg.ai.embed_model
                ),
            );
        }
        Err(e) => line("ollama", false, format!("{e:#} (run `ollama serve`)")),
    }
    let url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| "postgres://localhost/hermit".into());
    match hermit_store::Store::connect(&url).await {
        Ok(_) => line("postgres", true, format!("{url} (migrations applied)")),
        Err(e) => line(
            "postgres",
            false,
            format!("{e:#} (needed only for `hermit serve`; createdb hermit)"),
        ),
    }
    let _ = Path::new(".");
    Ok(if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
