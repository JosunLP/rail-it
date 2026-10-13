//! `railit-node` als eigener Prozess, angebunden über RaSTA auf UDP (REQ-EMB-005).

use std::io::{BufRead, BufReader};
use std::net::{SocketAddr, UdpSocket};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use railit_kernel::{Aspect, Command as Cmd, Request, RequestId, RoutePhase};
use railit_runtime::{Addressed, Runtime, UdpLink};

const EXAMPLE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../examples/beispielstadt.toml"
);

struct Node(Child);

impl Drop for Node {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn plan_file() -> PathBuf {
    let net = railit_import::toml_format::parse(&std::fs::read_to_string(EXAMPLE).unwrap())
        .unwrap()
        .topology;
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("node-udp.rplan");
    std::fs::write(&path, railit_planning::compile(&net, None).unwrap().image).unwrap();
    path
}

fn free_port() -> SocketAddr {
    UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
}

fn start_node(plan: &Path, addr: SocketAddr) -> Node {
    let mut child = Command::new(env!("CARGO_BIN_EXE_railit-node"))
        .args(["--plan", plan.to_str().unwrap(), "--allow-unsigned"])
        .args(["--listen", &addr.to_string(), "--channels", "2"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    assert!(line.starts_with("railit-node: udp://"), "{line}");
    Node(child)
}

/// Läuft Zyklen in Echtzeit, bis `done` gilt (höchstens `limit`).
fn run_until(
    rt: &mut Runtime<railit_sim::Simulator>,
    requests: &mut Vec<Addressed>,
    limit: Duration,
    mut done: impl FnMut(&Runtime<railit_sim::Simulator>, &[Cmd]) -> bool,
) -> bool {
    let start = Instant::now();
    while start.elapsed() < limit {
        let out = rt.cycle_addressed(requests);
        requests.clear();
        if done(rt, &out.commands) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

fn node_snapshot(rt: &Runtime<railit_sim::Simulator>) -> railit_runtime::NodeSnapshot {
    rt.snapshot().areas[0].safety.node.clone().unwrap()
}

/// REQ-EMB-005: Der Kern rechnet im Knotenprozess; der Dienst übernimmt seine Befehle, führt
/// einen geprüften Spiegel, stellt bei Ausfall alle Signale auf Halt und gleicht sich nach
/// dem Neustart des Knotens neu ab.
#[test]
fn node_process_over_udp() {
    let plan = plan_file();
    let addr = free_port();
    let node = start_node(&plan, addr);

    let net = railit_import::toml_format::parse(&std::fs::read_to_string(EXAMPLE).unwrap())
        .unwrap()
        .topology;
    let mut rt = railit_runtime::simulation(
        net,
        railit_rules::GenericRules::default(),
        railit_sim::SimConfig::default(),
        railit_types::Duration::from_millis(100),
    )
    .unwrap();
    rt.attach_node(0, Box::new(UdpLink::connect(addr).unwrap()))
        .unwrap();
    let mut requests = Vec::new();
    assert!(
        run_until(&mut rt, &mut requests, Duration::from_secs(10), |rt, _| rt
            .nodes_synced()),
        "Anlauf: {:?}",
        node_snapshot(&rt)
    );
    let s = node_snapshot(&rt);
    assert!(s.up && s.available && s.synced, "{s:?}");
    let status = rt.safety_status(0).unwrap();
    assert_eq!(status.redundancy.channels(), 2, "Redundanz des Knotens");

    // Fahrstraße über den Knoten.
    let (area, route) = rt.find_route("A-N1").unwrap();
    requests.push(Addressed {
        area,
        id: RequestId(1),
        request: Request::SetRoute(route),
    });
    let proceed = |rt: &Runtime<railit_sim::Simulator>, _: &[Cmd]| {
        rt.state().routes()[route.index().unwrap()].phase() == RoutePhase::Proceed
    };
    assert!(
        run_until(&mut rt, &mut requests, Duration::from_secs(20), proceed),
        "Fahrstraße: {:?}",
        node_snapshot(&rt)
    );
    let s = node_snapshot(&rt);
    assert_eq!(s.divergences, 0, "{s:?}");
    assert_eq!(s.protocol_errors, 0, "{s:?}");

    // Ausfall des Knotens: Rückfallebene stellt alle Signale auf Halt.
    drop(node);
    let a = rt.find_signal("A").unwrap().1;
    let fallback = |_: &Runtime<railit_sim::Simulator>, commands: &[Cmd]| {
        commands.contains(&Cmd::SetSignal {
            signal: a,
            aspect: Aspect::Stop,
        })
    };
    assert!(run_until(
        &mut rt,
        &mut requests,
        Duration::from_secs(5),
        fallback
    ));
    let s = node_snapshot(&rt);
    assert!(!s.available, "{s:?}");

    // Neustart: neue Verbindung, neuer Abgleich (der Knoten beginnt ohne Fahrstraßen).
    let _node = start_node(&plan, addr);
    assert!(
        run_until(&mut rt, &mut requests, Duration::from_secs(10), |rt, _| {
            rt.nodes_synced() && node_snapshot(rt).syncs >= 2
        }),
        "Neuer Abgleich: {:?}",
        node_snapshot(&rt)
    );
    assert_eq!(
        rt.state().routes()[route.index().unwrap()].phase(),
        RoutePhase::Idle
    );
}
