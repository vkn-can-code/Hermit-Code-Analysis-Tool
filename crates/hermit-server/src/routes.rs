//! HTTP routes.

use crate::AppState;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use futures::Stream;
use hermit_github::PullRequestEvent;
use hermit_store::NewAnalysis;
use serde::Deserialize;
use serde_json::{Value, json};
use std::convert::Infallible;
use std::sync::Arc;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::BroadcastStream;
use tower_http::services::{ServeDir, ServeFile};

type S = State<Arc<AppState>>;

pub(crate) fn router(state: Arc<AppState>) -> Router {
    let mut r = Router::new()
        .route("/webhooks/github", post(webhook))
        .route("/api/health", get(health))
        .route("/api/repos", get(repos))
        .route("/api/analyses", get(list).post(create_local))
        .route("/api/analyses/{id}", get(detail))
        .route("/api/events", get(events));
    if let Some(dir) = &state.opts.web_dir {
        let index = dir.join("index.html");
        r = r.fallback_service(ServeDir::new(dir).fallback(ServeFile::new(index)));
    }
    r.layer(tower_http::trace::TraceLayer::new_for_http())
        .with_state(state)
}

/// JSON error response.
fn err(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(json!({ "error": msg.into() }))).into_response()
}

fn internal(e: anyhow::Error) -> Response {
    tracing::error!("{e:#}");
    err(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}"))
}

async fn webhook(State(st): S, headers: HeaderMap, body: Bytes) -> Response {
    let Some(secret) = &st.opts.github_webhook_secret else {
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            "GITHUB_WEBHOOK_SECRET not configured",
        );
    };
    let sig = headers
        .get("x-hub-signature-256")
        .and_then(|v| v.to_str().ok());
    if !hermit_github::verify_signature(secret, &body, sig) {
        return err(StatusCode::UNAUTHORIZED, "invalid signature");
    }
    let kind = headers
        .get("x-github-event")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    if kind == "ping" {
        return Json(json!({ "ok": true, "pong": true })).into_response();
    }
    if kind != "pull_request" {
        return (StatusCode::ACCEPTED, Json(json!({ "ignored": kind }))).into_response();
    }
    if st.github.is_none() {
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            "GitHub App credentials not configured (GITHUB_APP_ID, GITHUB_PRIVATE_KEY_PATH)",
        );
    }
    let payload: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => return err(StatusCode::BAD_REQUEST, e.to_string()),
    };
    let Some(ev) = PullRequestEvent::parse(&payload) else {
        return err(StatusCode::BAD_REQUEST, "unrecognised pull_request payload");
    };
    if !ev.should_analyze() {
        return (StatusCode::ACCEPTED, Json(json!({ "ignored": ev.action }))).into_response();
    }
    let new = NewAnalysis {
        repo: &ev.repo,
        installation_id: Some(ev.installation_id),
        pr_number: Some(ev.number),
        title: &ev.title,
        author: &ev.author,
        base_ref: &ev.base_ref,
        head_ref: &format!("refs/hermit/pr-{}", ev.number),
        head_sha: &ev.head_sha,
    };
    let payload = json!({ "kind": "github", "event": ev });
    match st.store.enqueue(&new, payload).await {
        Ok(id) => {
            st.wake.notify_one();
            let _ = st
                .events
                .send(json!({ "id": id, "status": "queued", "stage": "queued" }).to_string());
            (StatusCode::ACCEPTED, Json(json!({ "analysis_id": id }))).into_response()
        }
        Err(e) => internal(e),
    }
}

#[derive(Deserialize)]
struct LocalReq {
    repo_path: String,
    #[serde(default = "default_base")]
    base: String,
    #[serde(default = "default_head")]
    head: String,
    #[serde(default)]
    title: String,
}
fn default_base() -> String {
    "main".into()
}
fn default_head() -> String {
    "HEAD".into()
}

/// Queue an analysis of a repository on this machine (dashboard "Analyze" form).
async fn create_local(State(st): S, Json(req): Json<LocalReq>) -> Response {
    if !st.opts.allow_local {
        return err(
            StatusCode::FORBIDDEN,
            "local analyses disabled (HERMIT_ALLOW_LOCAL=0)",
        );
    }
    let path = match std::fs::canonicalize(&req.repo_path) {
        Ok(p) => p,
        Err(_) => return err(StatusCode::BAD_REQUEST, "repository path does not exist"),
    };
    // Validate revisions before queueing so the user gets an immediate error.
    let (p, b, h) = (path.clone(), req.base.clone(), req.head.clone());
    let check =
        tokio::task::spawn_blocking(move || hermit_diff::ChangeSet::open(&p, &b, &h).map(|_| ()))
            .await;
    if let Ok(Err(e)) | Err(e) = check.map_err(anyhow::Error::from) {
        return err(StatusCode::BAD_REQUEST, format!("{e:#}"));
    }
    let repo_name = format!("local:{}", path.display());
    let title = if req.title.is_empty() {
        format!("{}..{}", req.base, req.head)
    } else {
        req.title.clone()
    };
    let new = NewAnalysis {
        repo: &repo_name,
        installation_id: None,
        pr_number: None,
        title: &title,
        author: "local",
        base_ref: &req.base,
        head_ref: &req.head,
        head_sha: "",
    };
    let payload = json!({ "kind": "local", "repo_path": path, "base": req.base, "head": req.head });
    match st.store.enqueue(&new, payload).await {
        Ok(id) => {
            st.wake.notify_one();
            let _ = st
                .events
                .send(json!({ "id": id, "status": "queued", "stage": "queued" }).to_string());
            (StatusCode::ACCEPTED, Json(json!({ "analysis_id": id }))).into_response()
        }
        Err(e) => internal(e),
    }
}

#[derive(Deserialize)]
struct ListQ {
    repo: Option<String>,
    limit: Option<i64>,
}

async fn list(State(st): S, Query(q): Query<ListQ>) -> Response {
    match st
        .store
        .list(q.repo.as_deref(), q.limit.unwrap_or(50).clamp(1, 500))
        .await
    {
        Ok(rows) => Json(rows).into_response(),
        Err(e) => internal(e),
    }
}

async fn detail(State(st): S, Path(id): Path<i64>) -> Response {
    match st.store.get(id).await {
        Ok(Some((row, report))) => {
            let markdown = report
                .as_ref()
                .and_then(|r| serde_json::from_value::<hermit_core::AnalysisReport>(r.clone()).ok())
                .map(|r| hermit_engine::report::markdown(&r));
            Json(json!({ "analysis": row, "report": report, "markdown": markdown })).into_response()
        }
        Ok(None) => err(StatusCode::NOT_FOUND, "no such analysis"),
        Err(e) => internal(e),
    }
}

async fn repos(State(st): S) -> Response {
    match st.store.repos().await {
        Ok(rows) => Json(
            rows.into_iter()
                .map(|(name, n, score)| json!({ "name": name, "analyses": n, "last_score": score }))
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(e) => internal(e),
    }
}

async fn health(State(st): S) -> Response {
    let db = sqlx_ping(&st).await;
    let cfg = hermit_engine::Config::default();
    let ollama = match hermit_ai::Ollama::new(&cfg.ai.ollama_url, std::time::Duration::from_secs(2))
    {
        Ok(c) => c.models().await.ok(),
        Err(_) => None,
    };
    let semgrep = which("semgrep");
    Json(json!({
        "ok": db,
        "database": db,
        "ollama": ollama.is_some(),
        "models": ollama.unwrap_or_default(),
        "semgrep": semgrep,
        "github_app": st.github.is_some(),
        "webhook_secret": st.opts.github_webhook_secret.is_some(),
        "allow_local": st.opts.allow_local,
        "version": env!("CARGO_PKG_VERSION"),
    }))
    .into_response()
}

async fn sqlx_ping(st: &AppState) -> bool {
    st.store.list(None, 1).await.is_ok()
}

fn which(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(bin).is_file()))
        .unwrap_or(false)
}

async fn events(State(st): S) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let stream = BroadcastStream::new(st.events.subscribe())
        .filter_map(|m| m.ok())
        .map(|m| Ok(Event::default().data(m)));
    Sse::new(stream).keep_alive(KeepAlive::default())
}
