//! Local LLM code review through Ollama, anchored on verified analysis facts.
//!
//! 1. **Retrieve**: related code is ranked by embedding similarity (cached) plus
//!    call-graph proximity to the changed symbols.
//! 2. **Prompt**: the model sees only the numbered diff, the deterministic findings,
//!    blast radius, tests, and retrieved context. It is told those facts are ground truth.
//! 3. **Constrain**: the response must match a JSON schema (Ollama structured output).
//! 4. **Validate**: comments on lines outside the diff, on unknown files, or that cite
//!    identifiers that exist nowhere in the code are dropped (anti-hallucination).

mod ollama;
mod prompt;
mod validate;

pub use ollama::Ollama;

use anyhow::Result;
use hermit_core::{AiReview, AnalysisReport};
use hermit_graph::RepoIndex;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

/// Reviewer settings.
#[derive(Debug, Clone)]
pub struct ReviewerConfig {
    pub ollama_url: String,
    pub model: String,
    pub embed_model: String,
    pub timeout: Duration,
    pub context_chunks: usize,
}

/// Embedding cache keyed by `sha256(model + text)`. Callers persist it (file or database).
pub type EmbedCache = HashMap<String, Vec<f32>>;

/// Inputs for one review.
pub struct ReviewInput<'a> {
    pub report: &'a AnalysisReport,
    /// Output of `ChangeSet::numbered_patch`.
    pub patch: &'a str,
    pub index: &'a RepoIndex,
}

/// Raw model output (schema in `prompt::schema`).
#[derive(Debug, Deserialize)]
struct RawReview {
    #[serde(default)]
    summary: String,
    #[serde(default)]
    comments: Vec<RawComment>,
    #[serde(default)]
    tests: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RawComment {
    file: String,
    line: u32,
    #[serde(default)]
    body: String,
    #[serde(default)]
    suggestion: Option<String>,
}

/// Runs retrieval + generation + validation.
pub async fn review(
    cfg: &ReviewerConfig,
    input: &ReviewInput<'_>,
    cache: &mut EmbedCache,
) -> Result<AiReview> {
    let start = Instant::now();
    let client = Ollama::new(&cfg.ollama_url, cfg.timeout)?;
    let fut = async {
        let context = retrieve(&client, cfg, input, cache).await;
        let messages = prompt::messages(input, &context);
        let raw_json = client
            .chat_json(&cfg.model, &messages, prompt::schema())
            .await?;
        let raw: RawReview = serde_json::from_str(&raw_json)
            .map_err(|e| anyhow::anyhow!("model returned invalid JSON: {e}"))?;
        Ok::<_, anyhow::Error>(raw)
    };
    let raw = tokio::time::timeout(cfg.timeout, fut)
        .await
        .map_err(|_| anyhow::anyhow!("AI review timed out after {:?}", cfg.timeout))??;

    let (comments, rejected) = validate::comments(input, raw.comments);
    let summary = if raw.summary.trim().is_empty() {
        prompt::fallback_summary(input.report)
    } else {
        raw.summary.trim().to_string()
    };
    let proposed_tests = normalize_tests(raw.tests);
    Ok(AiReview {
        model: cfg.model.clone(),
        summary,
        comments,
        proposed_tests,
        rejected,
        latency_ms: start.elapsed().as_millis() as u64,
    })
}

/// Small models sometimes return one test split line-by-line across array items;
/// rejoin those, then keep up to 2 non-trivial tests.
fn normalize_tests(tests: Vec<String>) -> Vec<String> {
    let tests: Vec<String> = tests
        .into_iter()
        .map(|t| t.trim_matches('\n').to_string())
        .filter(|t| !t.trim().is_empty())
        .collect();
    let multi_line = tests.iter().any(|t| t.contains('\n'));
    let joined = if !multi_line && tests.len() > 1 {
        vec![tests.join("\n")]
    } else {
        tests
    };
    joined
        .into_iter()
        .filter(|t| t.len() >= 30 && t.lines().count() >= 2)
        .take(2)
        .collect()
}

/// A piece of related code shown to the model.
pub(crate) struct Chunk {
    pub qualified: String,
    pub text: String,
    pub why: &'static str,
}

/// Ranks repo symbols by similarity to the changed code, boosted by call-graph proximity.
async fn retrieve(
    client: &Ollama,
    cfg: &ReviewerConfig,
    input: &ReviewInput<'_>,
    cache: &mut EmbedCache,
) -> Vec<Chunk> {
    let idx = input.index;
    let changed: HashSet<&str> = input
        .report
        .symbols
        .iter()
        .map(|s| s.symbol.qualified.as_str())
        .collect();

    // Graph neighbours: direct callers and callees of changed symbols.
    let mut neighbours: HashSet<String> = HashSet::new();
    for q in &changed {
        for e in idx.callers_of(q) {
            neighbours.insert(e.caller.clone());
        }
        for e in idx.callees_of(q) {
            neighbours.insert(e.callee.clone());
        }
    }
    for d in &input.report.new_dependencies {
        neighbours.insert(d.to.clone());
    }

    let mut candidates: Vec<(&String, String)> = idx
        .symbols
        .iter()
        .filter(|(q, s)| s.kind.is_callable() && !s.is_test && !changed.contains(q.as_str()))
        .filter_map(|(q, _)| idx.symbol_text(q, 60).map(|t| (q, t)))
        .collect();
    candidates.sort_by(|a, b| {
        neighbours
            .contains(b.0)
            .cmp(&neighbours.contains(a.0))
            .then(a.0.cmp(b.0))
    });
    candidates.truncate(400);

    let query: String = input
        .report
        .symbols
        .iter()
        .filter_map(|s| idx.symbol_text(&s.symbol.qualified, 80))
        .collect::<Vec<_>>()
        .join("\n\n");

    let mut scores: Vec<(f32, usize)> = Vec::new();
    match embed_cached(client, &cfg.embed_model, &query, &candidates, cache).await {
        Ok((qv, vecs)) => {
            for (i, v) in vecs.iter().enumerate() {
                let boost = if neighbours.contains(candidates[i].0) {
                    0.25
                } else {
                    0.0
                };
                scores.push((cosine(&qv, v) + boost, i));
            }
        }
        Err(e) => {
            tracing::warn!("embeddings unavailable, using call graph only: {e:#}");
            for (i, c) in candidates.iter().enumerate() {
                if neighbours.contains(c.0) {
                    scores.push((1.0, i));
                }
            }
        }
    }
    scores.sort_by(|a, b| b.0.total_cmp(&a.0));
    scores
        .into_iter()
        .take(cfg.context_chunks)
        .map(|(_, i)| Chunk {
            qualified: candidates[i].0.clone(),
            text: candidates[i].1.clone(),
            why: if neighbours.contains(candidates[i].0) {
                "call-graph neighbour"
            } else {
                "semantically similar"
            },
        })
        .collect()
}

async fn embed_cached(
    client: &Ollama,
    model: &str,
    query: &str,
    candidates: &[(&String, String)],
    cache: &mut EmbedCache,
) -> Result<(Vec<f32>, Vec<Vec<f32>>)> {
    let key = |t: &str| hex::encode(Sha256::digest(format!("{model}\0{t}")));
    let mut missing: Vec<(String, String)> = Vec::new();
    for (_, text) in candidates {
        let k = key(text);
        if !cache.contains_key(&k) && !missing.iter().any(|(mk, _)| *mk == k) {
            missing.push((k, text.clone()));
        }
    }
    for batch in missing.chunks(64) {
        let texts: Vec<&str> = batch.iter().map(|(_, t)| t.as_str()).collect();
        let vecs = client.embed(model, &texts).await?;
        for ((k, _), v) in batch.iter().zip(vecs) {
            cache.insert(k.clone(), v);
        }
    }
    let qv = client
        .embed(model, &[query])
        .await?
        .pop()
        .unwrap_or_default();
    let vecs = candidates
        .iter()
        .map(|(_, t)| cache.get(&key(t)).cloned().unwrap_or_default())
        .collect();
    Ok((qv, vecs))
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let (mut dot, mut na, mut nb) = (0f32, 0f32, 0f32);
    for (x, y) in a.iter().zip(b) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na.sqrt() * nb.sqrt())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn rejoins_split_tests() {
        let t = super::normalize_tests(vec![
            "def test_x():".into(),
            "    assert charge(1) == 10".into(),
        ]);
        assert_eq!(
            t,
            vec!["def test_x():\n    assert charge(1) == 10".to_string()]
        );
    }

    #[test]
    fn cosine_basics() {
        assert!((super::cosine(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-6);
        assert!(super::cosine(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6);
        assert_eq!(super::cosine(&[], &[]), 0.0);
    }
}
