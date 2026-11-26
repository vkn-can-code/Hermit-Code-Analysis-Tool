//! Repository-wide symbol index and call graph.
//!
//! Every source file at head is parsed (in parallel), call sites are resolved to
//! symbol definitions using imports, same-file scope and, as a last resort,
//! unique-name matching. The reverse edges give the *blast radius* of a change.

use hermit_core::{ImpactedSymbol, Language, Symbol, SymbolKind};
use hermit_lang::{CallSite, ParsedFile};
use rayon::prelude::*;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

/// Name-only matches with more candidates than this are treated as unresolvable.
const MAX_AMBIGUOUS: usize = 3;

/// One source file to index.
pub struct SourceFile {
    pub path: PathBuf,
    pub language: Language,
    pub text: String,
}

/// A resolved call edge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edge {
    /// Qualified caller (or `file::<module>` for top-level code).
    pub caller: String,
    pub callee: String,
    pub line: u32,
    /// `import`, `local`, `name`: how the edge was resolved (precision tier).
    pub how: &'static str,
}

/// Parsed repository with call graph.
#[derive(Default)]
pub struct RepoIndex {
    pub files: HashMap<PathBuf, ParsedFile>,
    pub symbols: HashMap<String, Symbol>,
    by_name: HashMap<String, Vec<String>>,
    /// callee -> edges pointing at it
    callers: HashMap<String, Vec<Edge>>,
    /// caller -> edges leaving it
    callees: HashMap<String, Vec<Edge>>,
    /// short module key -> file paths (`services.cache`, `services/cache`, `cache`)
    modules: HashMap<String, Vec<PathBuf>>,
    pub parse_errors: Vec<(PathBuf, String)>,
    /// Head source text per file (used to quote symbol bodies).
    pub texts: HashMap<PathBuf, String>,
}

impl RepoIndex {
    /// Parses all files and resolves the call graph.
    pub fn build(sources: Vec<SourceFile>) -> Self {
        let parsed: Vec<(PathBuf, String, Result<ParsedFile, String>)> = sources
            .into_par_iter()
            .map(|s| {
                let r = hermit_lang::parse(s.language, &s.path, &s.text).map_err(|e| e.to_string());
                (s.path, s.text, r)
            })
            .collect();

        let mut idx = RepoIndex::default();
        for (path, text, res) in parsed {
            idx.texts.insert(path.clone(), text);
            match res {
                Ok(pf) => {
                    for key in module_keys(&path) {
                        idx.modules.entry(key).or_default().push(path.clone());
                    }
                    for s in &pf.symbols {
                        idx.by_name
                            .entry(s.name.clone())
                            .or_default()
                            .push(s.qualified.clone());
                        idx.symbols.insert(s.qualified.clone(), s.clone());
                    }
                    idx.files.insert(path, pf);
                }
                Err(e) => idx.parse_errors.push((path, e)),
            }
        }
        idx.resolve_edges();
        idx
    }

    fn resolve_edges(&mut self) {
        let mut edges = Vec::new();
        for (path, pf) in &self.files {
            let aliases = self.alias_table(path, pf);
            for call in &pf.calls {
                let caller = call
                    .caller
                    .clone()
                    .unwrap_or_else(|| format!("{}::<module>", path.display()));
                for (callee, how) in self.resolve_call(path, pf, &aliases, call) {
                    if callee != caller {
                        edges.push(Edge {
                            caller: caller.clone(),
                            callee,
                            line: call.line,
                            how,
                        });
                    }
                }
            }
        }
        for e in edges {
            self.callees
                .entry(e.caller.clone())
                .or_default()
                .push(e.clone());
            self.callers.entry(e.callee.clone()).or_default().push(e);
        }
    }

    /// local alias -> (target files, original name; empty name = whole module)
    fn alias_table(&self, path: &Path, pf: &ParsedFile) -> HashMap<String, (Vec<PathBuf>, String)> {
        let mut t = HashMap::new();
        for imp in &pf.imports {
            let targets = self.resolve_module(path, &imp.module);
            for (name, alias) in &imp.names {
                if alias.is_empty() || alias == "*" {
                    continue;
                }
                // `from pkg import mod` imports a module, not a symbol.
                let sub = self.resolve_module(path, &format!("{}.{}", imp.module, name));
                if !name.is_empty() && name != "default" && !sub.is_empty() && targets.is_empty() {
                    t.insert(alias.clone(), (sub, String::new()));
                } else {
                    let original = if name == "default" {
                        String::new()
                    } else {
                        name.clone()
                    };
                    t.insert(alias.clone(), (targets.clone(), original));
                }
            }
        }
        t
    }

    fn resolve_call(
        &self,
        path: &Path,
        pf: &ParsedFile,
        aliases: &HashMap<String, (Vec<PathBuf>, String)>,
        call: &CallSite,
    ) -> Vec<(String, &'static str)> {
        let name = call.callee.as_str();

        // 1. Imported name: `from x import f; f()` / `import { f } from './x'`
        if call.receiver.is_none()
            && let Some((files, original)) = aliases.get(name)
        {
            let target = if original.is_empty() {
                name
            } else {
                original.as_str()
            };
            let hits = self.defined_in(files, target);
            if !hits.is_empty() {
                return tag(hits, "import");
            }
            if let Some(hits) = self.unique_by_name(target) {
                return tag(hits, "import");
            }
        }

        // 2. Module or type receiver: `cache.invalidate()`, `cache::invalidate()`, `S::flush()`
        if let Some(recv) = call.receiver.as_deref() {
            let head = recv
                .split(['.', ':'])
                .find(|s| !s.is_empty())
                .unwrap_or(recv);
            let last = recv.rsplit(['.', ':']).next().unwrap_or(recv);
            if let Some((files, original)) = aliases.get(head).or_else(|| aliases.get(last)) {
                // Imported type: method on that type.
                if !original.is_empty() {
                    let hits = self.method_of(original, name);
                    if !hits.is_empty() {
                        return tag(hits, "import");
                    }
                }
                let hits = self.defined_in(files, name);
                if !hits.is_empty() {
                    return tag(hits, "import");
                }
            }
            // `Type::method()` / `Type.method()` with a type defined in the repo.
            let hits = self.method_of(last, name);
            if !hits.is_empty() {
                return tag(hits, "local");
            }
            // `self.method()` / `this.method()` inside a class.
            if matches!(head, "self" | "this" | "Self" | "cls" | "super")
                && let Some(caller) = &call.caller
            {
                let class_prefix = caller.rsplit_once('.').map(|(p, _)| p.to_string());
                if let Some(prefix) = class_prefix {
                    let q = format!("{prefix}.{name}");
                    if self.symbols.contains_key(&q) {
                        return vec![(q, "local")];
                    }
                }
            }
        }

        // 3. Same file definition.
        let local: Vec<String> = pf
            .symbols
            .iter()
            .filter(|s| s.name == name && (s.kind.is_callable() || is_type(s.kind)))
            .map(|s| s.qualified.clone())
            .collect();
        if !local.is_empty() {
            return tag(local, "local");
        }

        // 4. Unique-enough global name.
        if let Some(hits) = self.unique_by_name(name) {
            let _ = path;
            return tag(hits, "name");
        }
        Vec::new()
    }

    fn defined_in(&self, files: &[PathBuf], name: &str) -> Vec<String> {
        files
            .iter()
            .filter_map(|f| self.files.get(f))
            .flat_map(|pf| pf.symbols.iter())
            .filter(|s| s.name == name)
            .map(|s| s.qualified.clone())
            .collect()
    }

    fn method_of(&self, type_name: &str, method: &str) -> Vec<String> {
        let suffix = format!("{type_name}.{method}");
        self.by_name
            .get(method)
            .map(|qs| {
                qs.iter()
                    .filter(|q| {
                        q.ends_with(&format!("::{suffix}")) || q.ends_with(&format!(".{suffix}"))
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    fn unique_by_name(&self, name: &str) -> Option<Vec<String>> {
        let qs: Vec<String> = self
            .by_name
            .get(name)?
            .iter()
            .filter(|q| {
                self.symbols
                    .get(*q)
                    .is_some_and(|s| s.kind.is_callable() || is_type(s.kind))
            })
            .cloned()
            .collect();
        (!qs.is_empty() && qs.len() <= MAX_AMBIGUOUS).then_some(qs)
    }

    /// Resolves an import string to repository files.
    pub fn resolve_module(&self, from: &Path, module: &str) -> Vec<PathBuf> {
        if module.is_empty() {
            return Vec::new();
        }
        let dir = from.parent().unwrap_or(Path::new(""));
        // Relative JS/TS import: ./x, ../x
        if module.starts_with("./") || module.starts_with("../") {
            let joined = normalize(&dir.join(module));
            let key = joined.to_string_lossy().replace('\\', "/");
            return self.modules.get(&key).cloned().unwrap_or_default();
        }
        // Relative Python import: .x / ..x
        if let Some(stripped) = module.strip_prefix('.') {
            let mut base = dir.to_path_buf();
            let mut rest = stripped;
            while let Some(r) = rest.strip_prefix('.') {
                base.pop();
                rest = r;
            }
            let key = normalize(&base.join(rest.replace('.', "/")))
                .to_string_lossy()
                .replace('\\', "/");
            return self.modules.get(&key).cloned().unwrap_or_default();
        }
        // Rust: crate::a::b, super::a, self::a
        let rust_like = module.contains("::");
        let cleaned = module
            .trim_start_matches("crate::")
            .trim_start_matches("self::")
            .trim_start_matches("super::");
        let key = if rust_like {
            cleaned.replace("::", "/")
        } else {
            cleaned.replace('.', "/")
        };
        if let Some(hit) = self.modules.get(&key) {
            return hit.clone();
        }
        // Fall back to the last segment (e.g. src/ layout, package roots).
        let last = key.rsplit('/').next().unwrap_or(&key);
        match self.modules.get(last) {
            Some(v) if v.len() == 1 => v.clone(),
            _ => Vec::new(),
        }
    }

    /// Direct callers of `qualified`.
    pub fn callers_of(&self, qualified: &str) -> &[Edge] {
        self.callers.get(qualified).map_or(&[], Vec::as_slice)
    }

    /// Direct callees of `qualified`.
    pub fn callees_of(&self, qualified: &str) -> &[Edge] {
        self.callees.get(qualified).map_or(&[], Vec::as_slice)
    }

    /// Head call sites that still call a name (used for deleted symbols).
    pub fn call_sites_named(&self, name: &str) -> Vec<(String, PathBuf, u32)> {
        let mut out = Vec::new();
        for (path, pf) in &self.files {
            for c in pf.calls.iter().filter(|c| c.callee == name) {
                let caller = c
                    .caller
                    .clone()
                    .unwrap_or_else(|| format!("{}::<module>", path.display()));
                out.push((caller, path.clone(), c.line));
            }
        }
        out
    }

    /// Reverse BFS from `roots` (changed symbols) up to `max_depth` hops.
    ///
    /// Returns every transitive caller with its distance and the root it came from.
    /// Roots missing from the index (deleted symbols) are expanded through head call
    /// sites that still use their name: those callers are now broken.
    pub fn blast_radius(
        &self,
        roots: &[(String, String)],
        max_depth: u32,
        sensitive: &[String],
    ) -> Vec<ImpactedSymbol> {
        let root_set: HashSet<&str> = roots.iter().map(|(q, _)| q.as_str()).collect();
        let mut seen: HashMap<String, ImpactedSymbol> = HashMap::new();
        let mut queue: VecDeque<(String, u32, String)> = VecDeque::new();

        for (q, name) in roots {
            if self.symbols.contains_key(q) {
                queue.push_back((q.clone(), 0, q.clone()));
            } else {
                for (caller, file, line) in self.call_sites_named(name) {
                    if root_set.contains(caller.as_str()) {
                        continue;
                    }
                    self.visit(&mut seen, &mut queue, caller, file, line, 1, q, sensitive);
                }
            }
        }
        while let Some((node, depth, via)) = queue.pop_front() {
            if depth >= max_depth {
                continue;
            }
            for e in self.callers_of(&node) {
                if root_set.contains(e.caller.as_str()) {
                    continue;
                }
                let file = self
                    .symbols
                    .get(&e.caller)
                    .map(|s| s.file.clone())
                    .unwrap_or_else(|| PathBuf::from(e.caller.split("::").next().unwrap_or("")));
                self.visit(
                    &mut seen,
                    &mut queue,
                    e.caller.clone(),
                    file,
                    e.line,
                    depth + 1,
                    &via,
                    sensitive,
                );
            }
        }
        let mut out: Vec<ImpactedSymbol> = seen.into_values().collect();
        out.sort_by(|a, b| a.depth.cmp(&b.depth).then(a.qualified.cmp(&b.qualified)));
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn visit(
        &self,
        seen: &mut HashMap<String, ImpactedSymbol>,
        queue: &mut VecDeque<(String, u32, String)>,
        caller: String,
        file: PathBuf,
        line: u32,
        depth: u32,
        via: &str,
        sensitive: &[String],
    ) {
        if seen.get(&caller).is_some_and(|s| s.depth <= depth) {
            return;
        }
        let lower = caller.to_lowercase();
        let is_sensitive = sensitive.iter().any(|k| lower.contains(&k.to_lowercase()));
        seen.insert(
            caller.clone(),
            ImpactedSymbol {
                qualified: caller.clone(),
                file,
                line,
                depth,
                via: via.to_string(),
                sensitive: is_sensitive,
            },
        );
        queue.push_back((caller, depth, via.to_string()));
    }

    /// Source text of a symbol (head), capped at `max_lines` lines.
    pub fn symbol_text(&self, qualified: &str, max_lines: usize) -> Option<String> {
        let s = self.symbols.get(qualified)?;
        let text = self.texts.get(&s.file)?;
        let lines: Vec<&str> = text
            .lines()
            .skip(s.range.start.saturating_sub(1) as usize)
            .take((s.range.len() as usize).min(max_lines))
            .collect();
        Some(lines.join("\n"))
    }

    pub fn edge_count(&self) -> usize {
        self.callers.values().map(Vec::len).sum()
    }
}

fn tag(v: Vec<String>, how: &'static str) -> Vec<(String, &'static str)> {
    v.into_iter().map(|q| (q, how)).collect()
}

fn is_type(k: SymbolKind) -> bool {
    matches!(k, SymbolKind::Class | SymbolKind::Struct | SymbolKind::Enum)
}

/// Lookup keys under which a file can be imported.
fn module_keys(path: &Path) -> Vec<String> {
    let s = path.to_string_lossy().replace('\\', "/");
    let mut keys = Vec::new();
    let stem = s
        .rsplit_once('.')
        .map_or(s.as_str(), |(stem, _)| stem)
        .to_string();
    // Package entry files resolve to their directory.
    for suffix in ["/__init__", "/index", "/mod", "/lib", "/main"] {
        if let Some(dir) = stem.strip_suffix(suffix) {
            push_with_src(&mut keys, dir);
        }
    }
    push_with_src(&mut keys, &stem);
    keys
}

fn push_with_src(keys: &mut Vec<String>, k: &str) {
    keys.push(k.to_string());
    // Rust `crate::x` maps to src/x.rs; Python/JS src-layout too.
    if let Some(rest) = k.strip_prefix("src/") {
        keys.push(rest.to_string());
    }
    if let Some((_, after)) = k.split_once("/src/") {
        keys.push(after.to_string());
    }
    if let Some(last) = k.rsplit('/').next()
        && last != k
    {
        keys.push(last.to_string());
    }
}

fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src(path: &str, text: &str) -> SourceFile {
        let p = PathBuf::from(path);
        SourceFile {
            language: Language::from_path(&p).unwrap(),
            path: p,
            text: text.into(),
        }
    }

    #[test]
    fn python_cross_file_blast_radius() {
        let idx = RepoIndex::build(vec![
            src(
                "services/cache.py",
                "def invalidate_cache(key):\n    pass\n",
            ),
            src(
                "services/billing.py",
                "from services.cache import invalidate_cache\n\ndef get_sub(org):\n    invalidate_cache(org)\n",
            ),
            src(
                "api/payments.py",
                "from services import billing\n\ndef pay_handler(req):\n    return billing.get_sub(req)\n",
            ),
            src(
                "tests/test_billing.py",
                "from services.billing import get_sub\n\ndef test_get_sub():\n    get_sub(1)\n",
            ),
        ]);
        let edges = idx.callers_of("services/cache.py::invalidate_cache");
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].caller, "services/billing.py::get_sub");
        assert_eq!(edges[0].how, "import");

        let impact = idx.blast_radius(
            &[(
                "services/cache.py::invalidate_cache".into(),
                "invalidate_cache".into(),
            )],
            3,
            &["payment".into()],
        );
        let names: Vec<(&str, u32)> = impact
            .iter()
            .map(|i| (i.qualified.as_str(), i.depth))
            .collect();
        assert!(
            names.contains(&("services/billing.py::get_sub", 1)),
            "{names:?}"
        );
        assert!(
            names.contains(&("api/payments.py::pay_handler", 2)),
            "{names:?}"
        );
        assert!(
            names.contains(&("tests/test_billing.py::test_get_sub", 2)),
            "{names:?}"
        );
        assert!(
            impact
                .iter()
                .find(|i| i.qualified.contains("pay_handler"))
                .unwrap()
                .sensitive
        );
    }

    #[test]
    fn ts_and_rust_resolution() {
        let idx = RepoIndex::build(vec![
            src(
                "src/db.ts",
                "export function query(q: string) { return q; }\n",
            ),
            src(
                "src/user.ts",
                "import { query as q } from './db';\nexport function find(id: string) { return q(id); }\n",
            ),
            src(
                "src/cache.rs",
                "pub struct Store;\nimpl Store { pub fn flush() {} }\n",
            ),
            src(
                "src/lib.rs",
                "use crate::cache::Store;\nfn run() { Store::flush(); }\n",
            ),
        ]);
        assert_eq!(
            idx.callers_of("src/db.ts::query")[0].caller,
            "src/user.ts::find"
        );
        assert_eq!(
            idx.callers_of("src/cache.rs::Store.flush")[0].caller,
            "src/lib.rs::run"
        );
    }

    #[test]
    fn deleted_symbol_breaks_callers() {
        let idx = RepoIndex::build(vec![src("a.py", "def user():\n    legacy()\n")]);
        let impact = idx.blast_radius(&[("old.py::legacy".into(), "legacy".into())], 3, &[]);
        assert_eq!(impact[0].qualified, "a.py::user");
        assert_eq!(impact[0].depth, 1);
    }
}
