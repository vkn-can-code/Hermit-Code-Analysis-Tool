//! Prompt construction. Everything the model sees is either code from the repo or a
//! fact produced by the deterministic pipeline.

use crate::{Chunk, ReviewInput};
use hermit_core::{AnalysisReport, FindingState};
use serde_json::{Value, json};
use std::fmt::Write;

const SYSTEM: &str = "You are Hermit, a senior software engineer reviewing a pull request.
Rules:
1. The VERIFIED FACTS section was produced by static analysis (AST parsing, Semgrep, call graph). Treat it as ground truth. Never contradict it and never invent other vulnerabilities, callers or tests.
2. Comment only on lines that appear in the DIFF with a line number. Use the number shown in the left column. Lines marked '-' were removed and cannot be commented on.
3. Only mention functions, variables and files that appear in the diff or the RELATED CODE section.
4. Prefer few, high-value comments: bugs, security, broken callers, missing error handling, missing tests. No style nitpicks. Zero comments is fine for a clean change.
5. `suggestion` must be replacement code for the commented line only (no diff markers), or omitted.
6. `summary`: 2-4 sentences for the reviewer: what the change does, its main risk, and what to check.
7. `tests`: up to 2 unit tests for the changed behaviour, in the project's language and test style. Each array item must be ONE complete test function (all its lines, joined with newlines), never a single line.
Respond with JSON only.";

/// JSON schema for Ollama structured output.
pub(crate) fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "summary": { "type": "string" },
            "comments": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "file": { "type": "string" },
                        "line": { "type": "integer" },
                        "body": { "type": "string" },
                        "suggestion": { "type": "string" }
                    },
                    "required": ["file", "line", "body"]
                }
            },
            "tests": {
                "type": "array",
                "description": "Each item is one complete test function as a multi-line string.",
                "items": { "type": "string" }
            }
        },
        "required": ["summary", "comments", "tests"]
    })
}

pub(crate) fn messages(input: &ReviewInput, context: &[Chunk]) -> Vec<Value> {
    vec![
        json!({ "role": "system", "content": SYSTEM }),
        json!({ "role": "user", "content": user_prompt(input, context) }),
    ]
}

fn local(q: &str) -> &str {
    q.rsplit_once("::").map_or(q, |(_, l)| l)
}

fn user_prompt(input: &ReviewInput, context: &[Chunk]) -> String {
    let r = input.report;
    let mut o = String::new();
    let _ = writeln!(o, "## VERIFIED FACTS");
    let _ = writeln!(o, "Risk score: {}/100 ({:?}).", r.risk.score, r.risk.tier);
    for f in &r.risk.factors {
        let _ = writeln!(o, "- {}: {}", f.name, f.detail);
    }
    let _ = writeln!(o, "\nChanged symbols:");
    for s in &r.symbols {
        let _ = writeln!(
            o,
            "- {} ({:?}, {:?}){}, complexity {} -> {}",
            s.symbol.qualified,
            s.symbol.kind,
            s.change,
            if s.signature_changed {
                ", SIGNATURE CHANGED"
            } else {
                ""
            },
            s.complexity_before,
            s.complexity_after
        );
    }
    if r.findings.is_empty() {
        let _ = writeln!(o, "\nSecurity findings: none.");
    } else {
        let _ = writeln!(o, "\nSecurity findings:");
        for f in &r.findings {
            let state = match f.state {
                FindingState::New => "NEW",
                FindingState::Fixed => "FIXED by this change",
            };
            let _ = writeln!(
                o,
                "- [{state}] {} {:?} at {}:{} ({}) - {}",
                f.cwe.as_deref().unwrap_or(""),
                f.severity,
                f.file.display(),
                f.line,
                f.title,
                f.message
            );
        }
    }
    if !r.impacted.is_empty() {
        let _ = writeln!(o, "\nCallers affected (blast radius):");
        for i in r.impacted.iter().take(20) {
            let _ = writeln!(
                o,
                "- {} at {}:{} (depth {}, via {}){}",
                local(&i.qualified),
                i.file.display(),
                i.line,
                i.depth,
                local(&i.via),
                if i.sensitive { " [sensitive]" } else { "" }
            );
        }
    }
    if !r.new_dependencies.is_empty() {
        let _ = writeln!(o, "\nNew dependencies introduced:");
        for d in &r.new_dependencies {
            let _ = writeln!(o, "- {} now calls {}", local(&d.from), d.to);
        }
    }
    let _ = writeln!(o, "\nTests that reach the change:");
    if r.tests.is_empty() {
        let _ = writeln!(o, "- none");
    }
    for t in r.tests.iter().take(10) {
        let _ = writeln!(o, "- {}", t.qualified);
    }
    if !r.untested.is_empty() {
        let _ = writeln!(
            o,
            "Not reached by any test: {}",
            r.untested
                .iter()
                .map(|u| local(u))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    let _ = writeln!(o, "\n## DIFF\n{}", input.patch);

    if !context.is_empty() {
        let _ = writeln!(o, "## RELATED CODE (read-only context)");
        for c in context {
            let _ = writeln!(o, "### {} ({})\n{}\n", c.qualified, c.why, c.text);
        }
    }
    let _ = writeln!(o, "Review the DIFF now and answer in the JSON format.");
    o
}

/// Deterministic summary used when the model returns none.
pub(crate) fn fallback_summary(r: &AnalysisReport) -> String {
    let fixed = r
        .findings
        .iter()
        .filter(|f| f.state == FindingState::Fixed)
        .count();
    let new = r.findings.len() - fixed;
    format!(
        "This change touches {} symbol(s) in {} file(s), affecting {} caller(s). {} new and {} fixed finding(s). Risk {}/100.",
        r.symbols.len(),
        r.files.len(),
        r.impacted.len(),
        new,
        fixed,
        r.risk.score
    )
}
