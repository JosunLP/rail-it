//! Plan-Image: Rundreise, Integrität, Signatur und Abweisung inkonsistenter Pläne.

use railit_plan::*;
use railit_types::bitset::{BitSet, BitSetMut};

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn route(
    name: &str,
    start: u32,
    end: RouteEnd,
    points: Vec<(PointId, PointPosition)>,
    tvds: Vec<u32>,
    conflicts: Vec<u32>,
) -> RouteDef {
    RouteDef {
        name: name.into(),
        start: SignalId::new(start),
        end,
        speed: None,
        points,
        tvds: tvds.into_iter().map(TvdId::new).collect(),
        overlap_tvds: vec![],
        overlap_points: vec![],
        flank: vec![],
        level_crossings: vec![],
        conflicts: conflicts.into_iter().map(RouteId::new).collect(),
        length: railit_types::Length::from_mm(500_000),
        overlap_length: railit_types::Length::from_mm(0),
    }
}

fn signal(name: &str, kind: SignalKind) -> SignalDef {
    SignalDef {
        name: name.into(),
        kind,
        system: SignalSystem::Ks,
        announces: None,
    }
}

/// Plan für Beispielstadt mit drei Fahrstraßen: A-N1 (mit Durchrutschweg, Flankenschutz und
/// BÜ), A-N2 (feindlich über W1, 40 km/h) und F-P1 (feindlich mit A-N1 über T1).
fn sample() -> Plan {
    let tvd = TvdId::new;
    let mut a_n1 = route(
        "A-N1",
        0,
        RouteEnd::Signal(SignalId::new(1)),
        vec![(PointId::new(0), PointPosition::Left)],
        vec![1, 2],
        vec![1, 2],
    );
    a_n1.overlap_tvds = vec![tvd(4)];
    a_n1.overlap_points = vec![(PointId::new(1), PointPosition::Left)];
    a_n1.flank = vec![
        Flank {
            protection: Protection::Signal(SignalId::new(5)),
            protects: tvd(1),
        },
        Flank {
            protection: Protection::Signal(SignalId::new(2)),
            protects: tvd(4),
        },
    ];
    a_n1.level_crossings = vec![LxId::new(0)];
    let mut a_n2 = route(
        "A-N2",
        0,
        RouteEnd::Signal(SignalId::new(2)),
        vec![(PointId::new(0), PointPosition::Right)],
        vec![1, 3],
        vec![0],
    );
    a_n2.speed = Some(Speed::from_kmh(40));
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
        signals: vec![
            signal("A", SignalKind::Combined),
            signal("N1", SignalKind::Main),
            signal("N2", SignalKind::Main),
            signal("F", SignalKind::Main),
            signal("P1", SignalKind::Main),
            signal("P2", SignalKind::Main),
            SignalDef {
                name: "a".into(),
                kind: SignalKind::Distant,
                system: SignalSystem::Hv,
                announces: Some(SignalId::new(0)),
            },
        ],
        tvds: ["TW", "TW1", "T1", "T2", "TW2", "TE"]
            .iter()
            .map(|n| TvdDef {
                name: (*n).into(),
                entry: *n == "TW" || *n == "TE",
            })
            .collect(),
        level_crossings: vec![LxDef {
            name: "BÜ1".into(),
            tvd: tvd(4),
        }],
        routes: vec![
            a_n1,
            a_n2,
            route(
                "F-P1",
                3,
                RouteEnd::Signal(SignalId::new(4)),
                vec![(PointId::new(1), PointPosition::Left)],
                vec![4, 2],
                vec![0],
            ),
        ],
        borders: vec![BorderDef {
            name: ">Nachbar".into(),
            initial: true,
            entry_signal: SignalId::new(3),
            approach_tvds: vec![tvd(5), tvd(4)],
            approach_points: vec![(PointId::new(1), PointPosition::Left)],
            approach_length: railit_types::Length::from_mm(800_000),
            entry_overlap_length: railit_types::Length::from_mm(120_000),
        }],
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
    let border = view.find_border(">Nachbar").unwrap();
    assert!(view.border_initial(border));
    assert_eq!(view.border_entry_signal(border), Some(SignalId::new(3)));
    assert_eq!(view.border_approach_tvds(border).count(), 2);
    assert_eq!(
        view.border_approach_points(border).collect::<Vec<_>>(),
        [(PointId::new(1), PointPosition::Left)]
    );
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
            what: "doppelte oder ungültige Weiche",
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
    p.routes[0].start = SignalId::new(9);
    assert!(matches!(
        p.to_image(),
        Err(PlanError::InvalidReference {
            what: "Startsignal",
            index: 0
        })
    ));
}

/// REQ-PLN-002: Durchrutschweg, Flankenschutz, Bahnübergänge und Signaltabelle.
#[test]
fn inconsistent_phase2_data_is_rejected() {
    let reject = |f: fn(&mut Plan), what: &str| {
        let mut p = sample();
        f(&mut p);
        assert!(p.to_image().is_err(), "nicht abgewiesen: {what}");
    };
    reject(
        |p| p.routes[0].overlap_tvds.push(TvdId::new(2)),
        "Durchrutschweg überschneidet Fahrweg",
    );
    reject(
        |p| {
            p.routes[0].conflicts.retain(|r| *r != RouteId::new(1));
            p.routes[1].conflicts.retain(|r| *r != RouteId::new(0));
        },
        "Fahrstraßen ab demselben Signal nicht feindlich",
    );
    reject(
        |p| {
            p.routes[0]
                .overlap_points
                .push((PointId::new(0), PointPosition::Left));
        },
        "Weiche doppelt",
    );
    reject(
        |p| p.routes[0].overlap_points = vec![(PointId::new(0), PointPosition::Left)],
        "Weiche außerhalb",
    );
    reject(
        |p| p.routes[0].flank[0].protects = TvdId::new(5),
        "Flankenschutz für fremden Abschnitt",
    );
    reject(
        |p| p.routes[0].flank[0].protection = Protection::Signal(SignalId::new(0)),
        "Startsignal als Schutzsignal",
    );
    reject(
        |p| {
            p.routes[0].flank[0].protection =
                Protection::Point(PointId::new(0), PointPosition::Right);
        },
        "Fahrwegweiche als Schutzweiche",
    );
    reject(
        |p| p.routes[0].level_crossings.push(LxId::new(0)),
        "Bahnübergang doppelt",
    );
    reject(
        |p| p.routes[1].level_crossings.push(LxId::new(0)),
        "Bahnübergang außerhalb",
    );
    reject(
        |p| p.signals[1].announces = Some(SignalId::new(0)),
        "Hauptsignal kündigt an",
    );
    reject(
        |p| p.signals[6].announces = Some(SignalId::new(6)),
        "Vorsignal kündigt Vorsignal an",
    );
    reject(
        |p| p.routes[0].start = SignalId::new(6),
        "Vorsignal als Startsignal",
    );
    reject(
        |p| p.routes[0].end = RouteEnd::Boundary,
        "Durchrutschweg ohne Zielsignal",
    );
    reject(
        |p| p.level_crossings[0].tvd = TvdId::new(9),
        "BÜ-Abschnitt ungültig",
    );
}

#[test]
fn inconsistent_border_data_is_rejected() {
    let reject = |f: fn(&mut Plan), what: &str| {
        let mut p = sample();
        f(&mut p);
        assert!(p.to_image().is_err(), "nicht abgewiesen: {what}");
    };
    reject(
        |p| p.borders[0].entry_signal = SignalId::new(6),
        "Vorsignal als Einfahrsignal",
    );
    reject(
        |p| p.borders[0].approach_tvds.clear(),
        "leerer Annäherungsabschnitt",
    );
    reject(
        |p| p.borders[0].approach_tvds.push(TvdId::new(5)),
        "Abschnitt doppelt",
    );
    reject(
        |p| p.borders[0].approach_points = vec![(PointId::new(0), PointPosition::Left)],
        "Weiche außerhalb des Annäherungsabschnitts",
    );
    reject(
        |p| {
            p.routes[2].end = RouteEnd::Border(BorderId::new(1));
            p.routes[2].overlap_tvds.clear();
        },
        "unbekannte Grenze",
    );
    let mut ok = sample();
    ok.routes[2].end = RouteEnd::Border(BorderId::new(0));
    let image = ok.to_image().unwrap();
    let view = PlanView::parse(&image).unwrap();
    assert_eq!(
        view.route(RouteId::new(2)).unwrap().end(),
        RouteEnd::Border(BorderId::new(0))
    );
}

#[test]
fn many_routes_span_multiple_conflict_words() {
    // 130 Fahrstraßen ⇒ drei u64-Wörter je Zeile; Konflikte quer über Wortgrenzen.
    let mut p = sample();
    p.routes.clear();
    p.level_crossings.clear();
    // Fahrstraßen ab demselben Signal wären feindlich: je Fahrstraße ein eigenes Startsignal.
    p.signals.truncate(6);
    for i in 6..130 {
        p.signals.push(signal(&format!("S{i}"), SignalKind::Main));
    }
    for i in 0..130u32 {
        p.routes.push(RouteDef {
            name: format!("R{i}"),
            start: SignalId::new(i),
            end: RouteEnd::Boundary,
            points: vec![],
            tvds: vec![TvdId::new(0)],
            conflicts: vec![],
            ..route("", 0, RouteEnd::Boundary, vec![], vec![], vec![])
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
