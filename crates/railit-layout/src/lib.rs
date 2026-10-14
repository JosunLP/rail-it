//! Spurplan: schematische Geometrie eines Netzes für Bedienplätze (Konzept, Abschnitt 14.3;
//! REQ-UI-001).
//!
//! Das Netzmodell kennt keine Koordinaten. Der Spurplan wird daraus berechnet:
//!
//! 1. **Ausrichtung:** Eine Breitensuche ab dem ersten Netzende legt für jeden Knoten fest,
//!    welche Seite links liegt (Gleis `a`→`b` oder `b`→`a`, Weichenspitze links oder rechts).
//! 2. **Spuren:** Die Spur (y) wird entlang der Gleise weitergereicht; der abzweigende Strang
//!    einer Weiche erhält eine eigene Spur (rechts abzweigend in Fahrtrichtung nach unten).
//! 3. **Längsachse:** Die x-Lage ist der längste Pfad über alle Verbindungen von links nach
//!    rechts. Gleise erhalten eine Breite nach ihrer Länge (gestaucht), Weichen eine feste.
//!    Gleise überbrücken die Lücken zwischen Weichen und Netzenden.
//! 4. **Punktobjekte:** Signale und Bahnübergänge liegen an ihrer Position auf dem Gleis;
//!    Signale zeigen in ihre Wirkrichtung.
//!
//! Das Ergebnis ([`Layout`]) ist in abstrakten Einheiten (Spurabstand [`LANE`]); Bedienplätze
//! skalieren es frei. [`to_svg`] erzeugt daraus eine statische Darstellung.

mod svg;

use std::collections::{BTreeMap, VecDeque};

use railit_topology::{
    Direction, NodeRef, PointPosition, Port, PortRef, SignalKind, SignalSystem, TerminalKind,
    Topology, TvdMember,
};
use serde::Serialize;

pub use svg::to_svg;

/// Abstand zweier Spuren.
pub const LANE: i32 = 60;
/// Breite einer Weiche.
pub const POINT_WIDTH: i32 = 48;
/// Rand um den Spurplan.
pub const MARGIN: i32 = 60;

/// Koordinate im Spurplan.
pub type Pos = [i32; 2];

/// Spurplan eines Netzes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Layout {
    /// Name des Netzes.
    pub network: String,
    /// Breite.
    pub width: i32,
    /// Höhe.
    pub height: i32,
    /// Gleise.
    pub tracks: Vec<TrackShape>,
    /// Weichen.
    pub points: Vec<PointShape>,
    /// Netzenden.
    pub terminals: Vec<TerminalShape>,
    /// Signale.
    pub signals: Vec<SignalShape>,
    /// Bahnübergänge.
    pub level_crossings: Vec<MarkShape>,
    /// Kreuzungen: beide Kreuzungsstellen (im Spurplan getrennt gezeichnet, verbunden).
    pub crossings: Vec<CrossingShape>,
    /// Grenzen zwischen Stellbereichen.
    pub borders: Vec<BorderShape>,
    /// Stellbereiche (horizontale Ausdehnung).
    pub areas: Vec<AreaShape>,
    /// Betriebsstellen (Beschriftung).
    pub stations: Vec<MarkShape>,
    /// Kreuzungsweichen (Beschriftung in ihrer Mitte).
    pub slips: Vec<MarkShape>,
}

/// Gleis als Linienzug.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TrackShape {
    /// Name.
    pub name: String,
    /// Gleisfreimeldeabschnitt.
    pub tvd: Option<String>,
    /// Stellbereich.
    pub area: Option<String>,
    /// Linienzug von links nach rechts.
    pub line: Vec<Pos>,
    /// Kreuzungsweiche, zu der das Gleis gehört.
    pub slip: Option<String>,
}

/// Weiche: Spitze, Verzweigungspunkt und beide Stränge.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PointShape {
    /// Name.
    pub name: String,
    /// Gleisfreimeldeabschnitt.
    pub tvd: Option<String>,
    /// Stellbereich.
    pub area: Option<String>,
    /// Ende an der Spitze.
    pub tip: Pos,
    /// Verzweigungspunkt.
    pub fork: Pos,
    /// Ende des linken Strangs.
    pub left: Pos,
    /// Ende des rechten Strangs.
    pub right: Pos,
    /// Kreuzungsweiche, zu der die Weiche (das Zungenpaar) gehört.
    pub slip: Option<String>,
}

/// Netzende.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TerminalShape {
    /// Name.
    pub name: String,
    /// `boundary`, `buffer_stop` oder `border`.
    pub kind: &'static str,
    /// Lage.
    pub at: Pos,
    /// Das Gleis geht nach `"left"` bzw. `"right"` weiter.
    pub opens: &'static str,
}

/// Signal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SignalShape {
    /// Name.
    pub name: String,
    /// Signalart (wie im Netzmodell).
    pub kind: SignalKind,
    /// Signalsystem.
    pub system: SignalSystem,
    /// Lage auf dem Gleis.
    pub at: Pos,
    /// Wirkrichtung im Spurplan: `"right"` oder `"left"`.
    pub facing: &'static str,
    /// Gleis.
    pub track: String,
}

/// Benannter Punkt (Bahnübergang, Betriebsstelle).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MarkShape {
    /// Name.
    pub name: String,
    /// Lage.
    pub at: Pos,
}

/// Kreuzung ohne bewegliche Teile: Lage beider Kreuzungsstellen.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CrossingShape {
    /// Name.
    pub name: String,
    /// Kreuzungsstellen.
    pub at: [Pos; 2],
}

/// Grenze zwischen zwei Stellbereichen.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BorderShape {
    /// Bereich links.
    pub left: String,
    /// Bereich rechts.
    pub right: String,
    /// Lage.
    pub at: Pos,
}

/// Horizontale Ausdehnung eines Stellbereichs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AreaShape {
    /// Name.
    pub name: String,
    /// Linker Rand.
    pub x0: i32,
    /// Rechter Rand.
    pub x1: i32,
}

/// Breite einer Kreuzungsweiche.
pub const SLIP_WIDTH: i32 = 2 * POINT_WIDTH;

/// Spur der Enden `a`, `b`, `c`, `d` einer Kreuzungsweiche relativ zu `a`: Das Stammgleis
/// `a`–`c` liegt waagerecht, die Diagonale `b`–`d` kreuzt es von unten links nach oben rechts.
const SLIP_LANES: [i32; 4] = [0, 1, 0, -1];

/// Lage einer Kreuzungsweiche im Spurplan.
#[derive(Copy, Clone, Debug)]
struct SlipPlaced {
    /// Die Enden `a`, `b` liegen links.
    ab_left: bool,
    /// Spuren der Enden `a` … `d`.
    lanes: [i32; 4],
}

impl SlipPlaced {
    /// Liegt Ende `end` rechts?
    const fn right(&self, end: usize) -> bool {
        (end >= 2) == self.ab_left
    }
}

/// Ausrichtung eines Knotens.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Orient {
    /// Gleis: `a` links.
    Forward,
    /// Gleis: `b` links.
    Backward,
    /// Weiche: Spitze links (Stränge rechts).
    TipLeft,
    /// Weiche: Spitze rechts.
    TipRight,
    /// Netzende links im Bild (Gleis geht nach rechts weiter).
    OpensRight,
    /// Netzende rechts im Bild.
    OpensLeft,
}

#[derive(Copy, Clone, Debug)]
struct Placed {
    orient: Orient,
    lane: i32,
}

/// Liegt `port` auf der rechten Seite des so ausgerichteten Knotens?
const fn right_side(orient: Orient, port: Port) -> bool {
    matches!(
        (orient, port),
        (Orient::Forward, Port::B)
            | (Orient::Backward, Port::A)
            | (Orient::TipLeft, Port::Left | Port::Right)
            | (Orient::TipRight, Port::Tip)
            | (Orient::OpensRight, Port::End)
    )
}

/// Ausrichtung eines Knotens, der über `port` von links (`from_left`) bzw. rechts betreten
/// wird.
fn orientation(node: NodeRef, port: Port, from_left: bool) -> Orient {
    match node {
        NodeRef::Track(_) => {
            if (port == Port::A) == from_left {
                Orient::Forward
            } else {
                Orient::Backward
            }
        }
        NodeRef::Point(_) => {
            if (port == Port::Tip) == from_left {
                Orient::TipLeft
            } else {
                Orient::TipRight
            }
        }
        NodeRef::Terminal(_) => {
            if from_left {
                Orient::OpensLeft
            } else {
                Orient::OpensRight
            }
        }
    }
}

/// Spurversatz des Strangs `port` gegenüber der Spitze.
fn leg_offset(topo: &Topology, node: NodeRef, port: Port, orient: Orient) -> i32 {
    let NodeRef::Point(id) = node else {
        return 0;
    };
    let diverging = topo
        .point(id)
        .and_then(|p| p.diverging)
        .unwrap_or(PointPosition::Right);
    let is_diverging = matches!(
        (port, diverging),
        (Port::Left, PointPosition::Left) | (Port::Right, PointPosition::Right)
    );
    if !is_diverging {
        return 0;
    }
    // In Fahrtrichtung rechts abzweigend: bei Spitze links nach unten, bei Spitze rechts
    // (Fahrt nach links) nach oben.
    let down = (diverging == PointPosition::Right) == (orient == Orient::TipLeft);
    if down { 1 } else { -1 }
}

fn width(topo: &Topology, node: NodeRef) -> i32 {
    match node {
        NodeRef::Track(id) => {
            let m = topo.track(id).map_or(0, |t| t.length.as_mm() / 1000);
            // Gestaucht: lange Gleise werden nicht beliebig lang.
            let w = f64::from(m).sqrt() * 7.0;
            #[allow(clippy::cast_possible_truncation)]
            let w = w.round() as i32;
            w.clamp(70, 260)
        }
        NodeRef::Point(_) => POINT_WIDTH,
        NodeRef::Terminal(_) => 0,
    }
}

fn ports(node: NodeRef) -> &'static [Port] {
    node.ports()
}

/// Berechnet den Spurplan eines Netzes.
#[allow(clippy::too_many_lines)]
pub fn layout(topo: &Topology) -> Layout {
    let mut placed: BTreeMap<NodeRef, Placed> = BTreeMap::new();
    let mut order: Vec<NodeRef> = Vec::new();
    let mut lane_base = 0;

    // Kreuzungsweichen werden als Block gelegt; ihre inneren Knoten nimmt die Suche nicht auf.
    let slips: Vec<&railit_topology::Slip> = topo.slips().collect();
    let slip_ends: Vec<[PortRef; 4]> = slips.iter().map(|x| x.ends).collect();
    let mut slip_of: BTreeMap<NodeRef, usize> = BTreeMap::new();
    for (i, x) in slips.iter().enumerate() {
        for t in &x.tracks {
            slip_of.insert(NodeRef::Track(*t), i);
        }
        for p in &x.points {
            slip_of.insert(NodeRef::Point(*p), i);
        }
    }
    let mut slip_placed: Vec<Option<SlipPlaced>> = vec![None; slips.len()];

    // Startknoten: Netzenden in Reihenfolge, danach alle übrigen Knoten (Teilnetze ohne Ende).
    let mut starts: Vec<NodeRef> = topo
        .terminals()
        .map(|(id, _)| NodeRef::Terminal(id))
        .collect();
    starts.extend(topo.tracks().map(|(id, _)| NodeRef::Track(id)));
    for start in starts {
        if placed.contains_key(&start) || slip_of.contains_key(&start) {
            continue;
        }
        let first = order.len();
        let orient = match start {
            NodeRef::Terminal(_) => Orient::OpensRight,
            NodeRef::Track(_) => Orient::Forward,
            NodeRef::Point(_) => Orient::TipLeft,
        };
        placed.insert(
            start,
            Placed {
                orient,
                lane: lane_base,
            },
        );
        let mut queue = VecDeque::from([start]);
        while let Some(node) = queue.pop_front() {
            order.push(node);
            let Some(here) = placed.get(&node).copied() else {
                continue;
            };
            for &port in ports(node) {
                let Some(other) = topo.linked(PortRef::new(node, port)) else {
                    continue;
                };
                let from_left = right_side(here.orient, port);
                // Spur am verlassenen Port.
                let lane_out = here.lane + leg_offset(topo, node, port, here.orient);
                if let Some(&i) = slip_of.get(&other.node) {
                    // Kreuzungsweiche als Ganzes legen, dann von ihren Enden weitersuchen.
                    if slip_placed[i].is_some() {
                        continue;
                    }
                    let Some(end) = slip_ends[i].iter().position(|e| *e == other) else {
                        continue;
                    };
                    let offset = lane_out - SLIP_LANES[end];
                    let placement = SlipPlaced {
                        ab_left: (end < 2) == from_left,
                        lanes: SLIP_LANES.map(|l| l + offset),
                    };
                    slip_placed[i] = Some(placement);
                    for (j, e) in slip_ends[i].iter().enumerate() {
                        let Some(next) = topo.linked(*e) else {
                            continue;
                        };
                        if placed.contains_key(&next.node) || slip_of.contains_key(&next.node) {
                            continue;
                        }
                        let orient = orientation(next.node, next.port, placement.right(j));
                        let lane =
                            placement.lanes[j] - leg_offset(topo, next.node, next.port, orient);
                        placed.insert(next.node, Placed { orient, lane });
                        queue.push_back(next.node);
                    }
                    continue;
                }
                if placed.contains_key(&other.node) {
                    continue;
                }
                let orient = orientation(other.node, other.port, from_left);
                // Spur des neuen Knotens (bei Weichen: Spitze).
                let lane = lane_out - leg_offset(topo, other.node, other.port, orient);
                placed.insert(other.node, Placed { orient, lane });
                queue.push_back(other.node);
            }
        }
        // Nächstes Teilnetz unterhalb.
        let max_lane = order[first..]
            .iter()
            .filter_map(|n| placed.get(n))
            .map(|p| p.lane + 1)
            .chain(
                slip_placed
                    .iter()
                    .flatten()
                    .flat_map(|x| x.lanes.map(|l| l + 1)),
            )
            .max()
            .unwrap_or(lane_base)
            .max(lane_base);
        lane_base = max_lane + 1;
    }

    // Längsachse: längster Pfad über alle Verbindungen von links nach rechts.
    let mut xs: BTreeMap<NodeRef, i32> = order.iter().map(|n| (*n, 0)).collect();
    let mut slip_x: Vec<i32> = vec![0; slips.len()];
    // Ende einer gelegten Kreuzungsweiche: (Weiche, Ende, rechts?).
    let slip_end = |port: PortRef| -> Option<(usize, usize, bool)> {
        let i = *slip_of.get(&port.node)?;
        let end = slip_ends[i].iter().position(|e| *e == port)?;
        let placement = slip_placed[i]?;
        Some((i, end, placement.right(end)))
    };
    for _ in 0..(order.len() + slips.len()).max(1) {
        let mut changed = false;
        // (linker Port, rechter Nachbar-Port, benötigtes x des Nachbarn)
        let mut edges: Vec<(PortRef, i32)> = Vec::new();
        for &node in &order {
            let Some(here) = placed.get(&node).copied() else {
                continue;
            };
            for &port in ports(node) {
                if right_side(here.orient, port) {
                    edges.push((PortRef::new(node, port), xs[&node] + width(topo, node)));
                }
            }
        }
        for (i, x) in slip_ends.iter().enumerate() {
            let Some(placement) = slip_placed[i] else {
                continue;
            };
            for (end, port) in x.iter().enumerate() {
                if placement.right(end) {
                    edges.push((*port, slip_x[i] + SLIP_WIDTH));
                }
            }
        }
        for (from, need) in edges {
            let Some(other) = topo.linked(from) else {
                continue;
            };
            if let Some((i, _, right)) = slip_end(other) {
                if !right && slip_x[i] < need {
                    slip_x[i] = need;
                    changed = true;
                }
                continue;
            }
            let Some(there) = placed.get(&other.node) else {
                continue;
            };
            if right_side(there.orient, other.port) {
                continue; // widersprüchlich ausgerichtet (Schleife): ohne Bedingung
            }
            if xs.get(&other.node).is_some_and(|x| *x < need) {
                xs.insert(other.node, need);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let min_lane = placed
        .values()
        .map(|p| p.lane)
        .chain(slip_placed.iter().flatten().flat_map(|x| x.lanes))
        .min()
        .unwrap_or(0);
    let lane_y = |lane: i32| MARGIN + (lane - min_lane) * LANE;
    let gx = |v: i32| MARGIN + v;

    // Lage eines Ports.
    let slip_pos = |i: usize, end: usize| -> Pos {
        let Some(placement) = slip_placed.get(i).copied().flatten() else {
            return [0, 0];
        };
        let x = slip_x[i] + if placement.right(end) { SLIP_WIDTH } else { 0 };
        [gx(x), lane_y(placement.lanes[end])]
    };
    let port_pos = |node: NodeRef, port: Port| -> Pos {
        if let Some((i, end, _)) = slip_end(PortRef::new(node, port)) {
            return slip_pos(i, end);
        }
        let Some(p) = placed.get(&node) else {
            return [0, 0];
        };
        let x0 = xs.get(&node).copied().unwrap_or(0);
        match node {
            NodeRef::Point(_) => {
                let right = right_side(p.orient, port);
                let px = if right { x0 + POINT_WIDTH } else { x0 };
                let lane = p.lane + leg_offset(topo, node, port, p.orient);
                [gx(px), lane_y(lane)]
            }
            NodeRef::Terminal(_) => [gx(x0), lane_y(p.lane)],
            NodeRef::Track(_) => {
                let right = right_side(p.orient, port);
                let px = if right { x0 + width(topo, node) } else { x0 };
                [gx(px), lane_y(p.lane)]
            }
        }
    };
    // Gleisenden liegen dort, wo der Nachbar beginnt bzw. endet; das Gleis bleibt auf seiner
    // Spur (Weichenstränge führen zur Spur des Gleises).
    let track_end = |node: NodeRef, port: Port| -> Pos {
        let mine = port_pos(node, port);
        match topo.linked(PortRef::new(node, port)) {
            Some(other)
                if matches!(other.node, NodeRef::Point(_) | NodeRef::Terminal(_))
                    || slip_end(other).is_some() =>
            {
                [port_pos(other.node, other.port)[0], mine[1]]
            }
            Some(other)
                if placed
                    .get(&node)
                    .is_some_and(|p| right_side(p.orient, port)) =>
            {
                // Gleis an Gleis: Grenze am Anfang des rechten Gleises.
                [port_pos(other.node, other.port)[0].max(mine[0]), mine[1]]
            }
            _ => mine,
        }
    };

    let area_of = |member: TvdMember| {
        topo.tvd_of(member).and_then(|t| {
            topo.area_of_tvd(t)
                .and_then(|a| topo.area(a))
                .map(|a| a.name.clone())
        })
    };
    let tvd_name = |member: TvdMember| {
        topo.tvd_of(member)
            .and_then(|t| topo.tvd(t))
            .map(|t| t.name.clone())
    };

    let mut tracks = Vec::new();
    let mut track_ends: BTreeMap<railit_types::TrackId, (Pos, Pos, bool)> = BTreeMap::new();
    for (id, track) in topo.tracks() {
        let node = NodeRef::Track(id);
        let Some(p) = placed.get(&node) else { continue };
        let (lp, rp) = if p.orient == Orient::Forward {
            (Port::A, Port::B)
        } else {
            (Port::B, Port::A)
        };
        let start = track_end(node, lp);
        let end = track_end(node, rp);
        track_ends.insert(id, (start, end, p.orient == Orient::Forward));
        let line = vec![start, end];
        tracks.push(TrackShape {
            name: track.name.clone(),
            tvd: tvd_name(TvdMember::Track(id)),
            area: area_of(TvdMember::Track(id)),
            line,
            slip: None,
        });
    }

    let mut points = Vec::new();
    for (id, point) in topo.points() {
        let node = NodeRef::Point(id);
        let Some(p) = placed.get(&node) else { continue };
        let tip = port_pos(node, Port::Tip);
        let fork_x = if p.orient == Orient::TipLeft {
            tip[0] + POINT_WIDTH / 3
        } else {
            tip[0] - POINT_WIDTH / 3
        };
        // Stränge enden auf der Spur ihres Nachbarn.
        let leg = |port: Port| {
            let own = port_pos(node, port);
            match topo.linked(PortRef::new(node, port)) {
                Some(o) if matches!(o.node, NodeRef::Track(_)) => [
                    own[0],
                    placed.get(&o.node).map_or(own[1], |q| lane_y(q.lane)),
                ],
                _ => own,
            }
        };
        points.push(PointShape {
            name: point.name.clone(),
            tvd: tvd_name(TvdMember::Point(id)),
            area: area_of(TvdMember::Point(id)),
            tip,
            fork: [fork_x, tip[1]],
            left: leg(Port::Left),
            right: leg(Port::Right),
            slip: None,
        });
    }
    let slip_labels = slip_shapes(
        topo,
        &slips,
        &slip_placed,
        &slip_pos,
        &mut tracks,
        &mut track_ends,
        &mut points,
    );

    let mut terminals = Vec::new();
    for (id, t) in topo.terminals() {
        let node = NodeRef::Terminal(id);
        let Some(p) = placed.get(&node) else { continue };
        terminals.push(TerminalShape {
            name: t.name.clone(),
            kind: match t.kind {
                TerminalKind::Boundary => "boundary",
                TerminalKind::BufferStop => "buffer_stop",
                TerminalKind::Border { .. } => "border",
            },
            at: port_pos(node, Port::End),
            opens: if p.orient == Orient::OpensRight {
                "right"
            } else {
                "left"
            },
        });
    }

    let along = |track: railit_types::TrackId, pos_mm: u32| -> Option<(Pos, bool)> {
        let (a, b, forward) = track_ends.get(&track).copied()?;
        let len = topo.track(track)?.length.as_mm().max(1);
        let f = f64::from(pos_mm.min(len)) / f64::from(len);
        let f = if forward { f } else { 1.0 - f };
        #[allow(clippy::cast_possible_truncation)]
        let px = a[0] + (f64::from(b[0] - a[0]) * f).round() as i32;
        Some(([px, a[1]], forward))
    };

    let mut signals = Vec::new();
    for (_, s) in topo.signals() {
        let Some((at, forward)) = along(s.track, s.position.as_mm()) else {
            continue;
        };
        let rightwards = (s.direction == Direction::AToB) == forward;
        signals.push(SignalShape {
            name: s.name.clone(),
            kind: s.kind,
            system: s.system,
            at,
            facing: if rightwards { "right" } else { "left" },
            track: topo
                .track(s.track)
                .map(|t| t.name.clone())
                .unwrap_or_default(),
        });
    }

    let level_crossings = topo
        .level_crossings()
        .filter_map(|(_, l)| {
            along(l.track, l.position.as_mm()).map(|(at, _)| MarkShape {
                name: l.name.clone(),
                at,
            })
        })
        .collect();

    let crossings = topo
        .crossings()
        .filter(|c| !slips.iter().any(|x| c.name == format!("{}.x", x.name)))
        .filter_map(|c| {
            let [(ta, pa), (tb, pb)] = c.legs;
            let (a, _) = along(ta, pa.as_mm())?;
            let (b, _) = along(tb, pb.as_mm())?;
            Some(CrossingShape {
                name: c.name.clone(),
                at: [a, b],
            })
        })
        .collect();

    let mut borders = Vec::new();
    for link in topo.area_links() {
        let (Some(a), Some(b)) = (topo.area(link.a), topo.area(link.b)) else {
            continue;
        };
        let pa = port_pos(link.a_port.node, link.a_port.port);
        let pb = port_pos(link.b_port.node, link.b_port.port);
        let at = [pa[0].max(pb[0]), pa[1]];
        let a_left = xs.get(&link.a_port.node) <= xs.get(&link.b_port.node);
        let (left, right) = if a_left { (a, b) } else { (b, a) };
        borders.push(BorderShape {
            left: left.name.clone(),
            right: right.name.clone(),
            at,
        });
    }

    let mut areas: Vec<AreaShape> = Vec::new();
    for (_, area) in topo.areas() {
        let xs: Vec<i32> = tracks
            .iter()
            .filter(|t| t.area.as_deref() == Some(area.name.as_str()))
            .flat_map(|t| t.line.iter().map(|p| p[0]))
            .chain(
                points
                    .iter()
                    .filter(|p| p.area.as_deref() == Some(area.name.as_str()))
                    .flat_map(|p| [p.tip[0], p.left[0]]),
            )
            .collect();
        if let (Some(x0), Some(x1)) = (xs.iter().min(), xs.iter().max()) {
            areas.push(AreaShape {
                name: area.name.clone(),
                x0: *x0,
                x1: *x1,
            });
        }
    }

    let top = signals
        .iter()
        .map(|s| s.at[1])
        .chain(tracks.iter().flat_map(|t| t.line.iter().map(|p| p[1])))
        .min()
        .unwrap_or(MARGIN);
    let mut stations = Vec::new();
    for (_, st) in topo.stations() {
        let xs: Vec<i32> = st
            .platforms
            .iter()
            .flat_map(|p| p.signals.iter())
            .filter_map(|s| topo.signal(*s))
            .filter_map(|s| signals.iter().find(|g| g.name == s.name))
            .map(|g| g.at[0])
            .collect();
        if xs.is_empty() {
            continue;
        }
        let sum: i64 = xs.iter().map(|v| i64::from(*v)).sum();
        let mid = i32::try_from(sum / i64::try_from(xs.len()).unwrap_or(1)).unwrap_or(0);
        stations.push(MarkShape {
            name: st.name.clone(),
            at: [mid, top - LANE / 2 - 10],
        });
    }

    let width = tracks
        .iter()
        .flat_map(|t| t.line.iter().map(|p| p[0]))
        .chain(terminals.iter().map(|t| t.at[0]))
        .max()
        .unwrap_or(0)
        + MARGIN;
    let height = placed
        .values()
        .map(|p| p.lane)
        .chain(slip_placed.iter().flatten().flat_map(|x| x.lanes))
        .map(lane_y)
        .max()
        .unwrap_or(0)
        + MARGIN;
    // Beschriftung der Betriebsstellen braucht oben Platz: alles nach unten schieben.
    let shift = (MARGIN + 10
        - stations
            .iter()
            .map(|s| s.at[1])
            .min()
            .unwrap_or(MARGIN + 10))
    .max(0);
    let mut layout = Layout {
        network: topo.name().to_string(),
        width,
        height: height + shift,
        tracks,
        points,
        terminals,
        signals,
        level_crossings,
        crossings,
        borders,
        areas,
        stations,
        slips: slip_labels,
    };
    if shift > 0 {
        shift_down(&mut layout, shift);
    }
    layout
}

/// Gleise und Zungenpaare der Kreuzungsweichen: gerade Linien zwischen ihren Enden. Liefert
/// die Beschriftungen.
#[allow(clippy::too_many_arguments)]
fn slip_shapes(
    topo: &Topology,
    slips: &[&railit_topology::Slip],
    placed: &[Option<SlipPlaced>],
    pos: &dyn Fn(usize, usize) -> Pos,
    tracks: &mut Vec<TrackShape>,
    track_ends: &mut BTreeMap<railit_types::TrackId, (Pos, Pos, bool)>,
    points: &mut Vec<PointShape>,
) -> Vec<MarkShape> {
    let tvd_name = |member: TvdMember| {
        topo.tvd_of(member)
            .and_then(|t| topo.tvd(t))
            .map(|t| t.name.clone())
    };
    let area_of = |member: TvdMember| {
        topo.tvd_of(member).and_then(|t| {
            topo.area_of_tvd(t)
                .and_then(|a| topo.area(a))
                .map(|a| a.name.clone())
        })
    };
    let mut labels = Vec::new();
    for (i, x) in slips.iter().enumerate() {
        if placed.get(i).copied().flatten().is_none() {
            continue;
        }
        let at = |end: usize| pos(i, end);
        // Gleise: a–c, b–d, Bogen a–d, Bogen b–c (siehe `Slip::tracks`).
        for (t, (from, to)) in x.tracks.iter().zip([(0, 2), (1, 3), (0, 3), (1, 2)]) {
            let (a, b) = (at(from), at(to));
            let (start, end) = if a[0] <= b[0] { (a, b) } else { (b, a) };
            track_ends.insert(*t, (a, b, true));
            tracks.push(TrackShape {
                name: topo.track(*t).map(|t| t.name.clone()).unwrap_or_default(),
                tvd: tvd_name(TvdMember::Track(*t)),
                area: area_of(TvdMember::Track(*t)),
                line: vec![start, end],
                slip: Some(x.name.clone()),
            });
        }
        // Zungenpaare: Spitze am Ende, links gerade, rechts in den Bogen.
        let ends: &[usize] = if x.points.len() == 4 {
            &[0, 1, 2, 3]
        } else {
            &[0, 3]
        };
        for (p, &end) in x.points.iter().zip(ends) {
            let (straight, curve) = match end {
                0 => (2, 3),
                1 => (3, 2),
                2 => (0, 1),
                _ => (1, 0),
            };
            let tip = at(end);
            let towards = |other: usize| {
                let o = at(other);
                [tip[0] + (o[0] - tip[0]) / 3, tip[1] + (o[1] - tip[1]) / 3]
            };
            let left = towards(straight);
            let right = towards(curve);
            points.push(PointShape {
                name: topo.point(*p).map(|p| p.name.clone()).unwrap_or_default(),
                tvd: tvd_name(TvdMember::Point(*p)),
                area: area_of(TvdMember::Point(*p)),
                tip,
                fork: tip,
                left,
                right,
                slip: Some(x.name.clone()),
            });
        }
        let (a, c) = (at(0), at(2));
        labels.push(MarkShape {
            name: x.name.clone(),
            at: [i32::midpoint(a[0], c[0]), a[1] - 12],
        });
    }
    labels
}

fn shift_down(l: &mut Layout, dy: i32) {
    let mv = |p: &mut Pos| p[1] += dy;
    for t in &mut l.tracks {
        t.line.iter_mut().for_each(mv);
    }
    for p in &mut l.points {
        for q in [&mut p.tip, &mut p.fork, &mut p.left, &mut p.right] {
            q[1] += dy;
        }
    }
    for t in &mut l.terminals {
        t.at[1] += dy;
    }
    for s in &mut l.signals {
        s.at[1] += dy;
    }
    for m in l
        .level_crossings
        .iter_mut()
        .chain(l.stations.iter_mut())
        .chain(l.slips.iter_mut())
    {
        m.at[1] += dy;
    }
    for b in &mut l.borders {
        b.at[1] += dy;
    }
    for c in &mut l.crossings {
        c.at.iter_mut().for_each(mv);
    }
}
