//! Erlaubniswechsel und Grenzsicherung (REQ-IXL-017 … REQ-IXL-020) am Beispiel der Nordbahn.
//!
//! ```text
//!   Adorf: … ─▶AN1 ┬ Ao1 ◀AF─ Ao ═ Strecke: S1 ─▶Sbk1 ─ S2 ═ Bstadt
//!                  AW2
//! ```
//!
//! Adorf hat eine Grenze „>Strecke“ (Einfahrsignal AF, Annäherungsabschnitt ATO, ATO1) und
//! übernimmt beim Anlauf die Erlaubnis. Die Strecke hat die Grenze „>Adorf“ (Einfahrsignal
//! Sbk1, Annäherungsabschnitt ST1, ST2).

mod common;

use common::{Harness, accepted, leak, rejected};
use proptest::prelude::*;
use railit_kernel::*;
use railit_planning::{PlanningConfig, compile_areas};
use railit_types::BorderId;

const NORDBAHN: &str = include_str!("../../../examples/nordbahn.toml");
const B: BorderId = BorderId::new(0);

fn area(name: &str) -> Harness {
    let net = railit_import::toml_format::parse(NORDBAHN)
        .unwrap()
        .topology;
    let areas = compile_areas(&net, None, &PlanningConfig::default()).unwrap();
    let plan = &areas.iter().find(|a| a.name == name).unwrap().compiled.plan;
    let mut h = Harness::with_plan(leak(plan));
    h.step(&[]);
    h
}

#[allow(clippy::fn_params_excessive_bools)]
fn peer(
    holding: bool,
    seq: u32,
    offer: bool,
    approach_clear: bool,
    request: bool,
) -> BorderMessage {
    BorderMessage {
        holding,
        seq,
        offer,
        approach_clear,
        request,
        exclusive: approach_clear,
    }
}

fn state(h: &Harness) -> BorderState {
    h.state.borders()[0]
}

fn alarms(events: &[Event]) -> Vec<Alarm> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Alarm(a) => Some(*a),
            _ => None,
        })
        .collect()
}

/// Adorf mit einem Nachbarn, der nichts hält und dessen Annäherungsabschnitt frei ist; nach
/// der Einschwingzeit hält Adorf die Erlaubnis.
fn adorf_holding() -> Harness {
    let mut h = area("Adorf");
    h.border_in[0] = Some(peer(false, 0, false, true, false));
    for _ in 0..55 {
        h.step(&[]);
    }
    assert!(state(&h).holding());
    h
}

/// REQ-IXL-019: Beim Anlauf ist der Annäherungsabschnitt verschlossen, bevor die erste
/// Anforderung zugelassen wird. Eine Fahrstraße, die im ersten Zyklus eine Weiche des
/// Annäherungsabschnitts in der anderen Lage verschließen würde, wird abgewiesen (vorher ging
/// der Kern in den sicheren Zustand; gefunden vom Übernahmetest der Runtime).
#[test]
fn first_cycle_request_respects_approach_lock() {
    let net = railit_import::toml_format::parse(NORDBAHN)
        .unwrap()
        .topology;
    let areas = compile_areas(&net, None, &PlanningConfig::default()).unwrap();
    let plan = &areas
        .iter()
        .find(|a| a.name == "Adorf")
        .unwrap()
        .compiled
        .plan;
    let mut h = Harness::with_plan(leak(plan));
    let r = h.route("AA-AN1");
    let events = h.step(&[Request::SetRoute(r)]);
    assert_eq!(h.last, Ok(()));
    assert_eq!(rejected(&events), Some(Rejection::Conflict));
    assert!(state(&h).approach_locked());
    h.run(5);
    assert_eq!(h.last, Ok(()));
}

/// REQ-IXL-017: Der beim Anlauf bestimmte Kern übernimmt die Erlaubnis erst, wenn der
/// Nachbar die Einschwingzeit lang meldet, sie nicht zu halten.
#[test]
fn designated_side_takes_permission_after_settle_time() {
    let mut h = area("Adorf");
    assert!(state(&h).approach_locked());
    assert!(h.tvd_state("ATO").border_locked());

    // Ohne Meldung des Nachbarn nie.
    for _ in 0..80 {
        h.step(&[]);
    }
    assert!(!state(&h).holding());

    h.border_in[0] = Some(peer(false, 0, false, true, false));
    for _ in 0..49 {
        h.step(&[]);
    }
    assert!(!state(&h).holding(), "vor Ablauf der Einschwingzeit");
    let mut events = Vec::new();
    for _ in 0..3 {
        events.extend(h.step(&[]));
    }
    assert!(state(&h).holding());
    assert!(events.contains(&Event::Permission {
        border: B,
        holding: true
    }));
    assert!(!state(&h).approach_locked());
    assert!(!h.tvd_state("ATO").border_locked());
    assert_eq!(h.border_out[0].unwrap().seq, 1);

    // Meldet der Nachbar zwischendurch „halte“, beginnt die Einschwingzeit neu.
    let mut h = area("Adorf");
    h.border_in[0] = Some(peer(false, 0, false, true, false));
    h.run(30);
    h.border_in[0] = Some(peer(true, 4, false, true, false));
    h.step(&[]);
    h.border_in[0] = Some(peer(false, 4, false, true, false));
    h.run(30);
    assert!(!state(&h).holding());
    h.run(25);
    assert!(state(&h).holding());
    assert_eq!(state(&h).seq(), 5, "größer als die Nummer des Nachbarn");
}

/// REQ-IXL-020: Fahrt über die Grenze nur mit Erlaubnis und freiem Annäherungsabschnitt des
/// Nachbarn.
#[test]
fn exit_needs_permission_and_clear_neighbour() {
    let mut h = area("Adorf");
    h.border_in[0] = Some(peer(false, 0, false, true, false));
    let exit = h.route("AN1->Strecke");
    let events = h.step(&[Request::SetRoute(exit)]);
    assert_eq!(rejected(&events), Some(Rejection::NoPermission(B)));

    h.run(55);
    assert!(state(&h).holding());
    h.border_in[0] = Some(peer(false, 0, false, false, false));
    assert!(accepted(&h.step(&[Request::SetRoute(exit)])));
    h.run(3);
    assert_eq!(h.phase("AN1->Strecke"), RoutePhase::Locked);
    assert_eq!(h.aspect("AN1"), Aspect::Stop);

    h.border_in[0] = Some(peer(false, 0, false, true, false));
    h.run(2);
    assert_eq!(h.aspect("AN1"), Aspect::Proceed);

    // Der Nachbar meldet seinen Abschnitt besetzt: Signal fällt.
    h.border_in[0] = Some(peer(false, 0, false, false, false));
    let events = h.step(&[]);
    assert!(alarms(&events).contains(&Alarm::SignalDropped(exit)));
    assert_eq!(h.aspect("AN1"), Aspect::Stop);
}

/// REQ-IXL-018: Abgabe nur auf Anforderung, ohne Fahrstraße über die Grenze und bei freien
/// Annäherungsabschnitten; danach ist der eigene Annäherungsabschnitt verschlossen.
#[test]
fn permission_is_given_only_when_safe() {
    let mut h = adorf_holding();
    let exit = h.route("AN1->Strecke");
    assert!(accepted(&h.step(&[Request::SetRoute(exit)])));
    h.run(3);

    h.border_in[0] = Some(peer(false, 0, false, true, true));
    h.run(5);
    assert!(state(&h).holding(), "Ausfahrt eingestellt");

    assert!(accepted(&h.step(&[Request::CancelRoute(exit)])));
    // Die Ausfahrt zeigte Fahrt: Zeitverschluss läuft.
    h.run(5);
    assert!(state(&h).holding());
    h.step_after(61_000, &[]);
    assert_eq!(h.phase("AN1->Strecke"), RoutePhase::Idle);

    // Eigener Annäherungsabschnitt besetzt: keine Abgabe.
    h.set_tvd("ATO", Occupancy::Occupied);
    h.run(3);
    assert!(state(&h).holding());
    h.set_tvd("ATO", Occupancy::Clear);

    // Nachbar meldet seinen Abschnitt nicht frei: keine Abgabe.
    h.border_in[0] = Some(peer(false, 0, false, false, true));
    h.run(3);
    assert!(state(&h).holding());

    h.border_in[0] = Some(peer(false, 0, false, true, true));
    let events = h.step(&[]);
    assert!(events.contains(&Event::Permission {
        border: B,
        holding: false
    }));
    let s = state(&h);
    assert!(!s.holding() && s.offering() && s.approach_locked());
    assert_eq!(s.seq(), 2);
    let out = h.border_out[0].unwrap();
    assert!(out.offer && !out.holding && out.seq == 2);
    assert!(
        out.approach_clear,
        "eigener Annäherungsabschnitt frei gemeldet"
    );

    // Jetzt ist die Ausfahrt gesperrt, die Einfahrt ab AF (Fortsetzung) erlaubt.
    assert_eq!(
        rejected(&h.step(&[Request::SetRoute(exit)])),
        Some(Rejection::NoPermission(B))
    );
    let entry = h.route("AF-AP1");
    assert!(accepted(&h.step(&[Request::SetRoute(entry)])));
}

/// REQ-IXL-017: Angebote werden nur mit größerer Nummer angenommen; ein altes Angebot, das
/// noch unterwegs ist, bleibt wirkungslos.
#[test]
fn only_newer_offers_are_taken() {
    let mut h = adorf_holding();
    h.border_in[0] = Some(peer(false, 0, false, true, true));
    h.step(&[]);
    assert_eq!(state(&h).seq(), 2);

    // Nachbar übernimmt, dann trifft sein Echo des alten Angebots ein: keine Übernahme.
    h.border_in[0] = Some(peer(true, 2, false, true, false));
    h.run(3);
    h.border_in[0] = Some(peer(false, 2, true, true, false));
    h.run(3);
    assert!(!state(&h).holding());

    // Neues Angebot mit Nummer 3: Übernahme.
    h.border_in[0] = Some(peer(false, 3, true, true, false));
    let events = h.step(&[]);
    assert!(state(&h).holding());
    assert_eq!(state(&h).seq(), 3);
    assert!(events.contains(&Event::Permission {
        border: B,
        holding: true
    }));
}

/// REQ-IXL-011, REQ-IXL-019: Züge aus dem Nachbarbereich dürfen den verschlossenen
/// Annäherungsabschnitt besetzen; hält dieser Kern die Erlaubnis, ist das unerlaubt.
#[test]
fn incoming_trains_are_authorized_only_without_permission() {
    let mut h = area("Adorf");
    h.set_tvd("ATO", Occupancy::Occupied);
    assert!(alarms(&h.step(&[])).is_empty());
    h.set_tvd("ATO", Occupancy::Clear);

    let mut h = adorf_holding();
    h.set_tvd("ATO", Occupancy::Occupied);
    assert_eq!(
        alarms(&h.step(&[])),
        vec![Alarm::UnauthorizedOccupancy(h.tvd("ATO"))]
    );
}

/// Ausfall der Verbindung: Alarm, Fahrt über die Grenze fällt.
#[test]
fn lost_neighbour_drops_exit_signal() {
    let mut h = adorf_holding();
    let exit = h.route("AN1->Strecke");
    assert!(accepted(&h.step(&[Request::SetRoute(exit)])));
    h.run(3);
    assert_eq!(h.aspect("AN1"), Aspect::Proceed);

    h.border_in[0] = None;
    let mut events = Vec::new();
    for _ in 0..25 {
        events.extend(h.step(&[]));
    }
    let alarms = alarms(&events);
    assert!(alarms.contains(&Alarm::BorderLost(B)));
    assert!(alarms.contains(&Alarm::SignalDropped(exit)));
    assert_eq!(h.aspect("AN1"), Aspect::Stop);
    assert_eq!(h.last, Ok(()));
}

/// Beide Seiten melden „halte“: Alarm, keine Fahrt.
#[test]
fn permission_conflict_is_alarmed() {
    let mut h = adorf_holding();
    let exit = h.route("AN1->Strecke");
    assert!(accepted(&h.step(&[Request::SetRoute(exit)])));
    h.run(3);
    h.border_in[0] = Some(peer(true, 9, false, true, false));
    let events = h.step(&[]);
    assert!(alarms(&events).contains(&Alarm::PermissionConflict(B)));
    assert_eq!(h.aspect("AN1"), Aspect::Stop);
}

/// REQ-IXL-014: Erlaubnis zurückholen nur als bestätigte Hilfshandlung und nach der
/// Einschwingzeit.
#[test]
fn take_back_permission_by_aux_operation() {
    let mut h = adorf_holding();
    h.border_in[0] = Some(peer(false, 0, false, true, true));
    h.step(&[]);
    assert!(!state(&h).holding());

    // Der Nachbar übernimmt das Angebot nicht.
    h.border_in[0] = Some(peer(false, 0, false, true, false));
    h.run(60);
    assert!(!state(&h).holding(), "ohne Hilfshandlung keine Rücknahme");

    let events = h.aux(AuxOperation::TakePermission(B));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::AuxExecuted { .. })),
        "{events:?}"
    );
    h.step(&[]);
    assert!(state(&h).holding());
    assert_eq!(state(&h).seq(), 3);

    // Hält der Nachbar, ist die Hilfshandlung nicht anwendbar.
    let mut h = area("Adorf");
    h.border_in[0] = Some(peer(true, 1, false, true, false));
    h.run(3);
    let events = h.aux(AuxOperation::TakePermission(B));
    assert_eq!(common::rejected(&events), Some(Rejection::AuxNotApplicable));
}

// ── Zwei echte Kerne ─────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
enum Action {
    /// Erlaubnis in Adorf (0) oder auf der Strecke (1) anfordern.
    Request(usize),
    /// Ausfahrt über die Grenze einstellen (Adorf: AN1, Strecke: Sbk2).
    Exit(usize),
    /// Ausfahrt zurücknehmen.
    Cancel(usize),
    /// Neustart eines Kerns (Zustand verloren).
    Restart(usize),
    /// Laufzeit der nächsten Meldungen (Zyklen).
    Delay(u64),
    Run(u8),
}

fn action() -> impl Strategy<Value = Action> {
    prop_oneof![
        3 => (0usize..2).prop_map(Action::Request),
        2 => (0usize..2).prop_map(Action::Exit),
        1 => (0usize..2).prop_map(Action::Cancel),
        1 => (0usize..2).prop_map(Action::Restart),
        1 => (1u64..=10).prop_map(Action::Delay),
        4 => (1u8..40).prop_map(Action::Run),
    ]
}

struct Link {
    /// (Zustellzyklus, Meldung) je Richtung: 0 = an Adorf, 1 = an Strecke.
    queues: [Vec<(u64, BorderMessage)>; 2],
    delay: u64,
}

/// REQ-IXL-017: Mit zwei echten Kernen, beliebigen Anforderungen, Ausfahrten, Neustarts
/// und Laufzeiten bis zu einer Sekunde halten nie beide Seiten zugleich die Erlaubnis; eine
/// Ausfahrt zeigt nur Fahrt, wenn ihre Seite die Erlaubnis hält.
#[allow(clippy::needless_range_loop)]
fn run_pair(actions: &[Action]) -> Result<(), TestCaseError> {
    let mut sides = [area("Adorf"), area("Strecke")];
    // Strecke: Grenze 0 ist „>Adorf“.
    assert_eq!(sides[1].plan().border_name(B), Some(">Adorf"));
    let exits = ["AN1->Strecke", "Sbk2->Adorf"];
    let mut link = Link {
        queues: [Vec::new(), Vec::new()],
        delay: 1,
    };
    let mut cycle = 0u64;
    for a in actions {
        let mut requests: [Vec<Request>; 2] = [Vec::new(), Vec::new()];
        let mut cycles = 1;
        match a {
            Action::Request(i) => requests[*i].push(Request::RequestPermission(B)),
            Action::Exit(i) => {
                let r = sides[*i].route(exits[*i]);
                requests[*i].push(Request::SetRoute(r));
            }
            Action::Cancel(i) => {
                let r = sides[*i].route(exits[*i]);
                requests[*i].push(Request::CancelRoute(r));
            }
            Action::Restart(i) => {
                let plan = *sides[*i].plan();
                sides[*i].state = OwnedState::new(&plan);
            }
            Action::Delay(d) => link.delay = *d,
            Action::Run(n) => cycles = u64::from(*n),
        }
        for _ in 0..cycles {
            cycle += 1;
            for (i, side) in sides.iter_mut().enumerate() {
                // Neueste fällige Meldung zustellen.
                let queue = &mut link.queues[i];
                let due: Vec<_> = queue.iter().filter(|(at, _)| *at <= cycle).collect();
                side.border_in[0] = due.last().map(|(_, m)| *m);
                queue.retain(|(at, _)| *at > cycle);
                side.border_out[0] = None;
                let req = std::mem::take(&mut requests[i]);
                side.step(&req);
                prop_assert_eq!(side.last, Ok(()), "sicherer Zustand");
            }
            for i in 0..2 {
                if let Some(m) = sides[i].border_out[0] {
                    link.queues[1 - i].push((cycle + link.delay, m));
                }
            }
            let holding = [state(&sides[0]).holding(), state(&sides[1]).holding()];
            prop_assert!(!(holding[0] && holding[1]), "beide halten die Erlaubnis");
            for i in 0..2 {
                let start = if i == 0 { "AN1" } else { "Sbk2" };
                if sides[i].aspect(start) == Aspect::Proceed {
                    prop_assert!(holding[i], "Fahrt ohne Erlaubnis");
                    prop_assert!(
                        state(&sides[1 - i]).approach_locked(),
                        "Fahrt in einen nicht verschlossenen Abschnitt"
                    );
                }
            }
        }
    }
    Ok(())
}

proptest! {
    // Standard: 256 Fälle; mehr über die Umgebungsvariable PROPTEST_CASES.
    #[test]
    fn never_both_sides_hold_permission(actions in prop::collection::vec(action(), 1..60)) {
        run_pair(&actions)?;
    }
}

/// Ablauf im Normalbetrieb: Die Strecke fordert die Erlaubnis an und erhält sie.
#[test]
fn pair_hands_over_permission() {
    let actions = [
        Action::Run(60),
        Action::Request(1),
        Action::Run(10),
        Action::Exit(1),
        Action::Run(10),
    ];
    run_pair(&actions).unwrap();
}

/// Bedienhandlung in der Kette Adorf – Strecke – Bstadt.
#[derive(Clone, Debug)]
enum ChainAction {
    /// Erlaubnis an Grenze (Bereich, Grenze) anfordern.
    Request(usize, usize),
    /// Ausfahrt einstellen (Index in [`CHAIN_EXITS`]).
    Exit(usize),
    Cancel(usize),
    Restart(usize),
    Delay(u64),
    Run(u8),
}

/// Ausfahrten über eine Grenze: (Bereich, Fahrstraße, Grenze).
const CHAIN_EXITS: [(usize, &str, usize); 4] = [
    (0, "AN1->Strecke", 0),
    (1, "Sbk2->Adorf", 0),
    (1, "Sbk1->Bstadt", 1),
    (2, "BP2->Strecke", 0),
];

/// Verbindungen: (Bereich, Grenze) ↔ (Bereich, Grenze).
const CHAIN_LINKS: [((usize, usize), (usize, usize)); 2] = [((0, 0), (1, 0)), ((1, 1), (2, 0))];

fn chain_action() -> impl Strategy<Value = ChainAction> {
    prop_oneof![
        3 => prop_oneof![Just((0, 0)), Just((1, 0)), Just((1, 1)), Just((2, 0))]
            .prop_map(|(a, b)| ChainAction::Request(a, b)),
        2 => (0usize..4).prop_map(ChainAction::Exit),
        1 => (0usize..4).prop_map(ChainAction::Cancel),
        1 => (0usize..3).prop_map(ChainAction::Restart),
        1 => (1u64..=10).prop_map(ChainAction::Delay),
        4 => (1u8..40).prop_map(ChainAction::Run),
    ]
}

fn border_state(h: &Harness, b: usize) -> BorderState {
    h.state.borders()[b]
}

/// REQ-IXL-017, REQ-IXL-021: In der Kette Adorf – Strecke – Bstadt zeigen nie Ausfahrten
/// beider Bahnhöfe zugleich Fahrt in die eingleisige Strecke, und an keiner Grenze halten
/// beide Seiten die Erlaubnis – auch nach Neustarts und bei Laufzeiten bis zu einer Sekunde.
fn run_chain(actions: &[ChainAction]) -> Result<(), TestCaseError> {
    let mut sides = [area("Adorf"), area("Strecke"), area("Bstadt")];
    assert_eq!(
        sides[1].plan().border_name(BorderId::new(1)),
        Some(">Bstadt")
    );
    // Zustellzyklus und Meldung je (Bereich, Grenze).
    let mut queues: Vec<Vec<(u64, BorderMessage)>> = vec![Vec::new(); 4];
    let slot = |a: usize, b: usize| match (a, b) {
        (0, 0) => 0,
        (1, 0) => 1,
        (1, _) => 2,
        _ => 3,
    };
    let mut delay = 1u64;
    let mut cycle = 0u64;
    for a in actions {
        let mut requests: [Vec<Request>; 3] = [Vec::new(), Vec::new(), Vec::new()];
        let mut cycles = 1;
        match a {
            ChainAction::Request(i, b) => {
                requests[*i].push(Request::RequestPermission(
                    BorderId::from_index(*b).unwrap(),
                ));
            }
            ChainAction::Exit(k) | ChainAction::Cancel(k) => {
                let (i, name, _) = CHAIN_EXITS[*k];
                let r = sides[i].route(name);
                requests[i].push(if matches!(a, ChainAction::Exit(_)) {
                    Request::SetRoute(r)
                } else {
                    Request::CancelRoute(r)
                });
            }
            ChainAction::Restart(i) => {
                let plan = *sides[*i].plan();
                sides[*i].state = OwnedState::new(&plan);
            }
            ChainAction::Delay(d) => delay = *d,
            ChainAction::Run(n) => cycles = u64::from(*n),
        }
        for _ in 0..cycles {
            cycle += 1;
            for (i, side) in sides.iter_mut().enumerate() {
                for b in 0..side.border_in.len() {
                    let queue = &mut queues[slot(i, b)];
                    let due: Vec<_> = queue.iter().filter(|(at, _)| *at <= cycle).collect();
                    side.border_in[b] = due.last().map(|(_, m)| *m);
                    queue.retain(|(at, _)| *at > cycle);
                    side.border_out[b] = None;
                }
                let req = std::mem::take(&mut requests[i]);
                side.step(&req);
                prop_assert_eq!(side.last, Ok(()), "sicherer Zustand");
            }
            for ((a, ab), (b, bb)) in CHAIN_LINKS {
                if let Some(m) = sides[a].border_out[ab] {
                    queues[slot(b, bb)].push((cycle + delay, m));
                }
                if let Some(m) = sides[b].border_out[bb] {
                    queues[slot(a, ab)].push((cycle + delay, m));
                }
                prop_assert!(
                    !(border_state(&sides[a], ab).holding()
                        && border_state(&sides[b], bb).holding()),
                    "beide halten die Erlaubnis"
                );
            }
            let proceed = |k: usize| {
                let (i, name, _) = CHAIN_EXITS[k];
                let start = name.split("->").next().unwrap();
                sides[i].aspect(start) == Aspect::Proceed
            };
            for (k, (i, _, b)) in CHAIN_EXITS.iter().copied().enumerate() {
                if proceed(k) {
                    prop_assert!(border_state(&sides[i], b).holding(), "Fahrt ohne Erlaubnis");
                }
            }
            prop_assert!(
                !(proceed(0) && proceed(3)),
                "beide Bahnhöfe schicken Züge in die Strecke"
            );
            prop_assert!(!(proceed(0) && proceed(1)), "Gegenfahrt Adorf – Strecke");
            prop_assert!(!(proceed(2) && proceed(3)), "Gegenfahrt Strecke – Bstadt");
        }
    }
    Ok(())
}

proptest! {
    #[test]
    fn never_two_stations_feed_the_single_track(
        actions in prop::collection::vec(chain_action(), 1..60)
    ) {
        run_chain(&actions)?;
    }
}

/// REQ-IXL-021: Die Strecke gibt die Erlaubnis Richtung Bstadt erst ab, wenn sie die
/// Erlaubnis Richtung Adorf hält. Vorher meldet sie ihren Abschnitt Adorf nicht als
/// ausschließlich verschlossen, sobald beide Seiten verschlossen sind.
#[test]
fn single_track_direction_changes_in_order() {
    use ChainAction::{Exit, Request, Run};
    // Anlauf: Adorf und die Strecke (Richtung Bstadt) halten. Bstadt fordert an: keine
    // Abgabe, solange Adorf hält; erst nach dem Wechsel Adorf → Strecke. Danach Ausfahrt
    // aus Bstadt.
    let actions = [
        Run(70),
        Request(2, 0),
        Run(20),
        Request(1, 0),
        Run(20),
        Exit(3),
        Run(20),
    ];
    run_chain(&actions).unwrap();
}
