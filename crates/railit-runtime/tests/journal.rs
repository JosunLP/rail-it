//! Journal eines Betriebs mit Simulator: schreiben, prüfen, abspielen (REQ-JRN-001/002).

use std::io::Write;
use std::sync::{Arc, Mutex};

use railit_journal::{JournalError, replay, verify};
use railit_kernel::{AuxOperation, OperatorId, Request, RequestId};
use railit_rules::GenericRules;
use railit_runtime::Runtime;
use railit_sim::{Fault, SimConfig, Simulator, TrainSpec};
use railit_types::{Duration, Length, Speed};

const EXAMPLE: &str = include_str!("../../../examples/beispielstadt.toml");

#[derive(Clone, Default)]
struct Shared(Arc<Mutex<Vec<u8>>>);

impl Write for Shared {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn run() -> (Runtime<Simulator>, Vec<u8>) {
    run_with_snapshots(None)
}

fn run_with_snapshots(every: Option<u64>) -> (Runtime<Simulator>, Vec<u8>) {
    let net = railit_import::toml_format::parse(EXAMPLE).unwrap().topology;
    let compiled = railit_planning::compile(&net, None).unwrap();
    let plan = railit_runtime::load_plan(compiled.image, None).unwrap();
    let sim = Simulator::new(net, SimConfig::default());
    let mut rt = Runtime::new(
        plan,
        GenericRules::default(),
        sim,
        Duration::from_millis(100),
    );
    let buf = Shared::default();
    if let Some(every) = every {
        rt.set_journal_snapshots(every);
    }
    rt.start_journal(0, Box::new(buf.clone()), 50).unwrap();

    let a = plan.find_signal("A").unwrap();
    rt.field_mut()
        .add_train(TrainSpec {
            name: "RE1".into(),
            signal: a,
            length: Length::from_mm(120_000),
            speed: Speed::from_kmh(60),
            ignore_signals: false,
            etcs: false,
        })
        .unwrap();
    let mut id = 0;
    let mut req = |rt: &mut Runtime<Simulator>, r: Request| {
        id += 1;
        rt.cycle(&[(RequestId(id), r)]);
    };
    req(&mut rt, Request::SetRoute(plan.find_route("A-N1").unwrap()));
    for _ in 0..600 {
        rt.cycle(&[]);
    }
    rt.field_mut()
        .inject(Fault::TvdDisturbed(plan.find_tvd("T2").unwrap()));
    rt.cycle(&[]);
    let op = AuxOperation::ResetTvd(plan.find_tvd("T2").unwrap());
    req(
        &mut rt,
        Request::AuxRequest {
            op,
            operator: OperatorId(1),
        },
    );
    req(
        &mut rt,
        Request::AuxConfirm {
            aux: railit_kernel::AuxId(0),
            operator: OperatorId(2),
            op,
        },
    );
    for _ in 0..20 {
        rt.cycle(&[]);
    }
    assert_eq!(rt.journal_error(), None);
    let bytes = buf.0.lock().unwrap().clone();
    (rt, bytes)
}

#[test]
fn journal_replays_to_the_same_state() {
    let (rt, bytes) = run();
    assert_eq!(rt.state().meta().aux_counter(), 1);

    let summary = verify(bytes.as_slice()).unwrap();
    assert_eq!(summary.cycles, 624);
    assert_eq!(summary.checkpoints, 12);
    assert_eq!(Some(summary.head.clone()), rt.journal_head(0));

    let (replayed, state) = replay(bytes.as_slice(), rt.plan()).unwrap();
    assert_eq!(replayed, summary);
    assert_eq!(&state, rt.state());
}

#[test]
fn tampering_breaks_the_chain() {
    let (_, bytes) = run();
    let text = String::from_utf8(bytes).unwrap();
    let mut lines: Vec<String> = text.lines().map(String::from).collect();

    // Eine Meldung verfälschen.
    let tampered = lines[300].replacen("\"clear\"", "\"occupied\"", 1);
    assert_ne!(tampered, lines[300]);
    let mut altered = lines.clone();
    altered[300] = tampered;
    let err = verify(altered.join("\n").as_bytes()).unwrap_err();
    assert_eq!(err, JournalError::Chain { line: 301 });

    // Eine Zeile entfernen.
    lines.remove(200);
    let err = verify(lines.join("\n").as_bytes()).unwrap_err();
    assert_eq!(err, JournalError::Chain { line: 201 });
}

#[test]
fn forged_journal_is_detected_by_replay() {
    use railit_journal::{Entry, JournalWriter};
    use railit_kernel::{Input, Occupancy};

    let (rt, bytes) = run();
    let entries: Vec<Entry> = railit_journal::entries(bytes.as_slice()).unwrap();
    let Entry::Header(header) = entries[0].clone() else {
        panic!("Kopf erwartet")
    };

    // Kette neu aufbauen, aber in Zyklus 400 einen Abschnitt als besetzt ausgeben.
    let mut w = JournalWriter::new(Vec::new(), header).unwrap();
    for e in &entries[1..] {
        let mut e = e.clone();
        if let Entry::Cycle(c) = &mut e
            && c.cycle == 400
        {
            for i in &mut c.inputs {
                if let Input::Tvd { occupancy, .. } = i {
                    *occupancy = Occupancy::Occupied;
                }
            }
        }
        w.append(&e).unwrap();
    }
    let forged = w.into_inner();

    verify(forged.as_slice()).unwrap();
    let err = replay(forged.as_slice(), rt.plan()).unwrap_err();
    assert!(
        matches!(err, JournalError::Divergence { cycle: 400, .. }),
        "{err}"
    );
}

#[test]
fn replay_needs_the_recorded_plan() {
    let (_, bytes) = run();
    let other = EXAMPLE.replace("Beispielstadt", "Andere Stadt");
    let net = railit_import::toml_format::parse(&other).unwrap().topology;
    let image = railit_planning::compile(&net, None).unwrap().image;
    let plan = railit_runtime::load_plan(image, None).unwrap();
    let err = replay(bytes.as_slice(), &plan).unwrap_err();
    assert!(matches!(err, JournalError::Plan(_)), "{err}");
}

/// REQ-JRN-002: In Netzen mit Stellbereichen hat jeder Kern sein eigenes Journal; jedes
/// lässt sich für sich abspielen, auch mit Grenzmeldungen als Eingaben.
#[test]
fn every_area_journal_replays() {
    let net = railit_import::toml_format::parse(include_str!("../../../examples/nordbahn.toml"))
        .unwrap()
        .topology;
    let mut rt = railit_runtime::simulation(
        net,
        GenericRules::default(),
        SimConfig::default(),
        Duration::from_millis(100),
    )
    .unwrap();
    assert_eq!(rt.area_count(), 3);
    let buffers: Vec<Shared> = (0..3).map(|_| Shared::default()).collect();
    for (i, b) in buffers.iter().enumerate() {
        rt.start_journal(i, Box::new(b.clone()), 25).unwrap();
    }
    for _ in 0..80 {
        rt.cycle(&[]);
    }
    let (strecke, border) = rt.find_border("Strecke>Adorf").unwrap();
    rt.cycle_addressed(&[railit_runtime::Addressed {
        area: strecke,
        id: RequestId(1),
        request: Request::RequestPermission(border),
    }]);
    for _ in 0..20 {
        rt.cycle(&[]);
    }
    let holds = rt.state_of(strecke).unwrap().borders()[0].holding();
    assert!(holds, "Strecke hat die Erlaubnis übernommen");

    for (i, b) in buffers.iter().enumerate() {
        let bytes = b.0.lock().unwrap().clone();
        let plan = rt.plan_of(i).unwrap();
        let (summary, state) = replay(bytes.as_slice(), plan).unwrap();
        assert_eq!(summary.cycles, 101);
        assert_eq!(&state, rt.state_of(i).unwrap());
    }
}

/// REQ-SAF-004, REQ-JRN-002: Abschaltung und Rücksetzen der Sicherheitsschicht stehen im
/// Journal; das Abspielen überspringt die Zyklen im sicheren Zustand und vergleicht danach
/// wieder.
#[test]
fn journal_covers_trip_and_reset_of_the_safety_layer() {
    use railit_journal::SafetyKind;
    use railit_runtime::SafetyConfig;
    use railit_runtime::railit_safety::{Fault, Redundancy};

    let net = railit_import::toml_format::parse(EXAMPLE).unwrap().topology;
    let compiled = railit_planning::compile(&net, None).unwrap();
    let plan = railit_runtime::load_plan(compiled.image, None).unwrap();
    let sim = Simulator::new(net, SimConfig::default());
    let mut rt = Runtime::new(
        plan,
        GenericRules::default(),
        sim,
        Duration::from_millis(100),
    );
    rt.set_safety(SafetyConfig {
        redundancy: Redundancy::TwoOutOfThree,
        ..SafetyConfig::default()
    })
    .unwrap();
    let buf = Shared::default();
    // Abzüge vor dem Abschalten und nach dem Rücksetzen; keine im abgeschalteten Zustand.
    rt.set_journal_snapshots(7);
    rt.start_journal(0, Box::new(buf.clone()), 10).unwrap();
    let a_n1 = plan.find_route("A-N1").unwrap();
    rt.cycle(&[(RequestId(1), Request::SetRoute(a_n1))]);
    for _ in 0..30 {
        rt.cycle(&[]);
    }
    rt.inject_channel_fault(0, 2, Fault::StrayCommand).unwrap();
    let out = rt.cycle(&[]);
    assert_eq!(out.safety.len(), 1, "Kanal 2 ausgeschlossen");
    assert_eq!(out.safe_state, None);
    rt.reintegrate_channel(0, 2).unwrap();
    rt.inject_channel_fault(0, 0, Fault::Stall).unwrap();
    rt.cycle(&[]);
    rt.inject_channel_fault(0, 1, Fault::StrayCommand).unwrap();
    let out = rt.cycle(&[]);
    assert!(out.safe_state.is_some(), "abgeschaltet");
    for _ in 0..20 {
        rt.cycle(&[]);
    }
    rt.reset_safety(0).unwrap();
    rt.cycle(&[(RequestId(2), Request::SetRoute(a_n1))]);
    for _ in 0..30 {
        rt.cycle(&[]);
    }
    assert_eq!(rt.journal_error(), None);

    let bytes = buf.0.lock().unwrap().clone();
    let (summary, state) = replay(bytes.as_slice(), &plan).unwrap();
    assert!(summary.snapshots >= 6, "{} Abzüge", summary.snapshots);
    let kinds: Vec<_> = summary.safety.iter().map(|r| r.kind).collect();
    assert_eq!(
        kinds,
        [
            SafetyKind::Excluded,
            SafetyKind::Reintegrated,
            SafetyKind::Excluded,
            SafetyKind::Tripped,
            SafetyKind::Reset
        ]
    );
    assert_eq!(&state, rt.state(), "Endzustand nach dem Rücksetzen");
}

/// Konzept, Abschnitt 15: Mit Zustandsabzügen spielt das Replay die Abschnitte parallel ab
/// und kommt zum selben Ergebnis wie am Stück.
#[test]
fn snapshots_split_the_replay() {
    let (rt, plain) = run();
    let (_, bytes) = run_with_snapshots(Some(50));
    let summary = verify(bytes.as_slice()).unwrap();
    assert_eq!(summary.cycles, 624);
    assert_eq!(summary.snapshots, 12);
    let (replayed, state) = replay(bytes.as_slice(), rt.plan()).unwrap();
    assert_eq!(replayed, summary);
    assert_eq!(&state, rt.state());
    assert_eq!(replay(plain.as_slice(), rt.plan()).unwrap().1, state);
}

/// Ein Abschnitt muss genau im nächsten Abzug enden: Ein ausgetauschter Abzug fällt beim
/// Replay auf, obwohl die Kette neu berechnet wurde.
#[test]
fn forged_snapshot_is_detected_by_replay() {
    use railit_journal::{Entry, JournalWriter};

    let (rt, bytes) = run_with_snapshots(Some(50));
    let mut entries = railit_journal::entries(bytes.as_slice()).unwrap();
    let snapshots: Vec<usize> = (0..entries.len())
        .filter(|i| matches!(entries[*i], Entry::Snapshot(_)))
        .collect();
    let (Entry::Snapshot(third), Entry::Snapshot(fourth)) =
        (entries[snapshots[2]].clone(), entries[snapshots[3]].clone())
    else {
        panic!("Abzüge erwartet")
    };
    // Der dritte Abzug trägt den Zustand des vierten.
    entries[snapshots[2]] = Entry::Snapshot(Box::new(railit_journal::Snapshot {
        cycle: third.cycle,
        state: fourth.state,
    }));
    let Entry::Header(header) = entries[0].clone() else {
        panic!("Kopf erwartet")
    };
    let mut w = JournalWriter::new(Vec::new(), header).unwrap();
    for e in &entries[1..] {
        w.append(e).unwrap();
    }
    let forged = w.into_inner();
    verify(forged.as_slice()).unwrap();
    let err = replay(forged.as_slice(), rt.plan()).unwrap_err();
    assert_eq!(
        err,
        JournalError::Divergence {
            cycle: third.cycle,
            what: "snapshot".into()
        }
    );

    // Ein Abzug, der nicht zum Zyklus davor gehört, ist ein Formatfehler.
    entries[snapshots[2]] = Entry::Snapshot(Box::new(railit_journal::Snapshot {
        cycle: third.cycle + 1,
        state: third.state.clone(),
    }));
    let Entry::Header(header) = entries[0].clone() else {
        panic!("Kopf erwartet")
    };
    let mut w = JournalWriter::new(Vec::new(), header).unwrap();
    for e in &entries[1..] {
        w.append(e).unwrap();
    }
    let misplaced = w.into_inner();
    assert!(matches!(
        verify(misplaced.as_slice()),
        Err(JournalError::Format(_))
    ));
}

/// Verbindung zum Knoten, die der Test trennen kann.
struct Cuttable {
    inner: railit_runtime::LockstepLink,
    cut: Arc<std::sync::atomic::AtomicBool>,
}

impl railit_runtime::NodeLink for Cuttable {
    fn clock_ms(&self, now: railit_types::Tick) -> u32 {
        self.inner.clock_ms(now)
    }

    fn exchange(&mut self, now: railit_types::Tick, outgoing: Vec<Vec<u8>>) -> Vec<Vec<u8>> {
        self.inner
            .set_cut(self.cut.load(std::sync::atomic::Ordering::Relaxed));
        self.inner.exchange(now, outgoing)
    }

    fn describe(&self) -> String {
        "trennbar".into()
    }
}

/// REQ-JRN-007: Im Knotenbetrieb schreibt der Spiegel jeden Zyklus des Knotens, den er
/// nachgerechnet und geprüft hat, und jeden Zustandsabgleich als Übernahme. Auch über eine
/// Verbindungsunterbrechung hinweg spielt das Journal zum Zustand des Spiegels ab.
#[test]
fn node_journal_replays_across_resyncs() {
    use railit_runtime::railit_safety::Redundancy;
    use railit_runtime::{LockstepLink, NodeSetup};
    use std::sync::atomic::{AtomicBool, Ordering};

    let net = railit_import::toml_format::parse(EXAMPLE).unwrap().topology;
    let compiled = railit_planning::compile(&net, None).unwrap();
    let plan = railit_runtime::load_plan(compiled.image, None).unwrap();
    let sim = Simulator::new(net, SimConfig::default());
    let mut rt = Runtime::new(
        plan,
        GenericRules::default(),
        sim,
        Duration::from_millis(100),
    );
    let setup = NodeSetup {
        redundancy: Redundancy::TwoOutOfThree,
        deadline_us: 1_000_000,
        gap_limit_us: 1_000_000,
    };
    let cut = Arc::new(AtomicBool::new(false));
    let link = Cuttable {
        inner: LockstepLink::spawn(plan, GenericRules::default(), setup).unwrap(),
        cut: Arc::clone(&cut),
    };
    rt.attach_node(0, Box::new(link)).unwrap();
    rt.set_journal_snapshots(100);
    let buf = Shared::default();
    rt.start_journal(0, Box::new(buf.clone()), 50).unwrap();

    let a = plan.find_signal("A").unwrap();
    rt.field_mut()
        .add_train(TrainSpec {
            name: "RE1".into(),
            signal: a,
            length: Length::from_mm(120_000),
            speed: Speed::from_kmh(60),
            ignore_signals: false,
            etcs: false,
        })
        .unwrap();
    let mut id = 0;
    let mut req = |rt: &mut Runtime<Simulator>, r: Request| {
        id += 1;
        rt.cycle(&[(RequestId(id), r)]);
    };
    for _ in 0..20 {
        rt.cycle(&[]);
    }
    assert!(rt.nodes_synced(), "Knoten nicht abgeglichen");
    req(&mut rt, Request::SetRoute(plan.find_route("A-N1").unwrap()));
    for _ in 0..300 {
        rt.cycle(&[]);
    }
    // Unterbrechung länger als die RaSTA-Überwachungszeit: neuer Aufbau, neuer Abgleich.
    cut.store(true, Ordering::Relaxed);
    for _ in 0..60 {
        rt.cycle(&[]);
    }
    cut.store(false, Ordering::Relaxed);
    for _ in 0..300 {
        rt.cycle(&[]);
    }
    assert!(
        rt.nodes_synced(),
        "Knoten nach der Unterbrechung nicht abgeglichen"
    );
    assert_eq!(rt.journal_error(), None);

    let bytes = buf.0.lock().unwrap().clone();
    let summary = verify(bytes.as_slice()).unwrap();
    assert!(summary.resyncs >= 2, "{} Übernahmen", summary.resyncs);
    assert!(summary.cycles > 500, "{} Zyklen", summary.cycles);
    assert!(summary.snapshots >= 3, "{} Abzüge", summary.snapshots);
    let (_, state) = replay(bytes.as_slice(), &plan).unwrap();
    assert_eq!(&state, rt.state(), "Endzustand des Spiegels");
}

/// REQ-JRN-008: Ein Zeitfenster spielt ab dem nächstgelegenen Abzug ab und liefert dieselben
/// Zyklen, Ausgaben und Zustände wie das Abspielen vom Anfang.
#[test]
fn window_replays_from_the_nearest_snapshot() {
    use railit_journal::replay_window;
    use railit_kernel::Event;

    let (rt, plain) = run();
    let (_, with_snapshots) = run_with_snapshots(Some(50));
    let collect = |bytes: &[u8], from: u64, to: u64| {
        let mut seen = Vec::new();
        let state = replay_window(bytes, rt.plan(), from, to, |c| {
            assert!((from..=to).contains(&c.now_ms), "{} außerhalb", c.now_ms);
            seen.push((c.cycle, c.now_ms, c.commands.to_vec(), c.events.to_vec()));
        })
        .unwrap();
        (seen, state)
    };
    let (a, state_a) = collect(&with_snapshots, 30_000, 40_000);
    let (b, state_b) = collect(&plain, 30_000, 40_000);
    assert_eq!(a.len(), 101, "Zyklen von 30 s bis 40 s");
    assert_eq!(a, b);
    assert_eq!(state_a, state_b);

    // Das ganze Journal endet im Endzustand; die Anforderung am Anfang wurde angenommen.
    let (all, end) = collect(&with_snapshots, 0, u64::MAX);
    assert_eq!(&end, rt.state());
    assert!(
        all.iter()
            .take(3)
            .any(|(_, _, _, events)| events.iter().any(|e| matches!(e, Event::Accepted(_)))),
        "Annahme am Anfang"
    );
}

/// REQ-JRN-009: Ein Mitleser verfolgt das wachsende Journal und hat nach jedem Zyklus genau
/// den Zustand des Stellwerks, auch wenn die Bytes in beliebigen Stücken ankommen.
#[test]
fn follower_tracks_the_running_interlocking() {
    use railit_journal::Follower;

    let net = railit_import::toml_format::parse(EXAMPLE).unwrap().topology;
    let compiled = railit_planning::compile(&net, None).unwrap();
    let plan = railit_runtime::load_plan(compiled.image, None).unwrap();
    let sim = Simulator::new(net, SimConfig::default());
    let mut rt = Runtime::new(
        plan,
        GenericRules::default(),
        sim,
        Duration::from_millis(100),
    );
    rt.set_journal_snapshots(40);
    let buf = Shared::default();
    rt.start_journal(0, Box::new(buf.clone()), 25).unwrap();
    rt.field_mut()
        .add_train(TrainSpec {
            name: "RE1".into(),
            signal: plan.find_signal("A").unwrap(),
            length: Length::from_mm(120_000),
            speed: Speed::from_kmh(60),
            ignore_signals: false,
            etcs: false,
        })
        .unwrap();

    let mut follower = Follower::new(&plan, None);
    let mut fed = 0usize;
    let mut rng = 0x9E37_79B9u64;
    let a_n1 = plan.find_route("A-N1").unwrap();
    for cycle in 0..500u64 {
        let requests = if cycle == 2 {
            vec![(RequestId(1), Request::SetRoute(a_n1))]
        } else {
            Vec::new()
        };
        rt.cycle(&requests);
        let bytes = buf.0.lock().unwrap().clone();
        // In zufälligen Stücken zuführen, die letzte Zeile auch halb.
        while fed < bytes.len() {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            let n = usize::try_from(rng % 700).unwrap() + 1;
            let end = (fed + n).min(bytes.len());
            follower.feed(&bytes[fed..end]).unwrap();
            fed = end;
        }
        assert_eq!(follower.state(), Some(rt.state()), "Zyklus {cycle}");
    }
    let summary = follower.summary().unwrap();
    assert_eq!(summary, verify(buf.0.lock().unwrap().as_slice()).unwrap());
    assert!(summary.snapshots >= 10, "{summary:?}");

    // Verfälschte Zeile: der Mitleser bleibt mit dem Fehler stehen.
    let text = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
    let mut lines: Vec<&str> = text.lines().collect();
    let tampered = lines[100].replacen("\"now_ms\":", "\"now_ms\":1", 1);
    assert_ne!(tampered, lines[100]);
    lines[100] = &tampered;
    let mut forged = Follower::new(&plan, None);
    let err = forged.feed(lines.join("\n").as_bytes()).unwrap_err();
    assert_eq!(err, JournalError::Chain { line: 101 });
    assert_eq!(forged.feed(b"\n").unwrap_err(), err, "bleibt stehen");
}

/// REQ-JRN-009: Auch das Journal des Spiegels im Knotenbetrieb lässt sich mitlesen.
#[test]
fn follower_reads_node_journal() {
    let (rt_bytes, plan) = {
        let net = railit_import::toml_format::parse(EXAMPLE).unwrap().topology;
        let compiled = railit_planning::compile(&net, None).unwrap();
        let plan = railit_runtime::load_plan(compiled.image, None).unwrap();
        let sim = Simulator::new(net, SimConfig::default());
        let mut rt = Runtime::new(
            plan,
            GenericRules::default(),
            sim,
            Duration::from_millis(100),
        );
        rt.use_lockstep_nodes().unwrap();
        rt.set_journal_snapshots(30);
        let buf = Shared::default();
        rt.start_journal(0, Box::new(buf.clone()), 10).unwrap();
        for i in 0..200u64 {
            let r = if i == 30 {
                vec![(
                    RequestId(1),
                    Request::SetRoute(plan.find_route("A-N1").unwrap()),
                )]
            } else {
                Vec::new()
            };
            rt.cycle(&r);
        }
        let bytes = buf.0.lock().unwrap().clone();
        let mut follower = railit_journal::Follower::new(&plan, None);
        follower.feed(&bytes).unwrap();
        assert_eq!(follower.state(), Some(rt.state()));
        (bytes, plan)
    };
    let summary = verify(rt_bytes.as_slice()).unwrap();
    assert!(summary.resyncs >= 1 && summary.cycles > 100, "{summary:?}");
    let _ = plan;
}
