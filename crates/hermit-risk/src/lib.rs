//! Explainable 0-100 risk score.
//!
//! The score is a sum of capped factors; each factor reports its points and the
//! evidence behind them so reviewers can see *why* a change is risky.

use hermit_core::{
    ChangedFile, ChangedSymbol, Finding, FindingState, ImpactedSymbol, RiskFactor, RiskReport,
    RiskTier, Severity,
};
use serde::{Deserialize, Serialize};

/// Weights and caps. Loaded from `[risk]` in `hermit.toml`; defaults below.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RiskWeights {
    pub critical: f64,
    pub high: f64,
    pub medium: f64,
    pub low: f64,
    pub security_cap: f64,
    /// Points per log2(1 + impacted callers).
    pub blast_per_log2: f64,
    /// Extra points when a sensitive area (auth, payment, ...) is impacted.
    pub sensitive_bonus: f64,
    pub blast_cap: f64,
    /// Points per changed signature / deleted symbol that still has callers.
    pub breaking_per_symbol: f64,
    pub breaking_cap: f64,
    /// Points per unit of net cyclomatic complexity added.
    pub complexity_per_point: f64,
    pub complexity_cap: f64,
    /// Changed lines that earn the full size cap.
    pub size_full_lines: f64,
    pub size_cap: f64,
    pub test_gap_cap: f64,
    /// Score at or above which the tier is Medium / High.
    pub medium_at: u32,
    pub high_at: u32,
}

impl Default for RiskWeights {
    fn default() -> Self {
        Self {
            critical: 45.0,
            high: 25.0,
            medium: 10.0,
            low: 3.0,
            security_cap: 60.0,
            blast_per_log2: 6.0,
            sensitive_bonus: 5.0,
            blast_cap: 20.0,
            breaking_per_symbol: 20.0,
            breaking_cap: 20.0,
            complexity_per_point: 1.5,
            complexity_cap: 15.0,
            size_full_lines: 500.0,
            size_cap: 10.0,
            test_gap_cap: 15.0,
            medium_at: 30,
            high_at: 60,
        }
    }
}

/// Everything the score is computed from.
pub struct RiskInput<'a> {
    pub files: &'a [ChangedFile],
    pub symbols: &'a [ChangedSymbol],
    pub findings: &'a [Finding],
    pub impacted: &'a [ImpactedSymbol],
    pub untested: &'a [String],
}

/// Computes the risk report.
pub fn assess(input: &RiskInput, w: &RiskWeights) -> RiskReport {
    let mut factors = Vec::new();

    // Security: only findings this change introduces.
    let new: Vec<&Finding> = input
        .findings
        .iter()
        .filter(|f| f.state == FindingState::New)
        .collect();
    let fixed = input.findings.len() - new.len();
    let sec_raw: f64 = new
        .iter()
        .map(|f| match f.severity {
            Severity::Critical => w.critical,
            Severity::High => w.high,
            Severity::Medium => w.medium,
            Severity::Low => w.low,
            Severity::Info => 0.0,
        })
        .sum();
    let count = |s: Severity| new.iter().filter(|f| f.severity == s).count();
    factors.push(RiskFactor {
        name: "security".into(),
        points: sec_raw.min(w.security_cap),
        max: w.security_cap,
        detail: format!(
            "{} new finding(s): {} critical, {} high, {} medium, {} low; {} fixed",
            new.len(),
            count(Severity::Critical),
            count(Severity::High),
            count(Severity::Medium),
            count(Severity::Low),
            fixed
        ),
    });

    // Blast radius.
    let callers = input.impacted.len();
    let sensitive: Vec<&ImpactedSymbol> = input.impacted.iter().filter(|i| i.sensitive).collect();
    let direct = input.impacted.iter().filter(|i| i.depth == 1).count();
    let blast_raw = w.blast_per_log2 * (1.0 + callers as f64).log2()
        + if sensitive.is_empty() {
            0.0
        } else {
            w.sensitive_bonus
        };
    let mut detail = format!("{callers} impacted caller(s), {direct} direct");
    if !sensitive.is_empty() {
        detail.push_str(&format!("; {} in sensitive areas", sensitive.len()));
    }
    factors.push(RiskFactor {
        name: "blast_radius".into(),
        points: blast_raw.min(w.blast_cap),
        max: w.blast_cap,
        detail,
    });

    // Breaking changes: signature changed or symbol deleted while callers still exist.
    let breaking: Vec<&ChangedSymbol> = input
        .symbols
        .iter()
        .filter(|s| s.symbol.kind.is_callable() && !s.symbol.is_test)
        .filter(|s| s.signature_changed || s.change == hermit_core::ChangeKind::Deleted)
        .filter(|s| {
            input
                .impacted
                .iter()
                .any(|i| i.via == s.symbol.qualified && i.depth == 1)
        })
        .collect();
    factors.push(RiskFactor {
        name: "breaking_change".into(),
        points: (w.breaking_per_symbol * breaking.len() as f64).min(w.breaking_cap),
        max: w.breaking_cap,
        detail: if breaking.is_empty() {
            "no signature change or deletion with live callers".into()
        } else {
            format!(
                "{} changed signature(s)/deletion(s) with live callers: {}",
                breaking.len(),
                breaking
                    .iter()
                    .map(|s| s.symbol.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        },
    });

    // Complexity: net added decision points across changed callables.
    let added: i64 = input
        .symbols
        .iter()
        .filter(|s| s.symbol.kind.is_callable())
        .map(|s| s.complexity_delta().max(0))
        .sum();
    let peak = input
        .symbols
        .iter()
        .map(|s| s.complexity_after)
        .max()
        .unwrap_or(0);
    factors.push(RiskFactor {
        name: "complexity".into(),
        points: (added as f64 * w.complexity_per_point).min(w.complexity_cap),
        max: w.complexity_cap,
        detail: format!("+{added} cyclomatic complexity; most complex changed function = {peak}"),
    });

    // Size.
    let lines: u32 = input.files.iter().map(|f| f.additions + f.deletions).sum();
    factors.push(RiskFactor {
        name: "size".into(),
        points: (lines as f64 / w.size_full_lines * w.size_cap).min(w.size_cap),
        max: w.size_cap,
        detail: format!("{lines} changed line(s) in {} file(s)", input.files.len()),
    });

    // Test gap.
    let callables = input
        .symbols
        .iter()
        .filter(|s| s.symbol.kind.is_callable() && !s.symbol.is_test)
        .filter(|s| s.change != hermit_core::ChangeKind::Deleted)
        .count();
    let gap = if callables == 0 {
        0.0
    } else {
        input.untested.len() as f64 / callables as f64
    };
    factors.push(RiskFactor {
        name: "test_gap".into(),
        points: (gap * w.test_gap_cap).min(w.test_gap_cap),
        max: w.test_gap_cap,
        detail: format!(
            "{} of {} changed function(s) not reached by any test",
            input.untested.len(),
            callables
        ),
    });

    let total: f64 = factors.iter().map(|f| f.points).sum();
    let score = total.round().clamp(0.0, 100.0) as u32;
    let tier = if score >= w.high_at {
        RiskTier::High
    } else if score >= w.medium_at {
        RiskTier::Medium
    } else {
        RiskTier::Low
    };
    for f in &mut factors {
        f.points = (f.points * 10.0).round() / 10.0 + 0.0; // + 0.0 normalises -0.0
    }
    RiskReport {
        score,
        tier,
        factors,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn finding(sev: Severity, state: FindingState) -> Finding {
        Finding {
            rule_id: "r".into(),
            title: "t".into(),
            message: "m".into(),
            severity: sev,
            cwe: None,
            file: PathBuf::from("a.py"),
            line: 1,
            end_line: 1,
            snippet: String::new(),
            state,
            engine: "builtin".into(),
        }
    }

    #[test]
    fn empty_change_is_low() {
        let r = assess(
            &RiskInput {
                files: &[],
                symbols: &[],
                findings: &[],
                impacted: &[],
                untested: &[],
            },
            &RiskWeights::default(),
        );
        assert_eq!(r.score, 0);
        assert_eq!(r.tier, RiskTier::Low);
        assert_eq!(r.factors.len(), 6);
    }

    #[test]
    fn critical_finding_plus_blast_is_high() {
        let impacted: Vec<ImpactedSymbol> = (0..7)
            .map(|i| ImpactedSymbol {
                qualified: format!("x{i}"),
                file: PathBuf::from("api/payments.py"),
                line: 1,
                depth: 1,
                via: "a".into(),
                sensitive: i == 0,
            })
            .collect();
        let findings = vec![
            finding(Severity::Critical, FindingState::New),
            finding(Severity::Critical, FindingState::Fixed),
        ];
        let r = assess(
            &RiskInput {
                files: &[],
                symbols: &[],
                findings: &findings,
                impacted: &impacted,
                untested: &[],
            },
            &RiskWeights::default(),
        );
        // 45 security + min(6*3 + 5, 20) = 65
        assert_eq!(r.score, 65);
        assert_eq!(r.tier, RiskTier::High);
        assert!(r.factors[0].detail.contains("1 fixed"));
    }
}
