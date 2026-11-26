//! Background workers: claim jobs, run the pipeline, publish results.

use crate::AppState;
use anyhow::{Context, Result};
use hermit_core::{AnalysisReport, RiskTier};
use hermit_github::{InlineComment, PullRequestEvent};
use hermit_store::Job;
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum Payload {
    Github {
        event: PullRequestEvent,
    },
    Local {
        repo_path: PathBuf,
        base: String,
        head: String,
    },
}

pub(crate) async fn run(st: Arc<AppState>, n: usize) {
    tracing::info!("worker {n} started");
    loop {
        match st.store.claim().await {
            Ok(Some(job)) => {
                let id = job.analysis_id;
                tracing::info!("worker {n}: analysis {id} (attempt {})", job.attempts);
                match process(&st, &job).await {
                    Ok(()) => {
                        let _ = st.store.complete_job(&job).await;
                    }
                    Err(e) => {
                        let msg = format!("{e:#}");
                        tracing::warn!("analysis {id} failed: {msg}");
                        let retry = st.store.fail_job(&job, &msg).await.unwrap_or(false);
                        if retry {
                            emit(&st, id, "queued", "retrying", None);
                        } else {
                            let _ = st.store.mark_failed(id, &msg).await;
                            emit(&st, id, "failed", "failed", None);
                        }
                    }
                }
            }
            Ok(None) => {
                // Idle: wait for a wake-up or poll again shortly (covers retries / other processes).
                let _ = tokio::time::timeout(Duration::from_secs(2), st.wake.notified()).await;
            }
            Err(e) => {
                tracing::error!("worker {n}: queue error: {e:#}");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
}

fn emit(st: &AppState, id: i64, status: &str, stage: &str, score: Option<u32>) {
    let _ = st.events.send(
        json!({ "id": id, "status": status, "stage": stage, "risk_score": score }).to_string(),
    );
}

async fn stage(st: &AppState, id: i64, stage: &str) {
    let _ = st.store.set_stage(id, "running", stage).await;
    emit(st, id, "running", stage, None);
}

async fn process(st: &Arc<AppState>, job: &Job) -> Result<()> {
    let id = job.analysis_id;
    let payload: Payload =
        serde_json::from_value(job.payload.clone()).context("bad job payload")?;

    // Resolve repository + revisions.
    let mut check: Option<(i64, i64)> = None; // (installation, check run id)
    let (repo_path, base, head, name, gh) = match &payload {
        Payload::Local {
            repo_path,
            base,
            head,
        } => (repo_path.clone(), base.clone(), head.clone(), None, None),
        Payload::Github { event } => {
            let app = st.github.clone().context("GitHub App not configured")?;
            stage(st, id, "fetch").await;
            match app
                .start_check(event.installation_id, &event.repo, &event.head_sha)
                .await
            {
                Ok(c) => check = Some((event.installation_id, c)),
                Err(e) => tracing::warn!("could not create check run: {e:#}"),
            }
            let path = app
                .checkout(
                    event.installation_id,
                    &st.opts.workspaces,
                    &event.repo,
                    &event.base_ref,
                    event.number,
                )
                .await?;
            (
                path,
                event.base_sha.clone(),
                event.head_sha.clone(),
                Some(event.repo.clone()),
                Some((app, event.clone())),
            )
        }
    };

    let result = analyze(st, id, repo_path, base, head, name).await;

    let (report, cfg) = match result {
        Ok(r) => r,
        Err(e) => {
            if let (Some((app, ev)), Some((inst, cid))) = (&gh, check) {
                let _ = app
                    .finish_check(
                        inst,
                        &ev.repo,
                        cid,
                        "neutral",
                        "Hermit could not analyze this PR",
                        &format!("{e:#}"),
                    )
                    .await;
            }
            return Err(e);
        }
    };

    st.store.save_report(id, &report).await?;
    emit(st, id, "done", "done", Some(report.risk.score));

    if let Some((app, ev)) = gh {
        stage(st, id, "publish").await;
        if let Err(e) = publish(&cfg, &app, &ev, check, &report).await {
            tracing::warn!("publishing to GitHub failed: {e:#}");
        }
        let _ = st.store.set_stage(id, "done", "done").await;
        emit(st, id, "done", "done", Some(report.risk.score));
    }
    Ok(())
}

async fn analyze(
    st: &Arc<AppState>,
    id: i64,
    repo: PathBuf,
    base: String,
    head: String,
    name: Option<String>,
) -> Result<(AnalysisReport, hermit_engine::Config)> {
    let cfg = {
        let (r, h) = (repo.clone(), head.clone());
        tokio::task::spawn_blocking(move || hermit_engine::load_config_at(&r, &h)).await??
    };
    // Stage updates flow through one channel so they are applied in order.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let st2 = st.clone();
    let updater = tokio::spawn(async move {
        while let Some(s) = rx.recv().await {
            stage(&st2, id, &s).await;
        }
    });
    let cfg2 = cfg.clone();
    let blocking = tokio::task::spawn_blocking(move || {
        let progress = |s: &str| {
            let _ = tx.send(s.to_string());
        };
        hermit_engine::analyze_with_progress(
            &hermit_engine::Request {
                repo: &repo,
                base: &base,
                head: &head,
                name,
            },
            &cfg2,
            &progress,
        )
    });
    let analysis = blocking.await;
    let _ = updater.await; // sender dropped with the closure: drains remaining stages
    let mut analysis = analysis??;

    if cfg.ai.enabled {
        stage(st, id, "ai").await;
        let mut cache = st.store.load_embeddings().await.unwrap_or_default();
        let before: std::collections::HashSet<String> = cache.keys().cloned().collect();
        match hermit_engine::ai_review(&mut analysis, &cfg, &mut cache).await {
            Ok(()) => {}
            Err(e) => tracing::warn!("AI review skipped: {e:#}"),
        }
        let fresh: HashMap<String, Vec<f32>> = cache
            .into_iter()
            .filter(|(k, _)| !before.contains(k))
            .collect();
        if !fresh.is_empty() {
            let _ = st.store.save_embeddings(&fresh).await;
        }
    }
    Ok((analysis.report, cfg))
}

async fn publish(
    cfg: &hermit_engine::Config,
    app: &hermit_github::App,
    ev: &PullRequestEvent,
    check: Option<(i64, i64)>,
    report: &AnalysisReport,
) -> Result<()> {
    let summary = hermit_engine::report::markdown_summary(report);
    app.upsert_comment(
        ev.installation_id,
        &ev.repo,
        ev.number,
        hermit_engine::report::COMMENT_MARKER,
        &summary,
    )
    .await?;

    if cfg.github.inline_comments
        && let Some(ai) = &report.ai
    {
        let comments: Vec<InlineComment> = ai
            .comments
            .iter()
            .map(|c| InlineComment {
                path: c.file.to_string_lossy().replace('\\', "/"),
                line: c.line,
                body: match &c.suggestion {
                    Some(s) => format!(
                        "{}\n\n```suggestion\n{}\n```\n<sub>Hermit AI · {}</sub>",
                        c.body, s, ai.model
                    ),
                    None => format!("{}\n\n<sub>Hermit AI · {}</sub>", c.body, ai.model),
                },
            })
            .collect();
        app.post_review(
            ev.installation_id,
            &ev.repo,
            ev.number,
            &ev.head_sha,
            &comments,
        )
        .await?;
    }

    if let Some((inst, cid)) = check {
        let fail = match cfg.github.fail_on.as_str() {
            "low" => true,
            "medium" => report.risk.tier != RiskTier::Low,
            "high" => report.risk.tier == RiskTier::High,
            _ => false,
        };
        let title = format!(
            "Risk {} ({}/100)",
            hermit_engine::report::tier_label(report.risk.tier),
            report.risk.score
        );
        app.finish_check(
            inst,
            &ev.repo,
            cid,
            if fail { "failure" } else { "success" },
            &title,
            &summary,
        )
        .await?;
    }
    Ok(())
}
