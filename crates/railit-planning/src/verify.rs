//! Unabhängige Gegenprüfung eines Plans (REQ-PLN-013).
//!
//! Der Generator sucht Fahrwege per Tiefensuche über [`Topology::exits`]. Die Gegenprüfung
//! nutzt davon bewusst nichts, sondern:
//!
//! - eigene Durchfahrregeln für Gleise und Weichen,
//! - **deterministische Läufe**: Jeder Lauf folgt einer festen Folge von Weichenlagen; alle
//!   Kombinationen werden wie bei einem Zählwerk („Odometer“) aufgezählt,
//! - eigene Herleitung von Durchrutschweg, Flankenschutz, Bahnübergängen, Geschwindigkeit
//!   und Vorsignalen,
//! - Mengen (`BTreeSet`) statt Bitsets für die Feindlichkeiten.
//!
//! Erst wenn beide Verfahren exakt dasselbe liefern, gilt der Plan als geprüft.

use std::collections::{BTreeMap, BTreeSet};

use railit_plan::{Plan, Protection, RouteDef, RouteEnd};
use railit_topology::{
    Direction, NodeRef, Port, PortRef, SignalKind, TerminalKind, Topology, TvdMember,
};
use railit_types::{
    BorderId, LxId, PointId, PointPosition, SignalId, Speed, TerminalId, TrackId, TvdId,
};

use crate::PlanningConfig;
use crate::error::{PlanningError, PlanningReport};

/// Vergleichbare Beschreibung eines Fahrwegs.
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
    Border(BorderId),
}

impl From<RouteEnd> for EndKey {
    fn from(end: RouteEnd) -> Self {
        match end {
            RouteEnd::Signal(s) => Self::Signal(s),
            RouteEnd::Boundary => Self::Boundary,
            RouteEnd::BufferStop => Self::BufferStop,
            RouteEnd::Border(b) => Self::Border(b),
        }
    }
}

const fn is_route_signal(kind: SignalKind) -> bool {
    match kind {
        SignalKind::Main | SignalKind::Combined | SignalKind::EtcsStopMarker => true,
        SignalKind::Distant | SignalKind::Shunting => false,
    }
}

const fn leave(entered: Port) -> Port {
    if matches!(entered, Port::A) {
        Port::B
    } else {
        Port::A
    }
}

const fn forward(dir: Direction) -> Port {
    if matches!(dir, Direction::AToB) {
        Port::B
    } else {
        Port::A
    }
}

fn tvd_of(net: &Topology, node: NodeRef) -> Option<TvdId> {
    match node {
        NodeRef::Track(t) => net.tvd_of(TvdMember::Track(t)),
        NodeRef::Point(p) => net.tvd_of(TvdMember::Point(p)),
        NodeRef::Terminal(_) => None,
    }
}

/// Nummer einer Grenze: Anzahl der Grenz-Netzenden vor ihr.
fn border_number(net: &Topology, terminal: TerminalId) -> Option<BorderId> {
    let before = net
        .terminals()
        .take_while(|(id, _)| *id != terminal)
        .filter(|(_, t)| matches!(t.kind, TerminalKind::Border { .. }))
        .count();
    BorderId::from_index(before)
}

/// Hauptsignal auf `track` am Ende in Fahrtrichtung, wenn über `entered_at` eingefahren.
fn end_signal(net: &Topology, track: TrackId, entered_at: Port) -> Option<SignalId> {
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

/// Ergebnis eines Laufs: Fahrweg und befahrene Gleise (ohne das Startgleis).
struct Walked {
    key: Key,
    tracks: Vec<TrackId>,
}

/// Ein deterministischer Lauf ab `start` mit den Weichenlagen `choices` (fehlende = links).
fn walk(
    net: &Topology,
    start: SignalId,
    choices: &[PointPosition],
) -> (Result<Walked, String>, usize) {
    let mut used = 0usize;
    let Some(signal) = net.signal(start) else {
        return (Err(format!("Signal {start} fehlt")), used);
    };
    let mut port = PortRef::new(NodeRef::Track(signal.track), forward(signal.direction));
    let mut points = BTreeSet::new();
    let mut tvds: Vec<TvdId> = Vec::new();
    let mut tracks = Vec::new();
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
        let done = |end, points: &BTreeSet<_>, tvds: &Vec<_>, tracks: &Vec<_>| Walked {
            key: Key {
                start,
                end,
                points: points.clone(),
                tvds: tvds.clone(),
            },
            tracks: tracks.clone(),
        };
        match (entry.node, entry.port) {
            (NodeRef::Terminal(t), _) => {
                let kind = match net.terminal(t).map(|t| t.kind) {
                    Some(TerminalKind::Boundary) => EndKey::Boundary,
                    Some(TerminalKind::Border { .. }) => match border_number(net, t) {
                        Some(b) => EndKey::Border(b),
                        None => return (Err("Grenze ohne Nummer".into()), used),
                    },
                    _ => EndKey::BufferStop,
                };
                return (Ok(done(kind, &points, &tvds, &tracks)), used);
            }
            (NodeRef::Track(t), entered) => {
                tracks.push(t);
                if let Some(s) = end_signal(net, t, entered) {
                    return (Ok(done(EndKey::Signal(s), &points, &tvds, &tracks)), used);
                }
                port = PortRef::new(entry.node, leave(entered));
            }
            (NodeRef::Point(p), Port::Tip) => {
                let pos = choices.get(used).copied().unwrap_or(PointPosition::Left);
                used += 1;
                points.insert((p, pos));
                let out = if pos == PointPosition::Left {
                    Port::Left
                } else {
                    Port::Right
                };
                port = PortRef::new(entry.node, out);
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
fn enumerate(net: &Topology, start: SignalId, out: &mut Vec<Walked>, problems: &mut Vec<String>) {
    let mut choices: Vec<PointPosition> = Vec::new();
    loop {
        let (result, used) = walk(net, start, &choices);
        match result {
            Ok(w) => out.push(w),
            Err(msg) => problems.push(msg),
        }
        choices.resize(used, PointPosition::Left);
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

/// Erwarteter Durchrutschweg: Abschnitte, Weichenlagen und Gleise mit Abstand ab Signal.
struct Overlap {
    tvds: Vec<TvdId>,
    points: BTreeSet<(PointId, PointPosition)>,
    tracks: Vec<(TrackId, u64, bool)>,
}

fn expected_overlap(net: &Topology, end: SignalId, length_mm: u64) -> Result<Overlap, String> {
    let s = net.signal(end).ok_or("Zielsignal fehlt")?;
    let mut port = PortRef::new(NodeRef::Track(s.track), forward(s.direction));
    let mut result = Overlap {
        tvds: Vec::new(),
        points: BTreeSet::new(),
        tracks: Vec::new(),
    };
    let mut distance = 0u64;
    let mut steps = 0usize;
    while distance < length_mm {
        steps += 1;
        if steps > net.tracks().len() + net.points().len() + 2 {
            return Err("Durchrutschweg endet nicht".into());
        }
        let entry = net.linked(port).ok_or("offener Port")?;
        if let Some(t) = tvd_of(net, entry.node)
            && result.tvds.last() != Some(&t)
        {
            result.tvds.push(t);
        }
        match (entry.node, entry.port) {
            (NodeRef::Terminal(t), _) => {
                if matches!(
                    net.terminal(t).map(|x| x.kind),
                    Some(TerminalKind::Border { .. })
                ) {
                    return Err("Durchrutschweg erreicht die Bereichsgrenze".into());
                }
                break;
            }
            (NodeRef::Track(t), entered) => {
                result.tracks.push((t, distance, entered == Port::A));
                distance += u64::from(net.track(t).ok_or("Gleis fehlt")?.length.as_mm());
                port = PortRef::new(entry.node, leave(entered));
            }
            (NodeRef::Point(p), Port::Tip) => {
                // Gerader Strang = nicht abzweigend; ohne Angabe links.
                let pos = match net.point(p).and_then(|x| x.diverging) {
                    Some(PointPosition::Left) => PointPosition::Right,
                    Some(PointPosition::Right) | None => PointPosition::Left,
                };
                result.points.insert((p, pos));
                let out = if pos == PointPosition::Left {
                    Port::Left
                } else {
                    Port::Right
                };
                port = PortRef::new(entry.node, out);
            }
            (NodeRef::Point(p), entered) => {
                let pos = if entered == Port::Left {
                    PointPosition::Left
                } else {
                    PointPosition::Right
                };
                result.points.insert((p, pos));
                port = PortRef::new(entry.node, Port::Tip);
            }
        }
    }
    Ok(result)
}

/// Erwarteter Flankenschutz einer Weiche (eigene Implementierung).
fn expected_flank(
    net: &Topology,
    point: PointId,
    pos: PointPosition,
) -> Result<Vec<Protection>, String> {
    let away = if pos == PointPosition::Left {
        Port::Right
    } else {
        Port::Left
    };
    let mut found = Vec::new();
    let mut todo = vec![(PortRef::new(NodeRef::Point(point), away), 0usize)];
    let mut seen = BTreeSet::from([NodeRef::Point(point)]);
    while let Some((start, depth)) = todo.pop() {
        if depth > 16 {
            return Err("Suchtiefe überschritten".into());
        }
        let mut port = start;
        loop {
            let entry = net.linked(port).ok_or("offener Port")?;
            if !seen.insert(entry.node) {
                return Err("Schleife".into());
            }
            match (entry.node, entry.port) {
                (NodeRef::Terminal(t), _) => {
                    if net.terminal(t).map(|x| x.kind) != Some(TerminalKind::BufferStop) {
                        return Err("ungeschützt an der Netzgrenze".into());
                    }
                    break;
                }
                (NodeRef::Track(t), entered) => {
                    // Zur Weiche zeigt ein Signal, dessen Richtung dem Weg entgegenläuft.
                    let toward = if entered == Port::A {
                        Direction::BToA
                    } else {
                        Direction::AToB
                    };
                    let mut candidates: Vec<(u32, SignalId)> = net
                        .signals()
                        .filter(|(_, s)| {
                            s.track == t
                                && s.direction == toward
                                && (is_route_signal(s.kind) || s.kind == SignalKind::Shunting)
                        })
                        .map(|(id, s)| {
                            let from_entry = if entered == Port::A {
                                s.position.as_mm()
                            } else {
                                net.track(t).map_or(0, |x| x.length.as_mm()) - s.position.as_mm()
                            };
                            (from_entry, id)
                        })
                        .collect();
                    candidates.sort_unstable();
                    if let Some((_, s)) = candidates.first() {
                        found.push(Protection::Signal(*s));
                        break;
                    }
                    port = PortRef::new(entry.node, leave(entered));
                }
                (NodeRef::Point(_), Port::Tip) => {
                    // Reihenfolge wie der Generator: erst links, dann rechts.
                    todo.push((PortRef::new(entry.node, Port::Right), depth + 1));
                    todo.push((PortRef::new(entry.node, Port::Left), depth + 1));
                    break;
                }
                (NodeRef::Point(q), entered) => {
                    let protect = if entered == Port::Left {
                        PointPosition::Right
                    } else {
                        PointPosition::Left
                    };
                    found.push(Protection::Point(q, protect));
                    break;
                }
            }
        }
    }
    Ok(found)
}

/// Erwartetes Vorsignal → Hauptsignal (eigene Implementierung).
fn expected_announces(net: &Topology) -> BTreeMap<SignalId, SignalId> {
    let mut out = BTreeMap::new();
    for (id, d) in net.signals().filter(|(_, s)| s.kind == SignalKind::Distant) {
        let mut track = d.track;
        let mut dir = d.direction;
        let mut after = Some(d.position.as_mm());
        for _ in 0..=net.tracks().len() + net.points().len() {
            let mut ahead: Vec<(u32, SignalId)> = net
                .signals()
                .filter(|(_, s)| s.track == track && s.direction == dir && is_route_signal(s.kind))
                .filter_map(|(sid, s)| {
                    let pos = s.position.as_mm();
                    let beyond = match (after, dir) {
                        (None, _) => true,
                        (Some(a), Direction::AToB) => pos > a,
                        (Some(a), Direction::BToA) => pos < a,
                    };
                    let key = if dir == Direction::AToB {
                        pos
                    } else {
                        u32::MAX - pos
                    };
                    beyond.then_some((key, sid))
                })
                .collect();
            ahead.sort_unstable();
            if let Some((_, main)) = ahead.first() {
                out.insert(id, *main);
                break;
            }
            // Weiter über Weichen, die nur stumpf befahren werden.
            let mut port = PortRef::new(NodeRef::Track(track), forward(dir));
            let onward = loop {
                let Some(entry) = net.linked(port) else {
                    break None;
                };
                match (entry.node, entry.port) {
                    (NodeRef::Point(p), Port::Left | Port::Right) => {
                        port = PortRef::new(NodeRef::Point(p), Port::Tip);
                    }
                    (NodeRef::Track(t), entered) => {
                        break Some((
                            t,
                            if entered == Port::A {
                                Direction::AToB
                            } else {
                                Direction::BToA
                            },
                        ));
                    }
                    _ => break None,
                }
            };
            let Some((t, d2)) = onward else { break };
            track = t;
            dir = d2;
            after = None;
        }
    }
    out
}

fn check_elements(net: &Topology, plan: &Plan, problems: &mut Vec<String>) {
    if plan.network != net.name() {
        problems.push("Netzname weicht ab".into());
    }
    let names = |it: Vec<&str>, plan_names: Vec<&str>, what: &str, problems: &mut Vec<String>| {
        if it != plan_names {
            problems.push(format!("{what} weichen vom Netz ab"));
        }
    };
    names(
        net.points().map(|(_, p)| p.name.as_str()).collect(),
        plan.points.iter().map(|p| p.name.as_str()).collect(),
        "Weichen",
        problems,
    );
    names(
        net.signals().map(|(_, s)| s.name.as_str()).collect(),
        plan.signals.iter().map(|s| s.name.as_str()).collect(),
        "Signale",
        problems,
    );
    names(
        net.tvds().map(|(_, t)| t.name.as_str()).collect(),
        plan.tvds.iter().map(|t| t.name.as_str()).collect(),
        "Gleisfreimeldeabschnitte",
        problems,
    );
    names(
        net.level_crossings()
            .map(|(_, l)| l.name.as_str())
            .collect(),
        plan.level_crossings
            .iter()
            .map(|l| l.name.as_str())
            .collect(),
        "Bahnübergänge",
        problems,
    );
    for ((id, p), def) in net.points().zip(&plan.points) {
        if net.tvd_of(TvdMember::Point(id)) != Some(def.tvd) {
            problems.push(format!(
                "Gleisfreimeldeabschnitt der Weiche „{}“ weicht ab",
                p.name
            ));
        }
    }
    for ((_, l), def) in net.level_crossings().zip(&plan.level_crossings) {
        if net.tvd_of(TvdMember::Track(l.track)) != Some(def.tvd) {
            problems.push(format!(
                "Abschnitt des Bahnübergangs „{}“ weicht ab",
                l.name
            ));
        }
    }
    let announces = expected_announces(net);
    for ((id, s), def) in net.signals().zip(&plan.signals) {
        if s.kind != def.kind
            || s.system != def.system
            || announces.get(&id).copied() != def.announces
        {
            problems.push(format!(
                "Signal „{}“: Art, System oder Ankündigung weicht ab",
                s.name
            ));
        }
    }
    let boundary_tvds: BTreeSet<TvdId> = net
        .terminals()
        .filter(|(_, t)| t.kind == TerminalKind::Boundary)
        .filter_map(|(id, _)| net.linked(PortRef::new(NodeRef::Terminal(id), Port::End)))
        .filter_map(|p| tvd_of(net, p.node))
        .collect();
    for ((id, t), def) in net.tvds().zip(&plan.tvds) {
        if boundary_tvds.contains(&id) != def.entry {
            problems.push(format!("Einfahrmerkmal von „{}“ weicht ab", t.name));
        }
    }
    let mut route_names = BTreeSet::new();
    for r in &plan.routes {
        if !route_names.insert(r.name.as_str()) {
            problems.push(format!("Fahrstraßenname „{}“ ist nicht eindeutig", r.name));
        }
    }
}

/// Vergleicht die Zusatzdaten einer Fahrstraße mit der eigenen Herleitung.
fn check_details(
    net: &Topology,
    cfg: PlanningConfig,
    r: &RouteDef,
    tracks: &[TrackId],
    problems: &mut Vec<String>,
) {
    let mut fail = |what: &str| problems.push(format!("Fahrstraße „{}“: {what} weicht ab", r.name));

    let speed: Option<Speed> = r
        .points
        .iter()
        .filter_map(|(p, pos)| {
            let point = net.point(*p)?;
            (point.diverging == Some(*pos)).then_some(point.diverging_speed)
        })
        .min();
    if speed != r.speed {
        fail("Geschwindigkeit");
    }

    let length = u64::from(cfg.overlap_length.as_mm());
    let overlap = if let RouteEnd::Signal(e) = r.end {
        let Ok(o) = expected_overlap(net, e, length) else {
            fail("Durchrutschweg");
            return;
        };
        Some(o)
    } else {
        None
    };
    let (o_tvds, o_points) = overlap.as_ref().map_or((Vec::new(), BTreeSet::new()), |o| {
        (o.tvds.clone(), o.points.clone())
    });
    if r.overlap_tvds != o_tvds
        || r.overlap_points.iter().copied().collect::<BTreeSet<_>>() != o_points
    {
        fail("Durchrutschweg");
    }

    // Längen (Plan v4): Fahrweg als Summe der befahrenen Gleise hinter dem Startgleis,
    // Durchrutschweg bis zum Ende seines letzten Gleises.
    let track_len = |t: &TrackId| u64::from(net.track(*t).map_or(0, |x| x.length.as_mm()));
    let route_length: u64 = tracks.iter().map(track_len).sum();
    if u64::from(r.length.as_mm()) != route_length {
        fail("Länge");
    }
    let overlap_length = overlap
        .as_ref()
        .and_then(|o| o.tracks.last())
        .map_or(0, |(t, start, _)| start + track_len(t));
    if u64::from(r.overlap_length.as_mm()) != overlap_length {
        fail("Länge des Durchrutschwegs");
    }

    // Flankenschutz: je Weiche (Fahrweg, dann Durchrutschweg), Duplikate nur einmal.
    let mut flank: Vec<(Protection, TvdId)> = Vec::new();
    for (p, pos) in r.points.iter().chain(&r.overlap_points) {
        let Some(protects) = net.tvd_of(TvdMember::Point(*p)) else {
            continue;
        };
        match expected_flank(net, *p, *pos) {
            Ok(list) => {
                for protection in list {
                    if !flank.iter().any(|(x, _)| *x == protection) {
                        flank.push((protection, protects));
                    }
                }
            }
            Err(_) => fail("Flankenschutz"),
        }
    }
    let plan_flank: BTreeSet<String> = r
        .flank
        .iter()
        .map(|f| format!("{:?}/{:?}", f.protection, f.protects))
        .collect();
    let want_flank: BTreeSet<String> = flank.iter().map(|(p, t)| format!("{p:?}/{t:?}")).collect();
    if plan_flank != want_flank {
        fail("Flankenschutz");
    }

    let mut lx: BTreeSet<LxId> = BTreeSet::new();
    for (id, l) in net.level_crossings() {
        if tracks.contains(&l.track) {
            lx.insert(id);
        }
        if let Some(o) = &overlap {
            for (t, start, a_to_b) in &o.tracks {
                if *t != l.track {
                    continue;
                }
                let len = net.track(*t).map_or(0, |x| u64::from(x.length.as_mm()));
                let pos = u64::from(l.position.as_mm());
                let along = if *a_to_b { pos } else { len - pos };
                if start + along <= length {
                    lx.insert(id);
                }
            }
        }
    }
    if r.level_crossings.iter().copied().collect::<BTreeSet<_>>() != lx {
        fail("Bahnübergänge");
    }
}

fn check_routes(net: &Topology, cfg: PlanningConfig, plan: &Plan, problems: &mut Vec<String>) {
    let mut walked = Vec::new();
    for (id, s) in net.signals() {
        if is_route_signal(s.kind) {
            enumerate(net, id, &mut walked, problems);
        }
    }
    let mut expected: BTreeMap<Key, (usize, Vec<TrackId>)> = BTreeMap::new();
    for w in walked {
        let entry = expected.entry(w.key).or_insert((0, w.tracks));
        entry.0 += 1;
    }
    let mut actual: BTreeMap<Key, usize> = BTreeMap::new();
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
        match expected.get(&key) {
            None => problems.push(format!(
                "Fahrstraße „{}“ ist im Netz nicht vorhanden",
                r.name
            )),
            Some((_, tracks)) => check_details(net, cfg, r, tracks, problems),
        }
        *actual.entry(key).or_default() += 1;
    }
    for (key, (count, _)) in &expected {
        let got = actual.get(key).copied().unwrap_or(0);
        if got != *count {
            let start = net.signal(key.start).map_or("?", |s| s.name.as_str());
            problems.push(format!(
                "Fahrstraße ab „{start}“: {got}× statt {count}× im Plan ({key:?})"
            ));
        }
    }
}

/// Einfahrsignal, Abschnitte und Weichenlagen eines Annäherungsabschnitts.
/// Einfahrsignal, Abschnitte, Weichenlagen, Länge bis zum Einfahrsignal, Länge seines
/// Durchrutschwegs.
type Approach = (
    SignalId,
    Vec<TvdId>,
    BTreeSet<(PointId, PointPosition)>,
    u64,
    u64,
);

/// Erwarteter Annäherungsabschnitt einer Grenze (eigene Herleitung).
fn expected_approach(
    net: &Topology,
    terminal: TerminalId,
    length_mm: u64,
) -> Result<Approach, String> {
    let mut tvds: Vec<TvdId> = Vec::new();
    let mut points = BTreeSet::new();
    let mut port = PortRef::new(NodeRef::Terminal(terminal), Port::End);
    let mut steps = 0usize;
    let mut distance = 0u64;
    let entry = loop {
        steps += 1;
        if steps > net.tracks().len() + net.points().len() + 2 {
            return Err("Annäherungsabschnitt endet nicht".into());
        }
        let at = net.linked(port).ok_or("offener Port")?;
        if let Some(t) = tvd_of(net, at.node)
            && tvds.last() != Some(&t)
        {
            tvds.push(t);
        }
        match (at.node, at.port) {
            (NodeRef::Track(t), entered) => {
                distance += u64::from(net.track(t).ok_or("Gleis fehlt")?.length.as_mm());
                if let Some(s) = end_signal(net, t, entered) {
                    break s;
                }
                port = PortRef::new(at.node, leave(entered));
            }
            (NodeRef::Point(p), Port::Left) => {
                points.insert((p, PointPosition::Left));
                port = PortRef::new(at.node, Port::Tip);
            }
            (NodeRef::Point(p), Port::Right) => {
                points.insert((p, PointPosition::Right));
                port = PortRef::new(at.node, Port::Tip);
            }
            _ => return Err("kein Einfahrsignal vor Weichenspitze oder Netzende".into()),
        }
    };
    let overlap = expected_overlap(net, entry, length_mm)?;
    let overlap_length = match overlap.tracks.last() {
        Some((t, start, _)) => {
            start + u64::from(net.track(*t).ok_or("Gleis fehlt")?.length.as_mm())
        }
        None => 0,
    };
    for t in overlap.tvds {
        if tvds.contains(&t) {
            return Err("Durchrutschweg führt zur Grenze zurück".into());
        }
        tvds.push(t);
    }
    points.extend(overlap.points);
    Ok((entry, tvds, points, distance, overlap_length))
}

fn check_borders(net: &Topology, cfg: PlanningConfig, plan: &Plan, problems: &mut Vec<String>) {
    let length = u64::from(cfg.overlap_length.as_mm());
    let mut count = 0usize;
    for (id, t) in net.terminals() {
        let TerminalKind::Border { initial } = t.kind else {
            continue;
        };
        let Some(def) = plan.borders.get(count) else {
            problems.push(format!("Grenze „{}“ fehlt im Plan", t.name));
            count += 1;
            continue;
        };
        count += 1;
        if def.name != t.name || def.initial != initial {
            problems.push(format!(
                "Grenze „{}“: Name oder Anlaufmerkmal weicht ab",
                t.name
            ));
        }
        match expected_approach(net, id, length) {
            Ok((entry, tvds, points, distance, overlap_length)) => {
                let plan_points: BTreeSet<_> = def.approach_points.iter().copied().collect();
                if def.entry_signal != entry
                    || def.approach_tvds != tvds
                    || plan_points != points
                    || plan_points.len() != def.approach_points.len()
                    || u64::from(def.approach_length.as_mm()) != distance
                    || u64::from(def.entry_overlap_length.as_mm()) != overlap_length
                {
                    problems.push(format!(
                        "Grenze „{}“: Annäherungsabschnitt weicht ab",
                        t.name
                    ));
                }
            }
            Err(e) => problems.push(format!("Grenze „{}“: {e}", t.name)),
        }
    }
    if plan.borders.len() != count {
        problems.push("Anzahl der Grenzen weicht ab".into());
    }
}

fn check_conflicts(plan: &Plan, problems: &mut Vec<String>) {
    struct Sets {
        path: BTreeSet<TvdId>,
        overlap: BTreeSet<TvdId>,
        path_points: BTreeSet<PointId>,
        positions: BTreeMap<PointId, PointPosition>,
        flank_signals: BTreeSet<SignalId>,
        end: Option<SignalId>,
    }
    let sets: Vec<Sets> = plan
        .routes
        .iter()
        .map(|r| {
            let mut positions: BTreeMap<PointId, PointPosition> =
                r.points.iter().chain(&r.overlap_points).copied().collect();
            let mut flank_signals = BTreeSet::new();
            for f in &r.flank {
                match f.protection {
                    Protection::Point(p, pos) => {
                        positions.insert(p, pos);
                    }
                    Protection::Signal(s) => {
                        flank_signals.insert(s);
                    }
                }
            }
            Sets {
                path: r.tvds.iter().copied().collect(),
                overlap: r.overlap_tvds.iter().copied().collect(),
                path_points: r.points.iter().map(|(p, _)| *p).collect(),
                positions,
                flank_signals,
                end: if let RouteEnd::Signal(s) = r.end {
                    Some(s)
                } else {
                    None
                },
            }
        })
        .collect();
    for (a, ra) in plan.routes.iter().enumerate() {
        let declared: BTreeSet<usize> = ra.conflicts.iter().filter_map(|c| c.index()).collect();
        for (b, rb) in plan.routes.iter().enumerate() {
            if a == b {
                continue;
            }
            let (x, y) = (&sets[a], &sets[b]);
            let overlap_into = |o: &Sets, p: &Sets, continuation: bool| {
                !o.overlap.is_disjoint(&p.path) && !continuation
            };
            let expected = ra.start == rb.start
                || !x.path.is_disjoint(&y.path)
                || !x.path_points.is_disjoint(&y.path_points)
                || overlap_into(x, y, x.end == Some(rb.start))
                || overlap_into(y, x, y.end == Some(ra.start))
                || !x.overlap.is_disjoint(&y.overlap)
                || x.positions
                    .iter()
                    .any(|(p, pos)| y.positions.get(p).is_some_and(|o| o != pos))
                || x.flank_signals.contains(&rb.start)
                || y.flank_signals.contains(&ra.start);
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
pub fn verify_with(
    net: &Topology,
    plan: &Plan,
    cfg: &PlanningConfig,
) -> Result<(), PlanningReport> {
    let mut problems = Vec::new();
    check_elements(net, plan, &mut problems);
    check_routes(net, *cfg, plan, &mut problems);
    check_borders(net, *cfg, plan, &mut problems);
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

/// Prüft `plan` mit der Standardkonfiguration.
///
/// # Errors
/// Wie [`verify_with`].
pub fn verify(net: &Topology, plan: &Plan) -> Result<(), PlanningReport> {
    verify_with(net, plan, &PlanningConfig::default())
}
