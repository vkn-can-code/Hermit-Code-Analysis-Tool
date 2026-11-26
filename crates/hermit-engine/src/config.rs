//! `hermit.toml` configuration.

use anyhow::{Context, Result};
use hermit_risk::RiskWeights;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Path prefixes to skip entirely.
    pub ignore: Vec<String>,
    pub graph: GraphConfig,
    pub security: SecurityConfig,
    pub risk: RiskWeights,
    pub ai: AiConfig,
    pub github: GithubConfig,
}

impl Config {
    pub fn from_toml(s: &str) -> Result<Self> {
        toml::from_str(s).context("invalid hermit.toml")
    }

    /// Environment overrides for deployment (`HERMIT_OLLAMA_URL`, `HERMIT_MODEL`, `HERMIT_AI=0`).
    pub fn with_env(mut self) -> Self {
        if let Ok(v) = std::env::var("HERMIT_OLLAMA_URL") {
            self.ai.ollama_url = v;
        }
        if let Ok(v) = std::env::var("HERMIT_MODEL") {
            self.ai.model = v;
        }
        if std::env::var("HERMIT_AI").is_ok_and(|v| v == "0" || v == "false") {
            self.ai.enabled = false;
        }
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GraphConfig {
    /// Max caller hops for the blast radius.
    pub max_depth: u32,
    /// Max hops when searching for tests that reach a change.
    pub test_depth: u32,
    /// Substrings marking business-critical code.
    pub sensitive: Vec<String>,
}

impl Default for GraphConfig {
    fn default() -> Self {
        Self {
            max_depth: 3,
            test_depth: 6,
            sensitive: [
                "auth",
                "login",
                "password",
                "token",
                "session",
                "payment",
                "billing",
                "checkout",
                "crypto",
                "admin",
                "permission",
            ]
            .map(String::from)
            .to_vec(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SecurityConfig {
    /// Use Semgrep when installed.
    pub semgrep: bool,
    /// Extra Semgrep configs. Registry packs (`p/...`) need network access.
    pub extra_configs: Vec<String>,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            semgrep: true,
            extra_configs: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AiConfig {
    pub enabled: bool,
    pub ollama_url: String,
    pub model: String,
    pub embed_model: String,
    /// Seconds before the AI stage gives up and the deterministic report is used.
    pub timeout_secs: u64,
    /// Max related code chunks retrieved for context.
    pub context_chunks: usize,
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            ollama_url: "http://127.0.0.1:11434".into(),
            model: "qwen2.5-coder:7b".into(),
            embed_model: "nomic-embed-text".into(),
            timeout_secs: 120,
            context_chunks: 6,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GithubConfig {
    /// Check Run fails at or above this tier: "low", "medium", "high", or "never".
    pub fail_on: String,
    /// Post inline review comments.
    pub inline_comments: bool,
}

impl Default for GithubConfig {
    fn default() -> Self {
        Self {
            fail_on: "high".into(),
            inline_comments: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_toml_keeps_defaults() {
        let c = Config::from_toml("[graph]\nmax_depth = 5\n[risk]\nhigh_at = 70\n").unwrap();
        assert_eq!(c.graph.max_depth, 5);
        assert_eq!(c.graph.test_depth, 6);
        assert_eq!(c.risk.high_at, 70);
        assert_eq!(c.risk.critical, 45.0);
        assert!(c.ai.enabled);
    }
}
