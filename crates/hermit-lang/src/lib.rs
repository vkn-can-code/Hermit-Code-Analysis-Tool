//! Tree-sitter based source analysis for Python, JavaScript, TypeScript and Rust.
//!
//! [`parse`] turns a source file into symbols (functions, methods, classes, ...),
//! call sites and imports. [`diff_symbols`] maps diff line ranges onto those
//! symbols to find out exactly which code entities a change touched.

mod walker;

use anyhow::{Context, Result};
use hermit_core::{ChangeKind, ChangedFile, ChangedSymbol, Language, Symbol};
use std::collections::HashMap;
use std::path::Path;
use tree_sitter::Parser;

/// A call expression found in a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallSite {
    /// Qualified name of the enclosing callable (`None` at module level).
    pub caller: Option<String>,
    /// Name of the called function or method (last path segment).
    pub callee: String,
    /// Receiver or path qualifier, e.g. `db` in `db.execute()` or `cache` in `cache::get()`.
    pub receiver: Option<String>,
    pub line: u32,
}

/// An import / use statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Import {
    /// Module path as written (`services.cache`, `./cache`, `crate::cache`).
    pub module: String,
    /// Imported names with their local alias (empty for `import x`).
    pub names: Vec<(String, String)>,
    pub line: u32,
}

/// Everything extracted from one source file.
#[derive(Debug, Clone, Default)]
pub struct ParsedFile {
    pub symbols: Vec<Symbol>,
    pub calls: Vec<CallSite>,
    pub imports: Vec<Import>,
}

/// Parses `source` (the content of `path`) written in `lang`.
pub fn parse(lang: Language, path: &Path, source: &str) -> Result<ParsedFile> {
    let mut parser = Parser::new();
    let ts_lang: tree_sitter::Language = match lang {
        Language::Python => tree_sitter_python::LANGUAGE.into(),
        Language::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
        Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        Language::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        Language::Rust => tree_sitter_rust::LANGUAGE.into(),
    };
    parser
        .set_language(&ts_lang)
        .context("loading tree-sitter grammar")?;
    let tree = parser
        .parse(source, None)
        .with_context(|| format!("parse failed: {}", path.display()))?;
    Ok(walker::walk(lang, path, source, tree.root_node()))
}

/// Determines which symbols a file change touched.
///
/// Every changed line is attributed to the innermost symbol containing it, so a
/// one-line edit inside a method marks the method, not the whole class.
/// `base` / `head` are the parsed file at each side (`None` if the file is absent there).
pub fn diff_symbols(
    file: &ChangedFile,
    base: Option<&ParsedFile>,
    head: Option<&ParsedFile>,
) -> Vec<ChangedSymbol> {
    let empty = ParsedFile::default();
    let base = base.unwrap_or(&empty);
    let head = head.unwrap_or(&empty);

    let base_by_name: HashMap<&str, &Symbol> = base
        .symbols
        .iter()
        .map(|s| (local_name(&s.qualified), s))
        .collect();
    let head_by_name: HashMap<&str, &Symbol> = head
        .symbols
        .iter()
        .map(|s| (local_name(&s.qualified), s))
        .collect();

    let mut touched_head: Vec<&Symbol> = Vec::new();
    for range in &file.added {
        for line in range.start..=range.end {
            if let Some(s) = innermost(&head.symbols, line)
                && !touched_head.iter().any(|t| t.qualified == s.qualified)
            {
                touched_head.push(s);
            }
        }
    }
    let mut touched_base: Vec<&Symbol> = Vec::new();
    for range in &file.removed {
        for line in range.start..=range.end {
            if let Some(s) = innermost(&base.symbols, line)
                && !touched_base.iter().any(|t| t.qualified == s.qualified)
            {
                touched_base.push(s);
            }
        }
    }

    let mut out: Vec<ChangedSymbol> = Vec::new();
    let mut seen: Vec<&str> = Vec::new();

    for s in &touched_head {
        let key = local_name(&s.qualified);
        seen.push(key);
        let before = base_by_name.get(key);
        out.push(ChangedSymbol {
            symbol: (*s).clone(),
            change: if before.is_some() {
                ChangeKind::Modified
            } else {
                ChangeKind::Added
            },
            signature_changed: before.is_some_and(|b| b.signature != s.signature),
            complexity_before: before.map_or(0, |b| b.complexity),
            complexity_after: s.complexity,
        });
    }
    for s in &touched_base {
        let key = local_name(&s.qualified);
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        match head_by_name.get(key) {
            // Only deletions inside a still-existing symbol.
            Some(h) => out.push(ChangedSymbol {
                symbol: (*h).clone(),
                change: ChangeKind::Modified,
                signature_changed: h.signature != s.signature,
                complexity_before: s.complexity,
                complexity_after: h.complexity,
            }),
            None => out.push(ChangedSymbol {
                symbol: (*s).clone(),
                change: ChangeKind::Deleted,
                signature_changed: true,
                complexity_before: s.complexity,
                complexity_after: 0,
            }),
        }
    }
    out.sort_by_key(|c| c.symbol.range.start);
    out
}

/// Part of a qualified name after `file::`, so renamed files still match symbols.
fn local_name(qualified: &str) -> &str {
    qualified.rsplit_once("::").map_or(qualified, |(_, n)| n)
}

fn innermost(symbols: &[Symbol], line: u32) -> Option<&Symbol> {
    symbols
        .iter()
        .filter(|s| s.range.contains(line))
        .min_by_key(|s| s.range.len())
}

#[cfg(test)]
mod tests;
