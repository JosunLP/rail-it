//! Ende-zu-Ende: Feldrechner (`railit-field`) und Stellwerksdienst (`railitd --field remote`)
//! als Prozesse, RaSTA über UDP (REQ-COM-008, REQ-SVC-013); Übernahme des Betriebs durch den
//! Bereitschaftsrechner (REQ-SVC-012, REQ-SVC-014).

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
    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

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

/// REQ-COM-011: Zwei Kanäle der Redundanzschicht; fällt einer aus, läuft der Betrieb über
/// den anderen weiter, ohne dass eine RaSTA-Verbindung abbricht.
#[test]
fn redundant_channels_survive_a_channel_failure() {
    let mut cmd = Command::new(field_binary());
    cmd.args([
        "--network",
        EXAMPLE,
        "--listen",
        "127.0.0.1:0",
        "--listen",
        "127.0.0.1:0",
        "--control",
        "127.0.0.1:0",
    ])
    .env_remove("RAILIT_FIELD_TOKEN");
    let field = Process::spawn(cmd, "railit-field: ");
    let (udp, control) = field.line.split_once(' ').unwrap();
    let (a, b) = udp.strip_prefix("udp://").unwrap().split_once(',').unwrap();
    let control = control.to_string();
    let plan = plan("redundant");
    let d = railitd(&[
        "--plan",
        plan.to_str().unwrap(),
        "--field",
        "remote",
        "--field-addr",
        a,
        "--field-addr",
        b,
        "--field-control",
        &control,
    ]);
    let url = d.line.clone();
    let all_up = || {
        get(&format!("{url}/v1/field/links"))
            .1
            .as_array()
            .is_some_and(|l| !l.is_empty() && l.iter().all(|x| x["up"] == true))
    };
    wait("Verbindungen über zwei Kanäle", all_up);
    let received = |k: usize| {
        get(&format!("{control}/v1/field/channels")).1[k]["received"]
            .as_u64()
            .unwrap()
    };
    wait("beide Kanäle tragen", || {
        received(0) > 0 && received(1) > 0
    });

    // Kanal 0 fällt aus: Die Verbindungen bleiben, Kanal 1 trägt allein.
    let (code, _) = post(
        &format!("{control}/v1/field/channels/0"),
        Some(json!({"down": true})),
    );
    assert_eq!(code, 200);
    let before = received(1);
    assert_eq!(post(&format!("{url}/v1/routes/A-N1/set"), None).0, 200);
    wait("A-N1 auf Fahrt", || route_phase(&url, "A-N1") == "proceed");
    assert!(received(1) > before);
    let (_, links) = get(&format!("{url}/v1/field/links"));
    assert!(
        links.as_array().unwrap().iter().all(|l| l["up"] == true),
        "{links}"
    );
    assert_eq!(
        get(&format!("{control}/v1/field/channels")).1[0]["down"],
        true
    );
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

fn health(url: &str) -> Value {
    get(&format!("{url}/v1/health")).1
}

/// Hauptrechner (optional mit Ausgabesperre) und Bereitschaftsrechner mit Übernahme am
/// selben Feldrechner; der Hauptrechner stellt A-N1 auf Fahrt und fällt dann aus.
fn takeover(tag: &str, gate: bool) -> (Process, Process, String) {
    let (field, udp, control) = start_field();
    let plan = plan(tag);
    let (j1, j2) = (
        tmp(&format!("{tag}-1.journal")),
        tmp(&format!("{tag}-2.journal")),
    );
    let _ = std::fs::remove_file(&j1);
    let _ = std::fs::remove_file(&j2);
    let common = [
        "--plan",
        plan.to_str().unwrap(),
        "--field",
        "remote",
        "--field-addr",
        &udp,
        "--field-control",
        &control,
    ];
    let mut primary_args = common.to_vec();
    primary_args.extend(["--journal", j1.to_str().unwrap()]);
    if gate {
        primary_args.extend(["--standby-gate-ms", "8000"]);
    }
    let mut primary = railitd(&primary_args);
    let p = primary.line.clone();
    wait("Hauptrechner verbunden", || {
        get(&format!("{p}/v1/field/links"))
            .1
            .as_array()
            .is_some_and(|l| !l.is_empty() && l.iter().all(|x| x["up"] == true))
    });
    let mut standby_args = common.to_vec();
    standby_args.extend([
        "--journal",
        j2.to_str().unwrap(),
        "--standby",
        &p,
        "--standby-token",
        TOKEN,
        "--takeover-after-ms",
        "500",
    ]);
    let standby = railitd(&standby_args);
    let s = standby.line.clone();
    wait("Bereitschaftsrechner liest mit", || {
        health(&s)["standby"]["areas"][0]["cycles"].as_u64() > Some(0)
    });
    assert_eq!(health(&s)["role"], "standby");
    // Bedienung am Bereitschaftsrechner ist gesperrt.
    assert_eq!(post(&format!("{s}/v1/routes/A-N2/set"), None).0, 503);

    assert_eq!(post(&format!("{p}/v1/routes/A-N1/set"), None).0, 200);
    wait("A-N1 auf Fahrt", || route_phase(&p, "A-N1") == "proceed");
    wait("Bereitschaftsrechner kennt die Fahrt", || {
        route_phase(&s, "A-N1") == "proceed"
    });
    // Betriebsführung: Zugnummer und Disposition am Hauptrechner (REQ-OPS-007).
    let (code, body) = post(
        &format!("{p}/v1/ops/trains/RB7/describe"),
        Some(json!({"signal": "F"})),
    );
    assert_eq!(code, 200, "{body}");
    assert_eq!(post(&format!("{p}/v1/ops/trains/RB7/hold"), None).0, 200);
    wait("Bereitschaftsrechner kennt die Zugnummer", || {
        health(&s)["standby"]["ops_trains"].as_u64() == Some(1)
    });
    if gate {
        let h = health(&p);
        assert_eq!(h["replication"]["gated"], true, "{h}");
        assert_eq!(h["replication"]["coverage"]["covered"], true, "{h}");
        assert!(health(&s)["standby"]["covered_ms"].as_u64() > Some(0));
    }
    primary.kill();
    drop(primary);
    wait("Bereitschaftsrechner führt", || {
        health(&s)["role"] == "primary"
    });
    let (_, status) = get(&format!("{control}/v1/status"));
    assert_eq!(status["field"]["leader"], 0x0A00_0002, "{status}");
    (field, standby, s)
}

/// Warme Übernahme: Die Zusage des Hauptrechners deckt sie; die Fahrstraße bleibt
/// verschlossen und fällt auf Halt; der neue Hauptrechner nimmt Bedienungen an.
#[test]
fn standby_takes_over_with_the_followed_state() {
    let (_field, _standby, s) = takeover("warm", true);
    let h = health(&s);
    assert_eq!(h["standby"]["phase"]["phase"], "leading", "{h}");
    assert_eq!(h["standby"]["phase"]["warm"], true, "{h}");
    assert_eq!(route_phase(&s, "A-N1"), "halted");
    // Zugnummer und Disposition sind mit übernommen (REQ-OPS-007).
    let (_, ops) = get(&format!("{s}/v1/ops"));
    let rb7 = &ops["trains"][0];
    assert_eq!(
        (&rb7["number"], &rb7["berth"], &rb7["hold"]),
        (&json!("RB7"), &json!("F"), &json!(true)),
        "{ops}"
    );
    let (_, state) = get(&format!("{s}/v1/state"));
    let a = state["signals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["name"] == "A")
        .unwrap()
        .clone();
    assert_eq!(a["commanded"], "stop", "{a}");
    // Feindliche Fahrstraße bleibt ausgeschlossen, Rücknahme mit Zeitverschluss.
    assert_eq!(post(&format!("{s}/v1/routes/A-N2/set"), None).0, 409);
    assert_eq!(post(&format!("{s}/v1/routes/A-N1/cancel"), None).0, 200);
    assert_eq!(route_phase(&s, "A-N1"), "cancelling");
    // Journal des neuen Hauptrechners läuft, Simulation geht an den Feldrechner.
    let (_, journals) = get(&format!("{s}/v1/journal"));
    assert!(
        journals["journals"][0]["bytes"].as_u64() > Some(0),
        "{journals}"
    );
    let (code, _) = post(
        &format!("{s}/v1/sim/trains"),
        Some(json!({"name": "RB2", "signal": "F", "length_m": 80, "speed_kmh": 60})),
    );
    assert_eq!(code, 200);
}

/// Ohne Ausgabesperre gibt es keine Zusage: Der Bereitschaftsrechner beginnt wie nach einem
/// Neustart.
#[test]
fn standby_restarts_without_a_promise() {
    let (_field, _standby, s) = takeover("cold", false);
    let h = health(&s);
    assert_eq!(h["standby"]["phase"]["warm"], false, "{h}");
    assert_eq!(route_phase(&s, "A-N1"), "idle");
}

const NORDBAHN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/nordbahn.toml");

fn free_udp() -> String {
    std::net::UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .to_string()
}

fn holds(url: &str, address: &str) -> bool {
    let (area, _) = address.split_once('>').unwrap();
    let (_, state) = get(&format!("{url}/v1/state"));
    state["areas"].as_array().is_some_and(|areas| {
        areas.iter().any(|a| {
            a["name"] == area
                && a["borders"].as_array().is_some_and(|b| {
                    b.iter()
                        .any(|b| b["name"] == address && b["holding"] == true)
                })
        })
    })
}

fn ils_up(url: &str) -> bool {
    get(&format!("{url}/v1/ils"))
        .1
        .as_array()
        .is_some_and(|l| !l.is_empty() && l.iter().all(|x| x["up"] == true))
}

/// Die Nordbahn-Tests starten je fünf Prozesse mit Echtzeitfristen; gleichzeitig ausgeführt
/// verdrängen sie sich gegenseitig. Sie laufen deshalb nacheinander.
static NORDBAHN_SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn nordbahn_serial() -> std::sync::MutexGuard<'static, ()> {
    NORDBAHN_SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Feldrechner der Nordbahn für drei Stellwerke: (Prozess, UDP-Adresse, URL der Steuerung).
fn nordbahn_field() -> (Process, String, String) {
    let mut cmd = Command::new(field_binary());
    cmd.args([
        "--network",
        NORDBAHN,
        "--listen",
        "127.0.0.1:0",
        "--control",
        "127.0.0.1:0",
    ]);
    for id in ["0A000001", "0A000002", "0A000003"] {
        cmd.args(["--interlocking", id]);
    }
    cmd.env_remove("RAILIT_FIELD_TOKEN");
    let field = Process::spawn(cmd, "railit-field: ");
    let (udp, control) = field.line.split_once(' ').unwrap();
    let (udp, control) = (
        udp.strip_prefix("udp://").unwrap().to_string(),
        control.to_string(),
    );
    (field, udp, control)
}

/// Pläne der Nordbahn je Stellbereich (unsigniert).
fn nordbahn_plans() -> PathBuf {
    let net = railit_import::toml_format::parse(&std::fs::read_to_string(NORDBAHN).unwrap())
        .unwrap()
        .topology;
    let dir = tmp("split-standby");
    let _ = std::fs::create_dir_all(&dir);
    let cfg = railit_planning::PlanningConfig::default();
    for a in railit_planning::compile_areas(&net, None, &cfg).unwrap() {
        std::fs::write(dir.join(format!("{}.rplan", a.name)), a.compiled.image).unwrap();
    }
    dir
}

/// `railitd` für einen Teil der Nordbahn am gemeinsamen Feldrechner.
fn split_railitd(plans: &std::path::Path, udp: &str, control: &str, args: &[&str]) -> Process {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_railitd"));
    cmd.args([
        "--network",
        NORDBAHN,
        "--allow-unsigned",
        "--listen",
        "127.0.0.1:0",
        "--plan",
        plans.to_str().unwrap(),
        "--field",
        "remote",
        "--field-addr",
        udp,
        "--field-control",
        control,
    ])
    .args(["--token", TOKEN])
    .args(args)
    .env_remove("RAILIT_TOKEN");
    Process::spawn(cmd, "railitd: ")
}

/// REQ-COM-010, REQ-SVC-012: Bereitschaftsrechner für einen Teil des Netzes. Stellwerk West
/// (Adorf, Strecke) hat Haupt- und Bereitschaftsrechner, Stellwerk Ost (Bstadt) kennt beide
/// Adressen. Fällt der Hauptrechner aus, übernimmt der Bereitschaftsrechner warm am
/// Feldrechner, SCI-ILS baut sich zu ihm neu auf, und der Erlaubniswechsel mit Ost geht weiter.
#[test]
fn standby_takes_over_a_part_of_the_network() {
    let _serial = nordbahn_serial();
    let (field, udp, control) = nordbahn_field();
    let udp = udp.as_str();
    let dir = nordbahn_plans();
    // Bei mehreren Stellbereichen ist das Journal ein Verzeichnis.
    let journal = tmp("split-standby-west.journal");
    let _ = std::fs::remove_dir_all(&journal);
    let (west_ils, standby_ils, east_ils) = (free_udp(), free_udp(), free_udp());
    let east_peer = format!("Bstadt={east_ils}");
    let spawn = |args: &[&str]| split_railitd(&dir, udp, &control, args);
    let mut west = spawn(&[
        "--field-id",
        "0A000001",
        "--areas",
        "Adorf,Strecke",
        "--ils-listen",
        &west_ils,
        "--ils-peer",
        &east_peer,
        "--journal",
        journal.to_str().unwrap(),
        "--standby-gate-ms",
        "30000",
    ]);
    let east = spawn(&[
        "--field-id",
        "0A000003",
        "--areas",
        "Bstadt",
        "--ils-listen",
        &east_ils,
        "--ils-peer",
        &format!("Strecke={west_ils}"),
        "--ils-peer",
        &format!("Strecke={standby_ils}"),
    ]);
    let (w, e) = (west.line.clone(), east.line.clone());
    wait("West und Ost verbunden", || {
        ils_up(&w) && ils_up(&e) && holds(&w, "Strecke>Bstadt")
    });
    let standby = spawn(&[
        "--field-id",
        "0A000002",
        "--areas",
        "Adorf,Strecke",
        "--ils-listen",
        &standby_ils,
        "--ils-peer",
        &east_peer,
        "--standby",
        &w,
        "--standby-token",
        TOKEN,
        "--takeover-after-ms",
        "500",
    ]);
    let s = standby.line.clone();
    wait("Bereitschaftsrechner liest beide Bereiche mit", || {
        let h = health(&s);
        h["standby"]["areas"]
            .as_array()
            .is_some_and(|a| a.len() == 2 && a.iter().all(|x| x["cycles"].as_u64() > Some(0)))
    });

    west.kill();
    drop(west);
    wait("Bereitschaftsrechner führt", || {
        health(&s)["role"] == "primary"
    });
    let h = health(&s);
    assert_eq!(h["standby"]["phase"]["warm"], true, "{h}");
    assert!(holds(&s, "Strecke>Bstadt"), "Erlaubnis übernommen");
    wait("SCI-ILS zum Bereitschaftsrechner", || {
        ils_up(&s) && ils_up(&e)
    });

    // Richtungswechsel mit dem neuen Rechner.
    assert_eq!(
        post(&format!("{s}/v1/borders/Strecke%3EAdorf/request"), None).0,
        200
    );
    wait("Strecke hält Strecke>Adorf", || holds(&s, "Strecke>Adorf"));
    assert_eq!(
        post(&format!("{e}/v1/borders/Bstadt%3EStrecke/request"), None).0,
        200
    );
    wait("Erlaubnis wechselt nach Ost", || {
        holds(&e, "Bstadt>Strecke")
    });
    assert!(!holds(&s, "Strecke>Bstadt"));
    drop((field, standby, east));
}

/// REQ-ETCS-004: ETCS über Prozessgrenzen. West (Adorf, Strecke) und Ost (Bstadt) rechnen in
/// eigenen Prozessen am selben Feldrechner; die Funkstrecke geht über dessen Steuerung. Die
/// Fahrerlaubnis aus dem Westen reicht über die Vorschau des Ostens bis BN1; nach der Grenze
/// führt das RBC im Osten den Zug weiter.
#[test]
fn etcs_train_crosses_into_another_process() {
    let _serial = nordbahn_serial();
    let (_field, udp, control) = nordbahn_field();
    let udp = udp.as_str();
    let dir = nordbahn_plans();
    let (west_ils, east_ils) = (free_udp(), free_udp());
    let spawn = |args: &[&str]| split_railitd(&dir, udp, &control, args);
    let west = spawn(&[
        "--field-id",
        "0A000001",
        "--areas",
        "Adorf,Strecke",
        "--ils-listen",
        &west_ils,
        "--ils-peer",
        &format!("Bstadt={east_ils}"),
    ]);
    let east = spawn(&[
        "--field-id",
        "0A000003",
        "--areas",
        "Bstadt",
        "--ils-listen",
        &east_ils,
        "--ils-peer",
        &format!("Strecke={west_ils}"),
    ]);
    let (w, e) = (west.line.clone(), east.line.clone());
    wait("West und Ost verbunden", || {
        ils_up(&w) && ils_up(&e) && holds(&w, "Strecke>Bstadt")
    });
    assert_eq!(post(&format!("{e}/v1/routes/BA-BN1/set"), None).0, 200);
    assert_eq!(
        post(&format!("{w}/v1/routes/Sbk1-%3EBstadt/set"), None).0,
        200
    );
    wait("BA-BN1 auf Fahrt", || {
        route_phase(&e, "BA-BN1") == "proceed"
    });
    let (code, body) = post(
        &format!("{control}/v1/sim/trains"),
        Some(
            json!({"name": "ICE7", "signal": "Sbk1", "length_m": 200, "speed_kmh": 250, "etcs": true}),
        ),
    );
    assert_eq!(code, 200, "{body}");
    let authority = |url: &str| {
        let (_, state) = get(&format!("{url}/v1/state"));
        state["etcs"]
            .as_array()
            .and_then(|l| l.iter().find(|t| t["train"] == "ICE7").cloned())
            .map(|t| t["authority"]["end"].clone())
    };
    wait("Fahrerlaubnis im Westen bis BN1", || {
        authority(&w) == Some(json!("BN1"))
    });
    let deadline = Instant::now() + Duration::from_mins(1);
    while authority(&e) != Some(json!("BN1")) {
        assert!(
            Instant::now() < deadline,
            "RBC im Osten führt den Zug nicht: {:?} {:?} {:?}",
            get(&format!("{control}/v1/sim/trains")).1,
            get(&format!("{e}/v1/state")).1["etcs"],
            get(&format!("{w}/v1/state")).1["etcs"],
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    // Ohne Notbremsung über die Grenze.
    let (_, trains) = get(&format!("{control}/v1/sim/trains"));
    assert_eq!(trains[0]["state"], "running", "{trains}");
}
