//! HTTP server: GitHub webhook receiver, REST API, live progress (SSE), dashboard
//! hosting, and the background workers that run analyses from the job queue.

mod routes;
mod worker;

use anyhow::{Context, Result};
use hermit_github::App;
use hermit_store::Store;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{Notify, broadcast};

/// Server settings (from environment / CLI flags).
#[derive(Debug, Clone)]
pub struct ServeOptions {
    pub bind: SocketAddr,
    pub database_url: String,
    pub workers: usize,
    pub workspaces: PathBuf,
    /// Built dashboard (`web/out`). Served at `/` when present.
    pub web_dir: Option<PathBuf>,
    /// Allow `POST /api/analyses` on local repository paths.
    pub allow_local: bool,
    pub github_app_id: Option<String>,
    pub github_private_key: Option<PathBuf>,
    pub github_webhook_secret: Option<String>,
}

impl ServeOptions {
    /// Reads `HERMIT_*`, `DATABASE_URL` and `GITHUB_*` environment variables.
    pub fn from_env() -> Result<Self> {
        let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        Ok(Self {
            bind: env("HERMIT_BIND")
                .unwrap_or_else(|| "127.0.0.1:8080".into())
                .parse()
                .context("HERMIT_BIND must be host:port")?,
            database_url: env("DATABASE_URL")
                .unwrap_or_else(|| "postgres://localhost/hermit".into()),
            workers: env("HERMIT_WORKERS")
                .and_then(|w| w.parse().ok())
                .unwrap_or(2),
            workspaces: env("HERMIT_WORKSPACES")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".hermit/workspaces")),
            web_dir: env("HERMIT_WEB_DIR").map(PathBuf::from).or_else(|| {
                let p = PathBuf::from("web/out");
                p.is_dir().then_some(p)
            }),
            allow_local: env("HERMIT_ALLOW_LOCAL").is_none_or(|v| v != "0" && v != "false"),
            github_app_id: env("GITHUB_APP_ID"),
            github_private_key: env("GITHUB_PRIVATE_KEY_PATH").map(PathBuf::from),
            github_webhook_secret: env("GITHUB_WEBHOOK_SECRET"),
        })
    }
}

/// Shared state for handlers and workers.
pub(crate) struct AppState {
    pub store: Store,
    pub github: Option<Arc<App>>,
    pub opts: ServeOptions,
    /// JSON progress events for SSE subscribers.
    pub events: broadcast::Sender<String>,
    /// Wakes idle workers when a job is enqueued.
    pub wake: Notify,
}

/// Starts the server and workers; runs until Ctrl-C.
pub async fn run(opts: ServeOptions) -> Result<()> {
    let store = Store::connect(&opts.database_url).await?;
    let github = match (&opts.github_app_id, &opts.github_private_key) {
        (Some(id), Some(key)) => {
            let pem = std::fs::read_to_string(key)
                .with_context(|| format!("reading {}", key.display()))?;
            Some(Arc::new(App::new(id, &pem)?))
        }
        _ => None,
    };
    std::fs::create_dir_all(&opts.workspaces)?;
    let (events, _) = broadcast::channel(256);
    let state = Arc::new(AppState {
        store,
        github,
        opts: opts.clone(),
        events,
        wake: Notify::new(),
    });

    for n in 0..opts.workers.max(1) {
        tokio::spawn(worker::run(state.clone(), n));
    }

    let app = routes::router(state.clone());
    let listener = tokio::net::TcpListener::bind(opts.bind).await?;
    tracing::info!(
        "Hermit listening on http://{} (github app: {}, workers: {})",
        opts.bind,
        state.github.is_some(),
        opts.workers
    );
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
