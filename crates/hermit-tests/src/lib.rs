//! Selective regression testing: finds the tests that exercise changed code.
//!
//! A test is selected when it reaches a changed symbol through the call graph
//! (distance = hops). Tests in a file whose name mirrors a changed module
//! (`billing.py` <-> `test_billing.py`) are added as a lower-confidence fallback.
//! Changed callables reached by no test are reported as coverage gaps.

use hermit_core::{ChangeKind, ChangedSymbol, Language, Symbol, TestCandidate};
use hermit_graph::RepoIndex;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::Path;

/// Distance assigned to naming-convention matches (no call path found).
pub const CONVENTION_DISTANCE: u32 = 99;

/// JavaScript test runner used to build commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsRunner {
    Jest,
    Vitest,
    Mocha,
    NodeTest,
}

impl JsRunner {
    /// Picks the runner from a package.json body.
    pub fn detect(package_json: Option<&str>) -> Self {
        let p = package_json.unwrap_or_default();
        if p.contains("\"vitest\"") {
            Self::Vitest
        } else if p.contains("\"jest\"") {
            Self::Jest
        } else if p.contains("\"mocha\"") {
            Self::Mocha
        } else if p.is_empty() {
            Self::Jest
        } else {
            Self::NodeTest
        }
    }
}

/// Result of test selection.
pub struct Selection {
    pub tests: Vec<TestCandidate>,
    /// Changed callables that no test reaches.
    pub untested: Vec<String>,
    /// Total test cases discovered in the repository.
    pub total_tests: usize,
}

/// Selects tests for `changed`. `max_depth` bounds the reverse call-graph search.
pub fn select(
    idx: &RepoIndex,
    changed: &[ChangedSymbol],
    max_depth: u32,
    js: JsRunner,
) -> Selection {
    let total_tests = idx.symbols.values().filter(|s| s.is_test).count();
    // test qualified -> (distance, covered changed symbols)
    let mut hits: HashMap<String, (u32, Vec<String>)> = HashMap::new();
    let mut covered: HashMap<String, bool> = HashMap::new();

    for c in changed {
        if c.symbol.is_test {
            // A changed test should itself be run.
            record(&mut hits, &c.symbol.qualified, 0, &c.symbol.qualified);
            continue;
        }
        let q = &c.symbol.qualified;
        if c.symbol.kind.is_callable() && c.change != ChangeKind::Deleted {
            covered.entry(q.clone()).or_insert(false);
        }
        // Reverse BFS restricted to this root so we know what each test covers.
        let mut queue = VecDeque::new();
        let mut seen: HashMap<String, u32> = HashMap::new();
        if idx.symbols.contains_key(q) && c.change != ChangeKind::Deleted {
            queue.push_back((q.clone(), 0u32));
        } else {
            for (caller, _, _) in idx.call_sites_named(&c.symbol.name) {
                queue.push_back((caller, 1));
            }
        }
        while let Some((node, d)) = queue.pop_front() {
            if seen.get(&node).is_some_and(|&p| p <= d) {
                continue;
            }
            seen.insert(node.clone(), d);
            if idx.symbols.get(&node).is_some_and(|s| s.is_test) {
                record(&mut hits, &node, d, q);
                covered.insert(q.clone(), true);
                continue;
            }
            if d >= max_depth {
                continue;
            }
            for e in idx.callers_of(&node) {
                queue.push_back((e.caller.clone(), d + 1));
            }
        }
    }

    // Naming-convention fallback for files with no call-path hits.
    for c in changed.iter().filter(|c| !c.symbol.is_test) {
        let stem = file_stem(&c.symbol.file);
        let reached = covered.get(&c.symbol.qualified).copied().unwrap_or(true);
        if reached || stem.is_empty() {
            continue;
        }
        for t in idx.symbols.values().filter(|s| s.is_test) {
            if test_file_matches(&t.file, &stem) {
                record(
                    &mut hits,
                    &t.qualified,
                    CONVENTION_DISTANCE,
                    &c.symbol.qualified,
                );
            }
        }
    }

    let mut tests: Vec<TestCandidate> = hits
        .into_iter()
        .filter_map(|(q, (distance, covers))| {
            let s = idx.symbols.get(&q)?;
            Some(TestCandidate {
                command: command_for(s, js),
                qualified: q,
                file: s.file.clone(),
                line: s.range.start,
                distance,
                covers,
            })
        })
        .collect();
    tests.sort_by(|a, b| {
        a.distance
            .cmp(&b.distance)
            .then(b.covers.len().cmp(&a.covers.len()))
            .then(a.qualified.cmp(&b.qualified))
    });

    let mut untested: Vec<String> = covered
        .into_iter()
        .filter(|(_, v)| !v)
        .map(|(k, _)| k)
        .collect();
    untested.sort();
    Selection {
        tests,
        untested,
        total_tests,
    }
}

fn record(hits: &mut HashMap<String, (u32, Vec<String>)>, test: &str, d: u32, covers: &str) {
    let e = hits.entry(test.to_string()).or_insert((d, Vec::new()));
    e.0 = e.0.min(d);
    if !e.1.iter().any(|c| c == covers) {
        e.1.push(covers.to_string());
    }
}

fn file_stem(p: &Path) -> String {
    p.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .split('.')
        .next()
        .unwrap_or_default()
        .to_string()
}

fn test_file_matches(test_file: &Path, stem: &str) -> bool {
    let t = file_stem(test_file);
    t == format!("test_{stem}")
        || t == format!("{stem}_test")
        || t == stem && is_test_name(test_file)
}

fn is_test_name(p: &Path) -> bool {
    let n = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    n.contains(".test.") || n.contains(".spec.")
}

/// Builds the shell command that runs exactly one test.
pub fn command_for(s: &Symbol, js: JsRunner) -> String {
    let file = s.file.to_string_lossy().replace('\\', "/");
    let local = s
        .qualified
        .rsplit_once("::")
        .map_or(s.qualified.as_str(), |(_, l)| l);
    match Language::from_path(&s.file) {
        Some(Language::Python) => format!("pytest '{file}::{}'", local.replace('.', "::")),
        Some(Language::Rust) => {
            // Module path inside the crate: mod tests { fn x } -> tests::x
            format!("cargo test {}", local.replace('.', "::"))
        }
        Some(_) => {
            let title = local.replace('\'', "\\'").replace('.', " ");
            match js {
                JsRunner::Vitest => format!("npx vitest run {file} -t '{title}'"),
                JsRunner::Jest => format!("npx jest {file} -t '{title}'"),
                JsRunner::Mocha => format!("npx mocha {file} --grep '{title}'"),
                JsRunner::NodeTest => format!("node --test --test-name-pattern='{title}' {file}"),
            }
        }
        None => file,
    }
}

/// Groups selected tests per file, for summary output.
pub fn by_file(tests: &[TestCandidate]) -> BTreeMap<String, Vec<&TestCandidate>> {
    let mut m: BTreeMap<String, Vec<&TestCandidate>> = BTreeMap::new();
    for t in tests {
        m.entry(t.file.to_string_lossy().into_owned())
            .or_default()
            .push(t);
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use hermit_graph::SourceFile;
    use std::path::PathBuf;

    fn src(path: &str, text: &str) -> SourceFile {
        let p = PathBuf::from(path);
        SourceFile {
            language: Language::from_path(&p).unwrap(),
            path: p,
            text: text.into(),
        }
    }

    fn changed(idx: &RepoIndex, q: &str) -> ChangedSymbol {
        ChangedSymbol {
            symbol: idx.symbols[q].clone(),
            change: ChangeKind::Modified,
            signature_changed: false,
            complexity_before: 1,
            complexity_after: 1,
        }
    }

    #[test]
    fn selects_reaching_tests_and_reports_gaps() {
        let idx = RepoIndex::build(vec![
            src(
                "app/cache.py",
                "def evict(k):\n    pass\n\ndef orphan():\n    pass\n",
            ),
            src(
                "app/billing.py",
                "from app.cache import evict\n\ndef charge(o):\n    evict(o)\n",
            ),
            src(
                "tests/test_billing.py",
                "from app.billing import charge\n\ndef test_charge():\n    charge(1)\n\ndef test_other():\n    pass\n",
            ),
        ]);
        let sel = select(
            &idx,
            &[
                changed(&idx, "app/cache.py::evict"),
                changed(&idx, "app/cache.py::orphan"),
            ],
            6,
            JsRunner::Jest,
        );
        assert_eq!(sel.total_tests, 2);
        assert_eq!(sel.tests.len(), 1);
        assert_eq!(sel.tests[0].qualified, "tests/test_billing.py::test_charge");
        assert_eq!(sel.tests[0].distance, 2);
        assert_eq!(
            sel.tests[0].command,
            "pytest 'tests/test_billing.py::test_charge'"
        );
        assert_eq!(sel.untested, vec!["app/cache.py::orphan".to_string()]);
    }

    #[test]
    fn convention_fallback() {
        let idx = RepoIndex::build(vec![
            src(
                "src/math.ts",
                "export function add(a, b) { return a + b; }\n",
            ),
            src(
                "src/math.test.ts",
                "test('adds', () => { expect(1).toBe(1); });\n",
            ),
        ]);
        let sel = select(
            &idx,
            &[changed(&idx, "src/math.ts::add")],
            6,
            JsRunner::Vitest,
        );
        assert_eq!(sel.tests[0].distance, CONVENTION_DISTANCE);
        assert_eq!(
            sel.tests[0].command,
            "npx vitest run src/math.test.ts -t 'adds'"
        );
    }
}
