//! Führt alle Betriebsszenarien unter `tests/scenarios/` aus.

use std::path::PathBuf;

#[test]
fn all_scenarios_pass() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/scenarios");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "keine Szenarien gefunden");

    let mut failed = Vec::new();
    for file in &files {
        let outcome = railit_runtime::scenario::run_file(file)
            .unwrap_or_else(|e| panic!("{}: {e}", file.display()));
        println!("{outcome}");
        if !outcome.passed {
            failed.push(outcome.to_string());
        }
    }
    assert!(
        failed.is_empty(),
        "fehlgeschlagene Szenarien:\n{}",
        failed.join("\n")
    );
}
