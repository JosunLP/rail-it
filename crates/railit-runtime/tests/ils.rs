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
