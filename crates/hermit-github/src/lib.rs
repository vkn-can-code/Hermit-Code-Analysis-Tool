//! GitHub App integration: webhook verification, installation tokens, PR checkout,
//! Check Runs, the summary comment and inline review comments.

use anyhow::{Context, Result, bail};
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::Sha256;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

const API: &str = "https://api.github.com";
const UA: &str = "hermit-code-analysis";

/// Verifies `X-Hub-Signature-256` (HMAC-SHA256 of the raw body) in constant time.
pub fn verify_signature(secret: &str, body: &[u8], header: Option<&str>) -> bool {
    let Some(sig) = header.and_then(|h| h.strip_prefix("sha256=")) else {
        return false;
    };
    let Ok(expected) = hex::decode(sig) else {
        return false;
    };
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(body);
    mac.verify_slice(&expected).is_ok()
}

/// The parts of a `pull_request` webhook Hermit needs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullRequestEvent {
    pub action: String,
    pub installation_id: i64,
    pub repo: String,
    pub number: i32,
    pub title: String,
    pub author: String,
    pub base_ref: String,
    pub base_sha: String,
    pub head_sha: String,
    pub draft: bool,
}

impl PullRequestEvent {
    /// Parses a webhook payload; `None` for payloads that are not analyzable PR events.
    pub fn parse(payload: &Value) -> Option<Self> {
        let pr = payload.get("pull_request")?;
        Some(Self {
            action: payload["action"].as_str()?.to_string(),
            installation_id: payload["installation"]["id"].as_i64()?,
            repo: payload["repository"]["full_name"].as_str()?.to_string(),
            number: pr["number"].as_i64()? as i32,
            title: pr["title"].as_str().unwrap_or_default().to_string(),
            author: pr["user"]["login"].as_str().unwrap_or_default().to_string(),
            base_ref: pr["base"]["ref"].as_str()?.to_string(),
            base_sha: pr["base"]["sha"].as_str()?.to_string(),
            head_sha: pr["head"]["sha"].as_str()?.to_string(),
            draft: pr["draft"].as_bool().unwrap_or(false),
        })
    }

    /// Whether this action should trigger an analysis.
    pub fn should_analyze(&self) -> bool {
        !self.draft
            && matches!(
                self.action.as_str(),
                "opened" | "synchronize" | "reopened" | "ready_for_review"
            )
    }
}

/// GitHub App credentials.
pub struct App {
    app_id: String,
    key: jsonwebtoken::EncodingKey,
    http: reqwest::Client,
    /// installation id -> (token, expiry unix seconds)
    tokens: Mutex<HashMap<i64, (String, u64)>>,
}

#[derive(Serialize)]
struct Claims {
    iat: u64,
    exp: u64,
    iss: String,
}

/// An inline review comment to post.
#[derive(Debug, Clone)]
pub struct InlineComment {
    pub path: String,
    pub line: u32,
    pub body: String,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl App {
    pub fn new(app_id: &str, private_key_pem: &str) -> Result<Self> {
        let key = jsonwebtoken::EncodingKey::from_rsa_pem(private_key_pem.as_bytes())
            .context("GitHub App private key is not a valid RSA PEM")?;
        Ok(Self {
            app_id: app_id.to_string(),
            key,
            http: reqwest::Client::builder()
                .user_agent(UA)
                .timeout(Duration::from_secs(30))
                .build()?,
            tokens: Mutex::new(HashMap::new()),
        })
    }

    fn jwt(&self) -> Result<String> {
        let t = now();
        let claims = Claims {
            iat: t - 60,
            exp: t + 540,
            iss: self.app_id.clone(),
        };
        Ok(jsonwebtoken::encode(
            &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256),
            &claims,
            &self.key,
        )?)
    }

    /// Installation access token, cached until 5 minutes before expiry.
    pub async fn token(&self, installation: i64) -> Result<String> {
        let mut cache = self.tokens.lock().await;
        if let Some((tok, exp)) = cache.get(&installation)
            && *exp > now() + 300
        {
            return Ok(tok.clone());
        }
        let resp: Value = self
            .http
            .post(format!(
                "{API}/app/installations/{installation}/access_tokens"
            ))
            .bearer_auth(self.jwt()?)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await?
            .error_for_status()
            .context("creating installation token")?
            .json()
            .await?;
        let tok = resp["token"]
            .as_str()
            .context("no token in response")?
            .to_string();
        let exp = resp["expires_at"]
            .as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map_or(now() + 3000, |d| d.timestamp() as u64);
        cache.insert(installation, (tok.clone(), exp));
        Ok(tok)
    }

    async fn call(
        &self,
        installation: i64,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value> {
        let tok = self.token(installation).await?;
        let mut req = self
            .http
            .request(method, format!("{API}{path}"))
            .bearer_auth(tok)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req.send().await?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            bail!(
                "GitHub {path} -> {status}: {}",
                text.chars().take(300).collect::<String>()
            );
        }
        Ok(if text.is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text)?
        })
    }

    /// Creates an in-progress Check Run; returns its id.
    pub async fn start_check(&self, inst: i64, repo: &str, head_sha: &str) -> Result<i64> {
        let v = self
            .call(
                inst,
                reqwest::Method::POST,
                &format!("/repos/{repo}/check-runs"),
                Some(json!({
                    "name": "Hermit",
                    "head_sha": head_sha,
                    "status": "in_progress",
                    "output": { "title": "Analyzing…", "summary": "Hermit is analyzing this pull request." }
                })),
            )
            .await?;
        v["id"].as_i64().context("check run id")
    }

    /// Completes a Check Run. `conclusion`: success | neutral | failure.
    pub async fn finish_check(
        &self,
        inst: i64,
        repo: &str,
        check_id: i64,
        conclusion: &str,
        title: &str,
        summary: &str,
    ) -> Result<()> {
        let summary: String = summary.chars().take(65_000).collect();
        self.call(
            inst,
            reqwest::Method::PATCH,
            &format!("/repos/{repo}/check-runs/{check_id}"),
            Some(json!({
                "status": "completed",
                "conclusion": conclusion,
                "output": { "title": title, "summary": summary }
            })),
        )
        .await?;
        Ok(())
    }

    /// Creates or updates Hermit's summary comment (found by `marker`).
    pub async fn upsert_comment(
        &self,
        inst: i64,
        repo: &str,
        pr: i32,
        marker: &str,
        body: &str,
    ) -> Result<()> {
        let comments = self
            .call(
                inst,
                reqwest::Method::GET,
                &format!("/repos/{repo}/issues/{pr}/comments?per_page=100"),
                None,
            )
            .await?;
        let existing = comments.as_array().and_then(|a| {
            a.iter()
                .find(|c| c["body"].as_str().is_some_and(|b| b.contains(marker)))
                .and_then(|c| c["id"].as_i64())
        });
        match existing {
            Some(id) => {
                self.call(
                    inst,
                    reqwest::Method::PATCH,
                    &format!("/repos/{repo}/issues/comments/{id}"),
                    Some(json!({ "body": body })),
                )
                .await?
            }
            None => {
                self.call(
                    inst,
                    reqwest::Method::POST,
                    &format!("/repos/{repo}/issues/{pr}/comments"),
                    Some(json!({ "body": body })),
                )
                .await?
            }
        };
        Ok(())
    }

    /// Posts one review containing all inline comments.
    pub async fn post_review(
        &self,
        inst: i64,
        repo: &str,
        pr: i32,
        head_sha: &str,
        comments: &[InlineComment],
    ) -> Result<()> {
        if comments.is_empty() {
            return Ok(());
        }
        let items: Vec<Value> = comments
            .iter()
            .map(|c| json!({ "path": c.path, "line": c.line, "side": "RIGHT", "body": c.body }))
            .collect();
        self.call(
            inst,
            reqwest::Method::POST,
            &format!("/repos/{repo}/pulls/{pr}/reviews"),
            Some(json!({
                "commit_id": head_sha,
                "event": "COMMENT",
                "body": "Hermit AI review (comments verified against the diff).",
                "comments": items
            })),
        )
        .await?;
        Ok(())
    }

    /// Fetches the PR's base and head into a local mirror under `workspaces`.
    ///
    /// The token is passed only on the command line for this fetch and is never written
    /// to the repository config. Returns the repository path.
    pub async fn checkout(
        &self,
        inst: i64,
        workspaces: &Path,
        repo: &str,
        base_ref: &str,
        pr: i32,
    ) -> Result<PathBuf> {
        let dir = workspaces.join(repo.replace('/', "__"));
        if !dir.join(".git").exists() && !dir.join("HEAD").exists() {
            std::fs::create_dir_all(&dir)?;
            git(&dir, &["init", "-q", "--bare"]).await?;
        }
        let tok = self.token(inst).await?;
        let url = format!("https://x-access-token:{tok}@github.com/{repo}.git");
        let base_spec = format!("+refs/heads/{base_ref}:refs/heads/{base_ref}");
        let pr_spec = format!("+refs/pull/{pr}/head:refs/hermit/pr-{pr}");
        git(
            &dir,
            &["fetch", "-q", "--no-tags", &url, &base_spec, &pr_spec],
        )
        .await
        .context("git fetch failed")?;
        Ok(dir)
    }
}

async fn git(dir: &Path, args: &[&str]) -> Result<()> {
    let out = tokio::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .await?;
    if !out.status.success() {
        // stderr may echo the remote URL; strip credentials before surfacing it.
        let err = String::from_utf8_lossy(&out.stderr);
        let clean = regex_lite_strip_token(&err);
        bail!(
            "git {} failed: {}",
            args.first().unwrap_or(&""),
            clean.trim()
        );
    }
    Ok(())
}

fn regex_lite_strip_token(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("x-access-token:") {
        out.push_str(&rest[..i]);
        out.push_str("x-access-token:***");
        rest = &rest[i + "x-access-token:".len()..];
        rest = rest.find('@').map_or("", |j| &rest[j..]);
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature() {
        let body = br#"{"zen":"hi"}"#;
        let mut mac = Hmac::<Sha256>::new_from_slice(b"s3cret").unwrap();
        mac.update(body);
        let sig = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));
        assert!(verify_signature("s3cret", body, Some(&sig)));
        assert!(!verify_signature("wrong", body, Some(&sig)));
        assert!(!verify_signature("s3cret", b"tampered", Some(&sig)));
        assert!(!verify_signature("s3cret", body, None));
    }

    #[test]
    fn parses_pr_event() {
        let v = json!({
            "action": "synchronize",
            "installation": {"id": 7},
            "repository": {"full_name": "me/app"},
            "pull_request": {
                "number": 142, "title": "Fix SQLi", "draft": false, "user": {"login": "dev"},
                "base": {"ref": "main", "sha": "aaa"}, "head": {"sha": "bbb"}
            }
        });
        let e = PullRequestEvent::parse(&v).unwrap();
        assert_eq!(
            (e.number, e.repo.as_str(), e.head_sha.as_str()),
            (142, "me/app", "bbb")
        );
        assert!(e.should_analyze());
    }

    #[test]
    fn strips_token() {
        let s = "fatal: unable to access 'https://x-access-token:ghs_SECRET@github.com/a/b.git/'";
        let c = regex_lite_strip_token(s);
        assert!(!c.contains("ghs_SECRET"));
        assert!(c.contains("x-access-token:***@github.com"));
    }
}
