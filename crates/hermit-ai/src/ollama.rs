//! Minimal Ollama HTTP client (`/api/chat`, `/api/embed`, `/api/tags`).

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Duration;

pub struct Ollama {
    base: String,
    http: reqwest::Client,
}

#[derive(Deserialize)]
struct ChatResponse {
    message: ChatMessage,
}

#[derive(Deserialize)]
struct ChatMessage {
    content: String,
}

#[derive(Deserialize)]
struct EmbedResponse {
    embeddings: Vec<Vec<f32>>,
}

#[derive(Deserialize)]
struct Tags {
    models: Vec<TagModel>,
}

#[derive(Deserialize)]
struct TagModel {
    name: String,
}

impl Ollama {
    pub fn new(base: &str, timeout: Duration) -> Result<Self> {
        Ok(Self {
            base: base.trim_end_matches('/').to_string(),
            http: reqwest::Client::builder().timeout(timeout).build()?,
        })
    }

    /// Installed model names.
    pub async fn models(&self) -> Result<Vec<String>> {
        let tags: Tags = self
            .http
            .get(format!("{}/api/tags", self.base))
            .send()
            .await
            .context("Ollama not reachable")?
            .error_for_status()?
            .json()
            .await?;
        Ok(tags.models.into_iter().map(|m| m.name).collect())
    }

    /// Chat completion constrained to `schema` (JSON Schema). Returns the raw JSON text.
    pub async fn chat_json(
        &self,
        model: &str,
        messages: &[Value],
        schema: Value,
    ) -> Result<String> {
        let body = json!({
            "model": model,
            "messages": messages,
            "stream": false,
            "format": schema,
            "options": { "temperature": 0.1, "num_ctx": 16384 },
            "keep_alive": "30m",
        });
        let resp = self
            .http
            .post(format!("{}/api/chat", self.base))
            .json(&body)
            .send()
            .await
            .context("Ollama not reachable")?;
        if !resp.status().is_success() {
            bail!(
                "Ollama chat failed: {} {}",
                resp.status(),
                resp.text().await.unwrap_or_default()
            );
        }
        let r: ChatResponse = resp.json().await?;
        Ok(r.message.content)
    }

    /// Embeds a batch of texts.
    pub async fn embed(&self, model: &str, input: &[&str]) -> Result<Vec<Vec<f32>>> {
        let resp = self
            .http
            .post(format!("{}/api/embed", self.base))
            .json(&json!({ "model": model, "input": input, "truncate": true }))
            .send()
            .await
            .context("Ollama not reachable")?;
        if !resp.status().is_success() {
            bail!(
                "Ollama embed failed: {} {}",
                resp.status(),
                resp.text().await.unwrap_or_default()
            );
        }
        let r: EmbedResponse = resp.json().await?;
        if r.embeddings.len() != input.len() {
            bail!(
                "Ollama returned {} embeddings for {} inputs",
                r.embeddings.len(),
                input.len()
            );
        }
        Ok(r.embeddings)
    }
}
