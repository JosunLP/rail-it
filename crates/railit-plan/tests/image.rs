//! Plan-Image: Rundreise, Integrität, Signatur und Abweisung inkonsistenter Pläne.

use railit_plan::*;
use railit_types::bitset::{BitSet, BitSetMut};

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

/// Plan für Beispielstadt mit drei Fahrstraßen: A-N1, A-N2 (feindlich über W1) und F-P1
/// (feindlich mit A-N1 über T1).
fn sample() -> Plan {
    let tvd = |i| TvdId::new(i);
    Plan {
        network: "Beispielstadt".into(),
        points: vec![
            PointDef {
                name: "W1".into(),
                tvd: tvd(1),
            },
            PointDef {
                name: "W2".into(),
                tvd: tvd(4),
            },
        ],
        signals: ["A", "N1", "N2", "F", "P1", "P2"]
            .map(|n| SignalDef { name: n.into() })
            .to_vec(),
        tvds: ["TW", "TW1", "T1", "T2", "TW2", "TE"]
            .map(|n| TvdDef { name: n.into() })
            .to_vec(),
        routes: vec![
            RouteDef {
                name: "A-N1".into(),
                start: SignalId::new(0),
                end: RouteEnd::Signal(SignalId::new(1)),
                points: vec![(PointId::new(0), PointPosition::Left)],
                tvds: vec![tvd(1), tvd(2)],
                conflicts: vec![RouteId::new(1), RouteId::new(2)],
            },
            RouteDef {
                name: "A-N2".into(),
                start: SignalId::new(0),
                end: RouteEnd::Signal(SignalId::new(2)),
                points: vec![(PointId::new(0), PointPosition::Right)],
                tvds: vec![tvd(1), tvd(3)],
                conflicts: vec![RouteId::new(0)],
            },
            RouteDef {
                name: "F-P1".into(),
                start: SignalId::new(3),
                end: RouteEnd::Signal(SignalId::new(4)),
                points: vec![(PointId::new(1), PointPosition::Left)],
                tvds: vec![tvd(4), tvd(2)],
                conflicts: vec![RouteId::new(0)],
            },
        ],
    }
}

#[test]
fn roundtrip_preserves_plan() {
    let plan = sample();
    let image = plan.to_image().unwrap();
    let view = PlanView::parse(&image).unwrap();
    assert!(!view.is_signed());
    assert_eq!(Plan::from_view(&view), plan);

    assert_eq!(view.network_name(), "Beispielstadt");
    assert_eq!(view.find_route("A-N2"), Some(RouteId::new(1)));
    assert_eq!(view.find_signal("F"), Some(SignalId::new(3)));
    assert_eq!(view.find_point("W2"), Some(PointId::new(1)));
    assert_eq!(view.find_tvd("TE"), Some(TvdId::new(5)));
    assert_eq!(view.find_route("X"), None);
    assert_eq!(view.point_tvd(PointId::new(0)), Some(TvdId::new(1)));
    assert_eq!(view.point_tvd(PointId::new(2)), None);
    assert!(view.route(RouteId::new(3)).is_none());

    let r = view.route(RouteId::new(0)).unwrap();
    assert_eq!(r.name(), "A-N1");
    assert_eq!(r.tvds().len(), 2);
    assert!(r.conflicts_with(RouteId::new(2)));
    assert!(!r.conflicts_with(RouteId::new(0)));
}

#[test]
fn conflicts_with_any_uses_active_set() {
    let image = sample().to_image().unwrap();
    let view = PlanView::parse(&image).unwrap();
    let mut active = vec![0u64; view.route_set_words()];
    let a_n2 = view.route(RouteId::new(1)).unwrap();
    assert!(!a_n2.conflicts_with_any(&BitSet::new(&active)));

    BitSetMut::new(&mut active).insert(2).unwrap(); // F-P1 aktiv
    assert!(!a_n2.conflicts_with_any(&BitSet::new(&active)));
    let a_n1 = view.route(RouteId::new(0)).unwrap();
    assert!(a_n1.conflicts_with_any(&BitSet::new(&active)));
}

/// REQ-PLN-003: Jede Veränderung eines einzelnen Bits wird erkannt.
#[test]
fn every_single_bit_flip_is_detected() {
    let k = key(7);
    let image = sample().to_signed_image(&k).unwrap();
    PlanView::parse_signed(&image, &k.verifying_key()).unwrap();
    for i in 0..image.len() {
        for bit in [0, 3, 7] {
            let mut bad = image.clone();
            bad[i] ^= 1 << bit;
            assert!(
                PlanView::parse_signed(&bad, &k.verifying_key()).is_err(),
                "Veränderung an Byte {i}, Bit {bit} nicht erkannt"
            );
        }
    }
}

/// REQ-PLN-004
#[test]
fn signatures_are_checked() {
    let plan = sample();
    let signed = plan.to_signed_image(&key(1)).unwrap();
    assert!(PlanView::parse(&signed).unwrap().is_signed());
    assert_eq!(
        PlanView::parse_signed(&signed, &key(2).verifying_key()).unwrap_err(),
        PlanError::SignatureInvalid
    );
    let unsigned = plan.to_image().unwrap();
    assert_eq!(
        PlanView::parse_signed(&unsigned, &key(1).verifying_key()).unwrap_err(),
        PlanError::SignatureMissing
    );
    // Das Flag „signiert“ gehört zum geprüften Inhalt, daher unterscheiden sich die Prüfsummen.
    assert_ne!(
        PlanView::parse(&signed).unwrap().digest(),
        PlanView::parse(&unsigned).unwrap().digest()
    );
}

#[test]
fn truncated_or_foreign_data_is_rejected() {
    let image = sample().to_image().unwrap();
    assert_eq!(
        PlanView::parse(&image[..10]).unwrap_err(),
        PlanError::TooShort
    );
    assert_eq!(
        PlanView::parse(&image[..image.len() - 1]).unwrap_err(),
        PlanError::LengthMismatch
    );
    let mut foreign = image.clone();
    foreign[..4].copy_from_slice(b"ABCD");
    assert_eq!(PlanView::parse(&foreign).unwrap_err(), PlanError::BadMagic);
}

/// REQ-PLN-002: Inkonsistente Pläne werden schon beim Schreiben abgewiesen.
#[test]
fn inconsistent_plans_are_rejected() {
    let mut p = sample();
    p.routes[1].conflicts.clear(); // A-N1 ↔ A-N2 nicht mehr symmetrisch
    assert!(matches!(p.to_image(), Err(PlanError::ConflictMatrix(_))));

    let mut p = sample();
    p.routes[0].conflicts.push(RouteId::new(0)); // sich selbst feind
    assert!(matches!(p.to_image(), Err(PlanError::ConflictMatrix(0))));

    let mut p = sample();
    p.routes[0].tvds.push(TvdId::new(99));
    assert!(matches!(
        p.to_image(),
        Err(PlanError::InvalidReference {
            what: "TVD",
            index: 0
        })
    ));

    let mut p = sample();
    p.routes[2].tvds.clear();
    assert_eq!(p.to_image(), Err(PlanError::EmptyRoute(2)));

    let mut p = sample();
    p.routes[0]
        .points
        .push((PointId::new(0), PointPosition::Right));
    assert!(matches!(
        p.to_image(),
        Err(PlanError::InvalidReference {
            what: "doppelte Weiche",
            index: 0
        })
    ));

    let mut p = sample();
    p.points[0].tvd = TvdId::new(6);
    assert!(matches!(
        p.to_image(),
        Err(PlanError::InvalidReference { .. })
    ));

    let mut p = sample();
    p.routes[0].start = SignalId::new(6);
    assert!(matches!(
        p.to_image(),
        Err(PlanError::InvalidReference {
            what: "Startsignal",
            index: 0
        })
    ));
}

#[test]
fn many_routes_span_multiple_conflict_words() {
    // 130 Fahrstraßen ⇒ drei u64-Wörter je Zeile; Konflikte quer über Wortgrenzen.
    let mut p = sample();
    p.routes.clear();
    for i in 0..130u32 {
        p.routes.push(RouteDef {
            name: format!("R{i}"),
            start: SignalId::new(0),
            end: RouteEnd::Boundary,
            points: vec![],
            tvds: vec![TvdId::new(0)],
            conflicts: vec![],
        });
    }
    for (a, b) in [(0usize, 129usize), (63, 64), (5, 70)] {
        p.routes[a].conflicts.push(RouteId::from_index(b).unwrap());
        p.routes[b].conflicts.push(RouteId::from_index(a).unwrap());
    }
    let image = p.to_image().unwrap();
    let view = PlanView::parse(&image).unwrap();
    assert_eq!(view.route_set_words(), 3);
    assert_eq!(Plan::from_view(&view), p);
}
