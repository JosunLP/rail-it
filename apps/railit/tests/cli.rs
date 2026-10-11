//! Ende-zu-Ende-Tests der Kommandozeile.

use std::path::PathBuf;
use std::process::{Command, Output};

fn railit(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_railit"))
        .args(args)
        .output()
        .unwrap()
}

fn example() -> String {
    concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/beispielstadt.toml"
    )
    .to_string()
}

fn temp_file(name: &str, content: &str) -> PathBuf {
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::write(&path, content).unwrap();
    path
}

#[test]
fn validate_example_succeeds() {
    let out = railit(&["net", "validate", &example()]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("„Beispielstadt“ ist gültig"), "{stdout}");
}

#[test]
fn validate_invalid_network_reports_json_and_exit_code_1() {
    let path = temp_file(
        "offen.toml",
        "[network]\nname = \"offen\"\n[[track]]\nname = \"1\"\nlength_m = 10\n[[tvd]]\nname = \"T\"\nelements = [\"1\"]\n",
    );
    let out = railit(&[
        "net",
        "validate",
        "--output",
        "json",
        path.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(1));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["valid"], false);
    let errors = json["errors"].as_array().unwrap();
    assert_eq!(errors.len(), 2, "{json:#}");
    assert!(errors.iter().all(|e| e["requirement"] == "REQ-TOP-002"));
}

#[test]
fn info_json_lists_elements() {
    let out = railit(&["net", "info", "-o", "json", &example()]);
    assert_eq!(out.status.code(), Some(0));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["network"], "Beispielstadt");
    assert_eq!(json["tracks"].as_array().unwrap().len(), 4);
    assert_eq!(json["signals"][0]["name"], "A");
    assert_eq!(json["signals"][0]["position_m"], 1000.0);
    assert_eq!(json["tvds"][1]["elements"][0], "W1");
}

#[test]
fn missing_file_exits_with_2() {
    let out = railit(&["net", "validate", "/gibt/es/nicht.toml"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        String::from_utf8(out.stderr)
            .unwrap()
            .contains("kann nicht gelesen werden")
    );
}
