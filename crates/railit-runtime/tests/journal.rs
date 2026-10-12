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
    let text = String::from_utf8(bytes).unwrap();
    let entries: Vec<Entry> = text
        .lines()
        .map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).unwrap();
            serde_json::from_value(v["entry"].clone()).unwrap()
        })
        .collect();
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
