//! Gekoppelte Weichen über EULYNX: ein Antrieb je Kopplung (REQ-TOP-017, REQ-IXL-026).

use railit_adapters::EulynxField;
use railit_kernel::{Command, Input, PointReport};
use railit_sim::{SimConfig, Simulator};
use railit_topology::{ElementRef, PointId};
use railit_types::{PointPosition, Tick};

const DKW: &str = include_str!("../../../examples/dkw.toml");

fn point(net: &railit_topology::Topology, name: &str) -> PointId {
    match net.lookup(name) {
        Some(ElementRef::Point(p)) => p,
        _ => panic!("{name}"),
    }
}

fn report(field: &EulynxField, p: PointId) -> Option<PointReport> {
    field.reports().into_iter().find_map(|i| match i {
        Input::Point { point, report } if point == p => Some(report),
        _ => None,
    })
}

fn run(field: &mut EulynxField, from_ms: u64, to_ms: u64) {
    let mut t = from_ms;
    while t <= to_ms {
        field.advance(Tick::from_millis(t));
        t += 50;
    }
}

/// REQ-IXL-026: Die vier Zungenpaare der DKW hängen an zwei Antrieben. Ein Befehl an die
/// zweite Weiche einer Kopplung stellt den gemeinsamen Antrieb, dessen Meldung für beide gilt.
#[test]
fn coupled_points_share_one_drive() {
    let net = railit_import::toml_format::parse(DKW).unwrap().topology;
    let (b, d) = (point(&net, "DKW1b"), point(&net, "DKW1d"));
    let (a, c) = (point(&net, "DKW1a"), point(&net, "DKW1c"));
    let mut field = EulynxField::new(Simulator::new(net, SimConfig::default())).unwrap();
    let points = field
        .links()
        .into_iter()
        .find(|l| l.controller == "OC-Weichen")
        .unwrap();
    assert_eq!(points.elements, 2);
    run(&mut field, 0, 1000);
    // Alle vier Weichen melden, obwohl nur zwei Elemente angebunden sind.
    for p in [a, b, c, d] {
        assert!(report(&field, p).is_some(), "{p:?}");
    }
    let start = report(&field, d).unwrap();
    let target = if start == PointReport::Left {
        PointPosition::Right
    } else {
        PointPosition::Left
    };
    field.apply(&[Command::MovePoint {
        point: d,
        position: target,
    }]);
    run(&mut field, 1050, 6000);
    assert_eq!(report(&field, d), Some(PointReport::from(target)));
    assert_eq!(report(&field, c), Some(PointReport::from(target.other())));
    // Der Simulator hat beide Zungenpaare bewegt.
    let sim = field.simulator();
    assert_eq!(sim.point_report(c), Some(PointReport::from(target.other())));
    assert_eq!(sim.point_report(d), Some(PointReport::from(target)));
}
