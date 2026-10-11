//! Unabhängige Gegenprüfung eines Plans (REQ-PLN-013).
//!
//! Der Generator sucht Fahrwege per Tiefensuche über [`Topology::exits`]. Die Gegenprüfung
//! nutzt davon bewusst nichts, sondern:
//!
//! - eigene Durchfahrregeln für Gleise und Weichen,
//! - **deterministische Läufe**: Jeder Lauf folgt einer festen Folge von Weichenlagen; alle
//!   Kombinationen werden wie bei einem Zählwerk („Odometer“) aufgezählt,
//! - Mengen (`BTreeSet`) statt Bitsets für die Feindlichkeiten.
//!
//! Erst wenn beide Verfahren exakt dieselben Fahrstraßen und Feindlichkeiten liefern, gilt
//! der Plan als geprüft.

use std::collections::{BTreeMap, BTreeSet};

use railit_plan::{Plan, RouteEnd};
use railit_topology::{
    Direction, NodeRef, Port, PortRef, SignalKind, TerminalKind, Topology, TvdMember,
};
use railit_types::{PointId, PointPosition, SignalId, TvdId};

use crate::error::{PlanningError, PlanningReport};

/// Vergleichbare Beschreibung einer Fahrstraße.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    start: SignalId,
    end: EndKey,
    points: BTreeSet<(PointId, PointPosition)>,
    tvds: Vec<TvdId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum EndKey {
    Signal(SignalId),
    Boundary,
    BufferStop,
}

impl From<RouteEnd> for EndKey {
    fn from(end: RouteEnd) -> Self {
        match end {
            RouteEnd::Signal(s) => Self::Signal(s),
            RouteEnd::Boundary => Self::Boundary,
            RouteEnd::BufferStop => Self::BufferStop,
        }
    }
}

const fn is_route_signal(kind: SignalKind) -> bool {
    match kind {
        SignalKind::Main | SignalKind::Combined | SignalKind::EtcsStopMarker => true,
        SignalKind::Distant | SignalKind::Shunting => false,
    }
}

/// Zielsignal auf Gleis `track`, wenn es in Fahrtrichtung `direction` befahren wird.
fn end_signal(net: &Topology, node: NodeRef, entered_at: Port) -> Option<SignalId> {
    let NodeRef::Track(track) = node else {
        return None;
    };
    let length = net.track(track)?.length;
    net.signals().find_map(|(id, s)| {
        let matches = s.track == track
            && is_route_signal(s.kind)
            && match (entered_at, s.direction) {
                (Port::A, Direction::AToB) => s.position == length,
                (Port::B, Direction::BToA) => s.position.as_mm() == 0,
                _ => false,
            };
        matches.then_some(id)
    })
}

fn tvd_of(net: &Topology, node: NodeRef) -> Option<TvdId> {
    match node {
        NodeRef::Track(t) => net.tvd_of(TvdMember::Track(t)),
        NodeRef::Point(p) => net.tvd_of(TvdMember::Point(p)),
        NodeRef::Terminal(_) => None,
    }
}

/// Ein deterministischer Lauf ab `start` mit den Weichenlagen `choices` (fehlende = links).
/// Gibt den Fahrweg und die Anzahl der verbrauchten Entscheidungen zurück.
fn walk(
    net: &Topology,
    start: SignalId,
    choices: &[PointPosition],
) -> (Result<Key, String>, usize) {
    let mut used = 0usize;
    let Some(signal) = net.signal(start) else {
        return (Err(format!("Signal {start} fehlt")), used);
    };
    let mut port = PortRef::new(
        NodeRef::Track(signal.track),
        if signal.direction == Direction::AToB {
            Port::B
        } else {
            Port::A
        },
    );
    let mut points = BTreeSet::new();
    let mut tvds: Vec<TvdId> = Vec::new();
    let limit = net.tracks().len() + net.points().len() + net.terminals().len() + 1;

    for _ in 0..=limit {
        let Some(entry) = net.linked(port) else {
            return (Err(format!("offener Port hinter „{}“", signal.name)), used);
        };
        if let Some(tvd) = tvd_of(net, entry.node)
            && tvds.last() != Some(&tvd)
        {
            tvds.push(tvd);
        }
        let end = |end| Key {
            start,
            end,
            points: points.clone(),
            tvds: tvds.clone(),
        };
        match (entry.node, entry.port) {
            (NodeRef::Terminal(t), _) => {
                let kind = match net.terminal(t).map(|t| t.kind) {
                    Some(TerminalKind::Boundary) => EndKey::Boundary,
                    _ => EndKey::BufferStop,
                };
                return (Ok(end(kind)), used);
            }
            (NodeRef::Track(_), entered) => {
                if let Some(s) = end_signal(net, entry.node, entered) {
                    return (Ok(end(EndKey::Signal(s))), used);
                }
                let leave = if entered == Port::A { Port::B } else { Port::A };
                port = PortRef::new(entry.node, leave);
            }
            (NodeRef::Point(p), Port::Tip) => {
                let pos = choices.get(used).copied().unwrap_or(PointPosition::Left);
                used += 1;
                points.insert((p, pos));
                let leave = if pos == PointPosition::Left {
                    Port::Left
                } else {
                    Port::Right
                };
                port = PortRef::new(entry.node, leave);
            }
            (NodeRef::Point(p), Port::Left) => {
                points.insert((p, PointPosition::Left));
                port = PortRef::new(entry.node, Port::Tip);
            }
            (NodeRef::Point(p), Port::Right) => {
                points.insert((p, PointPosition::Right));
                port = PortRef::new(entry.node, Port::Tip);
            }
            (NodeRef::Point(_), other) => {
                return (Err(format!("Weiche über Port „{other}“ befahren")), used);
            }
        }
    }
    (
        Err(format!("Weg ab „{}“ endet nicht (Schleife)", signal.name)),
        used,
    )
}

/// Alle Fahrwege ab `start`: Aufzählung der Weichenlagen wie ein Zählwerk.
fn enumerate(net: &Topology, start: SignalId, out: &mut Vec<Key>, problems: &mut Vec<String>) {
    let mut choices: Vec<PointPosition> = Vec::new();
    loop {
        let (result, used) = walk(net, start, &choices);
        match result {
            Ok(key) => out.push(key),
            Err(msg) => problems.push(msg),
        }
        choices.resize(used, PointPosition::Left);
        // Nächste Kombination: letzte „links“-Entscheidung auf „rechts“, Rest verwerfen.
        loop {
            match choices.pop() {
                Some(PointPosition::Left) => {
                    choices.push(PointPosition::Right);
                    break;
                }
                Some(PointPosition::Right) => {}
                None => return,
            }
        }
    }
}

fn check_elements(net: &Topology, plan: &Plan, problems: &mut Vec<String>) {
    if plan.network != net.name() {
        problems.push("Netzname weicht ab".into());
    }
    let point_names: Vec<_> = net.points().map(|(_, p)| p.name.as_str()).collect();
    let plan_points: Vec<_> = plan.points.iter().map(|p| p.name.as_str()).collect();
    if point_names != plan_points {
        problems.push("Weichen weichen vom Netz ab".into());
    }
    for ((id, p), def) in net.points().zip(&plan.points) {
        if net.tvd_of(TvdMember::Point(id)) != Some(def.tvd) {
            problems.push(format!(
                "Gleisfreimeldeabschnitt der Weiche „{}“ weicht ab",
                p.name
            ));
        }
    }
    let signals: Vec<_> = net.signals().map(|(_, s)| s.name.as_str()).collect();
    if signals
        != plan
            .signals
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>()
    {
        problems.push("Signale weichen vom Netz ab".into());
    }
    let tvds: Vec<_> = net.tvds().map(|(_, t)| t.name.as_str()).collect();
    if tvds
        != plan
            .tvds
            .iter()
            .map(|t| t.name.as_str())
            .collect::<Vec<_>>()
    {
        problems.push("Gleisfreimeldeabschnitte weichen vom Netz ab".into());
    }
    let mut names = BTreeSet::new();
    for r in &plan.routes {
        if !names.insert(r.name.as_str()) {
            problems.push(format!("Fahrstraßenname „{}“ ist nicht eindeutig", r.name));
        }
    }
}

fn check_routes(net: &Topology, plan: &Plan, problems: &mut Vec<String>) {
    let mut expected = Vec::new();
    for (id, s) in net.signals() {
        if is_route_signal(s.kind) {
            enumerate(net, id, &mut expected, problems);
        }
    }
    let mut expected_count: BTreeMap<Key, usize> = BTreeMap::new();
    for k in expected {
        *expected_count.entry(k).or_default() += 1;
    }
    let mut actual_count: BTreeMap<Key, (usize, &str)> = BTreeMap::new();
    for r in &plan.routes {
        let key = Key {
            start: r.start,
            end: r.end.into(),
            points: r.points.iter().copied().collect(),
            tvds: r.tvds.clone(),
        };
        if key.points.len() != r.points.len() {
            problems.push(format!(
                "Fahrstraße „{}“ nennt eine Weiche mehrfach",
                r.name
            ));
        }
        let entry = actual_count.entry(key).or_insert((0, r.name.as_str()));
        entry.0 += 1;
    }
    for (key, (count, name)) in &actual_count {
        match expected_count.get(key) {
            None => problems.push(format!("Fahrstraße „{name}“ ist im Netz nicht vorhanden")),
            Some(e) if e != count => {
                problems.push(format!(
                    "Fahrstraße „{name}“ ist {count}× statt {e}× enthalten"
                ));
            }
            Some(_) => {}
        }
    }
    for key in expected_count.keys() {
        if !actual_count.contains_key(key) {
            let start = net.signal(key.start).map_or("?", |s| s.name.as_str());
            problems.push(format!("Fahrstraße ab „{start}“ fehlt im Plan ({key:?})"));
        }
    }
}

fn check_conflicts(plan: &Plan, problems: &mut Vec<String>) {
    let sets: Vec<(BTreeSet<TvdId>, BTreeSet<PointId>)> = plan
        .routes
        .iter()
        .map(|r| {
            (
                r.tvds.iter().copied().collect(),
                r.points.iter().map(|(p, _)| *p).collect(),
            )
        })
        .collect();
    for (a, ra) in plan.routes.iter().enumerate() {
        let declared: BTreeSet<usize> = ra.conflicts.iter().filter_map(|c| c.index()).collect();
        for (b, rb) in plan.routes.iter().enumerate() {
            if a == b {
                continue;
            }
            let expected = ra.start == rb.start
                || !sets[a].0.is_disjoint(&sets[b].0)
                || !sets[a].1.is_disjoint(&sets[b].1);
            if expected != declared.contains(&b) {
                problems.push(format!(
                    "Feindlichkeit „{}“ ↔ „{}“: erwartet {expected}, im Plan {}",
                    ra.name, rb.name, !expected
                ));
            }
        }
    }
}

/// Prüft `plan` unabhängig vom Generator gegen `net`.
///
/// # Errors
/// [`PlanningReport`] mit einem [`PlanningError::Verification`] je Abweichung.
pub fn verify(net: &Topology, plan: &Plan) -> Result<(), PlanningReport> {
    let mut problems = Vec::new();
    check_elements(net, plan, &mut problems);
    check_routes(net, plan, &mut problems);
    check_conflicts(plan, &mut problems);
    if problems.is_empty() {
        Ok(())
    } else {
        Err(PlanningReport {
            errors: problems
                .into_iter()
                .map(PlanningError::Verification)
                .collect(),
        })
    }
}
