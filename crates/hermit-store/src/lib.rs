//! PostgreSQL persistence: repositories, analyses, a durable job queue and the
//! embedding cache. Queries are runtime-checked (no DATABASE_URL needed to build).

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use hermit_core::AnalysisReport;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use sqlx::postgres::{PgPool, PgPoolOptions};
use std::collections::HashMap;

/// Max attempts before a job is marked failed.
/// Shared column list for analysis rows (a literal, so queries stay static SQL).
macro_rules! row_select {
    ($extra:literal) => {
        concat!(
            "SELECT a.id, r.full_name, a.pr_number, a.title, a.author, a.base_ref, a.head_ref, ",
            "a.head_sha, a.status, a.stage, a.risk_score, a.risk_tier, a.error, a.created_at, a.finished_at",
            $extra,
            " FROM analyses a JOIN repositories r ON r.id = a.repo_id "
        )
    };
}

pub const MAX_ATTEMPTS: i32 = 3;
/// Running jobs older than this are considered abandoned (worker crashed) and retried.
const STALE_MINUTES: i32 = 15;

#[derive(Clone)]
pub struct Store {
    pub pool: PgPool,
}

/// Row summary for listings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisRow {
    pub id: i64,
    pub repo: String,
    pub pr_number: Option<i32>,
    pub title: String,
    pub author: String,
    pub base_ref: String,
    pub head_ref: String,
    pub head_sha: String,
    pub status: String,
    pub stage: String,
    pub risk_score: Option<i32>,
    pub risk_tier: Option<String>,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

/// New analysis request.
#[derive(Debug, Clone)]
pub struct NewAnalysis<'a> {
    pub repo: &'a str,
    pub installation_id: Option<i64>,
    pub pr_number: Option<i32>,
    pub title: &'a str,
    pub author: &'a str,
    pub base_ref: &'a str,
    pub head_ref: &'a str,
    pub head_sha: &'a str,
}

/// A claimed job.
#[derive(Debug, Clone)]
pub struct Job {
    pub id: i64,
    pub analysis_id: i64,
    pub payload: serde_json::Value,
    pub attempts: i32,
}

impl Store {
    /// Connects and runs migrations.
    pub async fn connect(url: &str) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .connect(url)
            .await
            .with_context(|| "connecting to PostgreSQL (check DATABASE_URL)")?;
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .context("running migrations")?;
        Ok(Self { pool })
    }

    /// Creates the analysis row and its job in one transaction.
    pub async fn enqueue(&self, a: &NewAnalysis<'_>, payload: serde_json::Value) -> Result<i64> {
        let mut tx = self.pool.begin().await?;
        let repo_id: i64 = sqlx::query(
            "INSERT INTO repositories (full_name, installation_id) VALUES ($1, $2)
             ON CONFLICT (full_name) DO UPDATE SET installation_id = COALESCE(EXCLUDED.installation_id, repositories.installation_id)
             RETURNING id",
        )
        .bind(a.repo)
        .bind(a.installation_id)
        .fetch_one(&mut *tx)
        .await?
        .get(0);
        let analysis_id: i64 = sqlx::query(
            "INSERT INTO analyses (repo_id, pr_number, title, author, base_ref, head_ref, head_sha)
             VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING id",
        )
        .bind(repo_id)
        .bind(a.pr_number)
        .bind(a.title)
        .bind(a.author)
        .bind(a.base_ref)
        .bind(a.head_ref)
        .bind(a.head_sha)
        .fetch_one(&mut *tx)
        .await?
        .get(0);
        sqlx::query("INSERT INTO jobs (analysis_id, payload) VALUES ($1, $2)")
            .bind(analysis_id)
            .bind(&payload)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(analysis_id)
    }

    /// Claims the next ready job (or a stale one), or `None` if the queue is empty.
    pub async fn claim(&self) -> Result<Option<Job>> {
        let row = sqlx::query(
            "UPDATE jobs SET status = 'running', locked_at = now(), attempts = attempts + 1
             WHERE id = (
                SELECT id FROM jobs
                WHERE (status = 'queued' AND run_after <= now())
                   OR (status = 'running' AND locked_at < now() - make_interval(mins => $1))
                ORDER BY id
                FOR UPDATE SKIP LOCKED
                LIMIT 1)
             RETURNING id, analysis_id, payload, attempts",
        )
        .bind(STALE_MINUTES)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| Job {
            id: r.get(0),
            analysis_id: r.get(1),
            payload: r.get(2),
            attempts: r.get(3),
        }))
    }

    pub async fn complete_job(&self, job: &Job) -> Result<()> {
        sqlx::query("UPDATE jobs SET status = 'done', locked_at = NULL WHERE id = $1")
            .bind(job.id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Requeues with exponential backoff, or marks failed after [`MAX_ATTEMPTS`].
    /// Returns true if the job will be retried.
    pub async fn fail_job(&self, job: &Job, err: &str) -> Result<bool> {
        let retry = job.attempts < MAX_ATTEMPTS;
        sqlx::query(
            "UPDATE jobs SET status = $2, last_error = $3, locked_at = NULL,
                    run_after = now() + make_interval(secs => $4)
             WHERE id = $1",
        )
        .bind(job.id)
        .bind(if retry { "queued" } else { "failed" })
        .bind(err)
        .bind(f64::from(10 * 2_i32.pow(job.attempts as u32)))
        .execute(&self.pool)
        .await?;
        Ok(retry)
    }

    pub async fn set_stage(&self, id: i64, status: &str, stage: &str) -> Result<()> {
        sqlx::query("UPDATE analyses SET status = $2, stage = $3 WHERE id = $1")
            .bind(id)
            .bind(status)
            .bind(stage)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn save_report(&self, id: i64, report: &AnalysisReport) -> Result<()> {
        let tier = serde_json::to_value(report.risk.tier)?
            .as_str()
            .unwrap_or_default()
            .to_string();
        sqlx::query(
            "UPDATE analyses SET status = 'done', stage = 'done', risk_score = $2, risk_tier = $3,
                    report = $4, head_sha = CASE WHEN head_sha = '' THEN $5 ELSE head_sha END,
                    error = NULL, finished_at = now()
             WHERE id = $1",
        )
        .bind(id)
        .bind(report.risk.score as i32)
        .bind(tier)
        .bind(serde_json::to_value(report)?)
        .bind(&report.head)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn mark_failed(&self, id: i64, err: &str) -> Result<()> {
        sqlx::query(
            "UPDATE analyses SET status = 'failed', stage = 'failed', error = $2, finished_at = now() WHERE id = $1",
        )
        .bind(id)
        .bind(err)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    fn row(r: &sqlx::postgres::PgRow) -> AnalysisRow {
        AnalysisRow {
            id: r.get(0),
            repo: r.get(1),
            pr_number: r.get(2),
            title: r.get(3),
            author: r.get(4),
            base_ref: r.get(5),
            head_ref: r.get(6),
            head_sha: r.get(7),
            status: r.get(8),
            stage: r.get(9),
            risk_score: r.get(10),
            risk_tier: r.get(11),
            error: r.get(12),
            created_at: r.get(13),
            finished_at: r.get(14),
        }
    }

    pub async fn list(&self, repo: Option<&str>, limit: i64) -> Result<Vec<AnalysisRow>> {
        let rows = sqlx::query(concat!(
            row_select!(""),
            "WHERE ($1::text IS NULL OR r.full_name = $1) ORDER BY a.id DESC LIMIT $2"
        ))
        .bind(repo)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(Self::row).collect())
    }

    /// Row plus full report JSON.
    pub async fn get(&self, id: i64) -> Result<Option<(AnalysisRow, Option<serde_json::Value>)>> {
        let r = sqlx::query(concat!(row_select!(", a.report"), "WHERE a.id = $1"))
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(r.map(|r| {
            let report: Option<serde_json::Value> = r.try_get("report").ok().flatten();
            (Self::row(&r), report)
        }))
    }

    pub async fn repos(&self) -> Result<Vec<(String, i64, Option<i32>)>> {
        let rows = sqlx::query(
            "SELECT r.full_name, count(a.id),
                    (SELECT risk_score FROM analyses WHERE repo_id = r.id AND status = 'done' ORDER BY id DESC LIMIT 1)
             FROM repositories r LEFT JOIN analyses a ON a.repo_id = r.id
             GROUP BY r.id ORDER BY max(a.id) DESC NULLS LAST",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .iter()
            .map(|r| (r.get(0), r.get(1), r.get(2)))
            .collect())
    }

    /// Loads cached embeddings (whole table; it is small: one row per code chunk).
    pub async fn load_embeddings(&self) -> Result<HashMap<String, Vec<f32>>> {
        let rows = sqlx::query("SELECT key, vec FROM embeddings")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.iter().map(|r| (r.get(0), r.get(1))).collect())
    }

    /// Persists new cache entries.
    pub async fn save_embeddings(&self, items: &HashMap<String, Vec<f32>>) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        for (k, v) in items {
            sqlx::query(
                "INSERT INTO embeddings (key, vec) VALUES ($1, $2) ON CONFLICT (key) DO NOTHING",
            )
            .bind(k)
            .bind(v)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }
}
