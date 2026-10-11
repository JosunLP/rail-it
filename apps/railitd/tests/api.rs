//! Ende-zu-Ende-Test: `railitd` als Prozess, Bedienung über die HTTP-API.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use railit_plan::SigningKey;
use serde_json::{Value, json};

const EXAMPLE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../examples/beispielstadt.toml"
);
const TOKEN: &str = "geheim";

/// Beendet den Dienst auch bei fehlgeschlagenen Tests.
struct Daemon {
    child: Child,
    url: String,
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn tmp(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name)
}

/// Kompiliert die Beispielstadt und schreibt Plan und Schlüssel.
fn prepare(tag: &str, signed: bool) -> (PathBuf, PathBuf) {
    let net = railit_import::toml_format::parse(&std::fs::read_to_string(EXAMPLE).unwrap())
        .unwrap()
        .topology;
    let key = SigningKey::from_bytes(&[9; 32]);
    let compiled = railit_planning::compile(&net, signed.then_some(&key)).unwrap();
    let plan = tmp(&format!("{tag}.rplan"));
    let public = tmp(&format!("{tag}.pub"));
    std::fs::write(&plan, compiled.image).unwrap();
    std::fs::write(
        &public,
        railit_runtime::keys::to_hex(key.verifying_key().as_bytes()),
    )
    .unwrap();
    (plan, public)
}

fn daemon(args: &[&str]) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_railitd"));
    cmd.args(["--network", EXAMPLE])
        .args(args)
        .env_remove("RAILIT_TOKEN");
    cmd
}

fn start(args: &[&str]) -> Daemon {
    let mut child = daemon(args)
        .args(["--listen", "127.0.0.1:0", "--speed", "20", "--token", TOKEN])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let url = line
        .trim()
        .strip_prefix("railitd: ")
        .expect(&line)
        .to_string();
    Daemon { child, url }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into()
}

fn get(d: &Daemon, path: &str, token: bool) -> (u16, Value) {
    let mut req = agent().get(format!("{}{path}", d.url));
    if token {
        req = req.header("Authorization", format!("Bearer {TOKEN}"));
    }
    let mut resp = req.call().unwrap();
    (
        resp.status().as_u16(),
        resp.body_mut().read_json().unwrap_or(Value::Null),
    )
}

fn post(d: &Daemon, path: &str, body: Option<Value>) -> (u16, Value) {
    let req = agent()
        .post(format!("{}{path}", d.url))
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

fn route_phase(d: &Daemon, name: &str) -> String {
    let (_, state) = get(d, "/v1/state", true);
    state["routes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == name)
        .unwrap()["phase"]
        .as_str()
        .unwrap()
        .to_string()
}

fn wait_for(d: &Daemon, route: &str, phase: &str) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while route_phase(d, route) != phase {
        assert!(Instant::now() < deadline, "{route} erreicht {phase} nicht");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn operate_station_over_api() {
    let (plan, public) = prepare("api", true);
    let d = start(&[
        "--plan",
        plan.to_str().unwrap(),
        "--trusted-key",
        public.to_str().unwrap(),
    ]);

    let (status, health) = get(&d, "/v1/health", false);
    assert_eq!(status, 200);
    assert_eq!(health["network"], "Beispielstadt");

    assert_eq!(get(&d, "/v1/state", false).0, 401, "ohne Token abgewiesen");

    let (_, info) = get(&d, "/v1/plan", true);
    assert_eq!(info["signed"], true);
    assert_eq!(info["routes"].as_array().unwrap().len(), 8);

    let (status, answer) = post(&d, "/v1/routes/A-N1/set", None);
    assert_eq!(
        (status, answer["type"].as_str()),
        (200, Some("accepted")),
        "{answer}"
    );
    wait_for(&d, "A-N1", "proceed");

    let (status, answer) = post(&d, "/v1/routes/A-N2/set", None);
    assert_eq!(status, 409);
    assert_eq!(answer["reason"], "conflict");

    assert_eq!(post(&d, "/v1/routes/X-Y/set", None).0, 404);

    let (status, answer) = post(&d, "/v1/points/W1/move", Some(json!({"position": "right"})));
    assert_eq!(
        (status, answer["reason"].as_str()),
        (409, Some("point_locked"))
    );

    let train = json!({"name": "RE1", "signal": "A", "length_m": 100, "speed_kmh": 80});
    assert_eq!(post(&d, "/v1/sim/trains", Some(train)).0, 200);
    wait_for(&d, "A-N1", "idle");
    let (_, trains) = get(&d, "/v1/sim/trains", true);
    assert_eq!(trains[0]["name"], "RE1");

    let fault = json!({"kind": "tvd_disturbed", "element": "T2"});
    assert_eq!(post(&d, "/v1/sim/faults", Some(fault)).0, 200);
    let (status, answer) = post(&d, "/v1/routes/A-N2/set", None);
    assert_eq!(
        (status, answer["reason"].as_str()),
        (409, Some("tvd_not_clear"))
    );
}

/// REQ-PLN-004: Ohne gültige Signatur startet der Dienst nicht.
#[test]
fn unsigned_or_foreign_plans_are_refused() {
    let (unsigned, public) = prepare("unsigned", false);
    let out = daemon(&[
        "--plan",
        unsigned.to_str().unwrap(),
        "--trusted-key",
        public.to_str().unwrap(),
    ])
    .output()
    .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("nicht signiert"));

    let (signed, _) = prepare("foreign", true);
    let other = tmp("other.pub");
    std::fs::write(
        &other,
        railit_runtime::keys::to_hex(SigningKey::from_bytes(&[1; 32]).verifying_key().as_bytes()),
    )
    .unwrap();
    let out = daemon(&[
        "--plan",
        signed.to_str().unwrap(),
        "--trusted-key",
        other.to_str().unwrap(),
    ])
    .output()
    .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("Signatur"));

    // Ohne Schlüssel nur mit ausdrücklicher Freigabe.
    let out = daemon(&["--plan", unsigned.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
}
