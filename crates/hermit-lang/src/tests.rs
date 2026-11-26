use super::*;
use hermit_core::{FileStatus, LineRange, SymbolKind};
use std::path::PathBuf;

fn names(p: &ParsedFile) -> Vec<&str> {
    p.symbols.iter().map(|s| s.qualified.as_str()).collect()
}

#[test]
fn python_symbols_calls_imports() {
    let src = r#"
from services.cache import invalidate_cache as inv
import os.path

class BillingService(Base):
    def charge(self, org_id, amount):
        if amount > 0 and org_id:
            inv(f"org:{org_id}")
        for x in range(3):
            pass
        return self.db.execute("q")

def helper():
    return BillingService().charge(1, 2)
"#;
    let p = parse(Language::Python, Path::new("services/billing.py"), src).unwrap();
    assert_eq!(
        names(&p),
        vec![
            "services/billing.py::BillingService",
            "services/billing.py::BillingService.charge",
            "services/billing.py::helper",
        ]
    );
    let charge = &p.symbols[1];
    assert_eq!(charge.kind, SymbolKind::Method);
    assert_eq!(charge.signature, "charge(self, org_id, amount)");
    // 1 + if + `and` + for = 4
    assert_eq!(charge.complexity, 4);
    assert_eq!(charge.range, LineRange::new(6, 11));

    let inv = p.calls.iter().find(|c| c.callee == "inv").unwrap();
    assert_eq!(
        inv.caller.as_deref(),
        Some("services/billing.py::BillingService.charge")
    );
    let exec = p.calls.iter().find(|c| c.callee == "execute").unwrap();
    assert_eq!(exec.receiver.as_deref(), Some("self.db"));

    assert_eq!(p.imports[0].module, "services.cache");
    assert_eq!(
        p.imports[0].names,
        vec![("invalidate_cache".to_string(), "inv".to_string())]
    );
    assert_eq!(p.imports[1].module, "os.path");
}

#[test]
fn python_tests_detected() {
    let src = "def test_charge():\n    assert charge(1)\n\nclass TestX:\n    def test_a(self):\n        pass\n";
    let p = parse(Language::Python, Path::new("tests/test_billing.py"), src).unwrap();
    assert!(
        p.symbols
            .iter()
            .filter(|s| s.name.starts_with("test"))
            .all(|s| s.is_test)
    );
    let p = parse(
        Language::Python,
        Path::new("app.py"),
        "def testify():\n  pass\n",
    )
    .unwrap();
    assert!(!p.symbols[0].is_test);
}

#[test]
fn typescript_symbols_and_tests() {
    let src = r#"
import { render as r, html } from './view';
import db from "../db";

export class UserService {
  async find(id: string): Promise<User> {
    return id ? db.query(id) : null;
  }
}

export const sanitize = (s: string) => s.replace(/</g, '');

function main() { new UserService().find("1"); r(html); }

describe('UserService', () => {
  it('finds a user', () => {
    expect(new UserService().find('1')).toBeTruthy();
  });
});
"#;
    let p = parse(Language::TypeScript, Path::new("src/user.test.ts"), src).unwrap();
    let n = names(&p);
    assert!(n.contains(&"src/user.test.ts::UserService.find"));
    assert!(n.contains(&"src/user.test.ts::sanitize"));
    assert!(n.contains(&"src/user.test.ts::main"));
    assert!(n.contains(&"src/user.test.ts::UserService.finds a user"));
    let find = p.symbols.iter().find(|s| s.name == "find").unwrap();
    assert_eq!(find.complexity, 2);
    assert_eq!(find.signature, "find(id: string): Promise<User>");
    let t = p.symbols.iter().find(|s| s.is_test).unwrap();
    assert_eq!(t.name, "finds a user");
    let call_in_test = p
        .calls
        .iter()
        .find(|c| c.callee == "find" && c.caller.as_deref() == Some(t.qualified.as_str()));
    assert!(call_in_test.is_some());
    assert_eq!(p.imports[0].module, "./view");
    assert!(p.imports[0].names.contains(&("render".into(), "r".into())));
    assert_eq!(p.imports[1].names, vec![("default".into(), "db".into())]);
}

#[test]
fn rust_symbols_calls_tests() {
    let src = r#"
use crate::cache::{invalidate, Store as S};

pub struct Billing;

impl Billing {
    pub fn charge(&self, amount: u64) -> Result<(), E> {
        match amount {
            0 => return Err(E),
            1..=10 => {}
            _ => {}
        }
        if amount > 5 && self.ok() { invalidate("k"); }
        S::flush();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn charges() { super::Billing.charge(1).unwrap(); }
}
"#;
    let p = parse(Language::Rust, Path::new("src/billing.rs"), src).unwrap();
    let n = names(&p);
    assert!(n.contains(&"src/billing.rs::Billing.charge"), "{n:?}");
    let charge = p.symbols.iter().find(|s| s.name == "charge").unwrap();
    assert_eq!(charge.kind, SymbolKind::Method);
    assert_eq!(
        charge.signature,
        "fn charge(&self, amount: u64) -> Result<(), E>"
    );
    // 1 + match(3 arms -> 2) + if + && = 5
    assert_eq!(charge.complexity, 5);
    let t = p.symbols.iter().find(|s| s.name == "charges").unwrap();
    assert!(t.is_test);
    // #[test] attribute line belongs to the test function.
    assert_eq!(
        t.range.start + 1,
        p.symbols
            .iter()
            .find(|s| s.name == "charges")
            .unwrap()
            .range
            .end
    );
    assert!(
        p.calls
            .iter()
            .any(|c| c.callee == "flush" && c.receiver.as_deref() == Some("S"))
    );
    assert!(
        p.imports
            .iter()
            .any(|i| i.module == "crate::cache" && i.names[0] == ("Store".into(), "S".into()))
    );
}

#[test]
fn rust_calls_inside_macros() {
    let src = "#[test]\nfn t() {\n    assert_eq!(checksum(&r, 2), 3);\n    assert!(r.read_at(5).is_none());\n}\n";
    let p = parse(Language::Rust, Path::new("src/lib.rs"), src).unwrap();
    assert!(
        p.calls.iter().any(|c| c.callee == "checksum"),
        "{:?}",
        p.calls
    );
    assert!(
        p.calls
            .iter()
            .any(|c| c.callee == "read_at" && c.receiver.as_deref() == Some("r"))
    );
    assert!(!p.calls.iter().any(|c| c.callee == "assert_eq"));
}

#[test]
fn doc_comments_extend_symbol_range() {
    let src = "impl R {\n    /// Docs.\n    pub fn f(&self) {}\n}\n";
    let p = parse(Language::Rust, Path::new("a.rs"), src).unwrap();
    let f = p.symbols.iter().find(|s| s.name == "f").unwrap();
    assert_eq!(f.range, LineRange::new(2, 3));
}

#[test]
fn diff_maps_lines_to_innermost_symbol() {
    let base_src = "class A:\n    def f(self, x):\n        return x\n\n    def g(self):\n        return 1\n\ndef old():\n    pass\n";
    let head_src = "class A:\n    def f(self, x, y):\n        if y:\n            return y\n        return x\n\n    def g(self):\n        return 1\n";
    let path = PathBuf::from("a.py");
    let base = parse(Language::Python, &path, base_src).unwrap();
    let head = parse(Language::Python, &path, head_src).unwrap();
    let file = ChangedFile {
        path: path.clone(),
        old_path: None,
        status: FileStatus::Modified,
        language: Some(Language::Python),
        added: vec![LineRange::new(2, 4)],
        removed: vec![LineRange::new(2, 2), LineRange::new(7, 9)],
        additions: 3,
        deletions: 4,
    };
    let changed = diff_symbols(&file, Some(&base), Some(&head));
    let f = changed.iter().find(|c| c.symbol.name == "f").unwrap();
    assert_eq!(f.change, ChangeKind::Modified);
    assert!(f.signature_changed);
    assert_eq!((f.complexity_before, f.complexity_after), (1, 2));
    let old = changed.iter().find(|c| c.symbol.name == "old").unwrap();
    assert_eq!(old.change, ChangeKind::Deleted);
    assert!(!changed.iter().any(|c| c.symbol.name == "g"));
    assert!(!changed.iter().any(|c| c.symbol.name == "A"));
}
