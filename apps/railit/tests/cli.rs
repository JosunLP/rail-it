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

fn tmp_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn keygen_compile_verify_info() {
    let dir = tmp_dir("plan-flow");
    let prefix = dir.join("railit");
    let s = |p: &PathBuf| p.to_str().unwrap().to_string();

    assert_eq!(
        railit(&["plan", "keygen", &s(&prefix)]).status.code(),
        Some(0)
    );
    assert_eq!(
        railit(&["plan", "keygen", &s(&prefix)]).status.code(),
        Some(2),
        "kein Überschreiben"
    );

    let plan = dir.join("bs.rplan");
    let key = s(&prefix.with_extension("key"));
    let public = s(&prefix.with_extension("pub"));
    let out = railit(&[
        "plan",
        "compile",
        &example(),
        "--out",
        &s(&plan),
        "--key",
        &key,
        "-o",
        "json",
    ]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        (json["routes"].as_u64(), json["signed"].as_bool()),
        (Some(8), Some(true))
    );

    let out = railit(&[
        "plan",
        "verify",
        &s(&plan),
        "--key",
        &public,
        "--network",
        &example(),
    ]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let out = railit(&["plan", "info", &s(&plan), "-o", "json"]);
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["routes"][0]["name"], "A-N1");
    assert_eq!(json["routes"][0]["conflicts"].as_array().unwrap().len(), 6);

    // Verfälschtes Image wird abgewiesen.
    let mut bytes = std::fs::read(&plan).unwrap();
    bytes[100] ^= 0x01;
    let bad = dir.join("bad.rplan");
    std::fs::write(&bad, bytes).unwrap();
    assert_eq!(railit(&["plan", "verify", &s(&bad)]).status.code(), Some(1));

    // Fremder Schlüssel wird abgewiesen.
    let other = dir.join("other");
    railit(&["plan", "keygen", &s(&other)]);
    let out = railit(&[
        "plan",
        "verify",
        &s(&plan),
        "--key",
        &s(&other.with_extension("pub")),
    ]);
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn scenarios_run_from_cli() {
    let scenario = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/scenarios/01_einfahrt_gleis1.toml"
    );
    let out = railit(&["scenario", "run", scenario]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(
        String::from_utf8(out.stdout)
            .unwrap()
            .contains("✔ Einfahrt in Gleis 1")
    );

    let dir = tmp_dir("scenario-fail");
    let failing = dir.join("fail.toml");
    std::fs::write(
        &failing,
        format!(
            "[scenario]\nname = \"absichtlich falsch\"\nnetwork = \"{}\"\n[[step]]\nset_route = \"A-N1\"\nexpect = {{ route = {{ \"A-N1\" = \"proceed\" }} }}\n",
            example().replace('\\', "/")
        ),
    )
    .unwrap();
    let out = railit(&["scenario", "run", failing.to_str().unwrap(), "-o", "json"]);
    assert_eq!(out.status.code(), Some(1));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["passed"], false);
}

#[test]
fn online_commands_report_unreachable_server() {
    let out = railit(&["--server", "http://127.0.0.1:1", "state"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        String::from_utf8(out.stderr)
            .unwrap()
            .contains("nicht erreichbar")
    );
}

/// REQ-JRN-001/002: Journal prüfen und abspielen über die Kommandozeile.
#[test]
fn journal_verify_and_replay() {
    let dir = tmp_dir("journal");
    let plan = dir.join("bs.rplan");
    let out = railit(&[
        "plan",
        "compile",
        &example(),
        "--out",
        plan.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(0));

    // Journal eines kurzen Betriebs erzeugen.
    let net = railit_import::toml_format::parse(&std::fs::read_to_string(example()).unwrap())
        .unwrap()
        .topology;
    let view = railit_runtime::load_plan(std::fs::read(&plan).unwrap(), None).unwrap();
    let route = view.find_route("A-N1").unwrap();
    let mut rt = railit_runtime::Runtime::new(
        view,
        railit_rules::GenericRules::default(),
        railit_sim::Simulator::new(net, railit_sim::SimConfig::default()),
        railit_types::Duration::from_millis(100),
    );
    let journal = dir.join("bs.journal");
    let _ = std::fs::remove_file(&journal);
    rt.start_journal(0, Box::new(std::fs::File::create(&journal).unwrap()), 10)
        .unwrap();
    rt.cycle(&[(
        railit_kernel::RequestId(1),
        railit_kernel::Request::SetRoute(route),
    )]);
    for _ in 0..99 {
        rt.cycle(&[]);
    }
    drop(rt);

    let out = railit(&["journal", "verify", journal.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("100 Zyklen"), "{stdout}");

    let out = railit(&[
        "--output",
        "json",
        "journal",
        "replay",
        journal.to_str().unwrap(),
        "--plan",
        plan.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["checkpoints"], 10);

    // REQ-JRN-008: Zustand zu einem Zeitpunkt und Ereignisse eines Zeitfensters.
    let (j, p) = (journal.to_str().unwrap(), plan.to_str().unwrap());
    let out = railit(&["journal", "state", j, "--plan", p, "--at", "8"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("A-N1 (Fahrt)"), "{stdout}");
    assert!(stdout.contains("W1 links"), "{stdout}");
    let out = railit(&[
        "--output", "json", "journal", "events", j, "--plan", p, "--to", "1",
    ]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let rows = v.as_array().unwrap();
    assert!(rows.iter().all(|r| r["now_ms"].as_u64().unwrap() <= 1000));
    assert!(
        rows.iter()
            .flat_map(|r| r["events"].as_array().unwrap())
            .any(|e| e["type"] == "accepted"),
        "{v}"
    );
    assert!(
        rows.iter()
            .flat_map(|r| r["commands"].as_array().unwrap())
            .any(|c| c["type"] == "move_point" && c["point"] == "W1"),
        "{v}"
    );

    // Manipuliert: Kette bricht, Exit-Code 1.
    let text = std::fs::read_to_string(&journal).unwrap();
    let tampered = dir.join("tampered.journal");
    std::fs::write(&tampered, text.replacen("\"right\"", "\"left\"", 1)).unwrap();
    let out = railit(&["journal", "verify", tampered.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("Hashkette in Zeile"), "{stdout}");
}

/// Netz mit Stellbereichen: je Bereich ein Plan-Image, jedes einzeln gegen sein Teilnetz
/// geprüft.
#[test]
fn compile_and_verify_area_network() {
    let network = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/nordbahn.toml");
    let dir = tmp_dir("area-plans");
    let out = railit(&[
        "plan",
        "compile",
        network,
        "--out",
        dir.to_str().unwrap(),
        "-o",
        "json",
    ]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let areas: Vec<&str> = json["areas"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["area"].as_str().unwrap())
        .collect();
    assert_eq!(areas, ["Adorf", "Strecke", "Bstadt"]);
    for area in areas {
        let plan = dir.join(format!("{area}.rplan"));
        let out = railit(&[
            "plan",
            "verify",
            plan.to_str().unwrap(),
            "--network",
            network,
        ]);
        assert_eq!(
            out.status.code(),
            Some(0),
            "{area}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    // Ein Bereichsplan passt nicht zum Netz eines anderen Bereichs.
    let wrong = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/beispielstadt.toml"
    );
    let plan = dir.join("Adorf.rplan");
    let out = railit(&["plan", "verify", plan.to_str().unwrap(), "--network", wrong]);
    assert_eq!(out.status.code(), Some(1));
}

/// REQ-SEC-001, REQ-SEC-004: Token erzeugen, Auditprotokoll prüfen.
#[test]
fn token_and_audit_verification() {
    let out = railit(&["auth", "token", "fdl-adorf", "-o", "json"]);
    assert_eq!(out.status.code(), Some(0));
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let token = json["token"].as_str().unwrap();
    let hash = json["token_sha256"].as_str().unwrap();
    assert!(token.starts_with("rit_") && token.len() == 68);
    assert_eq!(hash.len(), 64);
    let again = railit(&["auth", "token", "x", "-o", "json"]);
    assert_ne!(again.stdout, out.stdout, "Token sind zufällig");

    let dir = tmp_dir("audit");
    let path = dir.join("audit.log");
    let mut w = railit_journal::audit::AuditWriter::open(&path).unwrap();
    for action in ["set_route", "cancel_route"] {
        w.append(railit_journal::audit::AuditRecord {
            seq: 0,
            time_ms: 1,
            user: "fdl".into(),
            action: action.into(),
            target: Some("A-N1".into()),
            area: None,
            outcome: "ok".into(),
        })
        .unwrap();
    }
    drop(w);
    let out = railit(&["audit", "verify", path.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).contains("2 Einträge"));

    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, text.replacen("cancel_route", "set_route", 1)).unwrap();
    let out = railit(&["audit", "verify", path.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stdout).contains("UNGÜLTIG"));
}

#[test]
fn tls_options_are_checked() {
    let out = railit(&[
        "--server",
        "https://127.0.0.1:1",
        "--ca",
        "/gibt/es/nicht.pem",
        "state",
    ]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("nicht.pem"));
}

/// REQ-TOP-014: Export als railML 3, Prüfung mit Importwarnungen, Projektierung aus railML.
#[test]
fn railml_export_validate_compile() {
    let dir = tmp_dir("railml");
    let s = |p: &PathBuf| p.to_str().unwrap().to_string();
    let xml = dir.join("beispielstadt.railml");
    let out = railit(&["net", "export", &example(), "--out", &s(&xml)]);
    assert_eq!(out.status.code(), Some(0));
    assert!(std::fs::read_to_string(&xml).unwrap().contains("<railML"));

    let out = railit(&["--output", "json", "net", "validate", &s(&xml)]);
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["valid"], true);

    let handmade = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../crates/railit-import/tests/data/abzweig.railml"
    );
    let out = railit(&["--output", "json", "net", "validate", handmade]);
    assert_eq!(out.status.code(), Some(0));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(
        v["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w.as_str().unwrap().contains("sig_lf7")),
        "{v}"
    );

    let prefix = dir.join("railit");
    assert_eq!(
        railit(&["plan", "keygen", &s(&prefix)]).status.code(),
        Some(0)
    );
    let plan = dir.join("abzweig.rplan");
    let out = railit(&[
        "plan",
        "compile",
        handmade,
        "--out",
        &s(&plan),
        "--key",
        &s(&dir.join("railit.key")),
    ]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
