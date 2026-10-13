//! Führt alle Betriebsszenarien unter `tests/scenarios/` aus.

use std::path::PathBuf;

use railit_runtime::railit_safety::Redundancy;
use railit_runtime::scenario::RunOptions;

#[test]
fn all_scenarios_pass() {
    run_all(RunOptions {
        journal: true,
        ..RunOptions::default()
    });
}

/// Alle Läufe schreiben jeden Bereich im Journal mit; am Ende muss das Replay genau den
/// Endzustand ergeben (REQ-JRN-002, REQ-JRN-006, REQ-JRN-007).
///
/// REQ-SAF-001: Mit drei fehlerfreien Kanälen verhält sich das Stellwerk genau wie mit einem.
#[test]
fn all_scenarios_pass_with_three_channels() {
    run_all(RunOptions {
        channels: Some(Redundancy::TwoOutOfThree),
        node: false,
        journal: true,
    });
}

/// REQ-EMB-005: Jeder Stellbereich rechnet auf einem Knoten des sicheren Rechners, angebunden
/// über RaSTA mit Spiegel und Zustandsabgleich – alle Szenarien verhalten sich wie im Dienst.
#[test]
fn all_scenarios_pass_on_safe_computer_nodes() {
    run_all(RunOptions {
        channels: None,
        node: true,
        journal: true,
    });
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

fn run_all(opts: RunOptions) {
    let mut failed = Vec::new();
    for file in &scenario_files() {
        let outcome = railit_runtime::scenario::run_file_opts(file, opts)
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
