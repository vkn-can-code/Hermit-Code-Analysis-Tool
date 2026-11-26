//! Shared domain types for every stage of the Hermit analysis pipeline.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Source languages Hermit can parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Python,
    JavaScript,
    TypeScript,
    Tsx,
    Rust,
}

impl Language {
    /// Detects the language from a file extension.
    pub fn from_path(path: &std::path::Path) -> Option<Self> {
        match path.extension()?.to_str()? {
            "py" => Some(Self::Python),
            "js" | "mjs" | "cjs" | "jsx" => Some(Self::JavaScript),
            "ts" | "mts" | "cts" => Some(Self::TypeScript),
            "tsx" => Some(Self::Tsx),
            "rs" => Some(Self::Rust),
            _ => None,
        }
    }
}

/// How a file changed between base and head.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
}

/// Inclusive 1-based line range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LineRange {
    pub start: u32,
    pub end: u32,
}

impl LineRange {
    pub fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }

    pub fn contains(&self, line: u32) -> bool {
        line >= self.start && line <= self.end
    }

    pub fn overlaps(&self, other: &LineRange) -> bool {
        self.start <= other.end && other.start <= self.end
    }

    pub fn len(&self) -> u32 {
        self.end.saturating_sub(self.start) + 1
    }

    pub fn is_empty(&self) -> bool {
        self.end < self.start
    }
}

/// One file touched by the change set.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangedFile {
    /// Path at head (or at base for deleted files), relative to repo root.
    pub path: PathBuf,
    /// Previous path when the file was renamed.
    pub old_path: Option<PathBuf>,
    pub status: FileStatus,
    pub language: Option<Language>,
    /// Lines added at head (head line numbers).
    pub added: Vec<LineRange>,
    /// Lines removed at base (base line numbers).
    pub removed: Vec<LineRange>,
    pub additions: u32,
    pub deletions: u32,
}

impl ChangedFile {
    /// True if any added head line falls in `range`.
    pub fn touches_head(&self, range: &LineRange) -> bool {
        self.added.iter().any(|r| r.overlaps(range))
    }

    /// True if any removed base line falls in `range`.
    pub fn touches_base(&self, range: &LineRange) -> bool {
        self.removed.iter().any(|r| r.overlaps(range))
    }

    /// Path of this file at base (old path for renames).
    pub fn base_path(&self) -> &std::path::Path {
        self.old_path.as_deref().unwrap_or(&self.path)
    }
}

/// Kind of code entity found by the parser.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SymbolKind {
    Function,
    Method,
    Class,
    Struct,
    Enum,
    Trait,
    Impl,
    Module,
}

impl SymbolKind {
    /// Functions and methods: things that have a body and can be called.
    pub fn is_callable(self) -> bool {
        matches!(self, Self::Function | Self::Method)
    }
}

/// A named code entity with its location.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Symbol {
    /// Short name, e.g. `get_user_subscription`.
    pub name: String,
    /// Qualified name, e.g. `services/billing.py::BillingService.get_user_subscription`.
    pub qualified: String,
    pub kind: SymbolKind,
    pub file: PathBuf,
    pub range: LineRange,
    /// Normalised signature text (name + parameters), used for signature-change detection.
    pub signature: String,
    /// McCabe cyclomatic complexity of the body (1 for non-callables).
    pub complexity: u32,
    /// True if this symbol is a test case.
    pub is_test: bool,
}

/// How a symbol changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
}

/// A symbol touched by the diff, with before/after metrics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangedSymbol {
    pub symbol: Symbol,
    pub change: ChangeKind,
    pub signature_changed: bool,
    pub complexity_before: u32,
    pub complexity_after: u32,
}

impl ChangedSymbol {
    pub fn complexity_delta(&self) -> i64 {
        i64::from(self.complexity_after) - i64::from(self.complexity_before)
    }
}

/// Severity of a finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Low,
    Medium,
    High,
    Critical,
}

/// Whether a finding is new in this change or was removed by it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FindingState {
    /// Introduced or still present on changed head lines.
    New,
    /// Present at base, gone at head.
    Fixed,
}

/// A static-analysis or security result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub rule_id: String,
    pub title: String,
    pub message: String,
    pub severity: Severity,
    pub cwe: Option<String>,
    pub file: PathBuf,
    pub line: u32,
    pub end_line: u32,
    pub snippet: String,
    pub state: FindingState,
    /// `semgrep` or `builtin`.
    pub engine: String,
}

/// Caller reached from a changed symbol in the call graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpactedSymbol {
    pub qualified: String,
    pub file: PathBuf,
    pub line: u32,
    /// Hops from the nearest changed symbol (1 = direct caller).
    pub depth: u32,
    /// Changed symbol this impact traces back to.
    pub via: String,
    /// Path matched a configured sensitive area (auth, payment, ...).
    pub sensitive: bool,
}

/// A call edge that exists at head but not at base: new coupling introduced by the change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewDependency {
    /// Changed symbol that now makes the call.
    pub from: String,
    /// Qualified callee (resolved in the repo index).
    pub to: String,
    pub file: PathBuf,
    pub line: u32,
}

/// A recommended regression test.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestCandidate {
    pub qualified: String,
    pub file: PathBuf,
    pub line: u32,
    /// Call-graph distance to the nearest changed symbol.
    pub distance: u32,
    pub covers: Vec<String>,
    /// Shell command that runs just this test.
    pub command: String,
}

/// Risk tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RiskTier {
    Low,
    Medium,
    High,
}

/// One explainable contribution to the risk score.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskFactor {
    pub name: String,
    pub points: f64,
    pub max: f64,
    pub detail: String,
}

/// Final 0-100 risk assessment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskReport {
    pub score: u32,
    pub tier: RiskTier,
    pub factors: Vec<RiskFactor>,
}

/// Inline comment proposed by the AI reviewer (already validated against the diff).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewComment {
    pub file: PathBuf,
    pub line: u32,
    pub body: String,
    pub suggestion: Option<String>,
}

/// AI-generated part of the report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiReview {
    pub model: String,
    pub summary: String,
    pub comments: Vec<ReviewComment>,
    pub proposed_tests: Vec<String>,
    /// Comments the validator rejected (hallucinated location etc.).
    pub rejected: u32,
    pub latency_ms: u64,
}

/// Wall-clock time spent per pipeline stage.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Timings {
    pub diff_ms: u64,
    pub parse_ms: u64,
    pub security_ms: u64,
    pub graph_ms: u64,
    pub tests_ms: u64,
    pub risk_ms: u64,
    pub ai_ms: u64,
    pub total_ms: u64,
}

/// Complete analysis output for one base..head change set.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisReport {
    pub repo: String,
    pub base: String,
    pub head: String,
    pub files: Vec<ChangedFile>,
    pub symbols: Vec<ChangedSymbol>,
    pub findings: Vec<Finding>,
    pub impacted: Vec<ImpactedSymbol>,
    pub new_dependencies: Vec<NewDependency>,
    pub tests: Vec<TestCandidate>,
    /// Changed callables that no discovered test reaches.
    pub untested: Vec<String>,
    pub risk: RiskReport,
    pub ai: Option<AiReview>,
    pub timings: Timings,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn detects_languages() {
        assert_eq!(
            Language::from_path(Path::new("a/b.py")),
            Some(Language::Python)
        );
        assert_eq!(Language::from_path(Path::new("x.tsx")), Some(Language::Tsx));
        assert_eq!(
            Language::from_path(Path::new("lib.rs")),
            Some(Language::Rust)
        );
        assert_eq!(Language::from_path(Path::new("README.md")), None);
    }

    #[test]
    fn range_overlap() {
        let a = LineRange::new(10, 20);
        assert!(a.overlaps(&LineRange::new(20, 25)));
        assert!(!a.overlaps(&LineRange::new(21, 25)));
        assert!(a.contains(10) && a.contains(20) && !a.contains(21));
        assert_eq!(a.len(), 11);
    }
}
