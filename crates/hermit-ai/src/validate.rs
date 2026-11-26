//! Rejects model output that cannot be verified against the code.

use crate::{RawComment, ReviewInput};
use hermit_core::{ChangedFile, ReviewComment};
use regex::Regex;
use std::path::Path;
use std::sync::LazyLock;

/// Max distance (lines) a comment may be snapped to the nearest added line.
const SNAP: u32 = 3;

static BACKTICK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"`([^`\n]{1,80})`").unwrap());
static IDENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*(?:(?:\.|::)[A-Za-z_][A-Za-z0-9_]*)*(?:\(\))?$").unwrap()
});

/// Returns accepted comments and the number rejected.
pub(crate) fn comments(input: &ReviewInput, raw: Vec<RawComment>) -> (Vec<ReviewComment>, u32) {
    let mut out: Vec<ReviewComment> = Vec::new();
    let mut rejected = 0;
    for c in raw {
        match check(input, &c) {
            Some((file, line)) if !out.iter().any(|o| o.file == file.path && o.line == line) => {
                out.push(ReviewComment {
                    file: file.path.clone(),
                    line,
                    body: c.body.trim().to_string(),
                    suggestion: c
                        .suggestion
                        .map(|s| s.trim_matches('\n').to_string())
                        .filter(|s| !s.trim().is_empty() && s.lines().count() <= 15),
                });
            }
            _ => rejected += 1,
        }
    }
    (out, rejected)
}

fn check<'a>(input: &'a ReviewInput, c: &RawComment) -> Option<(&'a ChangedFile, u32)> {
    if c.body.trim().len() < 10 {
        return None;
    }
    let wanted = c
        .file
        .trim()
        .trim_start_matches("./")
        .trim_start_matches("a/")
        .trim_start_matches("b/");
    let file = input
        .report
        .files
        .iter()
        .find(|f| f.path == Path::new(wanted))?;
    let line = snap(file, c.line)?;
    if !identifiers_exist(input, file, &c.body) {
        return None;
    }
    Some((file, line))
}

/// Keeps lines on added code; moves near-misses to the closest added line.
fn snap(file: &ChangedFile, line: u32) -> Option<u32> {
    if file.added.iter().any(|r| r.contains(line)) {
        return Some(line);
    }
    file.added
        .iter()
        .flat_map(|r| [r.start, r.end])
        .map(|l| (l.abs_diff(line), l))
        .filter(|(d, _)| *d <= SNAP)
        .min()
        .map(|(_, l)| l)
}

/// Every backticked identifier must appear in the diff, the file, or the symbol index.
fn identifiers_exist(input: &ReviewInput, file: &ChangedFile, body: &str) -> bool {
    let file_text = input
        .index
        .texts
        .get(&file.path)
        .map(String::as_str)
        .unwrap_or("");
    for cap in BACKTICK.captures_iter(body) {
        let token = cap[1].trim();
        if !IDENT.is_match(token) {
            continue; // code fragments, literals: not checked
        }
        let name = token
            .trim_end_matches("()")
            .rsplit(['.', ':'])
            .next()
            .unwrap_or(token);
        if name.len() < 3 || is_common(name) {
            continue;
        }
        let known = input.patch.contains(name)
            || file_text.contains(name)
            || input.index.symbols.values().any(|s| s.name == name);
        if !known {
            tracing::debug!("rejecting comment citing unknown identifier `{name}`");
            return false;
        }
    }
    true
}

/// Language keywords / builtins the model may mention without them being in the repo.
fn is_common(name: &str) -> bool {
    const COMMON: &[&str] = &[
        "None",
        "True",
        "False",
        "null",
        "undefined",
        "self",
        "this",
        "async",
        "await",
        "return",
        "try",
        "except",
        "catch",
        "finally",
        "raise",
        "throw",
        "unwrap",
        "expect",
        "Result",
        "Option",
        "Some",
        "Err",
        "str",
        "int",
        "dict",
        "list",
        "len",
        "print",
        "console",
        "log",
        "Promise",
        "string",
        "number",
        "boolean",
        "const",
        "let",
        "var",
        "def",
        "class",
        "with",
        "pytest",
        "assert",
        "mock",
        "patch",
        "Exception",
        "Error",
        "TypeError",
        "ValueError",
        "KeyError",
        "fetchone",
        "execute",
        "commit",
        "rollback",
        "logging",
        "logger",
    ];
    COMMON.contains(&name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hermit_core::{AnalysisReport, FileStatus, LineRange, RiskReport, RiskTier, Timings};
    use hermit_graph::{RepoIndex, SourceFile};
    use std::path::PathBuf;

    fn report(files: Vec<ChangedFile>) -> AnalysisReport {
        AnalysisReport {
            repo: "r".into(),
            base: "a".into(),
            head: "b".into(),
            files,
            symbols: vec![],
            findings: vec![],
            impacted: vec![],
            new_dependencies: vec![],
            tests: vec![],
            untested: vec![],
            risk: RiskReport {
                score: 0,
                tier: RiskTier::Low,
                factors: vec![],
            },
            ai: None,
            timings: Timings::default(),
        }
    }

    fn raw(file: &str, line: u32, body: &str) -> RawComment {
        RawComment {
            file: file.into(),
            line,
            body: body.into(),
            suggestion: None,
        }
    }

    #[test]
    fn rejects_hallucinations() {
        let p = PathBuf::from("app/billing.py");
        let file = ChangedFile {
            path: p.clone(),
            old_path: None,
            status: FileStatus::Modified,
            language: None,
            added: vec![LineRange::new(10, 12)],
            removed: vec![],
            additions: 3,
            deletions: 0,
        };
        let r = report(vec![file]);
        let idx = RepoIndex::build(vec![SourceFile {
            path: p,
            language: hermit_core::Language::Python,
            text: "def charge(org):\n    invalidate_cache(org)\n".into(),
        }]);
        let input = ReviewInput {
            report: &r,
            patch: "   10 | + invalidate_cache(org)",
            index: &idx,
        };
        let (ok, rejected) = comments(
            &input,
            vec![
                raw(
                    "app/billing.py",
                    11,
                    "Handle errors raised by `invalidate_cache` here.",
                ),
                raw(
                    "app/billing.py",
                    14,
                    "Near miss is snapped to line 12 of the change.",
                ),
                raw(
                    "app/billing.py",
                    40,
                    "Far away from the diff, must be rejected.",
                ),
                raw(
                    "app/other.py",
                    10,
                    "File not in the change, must be rejected.",
                ),
                raw(
                    "app/billing.py",
                    10,
                    "Call `refresh_tokens_v2()` before charging.",
                ),
            ],
        );
        assert_eq!(ok.len(), 2, "{ok:?}");
        assert_eq!(ok[1].line, 12);
        assert_eq!(rejected, 3);
    }
}
