//! Knoten des sicheren Rechners gegen einen Referenzkern, gekoppelt über RaSTA im Speicher
//! (REQ-EMB-001 … 004).

use railit_embedded::wire::HostMsg;
use railit_embedded::{
    ChannelMemory, Node, NodeClient, NodeConfig, NodeCycle, NodeEvent, NodeSync,
};
use railit_kernel::*;
use railit_plan::PlanView;
use railit_rasta::{Config, Connection, MAX_PDU, Role};
use railit_rules::GenericRules;
use railit_safety::{Fault, Trip};
use railit_types::{PointId, PointPosition, RouteId, SignalId, Tick, TvdId};

const EXAMPLE: &str = include_str!("../../../examples/beispielstadt.toml");
const HOST: u32 = 0x0B00_0001;
const NODE: u32 = 0x0B00_0002;

fn plan() -> PlanView<'static> {
    let net = railit_import::toml_format::parse(EXAMPLE).unwrap().topology;
    let image = railit_planning::compile(&net, None).unwrap().image;
    PlanView::parse(Box::leak(image.into_boxed_slice())).unwrap()
}

fn node_config() -> NodeConfig {
    NodeConfig {
        rasta: Config::new(NODE, HOST, Role::Server),
        deadline_us: 50_000,
        gap_limit_us: 1_000_000,
    }
}

fn short_digest(state: &mut OwnedState) -> [u8; 8] {
    railit_safety::state_digest(&state.view()).0[..8]
        .try_into()
        .unwrap()
}

type TestNode<'s> = Node<'static, 's, GenericRules, 256, 256, 256>;

/// Feld wie im Voter-Test: Weichen und Signale folgen sofort.
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

/// Host mit Feld und Referenzkern; der Knoten läuft im selben Prozess.
struct Bench {
    plan: PlanView<'static>,
    client: NodeClient,
    reference: Interlocking<'static, GenericRules>,
    ref_state: OwnedState,
    field: Field,
    now_ms: u64,
    next: u64,
    /// Übertragung unterbrochen.
    cut: bool,
    compare: bool,
    /// Abgeschlossene Zustandsabgleiche.
    syncs: Vec<NodeSync>,
    /// Je Zyklus des Knotens: Zustand des Referenzkerns danach.
    history: Vec<(u64, OwnedState)>,
}

impl Bench {
    fn new(plan: PlanView<'static>) -> Self {
        let mut client = NodeClient::new(Config::new(HOST, NODE, Role::Client));
        client.open(0);
        Self {
            plan,
            client,
            reference: Interlocking::new(plan, GenericRules::default()),
            ref_state: OwnedState::new(&plan),
            field: Field::new(&plan),
            now_ms: 0,
            next: 0,
            cut: false,
            compare: true,
            syncs: Vec::new(),
            history: Vec::new(),
        }
    }

    fn route(&self, name: &str) -> RouteId {
        (0..self.plan.route_count())
            .filter_map(RouteId::from_index)
            .find(|r| self.plan.route(*r).unwrap().name() == name)
            .unwrap()
    }

    /// Ein Zyklus: Host sendet, Knoten rechnet, Host empfängt und wendet an.
    fn cycle(
        &mut self,
        node: &mut TestNode<'_>,
        extra: &[HostMsg],
        requests: &[Request],
    ) -> Option<NodeCycle> {
        self.now_ms += 100;
        let t = u32::try_from(self.now_ms).unwrap();
        let _ = self.client.tick(t);
        if !self.client.is_up() && !self.cut {
            self.client.open(t);
        }
        let mut inputs = self.field.reports();
        for r in requests {
            self.next += 1;
            inputs.push(Input::Request {
                id: RequestId(self.next),
                request: *r,
            });
        }
        let mut msgs: Vec<HostMsg> = inputs.iter().copied().map(HostMsg::Input).collect();
        msgs.extend_from_slice(extra);
        let delivered = self.client.is_up() && self.client.send(t, &msgs).is_ok();

        let mut buf = [0u8; MAX_PDU];
        while let Some(n) = self.client.poll_transmit(&mut buf) {
            if !self.cut {
                node.receive(t, &buf[..n]);
            }
        }
        let start = self.now_ms * 1000;
        node.tick(start, &mut || start + 200);
        while let Some(n) = node.poll_transmit(&mut buf) {
            if !self.cut {
                self.client.receive(t, &buf[..n]);
            }
        }

        // Referenzkern mit genau den Eingaben, die der Knoten erhalten hat.
        let used: &[Input] = if delivered && !self.cut { &inputs } else { &[] };
        let mut reference = VecOutputs::default();
        let _ = self.reference.step(
            &mut self.ref_state.view(),
            Tick::from_millis(self.now_ms),
            used,
            &mut reference,
        );
        let mut got = None;
        while let Some(event) = self.client.poll() {
            match event {
                NodeEvent::Cycle(c) => got = Some(c),
                NodeEvent::Synced(s) => self.syncs.push(s),
            }
        }
        if let Some(c) = &got {
            self.field.apply(&c.commands);
            if self.compare {
                assert_eq!(c.now_ms, self.now_ms);
                assert_eq!(c.commands, reference.commands, "Zyklus {}", c.cycle);
                assert_eq!(c.events, reference.events, "Zyklus {}", c.cycle);
                assert_eq!(usize::from(c.end.inputs), used.len(), "Zyklus {}", c.cycle);
                assert_eq!(
                    c.end.digest,
                    short_digest(&mut self.ref_state),
                    "Zyklus {}",
                    c.cycle
                );
            }
            self.history.push((c.cycle, self.ref_state.clone()));
        }
        got
    }
}

/// REQ-EMB-001: Der Knoten liefert über RaSTA genau die Ausgaben des Referenzkerns.
#[test]
fn node_matches_reference_kernel() {
    let plan = plan();
    let (mut a, mut b, mut c) = (
        ChannelMemory::<64>::new(),
        ChannelMemory::<64>::new(),
        ChannelMemory::<64>::new(),
    );
    let states = [a.state(&plan), b.state(&plan), c.state(&plan)];
    let mut node: TestNode<'_> = Node::new(
        Interlocking::new(plan, GenericRules::default()),
        states,
        node_config(),
    )
    .unwrap();
    let mut bench = Bench::new(plan);
    let a_n1 = bench.route("A-N1");
    let mut received = 0;
    for k in 0..200 {
        let requests = if k == 10 {
            vec![Request::SetRoute(a_n1)]
        } else {
            vec![]
        };
        if k == 60 {
            bench.field.tvds[0] = Occupancy::Occupied;
        }
        if bench.cycle(&mut node, &[], &requests).is_some() {
            received += 1;
        }
    }
    assert!(node.link_up());
    assert!(received > 190, "{received} Zyklen empfangen");
    assert_eq!(bench.client.protocol_errors(), 0);
    assert_eq!(node.stats().protocol_errors, 0);
    assert_eq!(node.stats().cycles, 200);
    assert!(
        bench.field.points.contains(&PointPosition::Left),
        "W1 umgestellt"
    );
}

/// REQ-EMB-002: Ohne Verbindung rechnet der Knoten weiter; die Meldungen fehlen, der Kern
/// nimmt die Fahrt zurück. Danach baut der Host die Verbindung neu auf.
#[test]
fn link_loss_is_fail_safe() {
    let plan = plan();
    let mut a = ChannelMemory::<64>::new();
    let mut b = ChannelMemory::<64>::new();
    let states = [a.state(&plan), b.state(&plan), None];
    let mut node: TestNode<'_> = Node::new(
        Interlocking::new(plan, GenericRules::default()),
        states,
        node_config(),
    )
    .unwrap();
    let mut bench = Bench::new(plan);
    let a_n1 = bench.route("A-N1");
    bench.cycle(&mut node, &[], &[]);
    bench.cycle(&mut node, &[], &[]);
    bench.cycle(&mut node, &[], &[Request::SetRoute(a_n1)]);
    for _ in 0..40 {
        bench.cycle(&mut node, &[], &[]);
    }
    let proceed = |node: &TestNode<'_>| {
        node.state()
            .unwrap()
            .signals()
            .iter()
            .any(|s| s.commanded() == Aspect::Proceed)
    };
    assert!(proceed(&node), "Fahrt vor der Unterbrechung");

    bench.cut = true;
    for _ in 0..50 {
        bench.cycle(&mut node, &[], &[]);
    }
    assert!(!node.link_up(), "Verbindung abgebaut");
    assert!(!proceed(&node), "ohne Meldungen keine Fahrt");
    assert!(node.stats().link_losses >= 1);

    bench.cut = false;
    let mut up = false;
    for _ in 0..50 {
        bench.cycle(&mut node, &[], &[]);
        up |= node.link_up();
    }
    assert!(up && bench.client.is_up(), "Verbindung wieder aufgebaut");
}

/// REQ-EMB-003: Handlungen an der Sicherheitsschicht über die Verbindung.
#[test]
fn safety_layer_over_the_wire() {
    let plan = plan();
    let (mut a, mut b, mut c) = (
        ChannelMemory::<64>::new(),
        ChannelMemory::<64>::new(),
        ChannelMemory::<64>::new(),
    );
    let states = [a.state(&plan), b.state(&plan), c.state(&plan)];
    let mut node: TestNode<'_> = Node::new(
        Interlocking::new(plan, GenericRules::default()),
        states,
        node_config(),
    )
    .unwrap();
    let mut bench = Bench::new(plan);
    for _ in 0..5 {
        bench.cycle(&mut node, &[], &[]);
    }
    let last = |bench: &mut Bench, node: &mut TestNode<'_>, extra: &[HostMsg]| {
        bench.cycle(node, extra, &[]).expect("Zyklus empfangen")
    };
    let c = last(
        &mut bench,
        &mut node,
        &[HostMsg::InjectFault(2, Fault::Stall)],
    );
    assert_eq!(c.end.active, [true, true, false]);
    assert_eq!(c.safety.len(), 1);
    let c = last(&mut bench, &mut node, &[HostMsg::Reintegrate(2)]);
    assert_eq!(c.end.active, [true, true, true]);

    bench.compare = false;
    last(
        &mut bench,
        &mut node,
        &[HostMsg::InjectFault(0, Fault::Stall)],
    );
    let c = last(
        &mut bench,
        &mut node,
        &[HostMsg::InjectFault(1, Fault::StrayCommand)],
    );
    assert!(
        matches!(c.end.trip, Some(Trip::Disagreement(_))),
        "{:?}",
        c.end
    );
    assert!(
        c.commands.iter().all(|c| !matches!(
            c,
            Command::SetSignal {
                aspect: Aspect::Proceed,
                ..
            }
        )),
        "kein Fahrtbefehl im sicheren Zustand"
    );
    let c = last(&mut bench, &mut node, &[HostMsg::Reset]);
    assert_eq!(c.end.trip, None);
    assert_eq!(c.end.active, [true; 3]);
}

/// REQ-EMB-004: Watchdog des Knotens.
#[test]
fn watchdog_trips_on_overrun() {
    let plan = plan();
    let mut a = ChannelMemory::<64>::new();
    let states = [a.state(&plan), None, None];
    let mut node: TestNode<'_> = Node::new(
        Interlocking::new(plan, GenericRules::default()),
        states,
        node_config(),
    )
    .unwrap();
    let r = node.tick(0, &mut || 10);
    assert_eq!(r.verdict.trip, None);
    let r = node.tick(100_000, &mut || 100_000 + 60_000);
    assert_eq!(
        r.verdict.trip,
        Some(Trip::DeadlineExceeded {
            elapsed_us: 60_000,
            deadline_us: 50_000
        })
    );
    assert!(node.status().trip.is_some());
}

/// Ein unlesbares Telegramm hinter gültigem Sicherheitscode baut die Verbindung ab.
#[test]
fn malformed_payload_drops_the_link() {
    let plan = plan();
    let mut a = ChannelMemory::<64>::new();
    let states = [a.state(&plan), None, None];
    let mut node: TestNode<'_> = Node::new(
        Interlocking::new(plan, GenericRules::default()),
        states,
        node_config(),
    )
    .unwrap();
    let mut host = Connection::new(Config::new(HOST, NODE, Role::Client));
    let mut buf = [0u8; MAX_PDU];
    let mut exchange = |host: &mut Connection, node: &mut TestNode<'_>, t: u32| {
        while let Some(n) = host.poll_transmit(&mut buf) {
            node.receive(t, &buf[..n]);
        }
        while let Some(n) = node.poll_transmit(&mut buf) {
            host.receive(t, &buf[..n], &mut |_| {});
        }
    };
    host.open(0);
    for t in 0..5 {
        exchange(&mut host, &mut node, t * 10);
    }
    assert!(node.link_up() && host.is_up());
    host.send(60, &[0xff, 0xff, 0xff, 0xff]).unwrap();
    exchange(&mut host, &mut node, 60);
    assert!(!node.link_up());
    assert_eq!(node.stats().protocol_errors, 1);
}

#[test]
fn memory_capacity_is_checked() {
    let plan = plan();
    assert!(ChannelMemory::<64>::fits(&plan));
    assert!(!ChannelMemory::<2>::fits(&plan));
    assert!(ChannelMemory::<2>::new().state(&plan).is_none());
}

/// REQ-EMB-006: Zustandsabgleich. Der Knoten überträgt seinen Zustand in Teilen über mehrere Zyklen, während
/// er weiterrechnet. Der daraus gebaute Spiegel gleicht dem Zustand des Knotens am Ende des
/// angegebenen Zyklus; jeder Zyklus meldet Anzahl der Eingaben und Digest.
#[test]
fn state_sync_rebuilds_the_mirror() {
    let plan = plan();
    let (mut a, mut b, mut sync) = (
        ChannelMemory::<64>::new(),
        ChannelMemory::<64>::new(),
        ChannelMemory::<64>::new(),
    );
    let states = [a.state(&plan), b.state(&plan), None];
    let mut node: TestNode<'_> = Node::new(
        Interlocking::new(plan, GenericRules::default()),
        states,
        node_config(),
    )
    .unwrap()
    .with_sync_buffer(sync.state(&plan).unwrap());
    let mut bench = Bench::new(plan);
    let a_n1 = bench.route("A-N1");
    for k in 0..40 {
        let requests = if k == 10 {
            vec![Request::SetRoute(a_n1)]
        } else {
            vec![]
        };
        let extra = if k == 30 { vec![HostMsg::Sync] } else { vec![] };
        bench.cycle(&mut node, &extra, &requests);
    }
    assert_eq!(bench.syncs.len(), 1, "ein Abgleich");
    let synced = &bench.syncs[0];
    assert_eq!(synced.plan, plan.digest()[..8]);
    let mut mirror = OwnedState::new(&plan);
    {
        let mut view = mirror.view();
        assert_eq!(synced.parts.len(), view.part_count());
        assert!(
            synced.parts.len() > railit_embedded::SYNC_PARTS_PER_TICK,
            "über mehrere Zyklen"
        );
        for part in &synced.parts {
            view.apply(*part).unwrap();
        }
    }
    assert_eq!(short_digest(&mut mirror), synced.digest);
    let (_, expected) = bench
        .history
        .iter()
        .find(|(cycle, _)| *cycle == synced.cycle)
        .expect("Zyklus des Abgleichs empfangen");
    assert_eq!(&mirror, expected);
    assert_eq!(bench.client.protocol_errors(), 0);
}

/// REQ-SAF-008: Die zyklische Speicherprüfung findet einen verfälschten Plan im Speicher und
/// führt in den sicheren Zustand; ein intakter Plan wird Durchlauf um Durchlauf bestätigt.
#[test]
fn memory_check_finds_a_corrupted_plan() {
    let plan = plan();
    let chunks = plan.body().len().div_ceil(4096);
    let cycles = 2 * u64::try_from(chunks).unwrap();
    let run = |body: &'static [u8]| {
        let mut a = ChannelMemory::<64>::new();
        let mut ram = [0u32; 128];
        let states = [a.state(&plan), None, None];
        let mut node: TestNode<'_> = Node::new(
            Interlocking::new(plan, GenericRules::default()),
            states,
            node_config(),
        )
        .unwrap()
        .with_memory_check(body, plan.digest(), &mut ram, 4096, 32);
        let mut trips = Vec::new();
        for cycle in 0..cycles {
            let t = cycle * 100_000;
            trips.push(node.tick(t, &mut || t + 10).verdict.trip);
        }
        (trips, node.stats().memory_passes)
    };
    let (trips, passes) = run(plan.body());
    assert!(trips.iter().all(Option::is_none));
    assert_eq!(passes, (2, cycles / 4));

    let mut corrupted = plan.body().to_vec();
    let middle = corrupted.len() / 2;
    corrupted[middle] ^= 0x10;
    let (trips, _) = run(Box::leak(corrupted.into_boxed_slice()));
    assert_eq!(
        trips[chunks - 1],
        Some(Trip::MemoryFault(railit_safety::MemoryRegion::Plan))
    );
    assert!(trips[..chunks - 1].iter().all(Option::is_none));
}
