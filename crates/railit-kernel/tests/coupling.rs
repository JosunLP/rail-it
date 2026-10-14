//! Gekoppelte Weichen im Kern (REQ-IXL-026) am DKW-Kopf (`examples/dkw.toml`).

mod common;

use common::{Harness, accepted, leak, rejected};
use railit_kernel::*;
use railit_types::PointPosition;

const DKW: &str = include_str!("../../../examples/dkw.toml");

fn harness() -> Harness {
    let net = railit_import::toml_format::parse(DKW).unwrap().topology;
    let plan = railit_planning::compile(&net, None).unwrap().plan;
    let mut h = Harness::with_plan(leak(&plan));
    h.step(&[]);
    h.run(2);
    h
}

fn moves(h: &Harness) -> Vec<(String, PointPosition)> {
    h.out
        .commands
        .iter()
        .filter_map(|c| match c {
            Command::MovePoint { point, position } => {
                Some((h.plan().point_name(*point).unwrap().to_string(), *position))
            }
            _ => None,
        })
        .collect()
}

/// REQ-IXL-026: Eine einzeln gestellte Weiche nimmt ihre Partnerweiche mit; gegenläufig
/// gekoppelt läuft sie in die andere Lage.
#[test]
fn single_move_takes_the_partner_along() {
    let mut h = harness();
    let target = h.position("DKW1a").other();
    h.out.clear();
    let a = h.point("DKW1a");
    let events = h.step(&[Request::MovePoint(a, target)]);
    assert!(accepted(&events), "{events:?}");
    let moved = moves(&h);
    assert_eq!(
        moved,
        [("DKW1a".into(), target), ("DKW1b".into(), target.other())]
    );
    h.run(2);
    assert_eq!(h.position("DKW1a"), target);
    assert_eq!(h.position("DKW1b"), target.other());
}

/// REQ-IXL-026, REQ-PLN-021: Eine Fahrstraße über die DKW stellt und verschließt alle vier
/// Zungenpaare; keines lässt sich danach einzeln umstellen.
#[test]
fn route_locks_all_four_points() {
    let mut h = harness();
    h.clear_route("A1-N3");
    assert_eq!(h.position("DKW1a"), PointPosition::Right);
    assert_eq!(h.position("DKW1b"), PointPosition::Left);
    assert_eq!(h.position("DKW1c"), PointPosition::Left);
    assert_eq!(h.position("DKW1d"), PointPosition::Right);
    for name in ["DKW1a", "DKW1b", "DKW1c", "DKW1d"] {
        let p = h.point(name);
        let pos = h.position(name).other();
        let events = h.step(&[Request::MovePoint(p, pos)]);
        assert!(
            matches!(rejected(&events), Some(Rejection::PointLocked(_))),
            "{name}: {events:?}"
        );
    }
    assert_eq!(h.last, Ok(()));
}

/// REQ-IXL-026, REQ-IXL-014: Auch die Hilfshandlung „Weiche umstellen“ stellt die
/// Partnerweiche mit.
#[test]
fn forced_move_takes_the_partner_along() {
    let mut h = harness();
    h.set_tvd("DKW1", Occupancy::Disturbed);
    h.run(1);
    let c = h.point("DKW1c");
    let target = h.position("DKW1c").other();
    h.out.clear();
    let events = h.aux(AuxOperation::ForcePoint(c, target));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::AuxExecuted { .. })),
        "{events:?}"
    );
    let moved = moves(&h);
    assert!(moved.contains(&("DKW1c".into(), target)), "{moved:?}");
    assert!(
        moved.contains(&("DKW1d".into(), target.other())),
        "{moved:?}"
    );
}
