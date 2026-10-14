//! Feldrechner über das Netz mit Umschalter (REQ-COM-008, REQ-SVC-013): zwei Stellwerke,
//! ein Feldrechner, Übertragung im Speicher mit gemeinsamer Uhr.

use std::net::SocketAddr;

use railit_adapters::net::{FieldClient, FieldServer, PRIMARY_ID, STANDBY_ID, SWITCH_MS};
use railit_kernel::{Aspect, Command, Input, SignalReport};
use railit_sim::{SimConfig, Simulator};
use railit_topology::ElementRef;
use railit_types::{PointPosition, Tick};

const EXAMPLE: &str = include_str!("../../../examples/beispielstadt.toml");
const STEP_MS: u64 = 50;

struct Bench {
    server: FieldServer,
    clients: [FieldClient; 2],
    /// Stellwerk läuft und ist erreichbar.
    alive: [bool; 2],
    /// Vom Feldrechner zum Stellwerk verworfene Objektsteuerungen (RaSTA-Kennung).
    cut: Vec<(usize, u32)>,
    now: u64,
}

fn addr(i: usize) -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], u16::try_from(1000 + i).unwrap()))
}

impl Bench {
    fn new() -> Self {
        let net = railit_import::toml_format::parse(EXAMPLE).unwrap().topology;
        let sim = Simulator::new(net.clone(), SimConfig::default());
        Self {
            server: FieldServer::new(sim, &[PRIMARY_ID, STANDBY_ID], SWITCH_MS).unwrap(),
            clients: [
                FieldClient::new(&net, PRIMARY_ID).unwrap(),
                FieldClient::new(&net, STANDBY_ID).unwrap(),
            ],
            alive: [false, false],
            cut: Vec::new(),
            now: 0,
        }
    }

    fn deliver(&mut self, from: usize, pdus: Vec<Vec<u8>>) {
        for pdu in pdus {
            let oc = railit_rasta::addresses(&pdu).map(|(r, _)| r);
            if self.alive[from] && !oc.is_some_and(|oc| self.cut.contains(&(from, oc))) {
                self.server.receive(addr(from), &pdu);
            }
        }
    }

    fn run(&mut self, ms: u64) {
        let end = self.now + ms;
        while self.now < end {
            self.now += STEP_MS;
            for to in 0..2 {
                if self.alive[to] {
                    let out = self.clients[to].tick(self.now);
                    self.deliver(to, out);
                }
            }
            for (to, pdu) in self.server.tick(Tick::from_millis(self.now)) {
                let i = usize::from(to.port() - 1000);
                let oc = railit_rasta::addresses(&pdu).map(|(_, s)| s);
                if self.alive[i] && !oc.is_some_and(|oc| self.cut.contains(&(i, oc))) {
                    self.clients[i].receive(self.now, &pdu);
                }
            }
        }
    }

    fn command(&mut self, client: usize, commands: &[Command]) {
        let out = self.clients[client].apply(self.now, commands);
        self.deliver(client, out);
    }

    fn signal(&self, name: &str) -> railit_types::SignalId {
        match self.server.simulator().topology().lookup(name) {
            Some(ElementRef::Signal(s)) => s,
            _ => panic!("Signal {name}"),
        }
    }

    fn point(&self, name: &str) -> railit_types::PointId {
        match self.server.simulator().topology().lookup(name) {
            Some(ElementRef::Point(p)) => p,
            _ => panic!("Weiche {name}"),
        }
    }

    fn clear(&mut self, client: usize, signal: &str) {
        let signal = self.signal(signal);
        self.command(
            client,
            &[Command::SetSignal {
                signal,
                aspect: Aspect::Proceed,
            }],
        );
        self.run(200);
    }

    fn shows(&self, signal: &str) -> Option<SignalReport> {
        self.server.simulator().signal_report(self.signal(signal))
    }

    fn move_point(&mut self, client: usize, point: &str, position: PointPosition) {
        let point = self.point(point);
        self.command(client, &[Command::MovePoint { point, position }]);
        self.run(5_000);
    }

    fn position(&self, point: &str) -> Option<railit_kernel::PointReport> {
        self.server.simulator().point_report(self.point(point))
    }
}

#[test]
fn leader_connects_and_controls_the_field() {
    let mut b = Bench::new();
    b.alive[0] = true;
    b.run(1_000);
    assert!(b.clients[0].connected());
    assert_eq!(b.server.leader(), Some(PRIMARY_ID));
    let reports = b.clients[0].reports();
    let net = b.server.simulator().topology();
    assert_eq!(
        reports.len(),
        net.points().len() + net.signals().len() + net.tvds().len() + net.level_crossings().len()
    );
    assert!(reports.iter().any(|i| matches!(i, Input::Signal { .. })));

    b.move_point(0, "W1", PointPosition::Left);
    assert_eq!(b.position("W1"), Some(PointPosition::Left.into()));
    b.clear(0, "A");
    assert_eq!(b.shows("A"), Some(SignalReport::Proceed));
}

#[test]
fn second_interlocking_is_rejected_while_the_leader_lives() {
    let mut b = Bench::new();
    b.alive[0] = true;
    b.run(1_000);
    b.alive[1] = true;
    b.run(3_000);
    assert_eq!(b.server.leader(), Some(PRIMARY_ID));
    assert!(!b.clients[1].connected());
    assert!(b.clients[1].reports().is_empty());
    let before = b.position("W2");
    b.move_point(1, "W2", PointPosition::Left);
    assert_eq!(
        b.position("W2"),
        before,
        "Bereitschaftsrechner stellt nicht"
    );
    assert!(b.server.status().rejected > 0);

    // Auch wenn die Verbindung zu einer einzelnen Objektsteuerung fehlt, bleibt die Führung.
    let oc = b.server.status().controllers;
    assert!(oc > 1);
    b.cut.push((0, 0x0B00_0002));
    b.run(4_000);
    assert_eq!(b.server.leader(), Some(PRIMARY_ID));
    assert!(!b.clients[1].connected());
}

#[test]
fn standby_leads_after_the_leader_is_lost() {
    let mut b = Bench::new();
    b.alive[0] = true;
    b.alive[1] = true;
    b.run(1_000);
    b.clear(0, "A");
    assert_eq!(b.shows("A"), Some(SignalReport::Proceed));

    // Hauptrechner fällt aus: Nach t_max fallen die Signale, nach der Umschaltzeit führt der
    // Bereitschaftsrechner.
    b.alive[0] = false;
    b.run(1_000);
    assert_eq!(b.server.leader(), Some(PRIMARY_ID), "noch nicht abgebaut");
    assert!(!b.clients[1].connected());
    b.run(1_500);
    assert_eq!(b.shows("A"), Some(SignalReport::Stop), "Rückfallebene");
    b.run(2_000);
    assert_eq!(b.server.leader(), Some(STANDBY_ID));
    assert!(b.clients[1].connected());
    assert_eq!(b.server.status().switches, 2);
    b.move_point(1, "W2", PointPosition::Left);
    assert_eq!(b.position("W2"), Some(PointPosition::Left.into()));

    // Der Hauptrechner kommt zurück: Er wird nicht mehr gehört.
    b.alive[0] = true;
    b.run(3_000);
    assert_eq!(b.server.leader(), Some(STANDBY_ID));
    assert!(!b.clients[0].connected());
    b.move_point(0, "W2", PointPosition::Right);
    assert_eq!(b.position("W2"), Some(PointPosition::Left.into()));
}

const NORDBAHN: &str = include_str!("../../../examples/nordbahn.toml");

/// REQ-COM-010: Bei Netzen mit Stellbereichen schaltet der Feldrechner je Bereich um. Zwei
/// Stellwerke führen je ihre Bereiche; ein drittes für das ganze Netz kommt nur zum Zug, wo
/// eines ausfällt.
#[test]
fn switch_per_area() {
    const WHOLE: u32 = 0x0A00_0003;
    let net = railit_import::toml_format::parse(NORDBAHN)
        .unwrap()
        .topology;
    let sim = Simulator::new(net.clone(), SimConfig::default());
    let mut server = FieldServer::new(sim, &[PRIMARY_ID, STANDBY_ID, WHOLE], SWITCH_MS).unwrap();
    let west_areas = ["Adorf".to_string(), "Strecke".to_string()];
    let mut clients = [
        FieldClient::for_areas(&net, PRIMARY_ID, Some(&west_areas)).unwrap(),
        FieldClient::for_areas(&net, STANDBY_ID, Some(&["Bstadt".to_string()])).unwrap(),
        FieldClient::new(&net, WHOLE).unwrap(),
    ];
    assert!(FieldClient::for_areas(&net, WHOLE, Some(&["Cdorf".to_string()])).is_err());
    // Jedes Element gehört zu genau einer Objektsteuerung.
    let count = |c: &FieldClient| c.links().iter().map(|l| l.elements).sum::<usize>();
    assert_eq!(count(&clients[0]) + count(&clients[1]), count(&clients[2]));
    assert!(
        clients[1]
            .links()
            .iter()
            .all(|l| l.controller.ends_with("-Bstadt"))
    );

    let mut alive = [true, true, false];
    let mut now = 0;
    let mut run =
        |server: &mut FieldServer, clients: &mut [FieldClient; 3], alive: [bool; 3], ms| {
            let end = now + ms;
            while now < end {
                now += STEP_MS;
                for (i, c) in clients.iter_mut().enumerate() {
                    if alive[i] {
                        for pdu in c.tick(now) {
                            server.receive(addr(i), &pdu);
                        }
                    }
                }
                for (to, pdu) in server.tick(Tick::from_millis(now)) {
                    let i = usize::from(to.port() - 1000);
                    if alive[i] {
                        clients[i].receive(now, &pdu);
                    }
                }
            }
        };
    run(&mut server, &mut clients, alive, 1_000);
    alive[2] = true;
    run(&mut server, &mut clients, alive, 2_000);
    let leaders = server.leaders();
    let lead = |area: &str| {
        leaders
            .iter()
            .find(|(a, _)| a.as_deref() == Some(area))
            .and_then(|(_, l)| *l)
    };
    assert_eq!(lead("Adorf"), Some(PRIMARY_ID));
    assert_eq!(lead("Strecke"), Some(PRIMARY_ID));
    assert_eq!(lead("Bstadt"), Some(STANDBY_ID));
    assert_eq!(server.leader(), None, "kein Stellwerk führt alles");
    assert!(clients[0].connected() && clients[1].connected());
    assert!(
        !clients[2].connected(),
        "das dritte Stellwerk führt nirgends"
    );
    let status = server.status();
    assert_eq!(status.areas.len(), 3);

    // Bstadt fällt aus: nach der Umschaltzeit führt dort das dritte Stellwerk, Adorf und die
    // Strecke bleiben beim ersten.
    alive[1] = false;
    run(&mut server, &mut clients, alive, 5_000);
    let leaders = server.leaders();
    let lead = |area: &str| {
        leaders
            .iter()
            .find(|(a, _)| a.as_deref() == Some(area))
            .and_then(|(_, l)| *l)
    };
    assert_eq!(lead("Bstadt"), Some(WHOLE));
    assert_eq!(lead("Adorf"), Some(PRIMARY_ID));
    assert!(clients[0].connected());
}
