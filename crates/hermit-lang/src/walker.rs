//! Single-pass syntax tree walker shared by all languages.

use crate::{CallSite, Import, ParsedFile};
use hermit_core::{Language, LineRange, Symbol, SymbolKind};
use std::path::Path;
use tree_sitter::Node;

struct Ctx<'a> {
    lang: Language,
    file: &'a Path,
    file_label: String,
    src: &'a [u8],
    test_file: bool,
    out: ParsedFile,
}

/// A lexical scope on the walk stack.
#[derive(Clone)]
struct Scope {
    /// Dotted local path, e.g. `BillingService.charge`.
    path: String,
    /// Qualified name of the innermost enclosing callable.
    callable: Option<String>,
    /// Innermost enclosing type kind (class / impl / trait), for method detection.
    in_type: bool,
    /// Inside a `describe(...)` block or test module.
    in_test_group: bool,
}

pub(crate) fn walk(lang: Language, file: &Path, source: &str, root: Node) -> ParsedFile {
    let file_label = file.to_string_lossy().replace('\\', "/");
    let name = file
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    let test_file = is_test_path(&file_label, name);
    let mut ctx = Ctx {
        lang,
        file,
        file_label,
        src: source.as_bytes(),
        test_file,
        out: ParsedFile::default(),
    };
    let scope = Scope {
        path: String::new(),
        callable: None,
        in_type: false,
        in_test_group: false,
    };
    visit(&mut ctx, root, &scope);
    ctx.out
}

fn is_test_path(path: &str, name: &str) -> bool {
    name.starts_with("test_")
        || name.ends_with("_test.py")
        || name.contains(".test.")
        || name.contains(".spec.")
        || path.contains("/tests/")
        || path.starts_with("tests/")
        || path.contains("__tests__/")
}

fn visit(ctx: &mut Ctx, node: Node, scope: &Scope) {
    let child_scope = match ctx.lang {
        Language::Python => python(ctx, node, scope),
        Language::JavaScript | Language::TypeScript | Language::Tsx => js(ctx, node, scope),
        Language::Rust => rust(ctx, node, scope),
    };

    let next = child_scope.as_ref().unwrap_or(scope);
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        visit(ctx, child, next);
    }
}

// ---------------------------------------------------------------- helpers

fn text<'a>(ctx: &Ctx<'a>, node: Node) -> &'a str {
    node.utf8_text(ctx.src).unwrap_or_default()
}

fn line(node: Node) -> u32 {
    node.start_position().row as u32 + 1
}

/// Symbol range including leading doc comments, attributes and decorators, so an edit
/// to a doc comment is attributed to the function it documents.
fn symbol_range(node: Node) -> LineRange {
    let mut start = node;
    if let Some(p) = node.parent()
        && p.kind() == "decorated_definition"
    {
        start = p;
    }
    let mut prev = start.prev_sibling();
    while let Some(p) = prev {
        let leading = matches!(
            p.kind(),
            "line_comment" | "block_comment" | "attribute_item" | "comment" | "decorator"
        );
        // Must be directly adjacent (no blank line in between).
        if !leading || p.end_position().row + 1 < start.start_position().row {
            break;
        }
        start = p;
        prev = p.prev_sibling();
    }
    LineRange::new(line(start), node.end_position().row as u32 + 1)
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn join(path: &str, name: &str) -> String {
    if path.is_empty() {
        name.to_string()
    } else {
        format!("{path}.{name}")
    }
}

/// Records a symbol and returns the scope for its body.
#[allow(clippy::too_many_arguments)]
fn add_symbol(
    ctx: &mut Ctx,
    node: Node,
    body: Option<Node>,
    name: &str,
    kind: SymbolKind,
    signature: String,
    is_test: bool,
    scope: &Scope,
) -> Scope {
    let path = join(&scope.path, name);
    let qualified = format!("{}::{}", ctx.file_label, path);
    let complexity = if kind.is_callable() {
        1 + body.map_or(0, |b| decisions(ctx.lang, b, true))
    } else {
        1
    };
    ctx.out.symbols.push(Symbol {
        name: name.to_string(),
        qualified: qualified.clone(),
        kind,
        file: ctx.file.to_path_buf(),
        range: symbol_range(node),
        signature,
        complexity,
        is_test,
    });
    let is_type = matches!(
        kind,
        SymbolKind::Class | SymbolKind::Impl | SymbolKind::Trait | SymbolKind::Struct
    );
    Scope {
        path,
        callable: if kind.is_callable() {
            Some(qualified)
        } else {
            scope.callable.clone()
        },
        in_type: is_type || (scope.in_type && !kind.is_callable()),
        in_test_group: scope.in_test_group,
    }
}

fn add_call(ctx: &mut Ctx, scope: &Scope, callee: &str, receiver: Option<&str>, node: Node) {
    if callee.is_empty() {
        return;
    }
    ctx.out.calls.push(CallSite {
        caller: scope.callable.clone(),
        callee: callee.to_string(),
        receiver: receiver.map(str::to_string),
        line: line(node),
    });
}

/// Counts McCabe decision points under `node`, not descending into nested callables.
fn decisions(lang: Language, node: Node, is_root: bool) -> u32 {
    let kind = node.kind();
    if !is_root && is_callable_node(lang, kind) {
        return 0;
    }
    let mut n: u32 = match lang {
        Language::Python => match kind {
            "if_statement"
            | "elif_clause"
            | "for_statement"
            | "while_statement"
            | "except_clause"
            | "conditional_expression"
            | "for_in_clause"
            | "if_clause"
            | "case_clause" => 1,
            "boolean_operator" => 1,
            _ => 0,
        },
        Language::JavaScript | Language::TypeScript | Language::Tsx => match kind {
            "if_statement" | "for_statement" | "for_in_statement" | "while_statement"
            | "do_statement" | "switch_case" | "catch_clause" | "ternary_expression" => 1,
            "binary_expression" => {
                let op = node.child_by_field_name("operator").map(|o| o.kind());
                u32::from(matches!(op, Some("&&" | "||" | "??")))
            }
            _ => 0,
        },
        Language::Rust => match kind {
            "if_expression" | "while_expression" | "for_expression" | "loop_expression"
            | "match_arm" | "try_expression" => 1,
            "binary_expression" => {
                let op = node.child_by_field_name("operator").map(|o| o.kind());
                u32::from(matches!(op, Some("&&" | "||")))
            }
            _ => 0,
        },
    };
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        n += decisions(lang, child, false);
    }
    // A match with k arms adds k-1 paths, not k.
    if lang == Language::Rust && kind == "match_block" && node.named_child_count() > 0 {
        n = n.saturating_sub(1);
    }
    n
}

fn is_callable_node(lang: Language, kind: &str) -> bool {
    match lang {
        Language::Python => matches!(kind, "function_definition" | "lambda"),
        Language::JavaScript | Language::TypeScript | Language::Tsx => matches!(
            kind,
            "function_declaration"
                | "generator_function_declaration"
                | "function_expression"
                | "arrow_function"
                | "method_definition"
        ),
        Language::Rust => matches!(kind, "function_item" | "closure_expression"),
    }
}

// ---------------------------------------------------------------- Python

fn python(ctx: &mut Ctx, node: Node, scope: &Scope) -> Option<Scope> {
    match node.kind() {
        "function_definition" => {
            let name = text(ctx, node.child_by_field_name("name")?);
            let params = node
                .child_by_field_name("parameters")
                .map(|p| text(ctx, p))
                .unwrap_or("()");
            let kind = if scope.in_type {
                SymbolKind::Method
            } else {
                SymbolKind::Function
            };
            let is_test = name.starts_with("test")
                && (ctx.test_file || scope.path.split('.').any(|p| p.starts_with("Test")));
            let sig = collapse_ws(&format!("{name}{params}"));
            let body = node.child_by_field_name("body");
            Some(add_symbol(ctx, node, body, name, kind, sig, is_test, scope))
        }
        "class_definition" => {
            let name = text(ctx, node.child_by_field_name("name")?);
            let bases = node
                .child_by_field_name("superclasses")
                .map(|p| text(ctx, p))
                .unwrap_or("");
            let sig = collapse_ws(&format!("class {name}{bases}"));
            Some(add_symbol(
                ctx,
                node,
                None,
                name,
                SymbolKind::Class,
                sig,
                false,
                scope,
            ))
        }
        "call" => {
            let f = node.child_by_field_name("function")?;
            match f.kind() {
                "identifier" => add_call(ctx, scope, text(ctx, f), None, node),
                "attribute" => {
                    let attr = f.child_by_field_name("attribute").map(|a| text(ctx, a));
                    let obj = f.child_by_field_name("object").map(|o| text(ctx, o));
                    add_call(ctx, scope, attr.unwrap_or_default(), obj, node);
                }
                _ => {}
            }
            None
        }
        "import_statement" => {
            let mut cursor = node.walk();
            for n in node.children_by_field_name("name", &mut cursor) {
                let (module, alias) = match n.kind() {
                    "aliased_import" => (
                        n.child_by_field_name("name")
                            .map(|x| text(ctx, x))
                            .unwrap_or(""),
                        n.child_by_field_name("alias").map(|x| text(ctx, x)),
                    ),
                    _ => (text(ctx, n), None),
                };
                let local = alias.unwrap_or_else(|| module.split('.').next().unwrap_or(module));
                ctx.out.imports.push(Import {
                    module: module.to_string(),
                    names: vec![(String::new(), local.to_string())],
                    line: line(node),
                });
            }
            None
        }
        "import_from_statement" => {
            let module = node
                .child_by_field_name("module_name")
                .map(|m| text(ctx, m))
                .unwrap_or("")
                .to_string();
            let mut names = Vec::new();
            let mut cursor = node.walk();
            for n in node.children_by_field_name("name", &mut cursor) {
                match n.kind() {
                    "aliased_import" => {
                        let name = n.child_by_field_name("name").map(|x| text(ctx, x));
                        let alias = n.child_by_field_name("alias").map(|x| text(ctx, x));
                        if let (Some(name), Some(alias)) = (name, alias) {
                            names.push((name.to_string(), alias.to_string()));
                        }
                    }
                    _ => {
                        let t = text(ctx, n).to_string();
                        names.push((t.clone(), t));
                    }
                }
            }
            ctx.out.imports.push(Import {
                module,
                names,
                line: line(node),
            });
            None
        }
        _ => None,
    }
}

// ---------------------------------------------------------------- JS / TS

const JS_TEST_FNS: &[&str] = &["it", "test"];
const JS_GROUP_FNS: &[&str] = &["describe", "suite", "context"];

fn js(ctx: &mut Ctx, node: Node, scope: &Scope) -> Option<Scope> {
    match node.kind() {
        "function_declaration" | "generator_function_declaration" => {
            let name = text(ctx, node.child_by_field_name("name")?);
            let sig = js_signature(ctx, name, node);
            let body = node.child_by_field_name("body");
            Some(add_symbol(
                ctx,
                node,
                body,
                name,
                SymbolKind::Function,
                sig,
                false,
                scope,
            ))
        }
        "method_definition" => {
            let name = text(ctx, node.child_by_field_name("name")?);
            let sig = js_signature(ctx, name, node);
            let body = node.child_by_field_name("body");
            Some(add_symbol(
                ctx,
                node,
                body,
                name,
                SymbolKind::Method,
                sig,
                false,
                scope,
            ))
        }
        "class_declaration" | "class" | "abstract_class_declaration" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| text(ctx, n))
                .unwrap_or("<anonymous>");
            let sig = format!("class {name}");
            Some(add_symbol(
                ctx,
                node,
                None,
                name,
                SymbolKind::Class,
                sig,
                false,
                scope,
            ))
        }
        "variable_declarator" => {
            let value = node.child_by_field_name("value")?;
            if !matches!(
                value.kind(),
                "arrow_function" | "function_expression" | "function"
            ) {
                return None;
            }
            let name_node = node.child_by_field_name("name")?;
            if name_node.kind() != "identifier" {
                return None;
            }
            let name = text(ctx, name_node);
            let sig = js_signature(ctx, name, value);
            let body = value.child_by_field_name("body");
            let kind = if scope.in_type {
                SymbolKind::Method
            } else {
                SymbolKind::Function
            };
            Some(add_symbol(ctx, node, body, name, kind, sig, false, scope))
        }
        "call_expression" => {
            let f = node.child_by_field_name("function")?;
            let (callee, receiver) = match f.kind() {
                "identifier" => (text(ctx, f), None),
                "member_expression" => (
                    f.child_by_field_name("property")
                        .map(|p| text(ctx, p))
                        .unwrap_or(""),
                    f.child_by_field_name("object").map(|o| text(ctx, o)),
                ),
                _ => ("", None),
            };
            // require('./x') behaves like an import.
            if callee == "require" {
                if let Some(arg) = first_string_arg(ctx, node) {
                    ctx.out.imports.push(Import {
                        module: arg,
                        names: Vec::new(),
                        line: line(node),
                    });
                }
                return None;
            }
            // describe('group', () => {...}) / it('does x', () => {...})
            let base = receiver.map_or(callee, |r| r.split('.').next().unwrap_or(r));
            let is_test_call = JS_TEST_FNS.contains(&base) || JS_TEST_FNS.contains(&callee);
            let is_group = JS_GROUP_FNS.contains(&base) || JS_GROUP_FNS.contains(&callee);
            if (is_test_call || is_group)
                && let Some(title) = first_string_arg(ctx, node)
            {
                let kind = if is_test_call {
                    SymbolKind::Function
                } else {
                    SymbolKind::Module
                };
                let body = node
                    .child_by_field_name("arguments")
                    .and_then(|a| a.named_child(1))
                    .and_then(|f| f.child_by_field_name("body"));
                let sig = format!("{callee}({title:?})");
                let mut s = add_symbol(ctx, node, body, &title, kind, sig, is_test_call, scope);
                s.in_test_group = true;
                return Some(s);
            }
            add_call(ctx, scope, callee, receiver, node);
            None
        }
        "new_expression" => {
            if let Some(c) = node.child_by_field_name("constructor") {
                add_call(ctx, scope, text(ctx, c), None, node);
            }
            None
        }
        "import_statement" => {
            let source = node.child_by_field_name("source")?;
            let module = strip_quotes(text(ctx, source));
            let mut names = Vec::new();
            collect_js_import_names(ctx, node, &mut names);
            ctx.out.imports.push(Import {
                module,
                names,
                line: line(node),
            });
            None
        }
        _ => None,
    }
}

fn js_signature(ctx: &Ctx, name: &str, node: Node) -> String {
    let params = node
        .child_by_field_name("parameters")
        .or_else(|| node.child_by_field_name("parameter"))
        .map(|p| text(ctx, p))
        .unwrap_or("()");
    let ret = node
        .child_by_field_name("return_type")
        .map(|r| text(ctx, r))
        .unwrap_or("");
    collapse_ws(&format!("{name}{params}{ret}"))
}

fn first_string_arg(ctx: &Ctx, call: Node) -> Option<String> {
    let args = call.child_by_field_name("arguments")?;
    let first = args.named_child(0)?;
    matches!(first.kind(), "string" | "template_string").then(|| strip_quotes(text(ctx, first)))
}

fn strip_quotes(s: &str) -> String {
    s.trim_matches(|c| c == '"' || c == '\'' || c == '`')
        .to_string()
}

fn collect_js_import_names(ctx: &Ctx, node: Node, out: &mut Vec<(String, String)>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "import_specifier" => {
                let name = child.child_by_field_name("name").map(|n| text(ctx, n));
                let alias = child.child_by_field_name("alias").map(|n| text(ctx, n));
                if let Some(name) = name {
                    out.push((name.to_string(), alias.unwrap_or(name).to_string()));
                }
            }
            "identifier" => {
                // default import
                let t = text(ctx, child).to_string();
                out.push(("default".into(), t));
            }
            "namespace_import" => {
                if let Some(id) = child.named_child(0) {
                    out.push((String::new(), text(ctx, id).to_string()));
                }
            }
            _ => collect_js_import_names(ctx, child, out),
        }
    }
}

// ---------------------------------------------------------------- Rust

fn rust(ctx: &mut Ctx, node: Node, scope: &Scope) -> Option<Scope> {
    match node.kind() {
        "function_item" => {
            let name = text(ctx, node.child_by_field_name("name")?);
            let params = node
                .child_by_field_name("parameters")
                .map(|p| text(ctx, p))
                .unwrap_or("()");
            let ret = node
                .child_by_field_name("return_type")
                .map(|r| format!(" -> {}", text(ctx, r)))
                .unwrap_or_default();
            let sig = collapse_ws(&format!("fn {name}{params}{ret}"));
            let kind = if scope.in_type {
                SymbolKind::Method
            } else {
                SymbolKind::Function
            };
            let is_test = has_test_attribute(ctx, node);
            let body = node.child_by_field_name("body");
            Some(add_symbol(ctx, node, body, name, kind, sig, is_test, scope))
        }
        "impl_item" => {
            let ty = node
                .child_by_field_name("type")
                .map(|t| text(ctx, t))
                .unwrap_or("?");
            let name = generic_base(ty);
            let sig = match node.child_by_field_name("trait") {
                Some(t) => format!("impl {} for {ty}", text(ctx, t)),
                None => format!("impl {ty}"),
            };
            // Methods are qualified by their type (Type.method) without an extra impl level.
            let mut s = add_symbol(ctx, node, None, &name, SymbolKind::Impl, sig, false, scope);
            s.in_type = true;
            Some(s)
        }
        "struct_item" | "enum_item" | "trait_item" | "union_item" => {
            let name = text(ctx, node.child_by_field_name("name")?);
            let kind = match node.kind() {
                "enum_item" => SymbolKind::Enum,
                "trait_item" => SymbolKind::Trait,
                _ => SymbolKind::Struct,
            };
            let sig = collapse_ws(&format!("{} {name}", node.kind().trim_end_matches("_item")));
            Some(add_symbol(ctx, node, None, name, kind, sig, false, scope))
        }
        "mod_item" => {
            let name = text(ctx, node.child_by_field_name("name")?);
            node.child_by_field_name("body")?;
            let mut s = add_symbol(
                ctx,
                node,
                None,
                name,
                SymbolKind::Module,
                format!("mod {name}"),
                false,
                scope,
            );
            s.in_type = false;
            s.in_test_group = name == "tests" || has_cfg_test(ctx, node);
            Some(s)
        }
        "call_expression" => {
            let f = node.child_by_field_name("function")?;
            match f.kind() {
                "identifier" => add_call(ctx, scope, text(ctx, f), None, node),
                "scoped_identifier" => {
                    let name = f.child_by_field_name("name").map(|n| text(ctx, n));
                    let path = f.child_by_field_name("path").map(|p| text(ctx, p));
                    add_call(ctx, scope, name.unwrap_or_default(), path, node);
                }
                "field_expression" => {
                    let field = f.child_by_field_name("field").map(|n| text(ctx, n));
                    let value = f.child_by_field_name("value").map(|v| text(ctx, v));
                    add_call(ctx, scope, field.unwrap_or_default(), value, node);
                }
                "generic_function" => {
                    if let Some(inner) = f.child_by_field_name("function") {
                        let t = text(ctx, inner);
                        let (recv, name) =
                            t.rsplit_once("::").map_or((None, t), |(r, n)| (Some(r), n));
                        add_call(ctx, scope, name, recv, node);
                    }
                }
                _ => {}
            }
            None
        }
        "macro_invocation" => {
            // Arguments of `assert_eq!(f(x), 1)` are an opaque token tree: scan it for calls.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "token_tree" {
                    macro_calls(ctx, child, scope);
                }
            }
            None
        }
        "use_declaration" => {
            let arg = node.child_by_field_name("argument")?;
            let mut items = Vec::new();
            rust_use_paths(ctx, arg, String::new(), &mut items);
            for (module, name, alias) in items {
                ctx.out.imports.push(Import {
                    module,
                    names: vec![(name, alias)],
                    line: line(node),
                });
            }
            None
        }
        _ => None,
    }
}

/// Finds `name(`, `recv.name(` and `path::name(` patterns inside a macro token tree.
fn macro_calls(ctx: &mut Ctx, tree: Node, scope: &Scope) {
    let mut cursor = tree.walk();
    let toks: Vec<Node> = tree.children(&mut cursor).collect();
    for (i, t) in toks.iter().enumerate() {
        if t.kind() == "token_tree" {
            macro_calls(ctx, *t, scope);
            // identifier immediately followed by a parenthesised token tree = call
            if i > 0
                && toks[i - 1].kind() == "identifier"
                && text(ctx, *t).starts_with('(')
                && !(i > 1 && toks[i - 2].kind() == "!")
            {
                let name = text(ctx, toks[i - 1]);
                let receiver = if i >= 3 && matches!(text(ctx, toks[i - 2]), "." | "::") {
                    Some(text(ctx, toks[i - 3]))
                } else {
                    None
                };
                add_call(ctx, scope, name, receiver, toks[i - 1]);
            }
        }
    }
}

fn generic_base(ty: &str) -> String {
    ty.split('<').next().unwrap_or(ty).trim().to_string()
}

fn has_test_attribute(ctx: &Ctx, node: Node) -> bool {
    let mut prev = node.prev_named_sibling();
    while let Some(p) = prev {
        if p.kind() != "attribute_item" {
            break;
        }
        let t = text(ctx, p);
        if t.contains("test") && !t.contains("cfg(test)") || t.contains("::test") {
            return true;
        }
        prev = p.prev_named_sibling();
    }
    false
}

fn has_cfg_test(ctx: &Ctx, node: Node) -> bool {
    node.prev_named_sibling()
        .is_some_and(|p| p.kind() == "attribute_item" && text(ctx, p).contains("cfg(test)"))
}

/// Flattens `use a::{b, c as d}` into (module, name, alias) triples.
fn rust_use_paths(ctx: &Ctx, node: Node, prefix: String, out: &mut Vec<(String, String, String)>) {
    let joined = |p: &str, s: &str| {
        if p.is_empty() {
            s.to_string()
        } else {
            format!("{p}::{s}")
        }
    };
    match node.kind() {
        "scoped_identifier" | "identifier" | "crate" | "self" | "super" => {
            let full = joined(&prefix, text(ctx, node));
            let (module, name) = full
                .rsplit_once("::")
                .map_or((String::new(), full.clone()), |(m, n)| {
                    (m.to_string(), n.to_string())
                });
            out.push((module, name.clone(), name));
        }
        "use_as_clause" => {
            let path = node
                .child_by_field_name("path")
                .map(|p| text(ctx, p))
                .unwrap_or("");
            let alias = node
                .child_by_field_name("alias")
                .map(|a| text(ctx, a))
                .unwrap_or("");
            let full = joined(&prefix, path);
            let (module, name) = full
                .rsplit_once("::")
                .map_or((String::new(), full.clone()), |(m, n)| {
                    (m.to_string(), n.to_string())
                });
            out.push((module, name, alias.to_string()));
        }
        "scoped_use_list" => {
            let path = node
                .child_by_field_name("path")
                .map(|p| text(ctx, p))
                .unwrap_or("");
            let new_prefix = joined(&prefix, path);
            if let Some(list) = node.child_by_field_name("list") {
                rust_use_paths(ctx, list, new_prefix, out);
            }
        }
        "use_list" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                rust_use_paths(ctx, child, prefix.clone(), out);
            }
        }
        "use_wildcard" => {
            let path = text(ctx, node)
                .trim_end_matches("::*")
                .trim_end_matches('*');
            out.push((
                joined(&prefix, path.trim_end_matches("::")),
                "*".into(),
                "*".into(),
            ));
        }
        _ => {}
    }
}
