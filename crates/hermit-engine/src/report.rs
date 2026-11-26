//! Markdown rendering of an [`AnalysisReport`] (CLI output and GitHub PR comment).

use hermit_core::{AnalysisReport, ChangeKind, FindingState, RiskTier, Severity};
use std::fmt::Write;

/// Hidden marker so the GitHub integration can update its own comment.
pub const COMMENT_MARKER: &str = "<!-- hermit-report -->";

const MAX_ROWS: usize = 15;

pub fn tier_label(t: RiskTier) -> &'static str {
    match t {
        RiskTier::Low => "LOW",
        RiskTier::Medium => "MEDIUM",
        RiskTier::High => "HIGH",
    }
}

fn tier_icon(t: RiskTier) -> &'static str {
    match t {
        RiskTier::Low => "🟢",
        RiskTier::Medium => "🟠",
        RiskTier::High => "🔴",
    }
}

fn sev_icon(s: Severity) -> &'static str {
    match s {
        Severity::Critical => "🛑",
        Severity::High => "🔴",
        Severity::Medium => "🟠",
        Severity::Low => "🟡",
        Severity::Info => "ℹ️",
    }
}

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(8)]
}

/// Strips the `file::` prefix from a qualified name.
fn local(q: &str) -> &str {
    q.rsplit_once("::").map_or(q, |(_, l)| l)
}

/// Escapes table cell content.
fn cell(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ")
}

/// Renders the full report as GitHub-flavoured markdown.
pub fn markdown(r: &AnalysisReport) -> String {
    render(r, true)
}

/// Same report without the AI inline comments (posted separately as a PR review).
pub fn markdown_summary(r: &AnalysisReport) -> String {
    render(r, false)
}

fn render(r: &AnalysisReport, ai_comments: bool) -> String {
    let mut o = String::new();
    let _ = writeln!(o, "{COMMENT_MARKER}");
    let _ = writeln!(
        o,
        "## {} Hermit risk: **{} ({} / 100)**\n",
        tier_icon(r.risk.tier),
        tier_label(r.risk.tier),
        r.risk.score
    );
    let adds: u32 = r.files.iter().map(|f| f.additions).sum();
    let dels: u32 = r.files.iter().map(|f| f.deletions).sum();
    let new_findings = r
        .findings
        .iter()
        .filter(|f| f.state == FindingState::New)
        .count();
    let fixed = r.findings.len() - new_findings;
    let _ = writeln!(
        o,
        "`{}..{}` · {} file(s) · +{adds} / -{dels} · {} symbol(s) changed · {} impacted caller(s) · {} new / {} fixed finding(s) · {} test(s) selected\n",
        short(&r.base),
        short(&r.head),
        r.files.len(),
        r.symbols.len(),
        r.impacted.len(),
        new_findings,
        fixed,
        r.tests.len()
    );

    if let Some(ai) = &r.ai {
        let _ = writeln!(o, "### Summary\n\n{}\n", ai.summary.trim());
    }

    // Risk factors
    let _ = writeln!(
        o,
        "### Risk breakdown\n\n| Factor | Points | Evidence |\n|---|---:|---|"
    );
    for f in &r.risk.factors {
        let _ = writeln!(
            o,
            "| {} | {:.1} / {:.0} | {} |",
            f.name,
            f.points,
            f.max,
            cell(&f.detail)
        );
    }
    o.push('\n');

    // Findings
    if !r.findings.is_empty() {
        let _ = writeln!(
            o,
            "### Security & static findings\n\n| | Finding | Location | CWE | Status |\n|---|---|---|---|---|"
        );
        for f in r.findings.iter().take(MAX_ROWS * 2) {
            let status = match f.state {
                FindingState::New => "new",
                FindingState::Fixed => "✅ fixed",
            };
            let _ = writeln!(
                o,
                "| {} | **{}**: {} | `{}:{}` | {} | {} |",
                sev_icon(f.severity),
                cell(&f.title),
                cell(&f.message),
                f.file.display(),
                f.line,
                f.cwe.as_deref().unwrap_or("-"),
                status
            );
        }
        o.push('\n');
    }

    // Changed symbols
    if !r.symbols.is_empty() {
        let _ = writeln!(
            o,
            "<details><summary><b>Changed symbols ({})</b></summary>\n\n| Symbol | Change | Complexity | Signature |\n|---|---|---|---|",
            r.symbols.len()
        );
        for s in &r.symbols {
            let change = match s.change {
                ChangeKind::Added => "added",
                ChangeKind::Modified => "modified",
                ChangeKind::Deleted => "deleted",
            };
            let _ = writeln!(
                o,
                "| `{}` ({}) | {} | {} → {} | {} |",
                s.symbol.qualified,
                format!("{:?}", s.symbol.kind).to_lowercase(),
                change,
                s.complexity_before,
                s.complexity_after,
                if s.signature_changed {
                    "⚠️ changed"
                } else {
                    "-"
                }
            );
        }
        let _ = writeln!(o, "\n</details>\n");
    }

    // Blast radius
    if !r.impacted.is_empty() {
        let _ = writeln!(
            o,
            "### Blast radius\n\n| Caller | Depth | Via | |\n|---|---:|---|---|"
        );
        for i in r.impacted.iter().take(MAX_ROWS) {
            let _ = writeln!(
                o,
                "| `{}` (`{}:{}`) | {} | `{}` | {} |",
                local(&i.qualified),
                i.file.display(),
                i.line,
                i.depth,
                local(&i.via),
                if i.sensitive { "🔐 sensitive" } else { "" }
            );
        }
        if r.impacted.len() > MAX_ROWS {
            let _ = writeln!(o, "\n…and {} more.", r.impacted.len() - MAX_ROWS);
        }
        o.push('\n');
    }

    // New dependencies
    if !r.new_dependencies.is_empty() {
        let _ = writeln!(o, "### New dependencies\n");
        for d in &r.new_dependencies {
            let _ = writeln!(
                o,
                "- `{}` now calls `{}` (`{}:{}`): its behaviour and failure modes now affect this code path.",
                local(&d.from),
                d.to,
                d.file.display(),
                d.line
            );
        }
        o.push('\n');
    }

    // AI inline comments (on GitHub these are posted inline instead)
    if let Some(ai) = &r.ai
        && ai_comments
        && !ai.comments.is_empty()
    {
        let _ = writeln!(o, "### AI review comments\n");
        for c in &ai.comments {
            let _ = writeln!(
                o,
                "- **`{}:{}`** — {}",
                c.file.display(),
                c.line,
                c.body.trim()
            );
            if let Some(s) = &c.suggestion {
                let _ = writeln!(o, "  ```suggestion\n{}\n  ```", s);
            }
        }
        o.push('\n');
    }

    // Tests
    let _ = writeln!(o, "### Recommended regression tests\n");
    if r.tests.is_empty() {
        let _ = writeln!(o, "No existing test reaches the changed code.\n");
    } else {
        let _ = writeln!(o, "```sh");
        for t in r.tests.iter().take(MAX_ROWS) {
            let _ = writeln!(o, "{}", t.command);
        }
        let _ = writeln!(o, "```\n");
    }
    if !r.untested.is_empty() {
        let _ = writeln!(
            o,
            "**Coverage gaps:** no test reaches {}.\n",
            r.untested
                .iter()
                .map(|u| format!("`{}`", local(u)))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    if let Some(ai) = &r.ai
        && !ai.proposed_tests.is_empty()
    {
        let _ = writeln!(o, "<details><summary><b>AI-proposed tests</b></summary>\n");
        for t in &ai.proposed_tests {
            let _ = writeln!(o, "```\n{}\n```\n", t.trim());
        }
        let _ = writeln!(o, "</details>\n");
    }

    let t = &r.timings;
    let ai_note =
        r.ai.as_ref()
            .map(|a| {
                format!(
                    " · AI `{}` {} ms ({} unverifiable comment(s) dropped)",
                    a.model, a.latency_ms, a.rejected
                )
            })
            .unwrap_or_default();
    let _ = writeln!(
        o,
        "<sub>Hermit · analysis {} ms (diff {} · parse {} · security {} · graph {} · tests {}){ai_note} · runs fully on-premise</sub>",
        t.total_ms, t.diff_ms, t.parse_ms, t.security_ms, t.graph_ms, t.tests_ms
    );
    o
}
