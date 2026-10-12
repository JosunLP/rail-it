//! Redundante Kanäle und Voter am Beispiel der Beispielstadt (REQ-SAF-001 … 004).
//!
//! Ein Referenzkern ohne Redundanz läuft neben der Sicherheitsschicht mit denselben
//! Eingaben. Solange die Schicht nicht im sicheren Zustand ist, müssen ihre Ausgaben genau
//! die des Referenzkerns sein – auch wenn ein Kanal gestört ist.

use proptest::prelude::*;
use railit_kernel::*;
use railit_plan::PlanView;
use railit_rules::GenericRules;
use railit_safety::{
    ChannelOutputs, Channels, ConfigError, Divergence, Fault, FixedOutputs, Redundancy,
    ReintegrateError, SafetyEvent, Trip,
};
use railit_types::{Duration, PointId, PointPosition, RouteId, SignalId, Tick, TvdId};

const EXAMPLE: &str = include_str!("../../../examples/beispielstadt.toml");

fn plan() -> PlanView<'static> {
    let net = railit_import::toml_format::parse(EXAMPLE).unwrap().topology;
    let image = railit_planning::compile(&net, None).unwrap().image;
    PlanView::parse(Box::leak(image.into_boxed_slice())).unwrap()
}

type Layer = Channels<'static, GenericRules, OwnedState, VecOutputs>;

fn layer(plan: PlanView<'static>, n: usize) -> Layer {
    let channel = || Some((OwnedState::new(&plan), VecOutputs::default()));
    let channels = match n {
        1 => [channel(), None, None],
        2 => [channel(), channel(), None],
        _ => [channel(), channel(), channel()],
    };
    Channels::new(Interlocking::new(plan, GenericRules::default()), channels).unwrap()
}

/// Einfaches Feld: Weichen und Signale folgen den Befehlen sofort, Abschnitte wie gesetzt.
struct Field {
    points: Vec<PointPosition>,
    signals: Vec<Aspect>,
    tvds: Vec<Occupancy>,
}

impl Field {
    fn new(plan: &PlanView<'_>) -> Self {
        Self {
            points: vec![PointPosition::Right; plan.point_count()],
            signals: vec![Aspect::Stop; plan.signal_count()],
            tvds: vec![Occupancy::Clear; plan.tvd_count()],
        }
    }

    fn reports(&self) -> Vec<Input> {
        let mut inputs = Vec::new();
        for (i, p) in self.points.iter().enumerate() {
            inputs.push(Input::Point {
                point: PointId::from_index(i).unwrap(),
                report: (*p).into(),
            });
        }
        for (i, s) in self.signals.iter().enumerate() {
            inputs.push(Input::Signal {
                signal: SignalId::from_index(i).unwrap(),
                report: match s {
                    Aspect::Stop => SignalReport::Stop,
                    Aspect::Proceed => SignalReport::Proceed,
                },
            });
        }
        for (i, o) in self.tvds.iter().enumerate() {
            inputs.push(Input::Tvd {
                tvd: TvdId::from_index(i).unwrap(),
                occupancy: *o,
            });
        }
        inputs
    }

    fn apply(&mut self, commands: &[Command]) {
        for c in commands {
            match *c {
                Command::MovePoint { point, position } => {
                    self.points[point.index().unwrap()] = position;
                }
                Command::SetSignal { signal, aspect } => {
                    self.signals[signal.index().unwrap()] = aspect;
                }
                _ => {}
            }
        }
    }
}

/// Referenzkern, Sicherheitsschicht und Feld.
struct Bench {
    reference: Interlocking<'static, GenericRules>,
    ref_state: OwnedState,
    layer: Layer,
    field: Field,
    now: Tick,
    next: u64,
}

#[derive(Debug)]
struct Step {
    reference: VecOutputs,
    voted: VecOutputs,
    trip: Option<Trip>,
    events: Vec<SafetyEvent>,
}

impl Bench {
    fn new(n: usize) -> Self {
        let plan = plan();
        Self {
            reference: Interlocking::new(plan, GenericRules::default()),
            ref_state: OwnedState::new(&plan),
            layer: layer(plan, n),
            field: Field::new(&plan),
            now: Tick::ZERO,
            next: 0,
        }
    }

    fn step(&mut self, requests: &[Request]) -> Step {
        self.now = self.now.checked_add(Duration::from_millis(100)).unwrap();
        let mut inputs = self.field.reports();
        for r in requests {
            self.next += 1;
            inputs.push(Input::Request {
                id: RequestId(self.next),
                request: *r,
            });
        }
        let mut reference = VecOutputs::default();
        let _ = self.reference.step(
            &mut self.ref_state.view(),
            self.now,
            &inputs,
            &mut reference,
        );
        let mut voted = VecOutputs::default();
        let verdict = self.layer.step(self.now, &inputs, &mut voted);
        self.field.apply(&voted.commands);
        Step {
            reference,
            voted,
            trip: verdict.trip,
            events: verdict.events().collect(),
        }
    }

    fn route(&self, name: &str) -> RouteId {
        let plan = self.reference.plan();
        (0..plan.route_count())
            .filter_map(RouteId::from_index)
            .find(|r| plan.route(*r).unwrap().name() == name)
            .unwrap()
    }
}

fn proceeds(out: &VecOutputs) -> usize {
    out.commands
        .iter()
        .filter(|c| {
            matches!(
                c,
                Command::SetSignal {
                    aspect: Aspect::Proceed,
                    ..
                }
            )
        })
        .count()
}

/// REQ-SAF-001: Ohne Störung gibt die Schicht genau die Ausgaben des Kerns weiter.
#[test]
fn healthy_channels_are_transparent() {
    for n in 1..=3 {
        let mut b = Bench::new(n);
        let a_n1 = b.route("A-N1");
        for cycle in 0..60 {
            let requests = if cycle == 3 {
                vec![Request::SetRoute(a_n1)]
            } else {
                vec![]
            };
            let s = b.step(&requests);
            assert_eq!(s.voted, s.reference, "{n} Kanäle, Zyklus {cycle}");
            assert_eq!(s.trip, None);
        }
        assert_eq!(
            b.layer.status().redundancy,
            Redundancy::from_channels(n).unwrap()
        );
        assert!(
            b.layer
                .primary()
                .unwrap()
                .signals()
                .iter()
                .any(|s| s.commanded() == Aspect::Proceed),
            "Fahrstraße hat Fahrt"
        );
    }
}

/// REQ-SAF-001/004: 2-von-2 – ein falscher Fahrtbefehl eines Kanals erreicht das Feld nie;
/// die Schicht geht in den sicheren Zustand.
#[test]
fn two_out_of_two_trips_on_stray_command() {
    let mut b = Bench::new(2);
    let a_n1 = b.route("A-N1");
    for _ in 0..3 {
        b.step(&[]);
    }
    assert!(b.layer.inject(1, Fault::StrayCommand));
    let s = b.step(&[Request::SetRoute(a_n1)]);
    assert_eq!(s.trip, Some(Trip::Disagreement(Divergence::Outputs)));
    assert_eq!(
        s.events,
        [SafetyEvent::Tripped(Trip::Disagreement(
            Divergence::Outputs
        ))]
    );
    assert_eq!(proceeds(&s.voted), 0);
    let signals = b.reference.plan().signal_count();
    let stops = s
        .voted
        .commands
        .iter()
        .filter(|c| {
            matches!(
                c,
                Command::SetSignal {
                    aspect: Aspect::Stop,
                    ..
                }
            )
        })
        .count();
    assert_eq!(stops, signals, "alle Signale Halt");
    assert!(
        s.voted
            .events
            .iter()
            .all(|e| matches!(e, Event::Rejected(_, Rejection::SafeState))),
        "{:?}",
        s.voted.events
    );
    assert_eq!(s.voted.events.len(), 1, "Anforderung abgelehnt");

    // Der sichere Zustand bleibt bestehen; Anforderungen werden abgelehnt.
    for _ in 0..30 {
        let s = b.step(&[Request::SetRoute(a_n1)]);
        assert_eq!(proceeds(&s.voted), 0);
        assert!(s.trip.is_some());
    }

    // Rücksetzen: Neubeginn wie nach einem Neustart.
    b.layer.reset();
    b.ref_state = OwnedState::new(b.reference.plan());
    for cycle in 0..40 {
        let requests = if cycle == 3 {
            vec![Request::SetRoute(a_n1)]
        } else {
            vec![]
        };
        let s = b.step(&requests);
        assert_eq!(s.voted, s.reference, "nach Rücksetzen, Zyklus {cycle}");
    }
}

/// REQ-SAF-001: 2-von-2 erkennt auch eine Abweichung nur im Zustand.
#[test]
fn two_out_of_two_detects_state_divergence() {
    let mut b = Bench::new(2);
    for _ in 0..3 {
        b.step(&[]);
    }
    b.field.tvds[0] = Occupancy::Occupied;
    b.layer.inject(0, Fault::DropInputs);
    let s = b.step(&[]);
    assert!(
        matches!(s.trip, Some(Trip::Disagreement(_))),
        "{:?}",
        s.trip
    );
}

/// REQ-SAF-002/003: 2-von-3 – der gestörte Kanal wird ausgeschlossen, der Betrieb läuft
/// unverändert weiter; nach Wiederaufnahme wählt er wieder mit.
#[test]
fn two_out_of_three_masks_one_channel() {
    for (fault, divergence) in [
        (Fault::StrayCommand, Divergence::Outputs),
        (Fault::Stall, Divergence::Missing),
        (Fault::DropInputs, Divergence::State),
    ] {
        let mut b = Bench::new(3);
        let a_n1 = b.route("A-N1");
        for _ in 0..3 {
            b.step(&[]);
        }
        if fault == Fault::DropInputs {
            b.field.tvds[0] = Occupancy::Occupied;
        }
        b.layer.inject(2, fault);
        let s = b.step(&[]);
        assert_eq!(s.trip, None, "{fault:?}");
        assert_eq!(s.voted, s.reference, "{fault:?}");
        assert_eq!(
            s.events,
            [SafetyEvent::ChannelExcluded {
                channel: 2,
                divergence
            }],
            "{fault:?}"
        );
        assert_eq!(b.layer.status().active, [true, true, false]);
        b.field.tvds[0] = Occupancy::Clear;

        let s = b.step(&[Request::SetRoute(a_n1)]);
        assert_eq!(s.voted, s.reference);
        assert_eq!(b.layer.reintegrate(0), Err(ReintegrateError::AlreadyActive));
        assert_eq!(b.layer.reintegrate(5), Err(ReintegrateError::NoSuchChannel));
        assert_eq!(b.layer.reintegrate(2), Ok(()));
        assert_eq!(b.layer.status().active, [true; 3]);
        for _ in 0..30 {
            let s = b.step(&[]);
            assert_eq!(s.voted, s.reference);
            assert!(s.events.is_empty(), "{:?}", s.events);
        }

        // Ein zweiter Fehler nach dem Ausschluss eines Kanals: 2-von-2 ⇒ sicherer Zustand.
        b.layer.inject(1, Fault::Stall);
        let s = b.step(&[]);
        assert!(s.trip.is_none());
        b.layer.inject(0, Fault::StrayCommand);
        let s = b.step(&[]);
        assert_eq!(s.trip, Some(Trip::Disagreement(Divergence::Outputs)));
        assert_eq!(proceeds(&s.voted), 0);
        assert_eq!(b.layer.reintegrate(1), Err(ReintegrateError::Tripped));
    }
}

/// REQ-SAF-005: Ein Auslösen von außen (Watchdog) wirkt wie eine Abweichung.
#[test]
fn external_trip_holds_all_signals() {
    let mut b = Bench::new(2);
    let a_n1 = b.route("A-N1");
    b.step(&[Request::SetRoute(a_n1)]);
    for _ in 0..20 {
        b.step(&[]);
    }
    assert!(b.field.signals.contains(&Aspect::Proceed));
    let trip = Trip::DeadlineExceeded {
        elapsed_us: 5_000,
        deadline_us: 1_000,
    };
    b.layer.trip(trip);
    let s = b.step(&[]);
    assert_eq!(s.trip, Some(trip));
    assert!(!b.field.signals.contains(&Aspect::Proceed));
}

/// Zu kleine Ausgabepuffer führen in den sicheren Zustand statt zu verlorenen Befehlen.
#[test]
fn output_overflow_trips() {
    let plan = plan();
    let channel = || Some((OwnedState::new(&plan), FixedOutputs::<2, 2>::new()));
    let mut layer = Channels::new(
        Interlocking::new(plan, GenericRules::default()),
        [channel(), channel(), None],
    )
    .unwrap();
    let mut field = Field::new(&plan);
    let mut out = VecOutputs::default();
    let mut tripped = None;
    for i in 1..10 {
        out.clear();
        let v = layer.step(
            Tick::ZERO
                .checked_add(Duration::from_millis(100 * i))
                .unwrap(),
            &field.reports(),
            &mut out,
        );
        field.apply(&out.commands);
        tripped = tripped.or(v.trip);
    }
    assert_eq!(tripped, Some(Trip::OutputOverflow));
    let mut small = FixedOutputs::<1, 0>::new();
    small.command(Command::ResetTvd(TvdId::new(0)));
    small.event(Event::AuxExpired(AuxId(0)));
    assert!(small.overflowed());
    ChannelOutputs::clear(&mut small);
    assert!(!small.overflowed());
}

#[test]
fn configuration_is_checked() {
    let plan = plan();
    let k = || Interlocking::new(plan, GenericRules::default());
    let ch = || Some((OwnedState::new(&plan), VecOutputs::default()));
    assert_eq!(
        Channels::<_, OwnedState, VecOutputs>::new(k(), [None, None, None]).err(),
        Some(ConfigError::NoChannel)
    );
    assert_eq!(
        Channels::new(k(), [ch(), None, ch()]).err(),
        Some(ConfigError::Gap)
    );
    let mut other = OwnedState::new(&plan);
    let _ = k().step(
        &mut other.view(),
        Tick::ZERO,
        &[],
        &mut VecOutputs::default(),
    );
    assert_eq!(
        Channels::new(k(), [ch(), Some((other, VecOutputs::default())), None]).err(),
        Some(ConfigError::StatesDiffer)
    );
}

#[derive(Clone, Debug)]
enum Action {
    Tvd(usize, Occupancy),
    Set(u32),
    Cancel(u32),
    Fault(Fault),
    Wait,
}

fn action() -> impl Strategy<Value = Action> {
    let occupancy = prop_oneof![Just(Occupancy::Clear), Just(Occupancy::Occupied)];
    let fault = prop_oneof![
        Just(Fault::DropInputs),
        Just(Fault::StrayCommand),
        Just(Fault::Stall)
    ];
    prop_oneof![
        2 => (0usize..6, occupancy).prop_map(|(i, o)| Action::Tvd(i, o)),
        3 => (0u32..8).prop_map(Action::Set),
        1 => (0u32..8).prop_map(Action::Cancel),
        1 => fault.prop_map(Action::Fault),
        3 => Just(Action::Wait),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// REQ-SAF-001/002: Unter beliebigen Bedienungen und Fehlern in einem Kanal gibt die
    /// Schicht entweder genau die Ausgaben des fehlerfreien Kerns weiter oder ist im sicheren
    /// Zustand. Bei 2-von-3 mit Fehlern in nur einem Kanal bleibt sie immer in Betrieb.
    #[test]
    fn voted_outputs_are_reference_outputs_or_safe(
        n in 2usize..=3,
        faulty in 0usize..3,
        actions in proptest::collection::vec(action(), 1..80),
    ) {
        let mut b = Bench::new(n);
        let faulty = faulty % n;
        for a in actions {
            let mut requests = Vec::new();
            match a {
                Action::Tvd(i, o) => { let len = b.field.tvds.len(); b.field.tvds[i % len] = o; }
                Action::Set(r) => requests.push(Request::SetRoute(RouteId::new(r))),
                Action::Cancel(r) => requests.push(Request::CancelRoute(RouteId::new(r))),
                Action::Fault(f) => { b.layer.inject(faulty, f); }
                Action::Wait => {}
            }
            let s = b.step(&requests);
            if s.trip.is_some() {
                prop_assert!(n == 2, "2-von-3 mit einem gestörten Kanal darf nicht abschalten");
                prop_assert_eq!(proceeds(&s.voted), 0);
                break;
            }
            prop_assert_eq!(&s.voted, &s.reference);
        }
    }
}
