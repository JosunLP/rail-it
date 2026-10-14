//! Ende-zu-Ende: Feldrechner (`railit-field`) und Stellwerksdienst (`railitd --field remote`)
//! als Prozesse, RaSTA über UDP (REQ-COM-008, REQ-SVC-013).

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const EXAMPLE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../examples/beispielstadt.toml"
);
const TOKEN: &str = "geheim";

/// Beendet den Prozess auch bei fehlgeschlagenen Tests.
struct Process {
    child: Child,
    /// Erste Zeile auf stdout ohne Präfix.
    line: String,
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Process {
    fn spawn(mut cmd: Command, prefix: &str) -> Self {
        let mut child = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let line = line
            .trim()
            .strip_prefix(prefix)
            .unwrap_or_else(|| panic!("{line}"))
            .to_string();
        Self { child, line }
    }
}

fn tmp(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name)
}

/// Pfad zu `railit-field` neben `railitd`; fehlt es, wird es gebaut.
fn field_binary() -> PathBuf {
    let railitd = PathBuf::from(env!("CARGO_BIN_EXE_railitd"));
    let path = railitd.with_file_name(format!("railit-field{}", std::env::consts::EXE_SUFFIX));
    if !path.exists() {
        let mut cargo = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
        cargo.args(["build", "-q", "-p", "railit-field"]);
        if railitd.components().any(|c| c.as_os_str() == "release") {
            cargo.arg("--release");
        }
        assert!(cargo.status().unwrap().success(), "railit-field bauen");
    }
    path
}

/// Feldrechner mit (UDP-Adresse, URL der Steuerung).
fn start_field() -> (Process, String, String) {
    let mut cmd = Command::new(field_binary());
    cmd.args([
        "--network",
        EXAMPLE,
        "--listen",
        "127.0.0.1:0",
        "--control",
        "127.0.0.1:0",
    ])
    .env_remove("RAILIT_FIELD_TOKEN");
    let p = Process::spawn(cmd, "railit-field: ");
    let (udp, http) = p.line.split_once(' ').unwrap();
    let (udp, http) = (
        udp.strip_prefix("udp://").unwrap().to_string(),
        http.to_string(),
    );
    (p, udp, http)
}

/// Plan der Beispielstadt (unsigniert).
fn plan(tag: &str) -> PathBuf {
    let net = railit_import::toml_format::parse(&std::fs::read_to_string(EXAMPLE).unwrap())
        .unwrap()
        .topology;
    let compiled = railit_planning::compile(&net, None).unwrap();
    let path = tmp(&format!("{tag}.rplan"));
    std::fs::write(&path, compiled.image).unwrap();
    path
}

fn railitd(args: &[&str]) -> Process {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_railitd"));
    cmd.args([
        "--network",
        EXAMPLE,
        "--allow-unsigned",
        "--listen",
        "127.0.0.1:0",
    ])
    .args(["--token", TOKEN])
    .args(args)
    .env_remove("RAILIT_TOKEN");
    Process::spawn(cmd, "railitd: ")
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(10)))
        .build()
        .into()
}

fn get(url: &str) -> (u16, Value) {
    let mut resp = agent()
        .get(url)
        .header("Authorization", format!("Bearer {TOKEN}"))
        .call()
        .unwrap();
    (
        resp.status().as_u16(),
        resp.body_mut().read_json().unwrap_or(Value::Null),
    )
}

fn post(url: &str, body: Option<Value>) -> (u16, Value) {
    let req = agent()
        .post(url)
        .header("Authorization", format!("Bearer {TOKEN}"));
    let mut resp = match body {
        Some(b) => req.send_json(b).unwrap(),
        None => req.send_empty().unwrap(),
    };
    (
        resp.status().as_u16(),
        resp.body_mut().read_json().unwrap_or(Value::Null),
    )
}

/// Wartet, bis `check` gilt (höchstens 30 s).
fn wait(what: &str, mut check: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !check() {
        assert!(Instant::now() < deadline, "{what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn route_phase(url: &str, route: &str) -> Value {
    let (_, state) = get(&format!("{url}/v1/state"));
    state["routes"]
        .as_array()
        .and_then(|r| r.iter().find(|r| r["name"] == route))
        .map_or(Value::Null, |r| r["phase"].clone())
}

#[test]
fn remote_field_over_rasta() {
    let (_field, udp, control) = start_field();
    let plan = plan("remote");
    let d = railitd(&[
        "--plan",
        plan.to_str().unwrap(),
        "--field",
        "remote",
        "--field-addr",
        &udp,
        "--field-control",
        &control,
    ]);
    let url = d.line.clone();
    wait("Verbindungen zum Feldrechner", || {
        let (_, links) = get(&format!("{url}/v1/field/links"));
        links
            .as_array()
            .is_some_and(|l| !l.is_empty() && l.iter().all(|x| x["up"] == true))
    });
    let (_, status) = get(&format!("{control}/v1/status"));
    assert_eq!(status["field"]["leader"], 0x0A00_0001, "{status}");

    // Fahrstraße über das Netz: Weichen laufen im Feldrechner um, das Signal zeigt Fahrt.
    let r = post(&format!("{url}/v1/routes/A-N1/set"), None);
    assert_eq!(r.0, 200, "{r:?}");
    wait("A-N1 auf Fahrt", || route_phase(&url, "A-N1") == "proceed");

    // Simulationsbefehle gehen an die Steuerung des Feldrechners.
    let (code, body) = post(
        &format!("{url}/v1/sim/trains"),
        Some(json!({"name": "RB1", "signal": "A", "length_m": 80, "speed_kmh": 60})),
    );
    assert_eq!(code, 200, "{body}");
    let (_, trains) = get(&format!("{url}/v1/sim/trains"));
    assert_eq!(trains[0]["name"], "RB1", "{trains}");
    wait("Zug fährt ein", || route_phase(&url, "A-N1") == "occupied");
    let (code, body) = post(
        &format!("{url}/v1/sim/faults"),
        Some(json!({"kind": "tvd_disturbed", "element": "T2"})),
    );
    assert_eq!(code, 200, "{body}");
}

#[test]
fn field_network_must_match() {
    let (_field, udp, control) = start_field();
    let nordbahn = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/nordbahn.toml");
    let out = Command::new(env!("CARGO_BIN_EXE_railitd"))
        .args(["--network", nordbahn, "--plan", "x", "--allow-unsigned"])
        .args([
            "--field",
            "remote",
            "--field-addr",
            &udp,
            "--field-control",
            &control,
        ])
        .env_remove("RAILIT_TOKEN")
        .output()
        .unwrap();
    assert!(!out.status.success());
}
