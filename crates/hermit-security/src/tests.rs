use super::*;
use hermit_core::Language;

const BASE: &str = "def get(db, org_id):\n    query = f\"SELECT * FROM accounts WHERE id = '{org_id}'\"\n    return db.execute(query).fetchone()\n";
const HEAD: &str = "def get(db, org_id):\n    query = \"SELECT * FROM accounts WHERE id = :id\"\n    result = db.execute(query, {\"id\": org_id}).fetchone()\n    token = \"ghp_abcdefghijklmnopqrstuvwxyz0123456789\"\n    return result\n";

fn change() -> (Vec<ChangedFile>, Sources) {
    let p = PathBuf::from("services/billing.py");
    let file = ChangedFile {
        path: p.clone(),
        old_path: None,
        status: FileStatus::Modified,
        language: Some(Language::Python),
        added: vec![LineRange::new(2, 5)],
        removed: vec![LineRange::new(2, 3)],
        additions: 4,
        deletions: 2,
    };
    let mut src = Sources::default();
    src.base.insert(p.clone(), BASE.into());
    src.head.insert(p, HEAD.into());
    (vec![file], src)
}

#[test]
fn builtin_detects_fix_and_new_secret() {
    let (files, src) = change();
    let findings = Scanner::builtin_only().scan(&files, &src).unwrap();
    let secret = findings
        .iter()
        .find(|f| f.rule_id == "hermit.secret.github-token")
        .unwrap();
    assert_eq!(secret.state, FindingState::New);
    assert_eq!(secret.line, 4);
    let fixed = findings
        .iter()
        .find(|f| f.cwe.as_deref() == Some("CWE-89"))
        .unwrap();
    assert_eq!(fixed.state, FindingState::Fixed);
    assert_eq!(fixed.line, 2);
}

#[test]
fn semgrep_detects_fix() {
    let scanner = Scanner::detect(Vec::new());
    if scanner.engine() != "semgrep" {
        eprintln!("semgrep not installed; skipping");
        return;
    }
    let (files, src) = change();
    let findings = scanner.scan(&files, &src).unwrap();
    let fixed = findings
        .iter()
        .find(|f| f.rule_id == "hermit.python.sql-injection")
        .expect("semgrep sql finding");
    assert_eq!(fixed.state, FindingState::Fixed);
    assert_eq!(fixed.severity, Severity::Critical);
    assert_eq!(fixed.cwe.as_deref(), Some("CWE-89"));
    assert!(
        !findings
            .iter()
            .any(|f| f.state == FindingState::New && f.cwe.as_deref() == Some("CWE-89"))
    );
}
