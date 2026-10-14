//! Übernahme des Betriebs durch einen Bereitschaftsrechner (REQ-SVC-012, REQ-IXL-025).
//!
//! Ein Hauptrechner betreibt ein Feld mit Zügen; seine Ausgaben gehen erst nach Quittung
//! hinaus. Ein Bereitschaftsrechner liest die Journale mit, in Stücken und verspätet, und
//! quittiert. An einer zufälligen Stelle fällt der Hauptrechner aus; die Objektsteuerungen
//! stellen ihre Signale auf Halt, die Züge fahren weiter, bis der Bereitschaftsrechner das
//! Feld übernimmt – mit dem mitgelesenen Zustand, wenn die Zusage des Hauptrechners die
//! Übernahme noch deckt. Danach geht der Betrieb mit zufälligen Bedienungen weiter.
//!
//! Geprüft: Kein Zug entgleist, schneidet eine Weiche auf, befährt einen offenen Bahnübergang
//! oder überfährt ein Halt zeigendes Signal; kein Kern geht in den sicheren Zustand; das
//! Journal des Bereitschaftsrechners spielt bis zu seinem Endzustand ab.
//!
//! Die Gegenprobe zeigt, wofür die Sperre gebraucht wird: Ohne sie übernimmt ein
//! Bereitschaftsrechner, dem die letzten Zyklen fehlen, eine Fahrstraße nicht, über die schon
//! ein Zug fährt, und stellt eine Weiche vor ihm um.

use std::cell::RefCell;
use std::io::Write;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use railit_journal::Follower;
use railit_kernel::{Aspect, Command, Input, OwnedState, Request, RequestId};
use railit_rules::GenericRules;
use railit_runtime::{
    Addressed, Coverage, FieldAdapter, ReplicationClock, ReplicationConfig, Runtime,
};
use railit_sim::{SimConfig, SimEvent, Simulator, TrainSpec};
use railit_topology::Topology;
use railit_types::{BorderId, Duration, Length, RouteId, SignalId, Speed, Tick};

const BEISPIELSTADT: &str = include_str!("../../../examples/beispielstadt.toml");
const NORDBAHN: &str = include_str!("../../../examples/nordbahn.toml");

const CYCLE_MS: u64 = 100;
const GRACE_MS: u64 = 3_000;

/// Feld, das sich Haupt- und Bereitschaftsrechner teilen (nacheinander).
#[derive(Clone)]
struct SharedField(Rc<RefCell<Simulator>>);

impl FieldAdapter for SharedField {
    fn advance(&mut self, now: Tick) {
        let mut sim = self.0.borrow_mut();
        if now > sim.now() {
            sim.advance(now);
        }
    }

    fn reports(&self, out: &mut Vec<Input>) {
        out.extend(self.0.borrow().reports());
    }

    fn apply(&mut self, commands: &[Command]) {
        self.0.borrow_mut().apply(commands);
    }

    fn etcs_reports(&self) -> Vec<(String, SignalId, bool)> {
        self.0.borrow().etcs_reports()
    }

    fn set_cab_authority(&mut self, train: &str, authority: Option<Vec<SignalId>>) {
        self.0.borrow_mut().set_cab_authority(train, authority);
    }
}

#[derive(Clone, Default)]
struct Pipe(Arc<Mutex<Vec<u8>>>);

impl Write for Pipe {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Pipe {
    fn bytes(&self) -> Vec<u8> {
        self.0.lock().unwrap().clone()
    }
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

struct Net {
    topology: Topology,
    entries: &'static [&'static str],
}

fn net(source: &str, entries: &'static [&'static str]) -> Net {
    Net {
        topology: railit_import::toml_format::parse(source).unwrap().topology,
        entries,
    }
}

fn runtime(net: &Net, sim: &Rc<RefCell<Simulator>>) -> Runtime<SharedField> {
    let field = SharedField(Rc::clone(sim));
    railit_runtime::simulation_with(
        net.topology.clone(),
        GenericRules::default(),
        SimConfig::default(),
        Duration::from_millis(CYCLE_MS),
        move |_| Ok(field),
    )
    .unwrap()
}

fn start_journals(rt: &mut Runtime<SharedField>) -> Vec<Pipe> {
    rt.set_journal_snapshots(50);
    (0..rt.area_count())
        .map(|area| {
            let pipe = Pipe::default();
            rt.start_journal(area, Box::new(pipe.clone()), 25).unwrap();
            pipe
        })
        .collect()
}

/// Zufällige Bedienungen eines Zyklus: Fahrstraßen einstellen und zurücknehmen, Erlaubnis
/// anfordern, Züge an den Einfahrsignalen einsetzen.
fn operate<F: FieldAdapter>(
    rt: &Runtime<F>,
    sim: &Rc<RefCell<Simulator>>,
    net: &Net,
    rng: &mut Rng,
    next_id: &mut u64,
    trains: &mut u32,
) -> Vec<Addressed> {
    let mut requests = Vec::new();
    let mut push = |area: usize, request: Request| {
        *next_id += 1;
        requests.push(Addressed {
            area,
            id: RequestId(*next_id),
            request,
        });
    };
    let area = usize::try_from(rng.below(rt.area_count() as u64)).unwrap();
    let plan = rt.plan_of(area).unwrap();
    match rng.below(20) {
        0..=3 => {
            let r =
                RouteId::from_index(usize::try_from(rng.below(plan.route_count() as u64)).unwrap())
                    .unwrap();
            push(area, Request::SetRoute(r));
        }
        4 => {
            let r =
                RouteId::from_index(usize::try_from(rng.below(plan.route_count() as u64)).unwrap())
                    .unwrap();
            push(area, Request::CancelRoute(r));
        }
        5 if plan.border_count() > 0 => {
            let b = BorderId::from_index(
                usize::try_from(rng.below(plan.border_count() as u64)).unwrap(),
            )
            .unwrap();
            push(area, Request::RequestPermission(b));
        }
        6 => {
            let entry = net.entries[usize::try_from(rng.below(net.entries.len() as u64)).unwrap()];
            let signal = net.topology.lookup(entry).and_then(|e| match e {
                railit_topology::ElementRef::Signal(s) => Some(s),
                _ => None,
            });
            if let Some(signal) = signal {
                *trains += 1;
                let _ = sim.borrow_mut().add_train(TrainSpec {
                    name: format!("Z{trains}"),
                    signal,
                    length: Length::from_mm(80_000),
                    speed: Speed::from_kmh(40 + u16::try_from(rng.below(60)).unwrap()),
                    ignore_signals: false,
                    etcs: false,
                });
            }
        }
        _ => {}
    }
    requests
}

/// Gefährliche Ereignisse im Feld.
fn hazards(sim: &Rc<RefCell<Simulator>>) -> Vec<SimEvent> {
    sim.borrow_mut()
        .take_events()
        .into_iter()
        .filter(|e| {
            matches!(
                e,
                SimEvent::TrainDerailed { .. }
                    | SimEvent::PointTrailed { .. }
                    | SimEvent::LevelCrossingNotClosed { .. }
                    | SimEvent::SignalPassedAtDanger { .. }
            )
        })
        .collect()
}

/// Rückfallebene der Objektsteuerungen beim Verlust ihres Stellwerks.
fn fallback(sim: &Rc<RefCell<Simulator>>, net: &Net) {
    let commands: Vec<Command> = net
        .topology
        .signals()
        .flat_map(|(signal, _)| {
            [
                Command::SetSignal {
                    signal,
                    aspect: Aspect::Stop,
                },
                Command::SetSubstitute { signal, on: false },
            ]
        })
        .collect();
    sim.borrow_mut().apply(&commands);
}

/// Bereitschaftsrechner: je Bereich ein Mitleser.
struct Standby {
    followers: Vec<Follower<'static>>,
    fed: Vec<usize>,
}

impl Standby {
    fn new(rt: &Runtime<SharedField>) -> Self {
        Self {
            followers: (0..rt.area_count())
                .map(|a| Follower::new(rt.plan_of(a).unwrap(), None))
                .collect(),
            fed: vec![0; rt.area_count()],
        }
    }

    /// Liest `share` Prozent des noch nicht Gelesenen (die letzte Zeile auch halb).
    fn read(&mut self, pipes: &[Pipe], share: u64) {
        for ((follower, fed), pipe) in self.followers.iter_mut().zip(&mut self.fed).zip(pipes) {
            let bytes = pipe.bytes();
            let open = bytes.len() - *fed;
            let take = usize::try_from(u64::try_from(open).unwrap() * share / 100).unwrap();
            let end = *fed + take;
            follower.feed(&bytes[*fed..end]).unwrap();
            *fed = end;
        }
    }

    /// Quittiert je Bereich den gelesenen Stand; liefert die letzte gültige Zusage.
    fn acknowledge(&self, rt: &mut Runtime<SharedField>) -> Option<Coverage> {
        let mut cover = None;
        for (area, follower) in self.followers.iter().enumerate() {
            let (Some(state), Some(summary)) = (follower.state(), follower.summary()) else {
                continue;
            };
            if let Ok(c) = rt.acknowledge(area, state.meta().cycle(), &summary.head) {
                cover = Some(c);
            }
        }
        cover
    }

    fn states(&self) -> Option<Vec<OwnedState>> {
        self.followers.iter().map(|f| f.state().cloned()).collect()
    }

    fn last_ms(&self) -> u64 {
        self.followers
            .iter()
            .filter_map(|f| f.summary().map(|s| s.last_ms))
            .max()
            .unwrap_or(0)
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    /// Übernahme mit dem mitgelesenen Zustand ohne Gefährdung.
    Warm,
    /// Die Zusage deckte die Übernahme nicht (Bereitschaftsrechner zu weit zurück).
    NotCovered,
    /// Gegenprobe: Gefährdung nach der Übernahme.
    Hazard(Vec<SimEvent>),
}

/// Ein Fall. Mit `gated` arbeitet der Hauptrechner mit Ausgabesperre und der
/// Bereitschaftsrechner übernimmt nur mit Zusage. Ohne (Gegenprobe) hört der
/// Bereitschaftsrechner `lag` Zyklen vor dem Ausfall auf zu lesen und übernimmt trotzdem.
#[allow(clippy::too_many_lines)]
fn run_case(net: &Net, seed: u64, gated: bool, lag: u64) -> Outcome {
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let sim = Rc::new(RefCell::new(Simulator::new(
        net.topology.clone(),
        SimConfig::default(),
    )));
    let mut primary = runtime(net, &sim);
    let pipes = start_journals(&mut primary);
    if gated {
        primary
            .enable_replication(ReplicationConfig {
                grace_ms: GRACE_MS,
                rearm_lag: 5,
                clock: ReplicationClock::Cycle,
            })
            .unwrap();
    }
    let mut standby = Standby::new(&primary);
    let (mut next_id, mut trains) = (0u64, 0u32);
    // Zusage: (Zeitpunkt der Antwort, Zusage).
    let mut promise: Option<(u64, Coverage)> = None;

    let life = 30 + rng.below(500);
    for cycle in 0..life {
        let requests = operate(&primary, &sim, net, &mut rng, &mut next_id, &mut trains);
        let out = primary.cycle_addressed(&requests);
        assert_eq!(out.safe_state, None, "Hauptrechner, Seed {seed}");
        let found = hazards(&sim);
        assert!(found.is_empty(), "Hauptrechner, Seed {seed}: {found:?}");
        // Der Bereitschaftsrechner liest meist alles, manchmal nur einen Teil oder nichts.
        let share = match rng.below(10) {
            _ if !gated && cycle + lag >= life => 0,
            0 => 0,
            1..=2 => rng.below(100),
            _ => 100,
        };
        standby.read(&pipes, share);
        if let Some(c) = standby.acknowledge(&mut primary) {
            promise = Some((primary.now().as_millis(), c));
        }
    }

    // Ausfall: Was noch unterwegs war, kommt vielleicht an; die Objektsteuerungen stellen
    // ihre Signale auf Halt; die Züge fahren weiter, bis das Feld übernommen ist.
    let died = primary.now();
    let share = if gated { rng.below(101) } else { 0 };
    standby.read(&pipes, share);
    drop(primary);
    fallback(&sim, net);
    let switch_ms = 200 + rng.below(1_500);
    let mut t = died;
    while t.as_millis() < died.as_millis() + switch_ms {
        t = t.checked_add(Duration::from_millis(CYCLE_MS)).unwrap();
        sim.borrow_mut().advance(t);
    }
    let found = hazards(&sim);
    assert!(found.is_empty(), "Umschaltung, Seed {seed}: {found:?}");

    let covered = promise.is_some_and(|(at, c)| c.covered && t.as_millis() < at + c.hold_ms);
    let Some(states) = standby.states().filter(|_| covered || !gated) else {
        return Outcome::NotCovered;
    };
    let mut rt = runtime(net, &sim);
    let pipes = start_journals(&mut rt);
    let now = Tick::from_millis(t.as_millis().max(standby.last_ms()));
    let out = rt.take_over(&states, now, "Übernahme").unwrap();
    if gated {
        assert_eq!(out.safe_state, None, "Übernahme, Seed {seed}");
    }

    for _ in 0..400 {
        let requests = operate(&rt, &sim, net, &mut rng, &mut next_id, &mut trains);
        let out = rt.cycle_addressed(&requests);
        let found = hazards(&sim);
        if !gated {
            if !found.is_empty() {
                return Outcome::Hazard(found);
            }
            continue;
        }
        assert_eq!(out.safe_state, None, "nach der Übernahme, Seed {seed}");
        assert!(
            found.is_empty(),
            "nach der Übernahme, Seed {seed}: {found:?}"
        );
    }
    if !gated {
        return Outcome::Warm;
    }

    // Das Journal des übernehmenden Rechners beginnt mit der Übernahme und spielt ab.
    for (area, pipe) in pipes.iter().enumerate() {
        let plan = rt.plan_of(area).unwrap();
        let (_, replayed) = railit_journal::replay(pipe.bytes().as_slice(), plan).unwrap();
        assert_eq!(&replayed, rt.state_of(area).unwrap(), "Seed {seed}");
    }
    Outcome::Warm
}

fn run_many(net: &Net, seeds: std::ops::Range<u64>) {
    let mut warm = 0;
    let total = seeds.end - seeds.start;
    for seed in seeds {
        if run_case(net, seed, true, 0) == Outcome::Warm {
            warm += 1;
        }
    }
    // Die Sperre hält den Bereitschaftsrechner fast immer gedeckt.
    assert!(warm * 10 >= total * 7, "nur {warm} von {total} gedeckt");
}

#[test]
fn takeover_in_beispielstadt_is_safe() {
    let cases = std::env::var("TAKEOVER_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(40);
    run_many(&net(BEISPIELSTADT, &["A", "F"]), 0..cases);
}

#[test]
fn takeover_in_nordbahn_is_safe() {
    let cases = std::env::var("TAKEOVER_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);
    run_many(&net(NORDBAHN, &["AA", "BF"]), 1_000..1_000 + cases);
}

/// Ein Zug fährt aus Gleis 1 über den Bahnübergang BÜ1 aus (N1-GE). Der Hauptrechner fällt
/// aus, als der Zug vor dem Bahnübergang ist. `reads_to_end`: Der Bereitschaftsrechner liest
/// und quittiert bis zum Ausfall; sonst hört er auf, als die Ausfahrt angefordert wird.
fn crossing(gated: bool, reads_to_end: bool) -> Outcome {
    let net = net(BEISPIELSTADT, &["A", "F"]);
    let sim = Rc::new(RefCell::new(Simulator::new(
        net.topology.clone(),
        SimConfig::default(),
    )));
    let mut primary = runtime(&net, &sim);
    let pipes = start_journals(&mut primary);
    if gated {
        primary
            .enable_replication(ReplicationConfig {
                grace_ms: GRACE_MS,
                rearm_lag: 5,
                clock: ReplicationClock::Cycle,
            })
            .unwrap();
    }
    let plan = *primary.plan();
    let te = plan.find_tvd("TE").unwrap();
    sim.borrow_mut()
        .add_train(TrainSpec {
            name: "RB3".into(),
            signal: plan.find_signal("A").unwrap(),
            length: Length::from_mm(80_000),
            speed: Speed::from_kmh(60),
            ignore_signals: false,
            etcs: false,
        })
        .unwrap();
    let mut standby = Standby::new(&primary);
    let mut promise: Option<(u64, Coverage)> = None;
    let mut reading = true;
    let route = |name: &str| Request::SetRoute(plan.find_route(name).unwrap());
    let mut id = 0;
    for cycle in 0..2_000u32 {
        let requests: Vec<(RequestId, Request)> = match cycle {
            1 => vec![(RequestId(1), route("A-N1"))],
            600 => {
                reading = reads_to_end;
                vec![(RequestId(2), route("N1-GE"))]
            }
            _ => Vec::new(),
        };
        id += requests.len();
        primary.cycle(&requests);
        assert!(hazards(&sim).is_empty());
        if reading {
            standby.read(&pipes, 100);
            if let Some(c) = standby.acknowledge(&mut primary) {
                promise = Some((primary.now().as_millis(), c));
            }
        }
        if cycle > 600 && sim.borrow().occupancy(te) == Some(railit_kernel::Occupancy::Occupied) {
            break;
        }
    }
    assert_eq!(id, 2);
    assert_eq!(
        sim.borrow().occupancy(te),
        Some(railit_kernel::Occupancy::Occupied),
        "Zug vor dem Bahnübergang"
    );

    let died = primary.now();
    drop(primary);
    fallback(&sim, &net);
    let t = died.checked_add(Duration::from_millis(500)).unwrap();
    sim.borrow_mut().advance(t);
    let covered = promise.is_some_and(|(at, c)| c.covered && t.as_millis() < at + c.hold_ms);
    if gated && !covered {
        return Outcome::NotCovered;
    }
    let mut rt = runtime(&net, &sim);
    start_journals(&mut rt);
    let now = Tick::from_millis(t.as_millis().max(standby.last_ms()));
    rt.take_over(&standby.states().unwrap(), now, "Übernahme")
        .unwrap();
    for _ in 0..600 {
        rt.cycle(&[]);
        let found = hazards(&sim);
        if !found.is_empty() {
            return Outcome::Hazard(found);
        }
    }
    Outcome::Warm
}

/// Gegenprobe: Ohne Ausgabesperre fehlt dem Bereitschaftsrechner die Ausfahrt; er übernimmt
/// den Bahnübergang als offen, öffnet ihn vor dem Zug.
#[test]
fn stale_takeover_without_gate_opens_the_crossing_before_a_train() {
    let Outcome::Hazard(found) = crossing(false, false) else {
        panic!("keine Gefährdung ohne Sperre");
    };
    assert!(
        found
            .iter()
            .any(|e| matches!(e, SimEvent::LevelCrossingNotClosed { .. })),
        "{found:?}"
    );
}

/// Mit Ausgabesperre: Liest der Bereitschaftsrechner nicht mehr mit, deckt ihn die Zusage
/// nicht – er darf nicht mit dem mitgelesenen Zustand übernehmen. Liest er bis zum Ausfall,
/// übernimmt er die Ausfahrt und der Bahnübergang bleibt geschlossen.
#[test]
fn gate_covers_only_a_current_standby() {
    assert_eq!(crossing(true, false), Outcome::NotCovered);
    assert_eq!(crossing(true, true), Outcome::Warm);
}
