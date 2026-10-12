//! Führt alle Betriebsszenarien unter `tests/scenarios/` aus.

use std::path::PathBuf;

use railit_runtime::railit_safety::Redundancy;

#[test]
fn all_scenarios_pass() {
    run_all(None);
}

/// REQ-SAF-001: Mit drei fehlerfreien Kanälen verhält sich das Stellwerk genau wie mit einem.
#[test]
fn all_scenarios_pass_with_three_channels() {
    run_all(Some(Redundancy::TwoOutOfThree));
}

fn scenario_files() -> Vec<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/scenarios");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "keine Szenarien gefunden");
    files
}

fn run_all(channels: Option<Redundancy>) {
    let mut failed = Vec::new();
    for file in &scenario_files() {
        let outcome = railit_runtime::scenario::run_file_with(file, channels)
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
