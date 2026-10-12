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
const NORDBAHN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/nordbahn.toml");
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
    daemon_for(EXAMPLE, args)
}

fn daemon_for(network: &str, args: &[&str]) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_railitd"));
    cmd.args(["--network", network])
        .args(args)
        .env_remove("RAILIT_TOKEN");
    cmd
}

fn start(args: &[&str]) -> Daemon {
    start_for(EXAMPLE, args)
}

fn start_for(network: &str, args: &[&str]) -> Daemon {
    let mut child = daemon_for(network, args)
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

fn post_as(d: &Daemon, path: &str, body: Option<Value>, operator: u32) -> (u16, Value) {
    let req = agent()
        .post(format!("{}{path}", d.url))
        .header("Authorization", format!("Bearer {TOKEN}"))
        .header("X-Railit-Operator", operator.to_string());
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

/// REQ-IXL-014, REQ-JRN-001/002: Hilfshandlung über die API, Journal prüfen und abspielen.
#[test]
fn aux_operation_and_journal() {
    let (plan, public) = prepare("aux", true);
    let journal = tmp("aux.journal");
    let _ = std::fs::remove_file(&journal);
    let d = start(&[
        "--plan",
        plan.to_str().unwrap(),
        "--trusted-key",
        public.to_str().unwrap(),
        "--journal",
        journal.to_str().unwrap(),
        "--checkpoint-every",
        "20",
    ]);

    assert_eq!(post(&d, "/v1/routes/A-N1/set", None).0, 200);
    wait_for(&d, "A-N1", "proceed");

    let op = json!({"kind": "release_route", "element": "A-N1"});
    let (status, answer) = post(&d, "/v1/aux", Some(op.clone()));
    assert_eq!(status, 400, "ohne Bedienerkennung: {answer}");

    let (status, answer) = post_as(&d, "/v1/aux", Some(op), 1);
    assert_eq!(status, 200, "{answer}");
    assert_eq!(answer["aux"]["type"], "aux_pending");
    let id = answer["aux"]["aux"].as_u64().unwrap();
    let (_, pending) = get(&d, "/v1/aux", true);
    assert_eq!(pending["pending"][0]["op"]["element"], "A-N1");

    let confirm = format!("/v1/aux/{id}/confirm");
    let (status, answer) = post_as(&d, &confirm, None, 1);
    assert_eq!(
        (status, answer["reason"].as_str()),
        (409, Some("same_operator"))
    );
    let (status, answer) = post_as(&d, &confirm, None, 2);
    assert_eq!(status, 200, "{answer}");
    assert_eq!(answer["aux"]["type"], "aux_executed");
    assert_eq!(answer["aux"]["counter"], 1);
    wait_for(&d, "A-N1", "idle");
    drop(d);

    let bytes = std::fs::read(&journal).unwrap();
    let summary = railit_journal::verify(bytes.as_slice()).unwrap();
    assert!(summary.cycles > 0);
    let plan = railit_runtime::load_plan(std::fs::read(&plan).unwrap(), None).unwrap();
    let (replayed, state) = railit_journal::replay(bytes.as_slice(), &plan).unwrap();
    assert_eq!(replayed.cycles, summary.cycles);
    assert_eq!(state.meta().aux_counter(), 1);
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

/// Kompiliert die Nordbahn je Stellbereich in ein Verzeichnis.
fn prepare_areas(tag: &str) -> (PathBuf, PathBuf) {
    let net = railit_import::toml_format::parse(&std::fs::read_to_string(NORDBAHN).unwrap())
        .unwrap()
        .topology;
    let key = SigningKey::from_bytes(&[7; 32]);
    let dir = tmp(tag);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = railit_planning::PlanningConfig::default();
    for area in railit_planning::compile_areas(&net, Some(&key), &cfg).unwrap() {
        std::fs::write(
            dir.join(format!("{}.rplan", area.name)),
            area.compiled.image,
        )
        .unwrap();
    }
    let public = tmp(&format!("{tag}.pub"));
    std::fs::write(
        &public,
        railit_runtime::keys::to_hex(key.verifying_key().as_bytes()),
    )
    .unwrap();
    (dir, public)
}

fn permission(d: &Daemon, area: &str, border: &str) -> bool {
    let (_, areas) = get(d, "/v1/areas", true);
    areas
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["name"] == area)
        .unwrap()["borders"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == border)
        .unwrap()["holding"]
        .as_bool()
        .unwrap()
}

/// REQ-IXL-017..020: Netz mit drei Stellbereichen, Erlaubniswechsel über die API.
#[test]
fn network_with_areas_over_api() {
    let (plans, public) = prepare_areas("nordbahn");
    let journals = tmp("nordbahn-journal");
    let _ = std::fs::remove_dir_all(&journals);
    let d = start_for(
        NORDBAHN,
        &[
            "--plan",
            plans.to_str().unwrap(),
            "--trusted-key",
            public.to_str().unwrap(),
            "--journal",
            journals.to_str().unwrap(),
        ],
    );

    let (_, health) = get(&d, "/v1/health", false);
    assert_eq!(health["network"], "Nordbahn");
    assert_eq!(health["areas"], json!(["Adorf", "Strecke", "Bstadt"]));

    let (_, info) = get(&d, "/v1/plan", true);
    let exit = info["routes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "AN1->Strecke")
        .unwrap();
    assert_eq!(exit["area"], "Adorf");

    // Anlauf: Adorf erhält die Erlaubnis Richtung Strecke.
    let deadline = Instant::now() + Duration::from_secs(20);
    while !permission(&d, "Adorf", "Adorf>Strecke") {
        assert!(Instant::now() < deadline, "Erlaubnis kommt nicht");
        std::thread::sleep(Duration::from_millis(50));
    }
    let (status, answer) = post(&d, "/v1/routes/AN1-%3EStrecke/set", None);
    assert_eq!(
        (status, answer["area"].as_str()),
        (200, Some("Adorf")),
        "{answer}"
    );
    assert_eq!(post(&d, "/v1/routes/AN1-%3EStrecke/cancel", None).0, 200);

    // Die Strecke fordert die Erlaubnis an; danach ist die Ausfahrt gesperrt.
    let (status, answer) = post(&d, "/v1/borders/Strecke%3EAdorf/request", None);
    assert_eq!(
        (status, answer["area"].as_str()),
        (200, Some("Strecke")),
        "{answer}"
    );
    let deadline = Instant::now() + Duration::from_secs(20);
    while !permission(&d, "Strecke", "Strecke>Adorf") {
        assert!(Instant::now() < deadline, "Erlaubniswechsel bleibt aus");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!permission(&d, "Adorf", "Adorf>Strecke"));
    let (status, answer) = post(&d, "/v1/routes/AN2-%3EStrecke/set", None);
    assert_eq!(
        (status, answer["reason"].as_str()),
        (409, Some("no_permission"))
    );
    assert_eq!(
        post(&d, "/v1/borders/Irgendwo%3ENirgends/request", None).0,
        404
    );
    drop(d);

    for area in ["Adorf", "Strecke", "Bstadt"] {
        let bytes = std::fs::read(journals.join(format!("{area}.journal"))).unwrap();
        assert!(railit_journal::verify(bytes.as_slice()).unwrap().cycles > 0);
    }
}

fn delete(d: &Daemon, path: &str) -> u16 {
    agent()
        .delete(format!("{}{path}", d.url))
        .header("Authorization", format!("Bearer {TOKEN}"))
        .call()
        .unwrap()
        .status()
        .as_u16()
}

/// REQ-OPS-001 … 005: Fahrplanbetrieb mit Zuglenkung und Disposition über die API.
#[test]
fn timetable_and_dispatching_over_api() {
    let (plans, public) = prepare_areas("nordbahn-ops");
    let timetable = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/nordbahn-fahrplan.toml"
    );
    let d = start_for(
        NORDBAHN,
        &[
            "--plan",
            plans.to_str().unwrap(),
            "--trusted-key",
            public.to_str().unwrap(),
            "--timetable",
            timetable,
            "--ars",
        ],
    );
    let train = |name: &str| {
        let (status, ops) = get(&d, "/v1/ops", true);
        assert_eq!(status, 200);
        ops["trains"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["number"] == name)
            .cloned()
            .unwrap_or(Value::Null)
    };
    assert_eq!(train("RE2")["berth"], "BP1", "steht bei Betriebsbeginn");

    // Die Zuglenkung führt RE1 nach Adorf; dort hält der Disponent ihn zurück.
    let deadline = Instant::now() + Duration::from_secs(30);
    while train("RE1")["at_stop"] != "Adorf/1" {
        assert!(Instant::now() < deadline, "RE1 erreicht Adorf nicht");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(post(&d, "/v1/ops/trains/RE1/hold", None).0, 200);
    assert_eq!(train("RE1")["hold"], true);
    assert_eq!(
        post(&d, "/v1/ops/trains/RE1/delay", Some(json!({"s": 30}))).0,
        200
    );
    assert_eq!(train("RE1")["extra_delay_s"], 30);
    let wrong = json!({"station": "Bstadt", "platform": "9"});
    assert_eq!(post(&d, "/v1/ops/trains/RE1/platform", Some(wrong)).0, 400);
    let change = json!({"station": "Bstadt", "platform": "1"});
    assert_eq!(post(&d, "/v1/ops/trains/RE1/platform", Some(change)).0, 200);
    assert_eq!(train("RE1")["next_stop"], "Adorf/1");
    assert_eq!(post(&d, "/v1/ops/trains/XX/hold", None).0, 400);

    // Zugnummer eines Zuges ohne Fahrplan eintragen und löschen.
    let describe = json!({"signal": "AN2"});
    assert_eq!(
        post(&d, "/v1/ops/trains/Lz9/describe", Some(describe)).0,
        200
    );
    assert_eq!(train("Lz9")["berth"], "AN2");
    assert_eq!(delete(&d, "/v1/ops/trains/Lz9"), 200);
    assert_eq!(train("Lz9"), Value::Null);

    assert_eq!(post(&d, "/v1/ops/ars", Some(json!({"on": false}))).0, 200);
    let (_, ops) = get(&d, "/v1/ops", true);
    assert_eq!(ops["ars"], false);
    assert_eq!(ops["timetable"], "Nordbahn Grundtakt");
}

/// REQ-ETCS-001/002: ETCS-Zug meldet sich an und erhält seine Fahrerlaubnis über die API.
#[test]
fn etcs_train_over_api() {
    let (plan, public) = prepare("etcs", true);
    let d = start(&[
        "--plan",
        plan.to_str().unwrap(),
        "--trusted-key",
        public.to_str().unwrap(),
    ]);
    let train =
        json!({"name": "ICE1", "signal": "A", "length_m": 100, "speed_kmh": 10, "etcs": true});
    assert_eq!(post(&d, "/v1/sim/trains", Some(train)).0, 200);
    assert_eq!(post(&d, "/v1/routes/A-N1/set", None).0, 200);
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let (_, etcs) = get(&d, "/v1/etcs", true);
        let t = &etcs[0];
        if t["authority"]["end"] == "N1" {
            assert_eq!(t["train"], "ICE1");
            assert_eq!(t["mode"], "full_supervision");
            assert_eq!(t["authority"]["sections"][0]["route"], "A-N1");
            assert!(t["authority"]["danger_point_mm"].as_u64() > t["authority"]["eoa_mm"].as_u64());
            break;
        }
        assert!(Instant::now() < deadline, "keine Fahrerlaubnis: {etcs}");
        std::thread::sleep(Duration::from_millis(50));
    }
}
