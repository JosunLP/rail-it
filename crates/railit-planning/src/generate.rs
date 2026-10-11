//! Fahrstraßengenerator.
//!
//! Tiefensuche über Ports ab jedem Startsignal. Die unabhängige Gegenprüfung in
//! [`crate::verify`] arbeitet bewusst anders (deterministische Läufe je Weichenkombination).

use std::collections::HashMap;

use railit_plan::{Plan, PointDef, RouteDef, RouteEnd, SignalDef, TvdDef};
use railit_topology::{
    Direction, NodeRef, Port, PortRef, SignalKind, TerminalKind, Topology, TvdMember,
};
use railit_types::bitset::{BitSet, BitSetMut, words_for};
use railit_types::{PointId, PointPosition, RouteId, SignalId, TrackId, TvdId};

use crate::error::{PlanningError, PlanningReport};

/// Signalarten, an denen Fahrstraßen beginnen und enden.
pub(crate) const fn delimits_routes(kind: SignalKind) -> bool {
    matches!(
        kind,
        SignalKind::Main | SignalKind::Combined | SignalKind::EtcsStopMarker
    )
}

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

/// REQ-PLN-010: Hauptsignale stehen am Gleisende an einer Gleisfreimeldegrenze.
/// Liefert die Zielsignale je (Gleis, Fahrtrichtung).
fn check_signals(
    net: &Topology,
    errors: &mut Vec<PlanningError>,
) -> HashMap<(TrackId, Direction), SignalId> {
    let mut ends = HashMap::new();
    for (id, s) in net.signals().filter(|(_, s)| delimits_routes(s.kind)) {
        let Some(track) = net.track(s.track) else {
            errors.push(PlanningError::Inconsistent(format!(
                "Signal „{}“ ohne Gleis",
                s.name
            )));
            continue;
        };
        let at_end = match s.direction {
            Direction::AToB => s.position == track.length,
            Direction::BToA => s.position.as_mm() == 0,
        };
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

/// Alle Fahrwege ab Startsignal `start`.
fn explore(
    net: &Topology,
    start: SignalId,
    ends: &HashMap<(TrackId, Direction), SignalId>,
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

        match entry.node {
            NodeRef::Terminal(t) => {
                let Some(terminal) = net.terminal(t) else {
                    continue;
                };
                drafts.push(Draft {
                    start,
                    end: match terminal.kind {
                        TerminalKind::Boundary => RouteEnd::Boundary,
                        TerminalKind::BufferStop => RouteEnd::BufferStop,
                    },
                    end_name: terminal.name.clone(),
                    points: frame.points,
                    tvds: frame.tvds,
                });
            }
            NodeRef::Track(t) => {
                let direction = if entry.port == Port::A {
                    Direction::AToB
                } else {
                    Direction::BToA
                };
                if let Some(&end) = ends.get(&(t, direction)) {
                    drafts.push(Draft {
                        start,
                        end: RouteEnd::Signal(end),
                        end_name: signal_name(net, end),
                        points: frame.points,
                        tvds: frame.tvds,
                    });
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
                    let name = net.point(*p).map_or("?", |p| p.name.as_str());
                    let side = if *pos == PointPosition::Left {
                        "L"
                    } else {
                        "R"
                    };
                    format!("{name}{side}")
                })
                .collect();
            format!("{b}[{}]", via.join(","))
        })
        .collect()
}

/// REQ-PLN-012: feindlich bei gemeinsamem Startsignal, Gleisfreimeldeabschnitt oder Weiche.
fn conflicts(net: &Topology, drafts: &[Draft]) -> Vec<Vec<RouteId>> {
    let tvd_words = words_for(net.tvds().len());
    let point_words = words_for(net.points().len());
    let sets: Vec<(Vec<u64>, Vec<u64>)> = drafts
        .iter()
        .map(|d| {
            let mut tvds = vec![0u64; tvd_words];
            let mut points = vec![0u64; point_words];
            for t in &d.tvds {
                if let Some(i) = t.index() {
                    let _ = BitSetMut::new(&mut tvds).insert(i);
                }
            }
            for (p, _) in &d.points {
                if let Some(i) = p.index() {
                    let _ = BitSetMut::new(&mut points).insert(i);
                }
            }
            (tvds, points)
        })
        .collect();

    drafts
        .iter()
        .enumerate()
        .map(|(a, da)| {
            drafts
                .iter()
                .enumerate()
                .filter(|&(b, db)| {
                    b != a
                        && (da.start == db.start
                            || BitSet::new(&sets[a].0).intersects(&BitSet::new(&sets[b].0))
                            || BitSet::new(&sets[a].1).intersects(&BitSet::new(&sets[b].1)))
                })
                .filter_map(|(b, _)| RouteId::from_index(b))
                .collect()
        })
        .collect()
}

/// Erzeugt den Plan für ein Netz: Signalstandorte prüfen, Fahrstraßen erzeugen und benennen,
/// Feindlichkeiten bestimmen.
///
/// # Errors
/// [`PlanningReport`] mit allen gefundenen Fehlern.
pub fn generate(net: &Topology) -> Result<Plan, PlanningReport> {
    let mut errors = Vec::new();
    let ends = check_signals(net, &mut errors);
    if !errors.is_empty() {
        return Err(PlanningReport { errors });
    }

    let mut drafts = Vec::new();
    for (id, s) in net.signals() {
        if delimits_routes(s.kind) {
            explore(net, id, &ends, &mut drafts, &mut errors);
        }
    }

    let mut points = Vec::new();
    for (id, p) in net.points() {
        match net.tvd_of(TvdMember::Point(id)) {
            Some(tvd) => points.push(PointDef {
                name: p.name.clone(),
                tvd,
            }),
            None => errors.push(PlanningError::Inconsistent(format!(
                "Weiche „{}“ ohne Gleisfreimeldeabschnitt",
                p.name
            ))),
        }
    }
    if !errors.is_empty() {
        return Err(PlanningReport { errors });
    }

    let names = route_names(net, &drafts);
    let conflicts = conflicts(net, &drafts);
    let routes = drafts
        .into_iter()
        .zip(names)
        .zip(conflicts)
        .map(|((d, name), conflicts)| RouteDef {
            name,
            start: d.start,
            end: d.end,
            points: d.points,
            tvds: d.tvds,
            conflicts,
        })
        .collect();

    Ok(Plan {
        network: net.name().to_string(),
        points,
        signals: net
            .signals()
            .map(|(_, s)| SignalDef {
                name: s.name.clone(),
            })
            .collect(),
        tvds: net
            .tvds()
            .map(|(_, t)| TvdDef {
                name: t.name.clone(),
            })
            .collect(),
        routes,
    })
}
