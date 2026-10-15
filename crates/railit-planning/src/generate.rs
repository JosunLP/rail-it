//! Fahrstraßengenerator.
//!
//! Tiefensuche über Ports ab jedem Startsignal. Die unabhängige Gegenprüfung in
//! [`crate::verify`] arbeitet bewusst anders (deterministische Läufe je Weichenkombination,
//! eigene Durchfahrregeln, Mengen statt Bitsets).

use std::collections::{BTreeMap, HashMap};

use railit_plan::{
    BorderDef, Flank, LxDef, Plan, PointDef, Protection, RouteDef, RouteEnd, SignalDef, TvdDef,
};
use railit_topology::{
    Direction, NodeRef, Port, PortRef, SignalKind, TerminalKind, Topology, TvdMember,
};
use railit_types::bitset::{BitSet, BitSetMut, words_for};
use railit_types::{
    BorderId, Length, LxId, PointId, PointPosition, RouteId, SignalId, Speed, TerminalId, TrackId,
    TvdId,
};

use crate::PlanningConfig;
use crate::error::{PlanningError, PlanningReport};

/// Port, über den ein Zug ein Gleis in `direction` verlässt.
pub(crate) const fn exit_port(direction: Direction) -> Port {
    match direction {
        Direction::AToB => Port::B,
        Direction::BToA => Port::A,
    }
}

/// Erzeugter, noch unbenannter Fahrweg.
#[derive(Clone, Debug)]
struct Draft {
    start: SignalId,
    end: RouteEnd,
    end_name: String,
    points: Vec<(PointId, PointPosition)>,
    tvds: Vec<TvdId>,
    tracks: Vec<TrackId>,
    overlap_tvds: Vec<TvdId>,
    overlap_points: Vec<(PointId, PointPosition)>,
    flank: Vec<Flank>,
    level_crossings: Vec<LxId>,
    speed: Option<Speed>,
    overlap_length: u64,
}

/// Teilweg während der Suche.
#[derive(Clone, Debug)]
struct Frame {
    exit: PortRef,
    points: Vec<(PointId, PointPosition)>,
    tvds: Vec<TvdId>,
    visited: Vec<NodeRef>,
}

fn member_tvd(net: &Topology, node: NodeRef) -> Option<TvdId> {
    match node {
        NodeRef::Track(t) => net.tvd_of(TvdMember::Track(t)),
        NodeRef::Point(p) => net.tvd_of(TvdMember::Point(p)),
        NodeRef::Terminal(_) => None,
    }
}

fn signal_name(net: &Topology, id: SignalId) -> String {
    net.signal(id).map(|s| s.name.clone()).unwrap_or_default()
}

fn point_name(net: &Topology, id: PointId) -> String {
    net.point(id).map(|p| p.name.clone()).unwrap_or_default()
}

const fn other(pos: PointPosition) -> PointPosition {
    pos.other()
}

const fn branch_port(pos: PointPosition) -> Port {
    match pos {
        PointPosition::Left => Port::Left,
        PointPosition::Right => Port::Right,
    }
}

/// Abstand eines Signals bis zum Ende seines Gleises in Wirkrichtung (mm).
pub(crate) fn signal_gap(net: &Topology, s: SignalId) -> u64 {
    let Some(signal) = net.signal(s) else {
        return 0;
    };
    let len = net.track(signal.track).map_or(0, |t| t.length.as_mm());
    u64::from(match signal.direction {
        Direction::AToB => len.saturating_sub(signal.position.as_mm()),
        Direction::BToA => signal.position.as_mm(),
    })
}

/// Liegt ein Bahnübergang oder eine Kreuzungsstelle zwischen dem Signal und dem Gleisende?
fn gap_blocked(net: &Topology, s: SignalId) -> bool {
    let Some(signal) = net.signal(s) else {
        return true;
    };
    let p = signal.position.as_mm();
    let inside = |x: u32| match signal.direction {
        Direction::AToB => x > p,
        Direction::BToA => x < p,
    };
    net.level_crossings_on(signal.track)
        .any(|(_, l)| inside(l.position.as_mm()))
        || net
            .crossings_on(signal.track)
            .any(|(_, at)| inside(at.as_mm()))
}

/// REQ-PLN-010: Hauptsignale stehen am Gleisende an einer Gleisfreimeldegrenze, höchstens
/// [`PlanningConfig::signal_gap`] davor und ohne Bahnübergang oder Kreuzung dazwischen.
/// Liefert die Zielsignale je (Gleis, Fahrtrichtung).
fn check_signals(
    net: &Topology,
    cfg: PlanningConfig,
    errors: &mut Vec<PlanningError>,
) -> HashMap<(TrackId, Direction), SignalId> {
    let mut ends = HashMap::new();
    for (id, s) in net.signals().filter(|(_, s)| s.kind.delimits_routes()) {
        let Some(track) = net.track(s.track) else {
            errors.push(PlanningError::Inconsistent(format!(
                "Signal „{}“ ohne Gleis",
                s.name
            )));
            continue;
        };
        let at_end =
            signal_gap(net, id) <= u64::from(cfg.signal_gap.as_mm()) && !gap_blocked(net, id);
        if !at_end {
            errors.push(PlanningError::SignalNotAtTrackEnd {
                signal: s.name.clone(),
            });
            continue;
        }
        let exit = PortRef::new(NodeRef::Track(s.track), exit_port(s.direction));
        match net.linked(exit).map(|p| p.node) {
            Some(NodeRef::Terminal(_)) | None => {
                errors.push(PlanningError::SignalFacesTerminal {
                    signal: s.name.clone(),
                });
                continue;
            }
            Some(beyond) => {
                if member_tvd(net, beyond) == net.tvd_of(TvdMember::Track(s.track)) {
                    errors.push(PlanningError::SignalNotAtTvdBoundary {
                        signal: s.name.clone(),
                    });
                    continue;
                }
            }
        }
        if ends.insert((s.track, s.direction), id).is_some() {
            errors.push(PlanningError::DuplicateSignals {
                track: track.name.clone(),
            });
        }
    }
    ends
}

/// REQ-PLN-017: Jedes Vorsignal kündigt genau ein Hauptsignal an, das ohne spitz befahrene
/// Weiche erreicht wird.
fn announced_signals(
    net: &Topology,
    errors: &mut Vec<PlanningError>,
) -> HashMap<SignalId, SignalId> {
    let mut out = HashMap::new();
    for (id, s) in net.signals().filter(|(_, s)| s.kind == SignalKind::Distant) {
        let fail = |reason: &str| PlanningError::DistantSignal {
            signal: s.name.clone(),
            reason: reason.into(),
        };
        // Erst auf dem eigenen Gleis vor dem Vorsignal suchen.
        let own = net
            .signals_on(s.track)
            .filter(|(_, m)| m.direction == s.direction && m.kind.delimits_routes())
            .filter(|(_, m)| match s.direction {
                Direction::AToB => m.position > s.position,
                Direction::BToA => m.position < s.position,
            })
            .min_by_key(|(_, m)| match s.direction {
                Direction::AToB => m.position.as_mm(),
                Direction::BToA => u32::MAX - m.position.as_mm(),
            })
            .map(|(m, _)| m);
        if let Some(main) = own {
            out.insert(id, main);
            continue;
        }
        let mut port = PortRef::new(NodeRef::Track(s.track), exit_port(s.direction));
        let mut found = None;
        for _ in 0..=net.tracks().len() + net.points().len() {
            let Some(entry) = net.linked(port) else { break };
            match (entry.node, entry.port) {
                (NodeRef::Terminal(_), _) => {
                    errors.push(fail("kein Hauptsignal bis zum Netzende"));
                    break;
                }
                (NodeRef::Point(_), Port::Tip) => {
                    errors.push(fail(
                        "Weg zum Hauptsignal führt über eine spitz befahrene Weiche",
                    ));
                    break;
                }
                (NodeRef::Point(p), _) => port = PortRef::new(NodeRef::Point(p), Port::Tip),
                (NodeRef::Track(t), entered) => {
                    let dir = if entered == Port::A {
                        Direction::AToB
                    } else {
                        Direction::BToA
                    };
                    let main = net
                        .signals_on(t)
                        .filter(|(_, m)| m.direction == dir && m.kind.delimits_routes())
                        .min_by_key(|(_, m)| match dir {
                            Direction::AToB => m.position.as_mm(),
                            Direction::BToA => u32::MAX - m.position.as_mm(),
                        })
                        .map(|(m, _)| m);
                    if let Some(main) = main {
                        found = Some(main);
                        break;
                    }
                    port = PortRef::new(entry.node, exit_port(dir));
                }
            }
        }
        if let Some(main) = found {
            out.insert(id, main);
        } else if !errors
            .iter()
            .any(|e| matches!(e, PlanningError::DistantSignal { signal, .. } if signal == &s.name))
        {
            errors.push(fail("kein Hauptsignal gefunden"));
        }
    }
    out
}

/// Grenzen zu Nachbarbereichen, nummeriert in der Reihenfolge ihrer Netzenden.
pub(crate) fn border_ids(net: &Topology) -> HashMap<TerminalId, BorderId> {
    net.terminals()
        .filter(|(_, t)| t.kind.is_border())
        .enumerate()
        .filter_map(|(i, (id, _))| Some((id, BorderId::from_index(i)?)))
        .collect()
}

/// Alle Fahrwege ab Startsignal `start`.
fn explore(
    net: &Topology,
    start: SignalId,
    ends: &HashMap<(TrackId, Direction), SignalId>,
    borders: &HashMap<TerminalId, BorderId>,
    drafts: &mut Vec<Draft>,
    errors: &mut Vec<PlanningError>,
) {
    let Some(signal) = net.signal(start) else {
        return;
    };
    let mut stack = vec![Frame {
        exit: PortRef::new(NodeRef::Track(signal.track), exit_port(signal.direction)),
        points: Vec::new(),
        tvds: Vec::new(),
        visited: vec![NodeRef::Track(signal.track)],
    }];
    let mut loop_reported = false;

    while let Some(mut frame) = stack.pop() {
        let Some(entry) = net.linked(frame.exit) else {
            errors.push(PlanningError::Inconsistent(format!(
                "offener Port hinter Signal „{}“",
                signal.name
            )));
            continue;
        };
        if frame.visited.contains(&entry.node) {
            if !loop_reported {
                errors.push(PlanningError::Loop {
                    signal: signal.name.clone(),
                });
                loop_reported = true;
            }
            continue;
        }
        frame.visited.push(entry.node);

        if let Some(tvd) = member_tvd(net, entry.node)
            && frame.tvds.last() != Some(&tvd)
        {
            if frame.tvds.contains(&tvd) {
                errors.push(PlanningError::RepeatedTvd {
                    route: format!("{}-…", signal.name),
                    tvd: net.tvd(tvd).map(|t| t.name.clone()).unwrap_or_default(),
                });
                continue;
            }
            frame.tvds.push(tvd);
        }

        let tracks = |visited: &[NodeRef]| -> Vec<TrackId> {
            visited
                .iter()
                .skip(1)
                .filter_map(|n| match n {
                    NodeRef::Track(t) => Some(*t),
                    _ => None,
                })
                .collect()
        };
        let finish = |frame: Frame, end: RouteEnd, end_name: String| Draft {
            start,
            end,
            end_name,
            tracks: tracks(&frame.visited),
            points: frame.points,
            tvds: frame.tvds,
            overlap_tvds: Vec::new(),
            overlap_points: Vec::new(),
            flank: Vec::new(),
            level_crossings: Vec::new(),
            speed: None,
            overlap_length: 0,
        };

        match entry.node {
            NodeRef::Terminal(t) => {
                if let Some((end, name)) = terminal_end(net, t, borders)
                    && feasible(net, &frame.points)
                {
                    drafts.push(finish(frame, end, name));
                }
            }
            NodeRef::Track(t) => {
                let direction = if entry.port == Port::A {
                    Direction::AToB
                } else {
                    Direction::BToA
                };
                if let Some(&end) = ends.get(&(t, direction)) {
                    if feasible(net, &frame.points) {
                        drafts.push(finish(frame, RouteEnd::Signal(end), signal_name(net, end)));
                    }
                } else {
                    frame.exit = PortRef::new(entry.node, exit_port(direction));
                    stack.push(frame);
                }
            }
            NodeRef::Point(_) => {
                // Rückwärts einreihen, damit „links“ vor „rechts“ erzeugt wird.
                let exits: Vec<_> = net.exits(entry).collect();
                for exit in exits.into_iter().rev() {
                    let mut branch = frame.clone();
                    branch.exit = exit.port;
                    if let Some(req) = exit.requires {
                        branch.points.push(req);
                    }
                    stack.push(branch);
                }
            }
        }
    }
}

/// REQ-PLN-021: Ein Weg, der gekoppelte Weichen gegen ihre Kopplung verlangt, existiert
/// nicht.
fn feasible(net: &Topology, points: &[(PointId, PointPosition)]) -> bool {
    points.iter().all(|(p, pos)| {
        net.coupled(*p, *pos)
            .is_none_or(|(q, qpos)| points.iter().all(|(x, xpos)| *x != q || *xpos == qpos))
    })
}

/// REQ-PLN-015: Durchrutschweg hinter dem Zielsignal bis zur ersten Gleisfreimeldegrenze
/// nach `length`. Spitz befahrene Weichen werden in den geraden Strang gelegt (bzw. links,
/// wenn kein abzweigender Strang bekannt ist).
#[allow(clippy::type_complexity)]
fn overlap(
    net: &Topology,
    end: SignalId,
    length: Length,
) -> Result<
    (
        Vec<TvdId>,
        Vec<(PointId, PointPosition)>,
        Vec<(TrackId, u32, Direction)>,
    ),
    String,
> {
    let signal = net.signal(end).ok_or("Zielsignal fehlt")?;
    let mut port = PortRef::new(NodeRef::Track(signal.track), exit_port(signal.direction));
    let mut tvds: Vec<TvdId> = Vec::new();
    let mut points = Vec::new();
    // Gleise mit Strecke ab Signal bis zum Gleisanfang und Fahrtrichtung (für Bahnübergänge).
    let mut tracks = Vec::new();
    let mut covered: u64 = 0;
    let need = u64::from(length.as_mm());
    let mut visited = vec![NodeRef::Track(signal.track)];

    while covered < need {
        let Some(entry) = net.linked(port) else {
            return Err("offener Port".into());
        };
        if visited.contains(&entry.node) {
            return Err("Durchrutschweg bildet eine Schleife".into());
        }
        visited.push(entry.node);
        if let Some(t) = member_tvd(net, entry.node)
            && tvds.last() != Some(&t)
        {
            tvds.push(t);
        }
        match (entry.node, entry.port) {
            (NodeRef::Terminal(t), _) => {
                if net.terminal(t).is_some_and(|x| x.kind.is_border()) {
                    return Err("Durchrutschweg reicht über die Grenze zum Nachbarbereich".into());
                }
                break;
            }
            (NodeRef::Track(t), entered) => {
                let dir = if entered == Port::A {
                    Direction::AToB
                } else {
                    Direction::BToA
                };
                tracks.push((t, u32::try_from(covered).unwrap_or(u32::MAX), dir));
                covered += u64::from(net.track(t).map_or(0, |x| x.length.as_mm()));
                port = PortRef::new(entry.node, exit_port(dir));
            }
            (NodeRef::Point(p), Port::Tip) => {
                let pos = net
                    .point(p)
                    .and_then(|x| x.diverging)
                    .map_or(PointPosition::Left, other);
                points.push((p, pos));
                port = PortRef::new(entry.node, branch_port(pos));
            }
            (NodeRef::Point(p), entered) => {
                let pos = if entered == Port::Left {
                    PointPosition::Left
                } else {
                    PointPosition::Right
                };
                points.push((p, pos));
                port = PortRef::new(entry.node, Port::Tip);
            }
        }
    }
    Ok((tvds, points, tracks))
}

/// REQ-PLN-019: Annäherungsabschnitt einer Grenze. Von der Grenze nach innen bis zum ersten
/// Hauptsignal in dieser Richtung (Einfahrsignal), dazu dessen Durchrutschweg. Vor dem
/// Einfahrsignal darf keine Weiche spitz befahren werden.
fn approach(
    net: &Topology,
    terminal: TerminalId,
    ends: &HashMap<(TrackId, Direction), SignalId>,
    length: Length,
) -> Result<BorderDef, String> {
    let t = net.terminal(terminal).ok_or("Netzende fehlt")?;
    let initial = matches!(t.kind, TerminalKind::Border { initial: true });
    let mut port = PortRef::new(NodeRef::Terminal(terminal), Port::End);
    let mut tvds: Vec<TvdId> = Vec::new();
    let mut points = Vec::new();
    let mut entry_signal = None;
    let mut approach_length: u64 = 0;
    for _ in 0..=net.tracks().len() + net.points().len() {
        let entry = net.linked(port).ok_or("offener Port")?;
        if let Some(tvd) = member_tvd(net, entry.node)
            && tvds.last() != Some(&tvd)
        {
            tvds.push(tvd);
        }
        match (entry.node, entry.port) {
            (NodeRef::Terminal(_), _) => {
                return Err("kein Einfahrsignal bis zum nächsten Netzende".into());
            }
            (NodeRef::Point(_), Port::Tip) => {
                return Err("vor dem Einfahrsignal liegt eine spitz befahrene Weiche".into());
            }
            (NodeRef::Point(p), entered) => {
                let pos = if entered == Port::Left {
                    PointPosition::Left
                } else {
                    PointPosition::Right
                };
                points.push((p, pos));
                port = PortRef::new(entry.node, Port::Tip);
            }
            (NodeRef::Track(tr), entered) => {
                let dir = if entered == Port::A {
                    Direction::AToB
                } else {
                    Direction::BToA
                };
                approach_length += u64::from(net.track(tr).map_or(0, |x| x.length.as_mm()));
                if let Some(&s) = ends.get(&(tr, dir)) {
                    entry_signal = Some(s);
                    break;
                }
                port = PortRef::new(entry.node, exit_port(dir));
            }
        }
    }
    let entry_signal = entry_signal.ok_or("kein Einfahrsignal gefunden")?;
    let (o_tvds, o_points, o_tracks) = overlap(net, entry_signal, length)?;
    // Längen ab dem Einfahrsignal, das kurz vor dem Gleisende stehen darf (REQ-PLN-010).
    let gap = signal_gap(net, entry_signal);
    let overlap_length = o_tracks.last().map_or(0, |(t, start, _)| {
        u64::from(*start) + u64::from(net.track(*t).map_or(0, |x| x.length.as_mm())) + gap
    });
    let approach_length = approach_length.saturating_sub(gap);
    let mm = |v: u64| Length::from_mm(u32::try_from(v).unwrap_or(u32::MAX));
    for tvd in o_tvds {
        if tvds.contains(&tvd) {
            return Err("Durchrutschweg des Einfahrsignals führt zurück zur Grenze".into());
        }
        tvds.push(tvd);
    }
    points.extend(o_points);
    if let Some((p, _)) = points.iter().find(|(p, _)| net.coupling_of(*p).is_some()) {
        return Err(format!(
            "gekoppelte Weiche „{}“ im Annäherungsabschnitt",
            point_name(net, *p)
        ));
    }
    Ok(BorderDef {
        name: t.name.clone(),
        initial,
        entry_signal,
        approach_tvds: tvds,
        approach_points: points,
        approach_length: mm(approach_length),
        entry_overlap_length: mm(overlap_length),
    })
}

/// REQ-PLN-016: Flankenschutz für die Weiche `point`, deren Strang `pos` die Fahrstraße
/// benutzt: Suche vom anderen Strang weg nach einem zur Weiche gerichteten Signal (Schutz-
/// signal) oder einer gegen die Weiche umstellbaren Weiche (Schutzweiche).
///
/// REQ-PLN-021: `coupled` enthält die Lagen, die die Fahrstraße über Kopplungen an Weichen
/// außerhalb ihres Fahr- und Durchrutschwegs erzwingt. Liegt eine solche Weiche so, dass
/// sie in den Flankenweg führt (z. B. das zweite Zungenpaar einer Kreuzungsweiche), geht die
/// Suche über ihre Spitze weiter.
fn flank_for(
    net: &Topology,
    point: PointId,
    pos: PointPosition,
    coupled: &BTreeMap<PointId, PointPosition>,
) -> Result<Vec<Protection>, String> {
    let mut out = Vec::new();
    let start = PortRef::new(NodeRef::Point(point), branch_port(other(pos)));
    search_flank(
        net,
        start,
        0,
        &mut vec![NodeRef::Point(point)],
        coupled,
        &mut out,
    )?;
    Ok(out)
}

fn search_flank(
    net: &Topology,
    from: PortRef,
    depth: usize,
    visited: &mut Vec<NodeRef>,
    coupled: &BTreeMap<PointId, PointPosition>,
    out: &mut Vec<Protection>,
) -> Result<(), String> {
    if depth > 16 {
        return Err("Suchtiefe überschritten".into());
    }
    let mut port = from;
    loop {
        let entry = net.linked(port).ok_or("offener Port")?;
        if visited.contains(&entry.node) {
            return Err("Flankenweg bildet eine Schleife".into());
        }
        visited.push(entry.node);
        match (entry.node, entry.port) {
            (NodeRef::Terminal(t), _) => {
                return match net.terminal(t).map(|x| x.kind) {
                    Some(TerminalKind::BufferStop) => Ok(()),
                    _ => Err("Flankenweg führt ungeschützt an die Netzgrenze".into()),
                };
            }
            (NodeRef::Track(t), entered) => {
                // Wir laufen von der Weiche weg; gesucht ist ein Signal, das zu ihr hin zeigt.
                let walk = if entered == Port::A {
                    Direction::AToB
                } else {
                    Direction::BToA
                };
                let toward = if walk == Direction::AToB {
                    Direction::BToA
                } else {
                    Direction::AToB
                };
                let nearest = net
                    .signals_on(t)
                    .filter(|(_, s)| {
                        s.direction == toward
                            && (s.kind.delimits_routes() || s.kind == SignalKind::Shunting)
                    })
                    .min_by_key(|(_, s)| match walk {
                        Direction::AToB => s.position.as_mm(),
                        Direction::BToA => u32::MAX - s.position.as_mm(),
                    })
                    .map(|(id, _)| id);
                if let Some(s) = nearest {
                    out.push(Protection::Signal(s));
                    return Ok(());
                }
                port = PortRef::new(entry.node, exit_port(walk));
            }
            (NodeRef::Point(_), Port::Tip) => {
                // Von beiden Strängen kann ein Fahrzeug über die Spitze zu uns rollen.
                search_flank(
                    net,
                    PortRef::new(entry.node, Port::Left),
                    depth + 1,
                    visited,
                    coupled,
                    out,
                )?;
                return search_flank(
                    net,
                    PortRef::new(entry.node, Port::Right),
                    depth + 1,
                    visited,
                    coupled,
                    out,
                );
            }
            (NodeRef::Point(q), entered) => {
                // Schutzweiche: in den anderen Strang legen, dann führt die Spitze von uns weg.
                let came_from = if entered == Port::Left {
                    PointPosition::Left
                } else {
                    PointPosition::Right
                };
                if coupled.get(&q) == Some(&came_from) {
                    // Durch Kopplung in den Flankenweg gelegt: weiter über die Spitze.
                    port = PortRef::new(entry.node, Port::Tip);
                    continue;
                }
                out.push(Protection::Point(q, other(came_from)));
                return Ok(());
            }
        }
    }
}

fn route_names(net: &Topology, drafts: &[Draft]) -> Vec<String> {
    let base: Vec<String> = drafts
        .iter()
        .map(|d| format!("{}-{}", signal_name(net, d.start), d.end_name))
        .collect();
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for b in &base {
        *counts.entry(b.as_str()).or_default() += 1;
    }
    base.iter()
        .zip(drafts)
        .map(|(b, d)| {
            if counts.get(b.as_str()).copied().unwrap_or(0) <= 1 {
                return b.clone();
            }
            let via: Vec<String> = d
                .points
                .iter()
                .map(|(p, pos)| {
                    let side = if *pos == PointPosition::Left {
                        "L"
                    } else {
                        "R"
                    };
                    format!("{}{side}", point_name(net, *p))
                })
                .collect();
            format!("{b}[{}]", via.join(","))
        })
        .collect()
}

fn bits(n: usize, items: impl IntoIterator<Item = Option<usize>>) -> Vec<u64> {
    let mut words = vec![0u64; words_for(n)];
    let mut set = BitSetMut::new(&mut words);
    for i in items.into_iter().flatten() {
        let _ = set.insert(i);
    }
    words
}

/// Mengen einer Fahrstraße für die Feindlichkeitsprüfung.
struct Sets {
    path: Vec<u64>,
    overlap: Vec<u64>,
    points: Vec<u64>,
    positions: BTreeMap<PointId, PointPosition>,
    flank_signals: Vec<SignalId>,
    end: Option<SignalId>,
}

/// REQ-PLN-012: Feindlichkeiten.
///
/// Zwei Fahrstraßen sind feindlich bei
/// 1. gleichem Startsignal,
/// 2. gemeinsamem Abschnitt im Fahrweg,
/// 3. gemeinsamer Weiche im Fahrweg,
/// 4. Durchrutschweg der einen im Fahrweg der anderen – außer die andere beginnt am
///    Zielsignal der einen (Fortsetzung),
/// 5. gemeinsamem Abschnitt in den Durchrutschwegen,
/// 6. verschiedenen geforderten Lagen derselben Weiche (Fahrweg, Durchrutschweg, Flanke),
/// 7. Start der einen an einem Schutzsignal der anderen.
fn conflicts(net: &Topology, routes: &[RouteDef]) -> Vec<Vec<RouteId>> {
    let tvd_n = net.tvds().len();
    let point_n = net.points().len();
    let sets: Vec<Sets> = routes
        .iter()
        .map(|r| {
            let mut positions = BTreeMap::new();
            for (p, pos) in r.points.iter().chain(&r.overlap_points) {
                positions.insert(*p, *pos);
            }
            let mut flank_signals = Vec::new();
            for f in &r.flank {
                match f.protection {
                    Protection::Point(p, pos) => {
                        positions.insert(p, pos);
                    }
                    Protection::Signal(s) => flank_signals.push(s),
                }
            }
            Sets {
                path: bits(tvd_n, r.tvds.iter().map(|t| t.index())),
                overlap: bits(tvd_n, r.overlap_tvds.iter().map(|t| t.index())),
                points: bits(point_n, r.points.iter().map(|(p, _)| p.index())),
                positions,
                flank_signals,
                end: match r.end {
                    RouteEnd::Signal(s) => Some(s),
                    _ => None,
                },
            }
        })
        .collect();
    let hit = |a: &[u64], b: &[u64]| BitSet::new(a).intersects(&BitSet::new(b));

    routes
        .iter()
        .enumerate()
        .map(|(a, ra)| {
            routes
                .iter()
                .enumerate()
                .filter(|&(b, rb)| {
                    if a == b {
                        return false;
                    }
                    let (sa, sb) = (&sets[a], &sets[b]);
                    ra.start == rb.start
                        || hit(&sa.path, &sb.path)
                        || hit(&sa.points, &sb.points)
                        || (hit(&sa.overlap, &sb.path) && sa.end != Some(rb.start))
                        || (hit(&sb.overlap, &sa.path) && sb.end != Some(ra.start))
                        || hit(&sa.overlap, &sb.overlap)
                        || sa
                            .positions
                            .iter()
                            .any(|(p, pos)| sb.positions.get(p).is_some_and(|o| o != pos))
                        || sa.flank_signals.contains(&rb.start)
                        || sb.flank_signals.contains(&ra.start)
                })
                .filter_map(|(b, _)| RouteId::from_index(b))
                .collect()
        })
        .collect()
}

/// Ergänzt einen Fahrweg um Geschwindigkeit, Durchrutschweg, Flankenschutz und Bahnübergänge.
fn complete(
    net: &Topology,
    cfg: PlanningConfig,
    d: &mut Draft,
    name: &str,
    errors: &mut Vec<PlanningError>,
) {
    // Geschwindigkeit: kleinste Geschwindigkeit der abzweigend befahrenen Weichen.
    d.speed = d
        .points
        .iter()
        .filter_map(|(p, pos)| {
            net.point(*p)
                .filter(|x| x.diverging == Some(*pos))
                .map(|x| x.diverging_speed)
        })
        .min();

    let mut overlap_tracks = Vec::new();
    if let RouteEnd::Signal(end) = d.end {
        match overlap(net, end, cfg.overlap_length) {
            Ok((tvds, points, tracks)) => {
                if let Some(t) = tvds.iter().find(|t| d.tvds.contains(t)) {
                    errors.push(PlanningError::RepeatedTvd {
                        route: name.into(),
                        tvd: net.tvd(*t).map(|x| x.name.clone()).unwrap_or_default(),
                    });
                }
                d.overlap_length = tracks.last().map_or(0, |(t, start, _)| {
                    u64::from(*start) + u64::from(net.track(*t).map_or(0, |x| x.length.as_mm()))
                });
                d.overlap_tvds = tvds;
                d.overlap_points = points;
                overlap_tracks = tracks;
            }
            Err(reason) => errors.push(PlanningError::Inconsistent(format!(
                "Durchrutschweg von „{name}“: {reason}"
            ))),
        }
    }

    // Flankenschutz für alle Weichen in Fahrweg und Durchrutschweg.
    let route_points: Vec<PointId> = d
        .points
        .iter()
        .chain(&d.overlap_points)
        .map(|(p, _)| *p)
        .collect();
    let coupled = couple_route(net, d, name, errors);
    for (p, pos) in d.points.clone().into_iter().chain(d.overlap_points.clone()) {
        let Some(protects) = net.tvd_of(TvdMember::Point(p)) else {
            continue;
        };
        match flank_for(net, p, pos, &coupled) {
            Ok(protections) => {
                for protection in protections {
                    let duplicate = d.flank.iter().any(|f| f.protection == protection);
                    let own_point =
                        matches!(protection, Protection::Point(q, _) if route_points.contains(&q));
                    let own_start = protection == Protection::Signal(d.start);
                    if own_point || own_start {
                        errors.push(PlanningError::UnprotectedFlank {
                            route: name.into(),
                            point: point_name(net, p),
                            reason: "Schutzelement gehört zur Fahrstraße selbst".into(),
                        });
                    } else if !duplicate {
                        d.flank.push(Flank {
                            protection,
                            protects,
                        });
                    }
                }
            }
            Err(reason) => errors.push(PlanningError::UnprotectedFlank {
                route: name.into(),
                point: point_name(net, p),
                reason,
            }),
        }
    }

    couple_flank(net, d, name, errors);

    // Bahnübergänge: alle im Fahrweg, im Durchrutschweg nur innerhalb seiner Länge.
    for t in &d.tracks {
        for (lx, _) in net.level_crossings_on(*t) {
            if !d.level_crossings.contains(&lx) {
                d.level_crossings.push(lx);
            }
        }
    }
    for (t, offset, dir) in overlap_tracks {
        let Some(track) = net.track(t) else { continue };
        for (lx, l) in net.level_crossings_on(t) {
            let along = match dir {
                Direction::AToB => l.position.as_mm(),
                Direction::BToA => track.length.as_mm().saturating_sub(l.position.as_mm()),
            };
            if u64::from(offset) + u64::from(along) <= u64::from(cfg.overlap_length.as_mm())
                && !d.level_crossings.contains(&lx)
            {
                d.level_crossings.push(lx);
            }
        }
    }
}

/// REQ-PLN-021: Partnerweichen der gekoppelten Weichen in Fahr- und Durchrutschweg, in
/// Reihenfolge des Fahrwegs, als Flankenschutz-Einträge. Liefert ihre erzwungenen Lagen.
fn couple_route(
    net: &Topology,
    d: &mut Draft,
    name: &str,
    errors: &mut Vec<PlanningError>,
) -> BTreeMap<PointId, PointPosition> {
    let mut coupled = BTreeMap::new();
    for (p, pos) in d.points.clone().into_iter().chain(d.overlap_points.clone()) {
        let Some((q, qpos)) = net.coupled(p, pos) else {
            continue;
        };
        let required = d
            .points
            .iter()
            .chain(&d.overlap_points)
            .find(|(x, _)| *x == q);
        match required {
            Some((_, have)) if *have != qpos => errors.push(PlanningError::Coupling {
                route: name.into(),
                point: point_name(net, q),
                reason: "Fahr- oder Durchrutschweg verlangt die Weiche gegen ihre Kopplung".into(),
            }),
            Some(_) => {}
            None => {
                coupled.insert(q, qpos);
                if let Some(protects) = net.tvd_of(TvdMember::Point(p)) {
                    let protection = Protection::Point(q, qpos);
                    if !d.flank.iter().any(|f| f.protection == protection) {
                        d.flank.push(Flank {
                            protection,
                            protects,
                        });
                    }
                }
            }
        }
    }
    coupled
}

/// REQ-PLN-021: Schutzweichen, die gekoppelt sind, nehmen ihre Partnerweiche mit. Jede
/// Weiche wird von der Fahrstraße in höchstens einer Lage verlangt.
fn couple_flank(net: &Topology, d: &mut Draft, name: &str, errors: &mut Vec<PlanningError>) {
    let mut i = 0;
    while let Some(f) = d.flank.get(i).copied() {
        i += 1;
        let Protection::Point(p, pos) = f.protection else {
            continue;
        };
        let in_route = |q: PointId| {
            d.points
                .iter()
                .chain(&d.overlap_points)
                .any(|(x, _)| *x == q)
        };
        if let Some((q, qpos)) = net.coupled(p, pos)
            && !in_route(q)
            && !d
                .flank
                .iter()
                .any(|x| x.protection == Protection::Point(q, qpos))
        {
            d.flank.push(Flank {
                protection: Protection::Point(q, qpos),
                protects: f.protects,
            });
        }
    }
    let mut positions: BTreeMap<PointId, PointPosition> = BTreeMap::new();
    let flank_points = d.flank.iter().filter_map(|f| match f.protection {
        Protection::Point(p, pos) => Some((p, pos)),
        Protection::Signal(_) => None,
    });
    for (p, pos) in d
        .points
        .iter()
        .chain(&d.overlap_points)
        .copied()
        .chain(flank_points)
    {
        if positions.insert(p, pos).is_some_and(|before| before != pos) {
            errors.push(PlanningError::Coupling {
                route: name.into(),
                point: point_name(net, p),
                reason: "Flankenschutz und Kopplung verlangen verschiedene Lagen".into(),
            });
        }
    }
}

/// Grenzen mit Annäherungsabschnitt, geordnet nach [`BorderId`].
fn border_defs(
    net: &Topology,
    border_map: &HashMap<TerminalId, BorderId>,
    ends: &HashMap<(TrackId, Direction), SignalId>,
    cfg: PlanningConfig,
    errors: &mut Vec<PlanningError>,
) -> Vec<BorderDef> {
    let mut borders: Vec<(BorderId, BorderDef)> = Vec::new();
    for (terminal, id) in border_map {
        let name = net
            .terminal(*terminal)
            .map(|t| t.name.clone())
            .unwrap_or_default();
        match approach(net, *terminal, ends, cfg.overlap_length) {
            Ok(def) => borders.push((*id, def)),
            Err(reason) => errors.push(PlanningError::Border {
                border: name,
                reason,
            }),
        }
    }
    borders.sort_by_key(|(id, _)| *id);
    borders.into_iter().map(|(_, d)| d).collect()
}

/// Ziel eines Fahrwegs an einem Netzende.
fn terminal_end(
    net: &Topology,
    t: TerminalId,
    borders: &HashMap<TerminalId, BorderId>,
) -> Option<(RouteEnd, String)> {
    let terminal = net.terminal(t)?;
    let end = match terminal.kind {
        TerminalKind::Boundary => RouteEnd::Boundary,
        TerminalKind::BufferStop => RouteEnd::BufferStop,
        TerminalKind::Border { .. } => RouteEnd::Border(*borders.get(&t)?),
    };
    Some((end, terminal.name.clone()))
}

/// Fahrstraßen aus den fertigen Fahrwegen.
fn route_defs(net: &Topology, drafts: Vec<Draft>, names: Vec<String>) -> Vec<RouteDef> {
    let mm = |v: u64| Length::from_mm(u32::try_from(v).unwrap_or(u32::MAX));
    drafts
        .into_iter()
        .zip(names)
        .map(|(d, name)| RouteDef {
            // Hauptsignale stehen am Gleisende oder kurz davor (REQ-PLN-010): Die Länge ist
            // der Rest des Startgleises, dazu die Gleise dahinter, abzüglich des Stücks hinter
            // dem Zielsignal; der Durchrutschweg zählt ab dem Zielsignal.
            length: mm({
                let tracks: u64 = d
                    .tracks
                    .iter()
                    .map(|t| u64::from(net.track(*t).map_or(0, |x| x.length.as_mm())))
                    .sum();
                let end_gap = match d.end {
                    RouteEnd::Signal(s) => signal_gap(net, s),
                    _ => 0,
                };
                (tracks + signal_gap(net, d.start)).saturating_sub(end_gap)
            }),
            overlap_length: mm(match d.end {
                RouteEnd::Signal(s) if d.overlap_length > 0 => {
                    d.overlap_length + signal_gap(net, s)
                }
                _ => d.overlap_length,
            }),
            name,
            start: d.start,
            end: d.end,
            speed: d.speed,
            points: d.points,
            tvds: d.tvds,
            overlap_tvds: d.overlap_tvds,
            overlap_points: d.overlap_points,
            flank: d.flank,
            level_crossings: d.level_crossings,
            conflicts: Vec::new(),
        })
        .collect()
}

/// Erzeugt den Plan für ein Netz.
///
/// # Errors
/// [`PlanningReport`] mit allen gefundenen Fehlern.
pub fn generate_with(net: &Topology, cfg: &PlanningConfig) -> Result<Plan, PlanningReport> {
    let mut errors = Vec::new();
    let ends = check_signals(net, *cfg, &mut errors);
    let announces = announced_signals(net, &mut errors);
    if !errors.is_empty() {
        return Err(PlanningReport { errors });
    }

    let border_map = border_ids(net);
    let borders = border_defs(net, &border_map, &ends, *cfg, &mut errors);

    let mut drafts = Vec::new();
    for (id, s) in net.signals() {
        if s.kind.delimits_routes() {
            explore(net, id, &ends, &border_map, &mut drafts, &mut errors);
        }
    }
    let names = route_names(net, &drafts);
    for (d, name) in drafts.iter_mut().zip(&names) {
        complete(net, *cfg, d, name, &mut errors);
    }

    let mut points = Vec::new();
    for (id, p) in net.points() {
        match net.tvd_of(TvdMember::Point(id)) {
            Some(tvd) => points.push(PointDef {
                name: p.name.clone(),
                tvd,
                coupled: net.coupling_of(id).map(|c| {
                    let partner = if c.points[0] == id {
                        c.points[1]
                    } else {
                        c.points[0]
                    };
                    (partner, c.inverted)
                }),
            }),
            None => errors.push(PlanningError::Inconsistent(format!(
                "Weiche „{}“ ohne Gleisfreimeldeabschnitt",
                p.name
            ))),
        }
    }
    let mut level_crossings = Vec::new();
    for (_, l) in net.level_crossings() {
        match net.tvd_of(TvdMember::Track(l.track)) {
            Some(tvd) => level_crossings.push(LxDef {
                name: l.name.clone(),
                tvd,
            }),
            None => errors.push(PlanningError::Inconsistent(format!(
                "Bahnübergang „{}“ ohne Abschnitt",
                l.name
            ))),
        }
    }
    if !errors.is_empty() {
        return Err(PlanningReport { errors });
    }

    let mut routes = route_defs(net, drafts, names);
    let conflicts = conflicts(net, &routes);
    for (r, c) in routes.iter_mut().zip(conflicts) {
        r.conflicts = c;
    }

    // Einfahrabschnitte: Abschnitte mit einem Gleis an einer Netzgrenze.
    let entry_tvds: Vec<TvdId> = net
        .terminals()
        .filter(|(_, t)| t.kind == TerminalKind::Boundary)
        .filter_map(|(id, _)| net.linked(PortRef::new(NodeRef::Terminal(id), Port::End)))
        .filter_map(|p| member_tvd(net, p.node))
        .collect();

    Ok(Plan {
        network: net.name().to_string(),
        points,
        signals: net
            .signals()
            .map(|(id, s)| SignalDef {
                name: s.name.clone(),
                kind: s.kind,
                system: s.system,
                announces: announces.get(&id).copied(),
            })
            .collect(),
        tvds: net
            .tvds()
            .map(|(id, t)| TvdDef {
                name: t.name.clone(),
                entry: entry_tvds.contains(&id),
            })
            .collect(),
        level_crossings,
        routes,
        borders,
    })
}

/// Erzeugt den Plan mit der Standardkonfiguration.
///
/// # Errors
/// Wie [`generate_with`].
pub fn generate(net: &Topology) -> Result<Plan, PlanningReport> {
    generate_with(net, &PlanningConfig::default())
}
