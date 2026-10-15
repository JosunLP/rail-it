//! Nachbarstellwerke über EULYNX SCI-ILS und RaSTA (REQ-COM-009, REQ-IXL-017, REQ-IXL-021).
//!
//! Die drei Kerne der Nordbahn tauschen ihre Grenzmeldungen über je eine RaSTA-Verbindung
//! je Paar aus. Zufällige Anforderungen der Erlaubnis und zufällige Störungen der Strecken
//! (Unterbrechung, Verlust, Verfälschung, Laufzeit bis über `t_max` hinaus) wechseln sich ab.
//!
//! Dazu stellen und nehmen beide Bahnhöfe Ausfahrten in die Strecke.
//!
//! Geprüft nach jedem Zyklus: Nie halten beide Seiten einer Grenze die Erlaubnis, nie zeigen
//! Ausfahrten beider Bahnhöfe in die eingleisige Strecke Fahrt, keine Ausfahrt zeigt Fahrt
//! ohne Erlaubnis, kein Kern geht in den sicheren Zustand. Am Ende, mit ungestörten Strecken,
//! stehen alle Verbindungen wieder.

use proptest::prelude::*;
use railit_kernel::{Aspect, OwnedState, Request, RequestId};
use railit_rules::GenericRules;
use railit_runtime::railit_adapters::LinkFaults;
use railit_runtime::{Addressed, Runtime};
use railit_sim::{SimConfig, Simulator};
use railit_types::Duration;

const NORDBAHN: &str = include_str!("../../../examples/nordbahn.toml");
const LINKS: [&str; 2] = ["Adorf-Strecke", "Strecke-Bstadt"];
const BORDERS: [&str; 4] = [
    "Adorf>Strecke",
    "Strecke>Adorf",
    "Strecke>Bstadt",
    "Bstadt>Strecke",
];

/// Ausfahrten in die Strecke: Fahrstraße, Startsignal, Grenze.
const EXITS: [(&str, &str, &str); 4] = [
    ("AN1->Strecke", "AN1", "Adorf>Strecke"),
    ("AN2->Strecke", "AN2", "Adorf>Strecke"),
    ("BP1->Strecke", "BP1", "Bstadt>Strecke"),
    ("BP2->Strecke", "BP2", "Bstadt>Strecke"),
];

fn nordbahn() -> Runtime<Simulator> {
    let net = railit_import::parse("nordbahn.toml", NORDBAHN)
        .unwrap()
        .topology;
    let mut rt = railit_runtime::simulation(
        net,
        GenericRules::default(),
        SimConfig::default(),
        Duration::from_millis(100),
    )
    .unwrap();
    rt.use_ils().unwrap();
    rt
}

fn holding(rt: &Runtime<Simulator>, address: &str) -> bool {
    let (area, border) = rt.find_border(address).unwrap();
    let state: &OwnedState = rt.state_of(area).unwrap();
    state.borders()[border.index().unwrap()].holding()
}

fn proceed(rt: &Runtime<Simulator>, signal: &str) -> bool {
    let (area, id) = rt.find_signal(signal).unwrap();
    rt.state_of(area).unwrap().signals()[id.index().unwrap()].commanded() == Aspect::Proceed
}

fn check(rt: &Runtime<Simulator>) -> Result<(), TestCaseError> {
    let [a, sa, sb, b] = BORDERS.map(|x| holding(rt, x));
    prop_assert!(!(a && sa), "beide Seiten halten Adorf/Strecke");
    prop_assert!(!(sb && b), "beide Seiten halten Strecke/Bstadt");
    let mut from = [false; 2];
    for (k, (_, signal, border)) in EXITS.iter().enumerate() {
        if proceed(rt, signal) {
            prop_assert!(holding(rt, border), "{signal} zeigt Fahrt ohne Erlaubnis");
            from[k / 2] = true;
        }
    }
    prop_assert!(
        !(from[0] && from[1]),
        "Gegenfahrt in die eingleisige Strecke"
    );
    Ok(())
}

#[derive(Clone, Debug)]
enum Action {
    /// Erlaubnis an einer Grenze anfordern.
    Request(usize),
    /// Ausfahrt stellen (`true`) oder zurücknehmen.
    Exit(usize, bool),
    /// Strecke stören.
    Fault(usize, LinkFaults),
    /// So viele Zyklen laufen.
    Run(u16),
}

fn action() -> impl Strategy<Value = Action> {
    let faults = (any::<bool>(), 0u8..60, 0u8..40, 0u32..2500).prop_map(
        |(down, loss_percent, corrupt_percent, delay_ms)| LinkFaults {
            down,
            loss_percent,
            corrupt_percent,
            delay_ms,
        },
    );
    prop_oneof![
        (0..BORDERS.len()).prop_map(Action::Request),
        (0..EXITS.len(), any::<bool>()).prop_map(|(k, set)| Action::Exit(k, set)),
        (0..LINKS.len(), faults).prop_map(|(l, f)| Action::Fault(l, f)),
        (1u16..120).prop_map(Action::Run),
    ]
}

fn run(actions: &[Action]) -> Result<(), TestCaseError> {
    let mut rt = nordbahn();
    let mut next = 0u64;
    let cycle = |rt: &mut Runtime<Simulator>, requests: &[Addressed]| {
        let out = rt.cycle_addressed(requests);
        prop_assert_eq!(out.safe_state, None);
        check(rt)
    };
    for a in actions {
        match a {
            Action::Request(i) => {
                let (area, border) = rt.find_border(BORDERS[*i]).unwrap();
                next += 1;
                let r = Addressed {
                    area,
                    id: RequestId(next),
                    request: Request::RequestPermission(border),
                };
                cycle(&mut rt, &[r])?;
            }
            Action::Exit(k, set) => {
                let (area, route) = rt.find_route(EXITS[*k].0).unwrap();
                next += 1;
                let r = Addressed {
                    area,
                    id: RequestId(next),
                    request: if *set {
                        Request::SetRoute(route)
                    } else {
                        Request::CancelRoute(route)
                    },
                };
                cycle(&mut rt, &[r])?;
            }
            Action::Fault(l, f) => rt.set_ils_faults(LINKS[*l], *f).unwrap(),
            Action::Run(n) => {
                for _ in 0..*n {
                    cycle(&mut rt, &[])?;
                }
            }
        }
    }
    for link in LINKS {
        rt.set_ils_faults(link, LinkFaults::default()).unwrap();
    }
    for _ in 0..100 {
        cycle(&mut rt, &[])?;
    }
    prop_assert!(rt.ils_links().iter().all(|l| l.up), "{:?}", rt.ils_links());
    Ok(())
}

/// Wenige Fälle im gewöhnlichen Testlauf; die CI setzt `PROPTEST_CASES`.
fn config() -> ProptestConfig {
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(48);
    ProptestConfig {
        cases,
        ..ProptestConfig::default()
    }
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn permission_stays_exclusive_over_disturbed_ils(
        actions in prop::collection::vec(action(), 1..40)
    ) {
        run(&actions)?;
    }
}

/// Richtungswechsel über SCI-ILS mit gestörten Strecken: Die Erlaubnis wandert, sobald die
/// Meldungen durchkommen, und nur dann.
#[test]
fn direction_change_waits_for_the_link() {
    let mut rt = nordbahn();
    let mut next = 0u64;
    let mut request = |rt: &mut Runtime<Simulator>, address: &str| {
        let (area, border) = rt.find_border(address).unwrap();
        next += 1;
        rt.cycle_addressed(&[Addressed {
            area,
            id: RequestId(next),
            request: Request::RequestPermission(border),
        }]);
    };
    let run = |rt: &mut Runtime<Simulator>, n: usize| {
        for _ in 0..n {
            let out = rt.cycle_addressed(&[]);
            assert_eq!(out.safe_state, None);
            check(rt).unwrap();
        }
    };
    run(&mut rt, 70);
    assert!(holding(&rt, "Adorf>Strecke") && holding(&rt, "Strecke>Bstadt"));

    // Unterbrochen: Die Strecke fordert an, Adorf erfährt es nicht.
    let down = LinkFaults {
        down: true,
        ..LinkFaults::default()
    };
    rt.set_ils_faults("Adorf-Strecke", down).unwrap();
    request(&mut rt, "Strecke>Adorf");
    run(&mut rt, 100);
    assert!(holding(&rt, "Adorf>Strecke"));
    assert!(!holding(&rt, "Strecke>Adorf"));
    assert!(!rt.ils_links()[0].up);

    // Stark gestört, aber verbunden: Die Erlaubnis wandert.
    let noisy = LinkFaults {
        loss_percent: 30,
        corrupt_percent: 20,
        delay_ms: 150,
        down: false,
    };
    rt.set_ils_faults("Adorf-Strecke", noisy).unwrap();
    run(&mut rt, 200);
    assert!(!holding(&rt, "Adorf>Strecke"));
    assert!(holding(&rt, "Strecke>Adorf"));
    let status = &rt.ils_links()[0];
    assert!(status.up && status.discarded > 0, "{status:?}");
}

/// Zwei Prozesse (hier zwei Runtimes): Adorf und die Strecke in einem, Bstadt im anderen; die
/// Grenzmeldungen zwischen Strecke und Bstadt laufen über SCI-ILS auf UDP (REQ-COM-010).
mod split {
    use super::*;
    use railit_runtime::railit_ops::{OpsConfig, OpsEvent, Timetable};

    fn part(areas: &[&str]) -> Runtime<Simulator> {
        let net = railit_import::parse("nordbahn.toml", NORDBAHN)
            .unwrap()
            .topology;
        railit_runtime::simulation_areas(
            net,
            areas,
            GenericRules::default(),
            SimConfig::default(),
            Duration::from_millis(100),
        )
        .unwrap()
    }

    fn holds(rt: &Runtime<Simulator>, address: &str) -> bool {
        let (area, border) = rt.find_border(address).unwrap();
        rt.state_of(area).unwrap().borders()[border.index().unwrap()].holding()
    }

    fn request(rt: &mut Runtime<Simulator>, id: u64, address: &str) {
        let (area, border) = rt.find_border(address).unwrap();
        rt.cycle_addressed(&[Addressed {
            area,
            id: RequestId(id),
            request: Request::RequestPermission(border),
        }]);
    }

    fn set_route(rt: &mut Runtime<Simulator>, id: u64, name: &str) {
        let (area, route) = rt.find_route(name).unwrap();
        rt.cycle_addressed(&[Addressed {
            area,
            id: RequestId(id),
            request: Request::SetRoute(route),
        }]);
    }

    fn connect() -> (Runtime<Simulator>, Runtime<Simulator>) {
        let any: std::net::SocketAddr = "127.0.0.1:0".parse().unwrap();
        let mut west = part(&["Adorf", "Strecke"]);
        let mut east = part(&["Bstadt"]);
        assert_eq!(west.remote_neighbours(), ["Bstadt"]);
        assert_eq!(east.remote_neighbours(), ["Strecke"]);
        // Adressen erst nach dem Binden bekannt: zuerst mit Platzhalter binden geht nicht,
        // also zwei feste Sockets über Port 0 und gegenseitig eintragen.
        let probe_w = std::net::UdpSocket::bind(any).unwrap();
        let probe_e = std::net::UdpSocket::bind(any).unwrap();
        let (aw, ae) = (probe_w.local_addr().unwrap(), probe_e.local_addr().unwrap());
        drop((probe_w, probe_e));
        west.use_ils_udp(aw, vec![("Bstadt".into(), ae)]).unwrap();
        east.use_ils_udp(ae, vec![("Strecke".into(), aw)]).unwrap();
        (west, east)
    }

    fn run(west: &mut Runtime<Simulator>, east: &mut Runtime<Simulator>, n: usize, both: bool) {
        for _ in 0..n {
            assert_eq!(west.cycle_addressed(&[]).safe_state, None);
            if both {
                assert_eq!(east.cycle_addressed(&[]).safe_state, None);
            }
            // Laufzeit auf localhost: kurz warten, damit die Datagramme da sind.
            std::thread::sleep(std::time::Duration::from_micros(300));
        }
    }

    #[test]
    fn border_between_processes() {
        let (mut west, mut east) = connect();
        assert!(
            west.use_ils_udp("127.0.0.1:0".parse().unwrap(), vec![])
                .is_err()
        );
        run(&mut west, &mut east, 80, true);
        assert!(
            west.ils_links().iter().all(|l| l.up),
            "{:?}",
            west.ils_links()
        );
        assert!(east.ils_links()[0].up && east.ils_links()[0].remote.as_deref() == Some("Strecke"));
        assert!(holds(&west, "Adorf>Strecke") && holds(&west, "Strecke>Bstadt"));
        assert!(!holds(&east, "Bstadt>Strecke"));

        // Richtungswechsel über beide Grenzen, die zweite über UDP.
        request(&mut west, 1, "Strecke>Adorf");
        run(&mut west, &mut east, 40, true);
        request(&mut east, 2, "Bstadt>Strecke");
        run(&mut west, &mut east, 60, true);
        assert!(holds(&west, "Strecke>Adorf") && !holds(&west, "Strecke>Bstadt"));
        assert!(holds(&east, "Bstadt>Strecke"));

        // Ausfahrt aus Bstadt in die Strecke: Fahrt nur mit aktueller Meldung über UDP.
        set_route(&mut east, 3, "BP2->Strecke");
        run(&mut west, &mut east, 80, true);
        let (area, id) = east.find_signal("BP2").unwrap();
        let aspect = |rt: &Runtime<Simulator>| {
            rt.state_of(area).unwrap().signals()[id.index().unwrap()].commanded()
        };
        assert_eq!(aspect(&east), Aspect::Proceed);

        // Der westliche Prozess fällt aus: Bstadt nimmt die Fahrt zurück.
        let mut lost = false;
        for _ in 0..40 {
            let out = east.cycle_addressed(&[]);
            lost |= out.events.iter().any(|e| {
                matches!(
                    e.event,
                    railit_kernel::Event::Alarm(railit_kernel::Alarm::BorderLost(_))
                )
            });
        }
        assert!(lost, "Grenze nicht gestört gemeldet");
        assert_eq!(aspect(&east), Aspect::Stop);
        assert!(!east.ils_links()[0].up);
    }

    /// REQ-ETCS-004: Die Fahrerlaubnis eines ETCS-Zuges reicht über die Grenze in einen
    /// anderen Prozess, so weit das RBC dort sie vorausmeldet; fällt die Verbindung aus, endet
    /// sie wieder an der Grenze.
    #[test]
    fn etcs_authority_reaches_into_another_process() {
        let (mut west, mut east) = connect();
        run(&mut west, &mut east, 80, true);
        assert!(holds(&west, "Strecke>Bstadt"));
        let net = railit_import::parse("nordbahn.toml", NORDBAHN)
            .unwrap()
            .topology;
        let Some(railit_topology::ElementRef::Signal(sbk1)) = net.lookup("Sbk1") else {
            panic!("Sbk1");
        };
        west.field_mut()
            .add_train(railit_sim::TrainSpec {
                name: "ICE1".into(),
                signal: sbk1,
                length: railit_types::Length::from_mm(200_000),
                speed: railit_types::Speed::from_kmh(0),
                ignore_signals: false,
                etcs: true,
                pzb: false,
            })
            .unwrap();
        let end = |west: &Runtime<Simulator>| {
            west.rbc()
                .authority("ICE1")
                .map(|a| (a.end.clone(), a.eoa_mm))
        };
        set_route(&mut west, 6, "Sbk1->Bstadt");
        run(&mut west, &mut east, 20, true);
        // Ohne Fahrstraße in Bstadt: bis zum Einfahrsignal BA.
        let (e, short) = end(&west).unwrap_or_else(|| panic!("{:?}", west.etcs_snapshot()));
        assert_eq!(e, "BA");
        set_route(&mut east, 7, "BA-BN1");
        let mut cycles = 0;
        while end(&west).is_none_or(|(e, _)| e != "BN1") {
            cycles += 1;
            assert!(cycles < 150, "{:?}", west.etcs_snapshot());
            run(&mut west, &mut east, 1, true);
        }
        let (_, long) = end(&west).unwrap();
        assert!(long > short);
        let passable = west.rbc().authority("ICE1").unwrap().passable.clone();
        assert!(
            passable
                .iter()
                .any(|(area, _)| *area == railit_etcs::REMOTE_AREA),
            "{passable:?}"
        );

        // Bstadt schweigt: Die Vorschau veraltet, die Fahrerlaubnis endet an der Grenze.
        for _ in 0..20 {
            west.cycle_addressed(&[]);
        }
        let (e, _) = end(&west).unwrap_or_else(|| panic!("{:?}", west.etcs_snapshot()));
        assert_eq!(e, "Strecke>Bstadt");
    }

    /// Zwei Prozesse mit Zuglenkung und dem Nordbahn-Fahrplan, aus dem nur `train` fährt
    /// (jeder Prozess hat hier einen eigenen Simulator; ein Zug des anderen Prozesses bliebe
    /// dort physisch stehen und sperrte die eingleisige Strecke).
    fn handover(
        train: &str,
    ) -> (
        Runtime<Simulator>,
        Runtime<Simulator>,
        Vec<OpsEvent>,
        Vec<OpsEvent>,
    ) {
        const FAHRPLAN: &str = include_str!("../../../examples/nordbahn-fahrplan.toml");
        let net = railit_import::parse("nordbahn.toml", NORDBAHN)
            .unwrap()
            .topology;
        let mut timetable = Timetable::parse(FAHRPLAN).unwrap();
        timetable.trains.retain(|t| t.number == train);
        let (mut west, mut east) = connect();
        let cfg = OpsConfig {
            ars: true,
            ..OpsConfig::default()
        };
        for rt in [&mut west, &mut east] {
            rt.enable_ops(&net, cfg, timetable.clone()).unwrap();
        }
        let (mut w_events, mut e_events) = (Vec::new(), Vec::new());
        for i in 0..6000 {
            w_events.extend(west.cycle_addressed(&[]).ops);
            e_events.extend(east.cycle_addressed(&[]).ops);
            std::thread::sleep(std::time::Duration::from_micros(200));
            let requested = |events: &[OpsEvent]| {
                events.iter().any(|e| {
                    matches!(e, OpsEvent::TakenOver { .. })
                }) && events.iter().rev().take_while(|e| !matches!(e, OpsEvent::TakenOver { .. })).any(
                    |e| matches!(e, OpsEvent::RouteRequested { train: t, .. } if t == train),
                )
            };
            if i > 10 && (requested(&w_events) || requested(&e_events)) {
                break;
            }
        }
        (west, east, w_events, e_events)
    }

    fn spawned(events: &[OpsEvent]) -> Vec<String> {
        events
            .iter()
            .filter_map(|e| match e {
                OpsEvent::Spawned { train, .. } => Some(train.clone()),
                _ => None,
            })
            .collect()
    }

    fn taken_over(events: &[OpsEvent], train: &str) -> Option<String> {
        events.iter().find_map(|e| match e {
            OpsEvent::TakenOver { train: t, signal } if t == train => Some(signal.clone()),
            _ => None,
        })
    }

    /// REQ-OPS-006: Betriebsführung über Prozessgrenzen, von West nach Ost. Jeder Prozess setzt
    /// die Züge seiner Bereiche ein und lenkt sie; an der Grenze zum anderen Prozess übergibt
    /// er Zugnummer und Fahrplanstand über SCI-ILS, und drüben stellt die Zuglenkung ab dem
    /// Einfahrsignal weiter.
    #[test]
    fn trains_are_handed_over_eastwards() {
        let (west, east, w_events, e_events) = handover("RE1");
        assert_eq!(spawned(&w_events), ["RE1"]);
        assert!(spawned(&e_events).is_empty());
        assert!(
            w_events.iter().any(|e| matches!(e, OpsEvent::HandedOver { train, area, .. } if train == "RE1" && area == "Bstadt")),
            "{w_events:?}"
        );
        let entry = taken_over(&e_events, "RE1")
            .unwrap_or_else(|| panic!("RE1 nicht übernommen: {e_events:?}"));
        assert_eq!(east.ops().unwrap().berth_of("RE1"), Some(entry.as_str()));
        assert!(
            e_events
                .iter()
                .any(|e| matches!(e, OpsEvent::RouteRequested { train, .. } if train == "RE1")),
            "{e_events:?}"
        );
        // Im abgebenden Prozess steht der Zug als übergeben; nichts wartet auf die Verbindung.
        let snap = west.ops().unwrap().snapshot();
        let re1 = snap.trains.iter().find(|t| t.number == "RE1").unwrap();
        assert_eq!(re1.elsewhere.as_deref(), Some("Bstadt"));
        assert_eq!(re1.berth, None);
        assert!(west.ils_links().iter().all(|l| l.pending_handovers == 0));
        // Vormeldung und Übergabe.
        assert_eq!(east.ils_links()[0].handovers, 2);
        let announced = e_events
            .iter()
            .position(|e| matches!(e, OpsEvent::Announced { train, .. } if train == "RE1"));
        let taken = e_events
            .iter()
            .position(|e| matches!(e, OpsEvent::TakenOver { train, .. } if train == "RE1"));
        assert!(announced.is_some() && announced < taken, "{e_events:?}");
        // Fremde Signale nimmt die Zugnummernmeldung nicht an.
        let mut west = west;
        assert!(west.ops_mut().unwrap().describe("X1", "BP1").is_err());
    }

    /// REQ-OPS-006: dasselbe von Ost nach West (der annehmende Prozess baut die Verbindung
    /// auf).
    #[test]
    fn trains_are_handed_over_westwards() {
        let (west, _east, w_events, e_events) = handover("RE2");
        assert_eq!(spawned(&e_events), ["RE2"]);
        assert!(spawned(&w_events).is_empty());
        let entry = taken_over(&w_events, "RE2")
            .unwrap_or_else(|| panic!("RE2 nicht übernommen: {w_events:?}\n{e_events:?}"));
        assert_eq!(west.ops().unwrap().berth_of("RE2"), Some(entry.as_str()));
        // Fahrplanstand: Bstadt ist erledigt, nächster Halt Adorf.
        let snap = west.ops().unwrap().snapshot();
        let re2 = snap.trains.iter().find(|t| t.number == "RE2").unwrap();
        assert_eq!(re2.next_stop.as_deref(), Some("Adorf/2"), "{re2:?}");
    }

    /// REQ-COM-010: Bereitschaftsrechner für einen Teil des Netzes. Der Nachbar kennt beide
    /// Adressen des westlichen Stellwerks; fällt der Hauptrechner aus, übernimmt der
    /// Bereitschaftsrechner dessen Zustand, die SCI-ILS-Verbindung baut sich zu ihm neu auf,
    /// und der Erlaubniswechsel geht mit den übernommenen laufenden Nummern weiter.
    #[test]
    fn standby_takes_over_a_part_of_the_network() {
        let any: std::net::SocketAddr = "127.0.0.1:0".parse().unwrap();
        let free = || {
            let s = std::net::UdpSocket::bind(any).unwrap();
            s.local_addr().unwrap()
        };
        let (aw, aws, ae) = (free(), free(), free());
        let mut west = part(&["Adorf", "Strecke"]);
        let mut standby = part(&["Adorf", "Strecke"]);
        let mut east = part(&["Bstadt"]);
        west.use_ils_udp(aw, vec![("Bstadt".into(), ae)]).unwrap();
        standby
            .use_ils_udp(aws, vec![("Bstadt".into(), ae)])
            .unwrap();
        east.use_ils_udp(ae, vec![("Strecke".into(), aw), ("Strecke".into(), aws)])
            .unwrap();
        run(&mut west, &mut east, 80, true);
        assert!(holds(&west, "Strecke>Bstadt"));
        let seq = |rt: &Runtime<Simulator>, address: &str| {
            let (area, border) = rt.find_border(address).unwrap();
            rt.state_of(area).unwrap().borders()[border.index().unwrap()].seq()
        };
        let before = seq(&west, "Strecke>Bstadt");

        // Der Hauptrechner fällt aus; der Bereitschaftsrechner übernimmt seinen Zustand.
        let states: Vec<OwnedState> = (0..2).map(|a| west.state_of(a).unwrap().clone()).collect();
        let now = west.now();
        drop(west);
        standby.take_over(&states, now, "Übernahme").unwrap();
        assert!(holds(&standby, "Strecke>Bstadt"));
        let mut up = false;
        for _ in 0..200 {
            assert_eq!(standby.cycle_addressed(&[]).safe_state, None);
            assert_eq!(east.cycle_addressed(&[]).safe_state, None);
            std::thread::sleep(std::time::Duration::from_micros(300));
            up = standby.ils_links().iter().all(|l| l.up) && east.ils_links()[0].up;
            if up {
                break;
            }
        }
        assert!(up, "SCI-ILS baut sich zum Bereitschaftsrechner nicht auf");

        // Richtungswechsel mit dem neuen Rechner.
        request(&mut standby, 1, "Strecke>Adorf");
        run(&mut standby, &mut east, 40, true);
        request(&mut east, 2, "Bstadt>Strecke");
        run(&mut standby, &mut east, 80, true);
        assert!(holds(&east, "Bstadt>Strecke"), "Erlaubnis nicht gewechselt");
        assert!(!holds(&standby, "Strecke>Bstadt"));
        assert!(seq(&standby, "Strecke>Bstadt") > before);
    }
}
