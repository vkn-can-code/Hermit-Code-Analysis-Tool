//! Regex rules that need no external tool.
//!
//! Secret detection always runs (Semgrep's OSS rules do not cover it well).
//! The injection rules are a coarse fallback used only when Semgrep is absent.

use hermit_core::{Finding, FindingState, Language, Severity};
use regex::Regex;
use std::path::Path;
use std::sync::LazyLock;

struct Rule {
    id: &'static str,
    title: &'static str,
    message: &'static str,
    cwe: &'static str,
    severity: Severity,
    langs: &'static [Language],
    re: Regex,
}

const ALL: &[Language] = &[
    Language::Python,
    Language::JavaScript,
    Language::TypeScript,
    Language::Tsx,
    Language::Rust,
];
const PY: &[Language] = &[Language::Python];
const JS: &[Language] = &[Language::JavaScript, Language::TypeScript, Language::Tsx];

fn rule(
    id: &'static str,
    title: &'static str,
    message: &'static str,
    cwe: &'static str,
    severity: Severity,
    langs: &'static [Language],
    re: &str,
) -> Rule {
    Rule {
        id,
        title,
        message,
        cwe,
        severity,
        langs,
        re: Regex::new(re).expect("valid builtin regex"),
    }
}

static SECRETS: LazyLock<Vec<Rule>> = LazyLock::new(|| {
    vec![
        rule(
            "hermit.secret.aws-access-key",
            "Hard-coded AWS access key",
            "AWS access key ID committed to source. Revoke it and load credentials from the environment.",
            "CWE-798",
            Severity::Critical,
            ALL,
            r"\b(AKIA|ASIA)[0-9A-Z]{16}\b",
        ),
        rule(
            "hermit.secret.private-key",
            "Hard-coded private key",
            "Private key material committed to source.",
            "CWE-798",
            Severity::Critical,
            ALL,
            r"-----BEGIN (RSA |EC |OPENSSH |DSA )?PRIVATE KEY-----",
        ),
        rule(
            "hermit.secret.github-token",
            "Hard-coded GitHub token",
            "GitHub token committed to source. Revoke it immediately.",
            "CWE-798",
            Severity::Critical,
            ALL,
            r"\b(ghp|gho|ghs|ghu|github_pat)_[A-Za-z0-9_]{30,}\b",
        ),
        rule(
            "hermit.secret.generic-credential",
            "Hard-coded credential",
            "Password/secret literal assigned in code. Load it from configuration or a secret store.",
            "CWE-798",
            Severity::High,
            ALL,
            r#"(?i)\b(password|passwd|secret|api_?key|access_?token|auth_?token)\b\s*[:=]\s*["'][^"'\s$\{]{8,}["']"#,
        ),
    ]
});

static FALLBACK: LazyLock<Vec<Rule>> = LazyLock::new(|| {
    vec![
        rule(
            "hermit.builtin.python-sql-fstring",
            "SQL injection",
            "SQL built with an f-string / concatenation. Use bound parameters.",
            "CWE-89",
            Severity::Critical,
            PY,
            r#"(?i)(f"(SELECT|INSERT|UPDATE|DELETE)\b[^"]*\{|f'(SELECT|INSERT|UPDATE|DELETE)\b[^']*\{|"(SELECT|INSERT|UPDATE|DELETE)\b[^"]*"\s*(\+|%)|'(SELECT|INSERT|UPDATE|DELETE)\b[^']*'\s*(\+|%))"#,
        ),
        rule(
            "hermit.builtin.js-sql-template",
            "SQL injection",
            "SQL built with template interpolation / concatenation. Use placeholders.",
            "CWE-89",
            Severity::Critical,
            JS,
            r#"(?i)(`(SELECT|INSERT|UPDATE|DELETE)\b[^`]*\$\{|"(SELECT|INSERT|UPDATE|DELETE)\b[^"]*"\s*\+|'(SELECT|INSERT|UPDATE|DELETE)\b[^']*'\s*\+)"#,
        ),
        rule(
            "hermit.builtin.python-shell",
            "OS command injection",
            "Shell execution of a runtime string.",
            "CWE-78",
            Severity::Critical,
            PY,
            r"(shell\s*=\s*True|os\.system\(\s*[^)\x22\x27])",
        ),
        rule(
            "hermit.builtin.eval",
            "Code injection",
            "eval() on dynamic input executes arbitrary code.",
            "CWE-95",
            Severity::High,
            &[
                Language::Python,
                Language::JavaScript,
                Language::TypeScript,
                Language::Tsx,
            ],
            r#"\beval\(\s*[^)"'\s]"#,
        ),
        rule(
            "hermit.builtin.pickle",
            "Unsafe deserialization",
            "pickle can execute code when loading untrusted data.",
            "CWE-502",
            Severity::High,
            PY,
            r"\bpickle\.loads?\(",
        ),
        rule(
            "hermit.builtin.inner-html",
            "Cross-site scripting",
            "Dynamic content assigned to innerHTML.",
            "CWE-79",
            Severity::High,
            JS,
            r#"\.(inner|outer)HTML\s*=\s*[^"'\s;]"#,
        ),
        rule(
            "hermit.builtin.weak-hash",
            "Weak cryptographic hash",
            "MD5/SHA-1 are broken for security use.",
            "CWE-327",
            Severity::Medium,
            &[
                Language::Python,
                Language::JavaScript,
                Language::TypeScript,
                Language::Tsx,
            ],
            r#"(hashlib\.(md5|sha1)\(|createHash\(\s*["'](md5|sha1)["'])"#,
        ),
        rule(
            "hermit.builtin.rust-unsafe",
            "Unsafe code",
            "New unsafe block.",
            "CWE-119",
            Severity::Low,
            &[Language::Rust],
            r"\bunsafe\s*\{",
        ),
    ]
});

/// Scans one file. `fallback` enables the injection rules (Semgrep absent).
pub(crate) fn scan(path: &Path, text: &str, fallback: bool) -> Vec<Finding> {
    let Some(lang) = Language::from_path(path) else {
        return Vec::new();
    };
    let mut rules: Vec<&Rule> = SECRETS.iter().collect();
    if fallback {
        rules.extend(FALLBACK.iter());
    }
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') || trimmed.starts_with("//") {
            continue;
        }
        for r in rules.iter().filter(|r| r.langs.contains(&lang)) {
            if r.re.is_match(line) {
                let n = i as u32 + 1;
                out.push(Finding {
                    rule_id: r.id.into(),
                    title: r.title.into(),
                    message: r.message.into(),
                    severity: r.severity,
                    cwe: Some(r.cwe.into()),
                    file: path.to_path_buf(),
                    line: n,
                    end_line: n,
                    snippet: line.trim().chars().take(200).collect(),
                    state: FindingState::New,
                    engine: "builtin".into(),
                });
            }
        }
    }
    out
}
