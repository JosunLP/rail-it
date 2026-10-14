//! Aufteilung eines Netzes in Stellbereiche (REQ-TOP-013).
//!
//! Jeder Stellbereich wird von einem eigenen Sicherheitskern gesteuert. Dafür entsteht aus
//! dem Gesamtnetz je Bereich ein **Teilnetz**: alle Gleise und Weichen seiner Abschnitte, die
//! Netzenden daran, Signale und Bahnübergänge darauf. Jede Verbindung in einen anderen
//! Bereich wird durch ein Netzende der Art [`TerminalKind::Border`] ersetzt. Das Teilnetz
//! wird wie jedes andere Netz validiert und projektiert.
//!
//! Die Elemente des Teilnetzes behalten ihre Namen; ihre IDs werden neu vergeben.
//! [`AreaCut`] enthält die Abbildung zurück auf das Gesamtnetz.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::vec::Vec;

use railit_types::{AreaId, LxId, PointId, SignalId, TerminalId, TrackId, TvdId};

use crate::builder::TopologyBuilder;
use crate::error::{Report, TopologyError};
use crate::model::{NodeRef, Port, PortRef, TerminalKind, TvdMember};
use crate::topology::Topology;

/// Grenze eines Teilnetzes zu einem Nachbarbereich.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct CutBorder {
    /// Netzende im Teilnetz, das die Grenze darstellt.
    pub terminal: TerminalId,
    /// Nachbarbereich.
    pub neighbour: AreaId,
    /// Port im Gesamtnetz auf dieser Seite der Grenze.
    pub inside: PortRef,
    /// Port im Gesamtnetz auf der Seite des Nachbarn.
    pub outside: PortRef,
}

/// Teilnetz eines Stellbereichs mit Abbildung auf das Gesamtnetz.
///
/// Die Vektoren bilden lokale IDs (Index) auf globale IDs ab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AreaCut {
    /// Bereich im Gesamtnetz.
    pub area: AreaId,
    /// Validiertes Teilnetz.
    pub topology: Topology,
    /// Gleise.
    pub tracks: Vec<TrackId>,
    /// Weichen.
    pub points: Vec<PointId>,
    /// Netzenden; `None` für Grenzen.
    pub terminals: Vec<Option<TerminalId>>,
    /// Signale.
    pub signals: Vec<SignalId>,
    /// Gleisfreimeldeabschnitte.
    pub tvds: Vec<TvdId>,
    /// Bahnübergänge.
    pub lxs: Vec<LxId>,
    /// Grenzen in der Reihenfolge ihrer Netzenden.
    pub borders: Vec<CutBorder>,
}

fn invalid(error: TopologyError) -> Report {
    Report {
        errors: alloc::vec![error],
        warnings: Vec::new(),
    }
}

impl Topology {
    /// Bildet das Teilnetz des Stellbereichs `area`.
    ///
    /// Grenzen heißen `>Nachbar` (bei mehreren Grenzen zum selben Nachbarn `>Nachbar/2`, …).
    /// Den Erlaubniswechsel beginnt beim Anlauf der Bereich mit der kleineren ID.
    ///
    /// # Errors
    /// [`Report`], wenn der Bereich unbekannt ist oder das Teilnetz ungültig wäre.
    #[allow(clippy::too_many_lines)]
    pub fn area_cut(&self, area: AreaId) -> Result<AreaCut, Report> {
        let unknown = || {
            invalid(TopologyError::UnknownId {
                kind: "Stellbereich",
                raw: area.raw(),
            })
        };
        let area_def = self.area(area).ok_or_else(unknown)?;
        let mut b = TopologyBuilder::new(format!("{}/{}", self.name(), area_def.name));
        let err = |e: TopologyError| invalid(e);

        let inside = |node: NodeRef| self.area_of_node(node) == Some(area);

        // Gleise und Weichen des Bereichs, in globaler Reihenfolge.
        let mut tracks = Vec::new();
        let mut track_map = BTreeMap::new();
        for (id, t) in self.tracks().filter(|(id, _)| inside(NodeRef::Track(*id))) {
            let local = b.add_track(t.name.clone(), t.length).map_err(err)?;
            track_map.insert(id, local);
            tracks.push(id);
        }
        let mut points = Vec::new();
        let mut point_map = BTreeMap::new();
        for (id, p) in self.points().filter(|(id, _)| inside(NodeRef::Point(*id))) {
            let local = b.add_point(p.name.clone()).map_err(err)?;
            if let Some(div) = p.diverging {
                b.set_point_diverging(local, div, p.diverging_speed)
                    .map_err(err)?;
            }
            point_map.insert(id, local);
            points.push(id);
        }
        let local_node = |node: NodeRef| match node {
            NodeRef::Track(t) => track_map.get(&t).map(|l| NodeRef::Track(*l)),
            NodeRef::Point(p) => point_map.get(&p).map(|l| NodeRef::Point(*l)),
            NodeRef::Terminal(_) => None,
        };

        // Netzenden des Gesamtnetzes, die an den Bereich anschließen.
        let mut terminals = Vec::new();
        let mut terminal_map = BTreeMap::new();
        for (id, t) in self
            .terminals()
            .filter(|(id, _)| inside(NodeRef::Terminal(*id)))
        {
            let local = b.add_terminal(t.name.clone(), t.kind).map_err(err)?;
            terminal_map.insert(id, local);
            terminals.push(Some(id));
        }

        // Verbindungen: innerhalb des Bereichs übernehmen, nach außen Grenzen anlegen.
        let mut borders = Vec::new();
        let mut per_neighbour: BTreeMap<AreaId, usize> = BTreeMap::new();
        let nodes: Vec<NodeRef> = tracks
            .iter()
            .map(|t| NodeRef::Track(*t))
            .chain(points.iter().map(|p| NodeRef::Point(*p)))
            .collect();
        for node in nodes {
            for &port in node.ports() {
                let here = PortRef::new(node, port);
                let Some(there) = self.linked(here) else {
                    continue;
                };
                let Some(local_here) = local_node(node).map(|n| PortRef::new(n, port)) else {
                    continue;
                };
                match there.node {
                    NodeRef::Terminal(t) => {
                        if let Some(&lt) = terminal_map.get(&t) {
                            b.connect(local_here, PortRef::new(NodeRef::Terminal(lt), Port::End))
                                .map_err(err)?;
                        }
                    }
                    other if inside(other) => {
                        // Jede innere Verbindung nur einmal anlegen.
                        if here < there
                            && let Some(n) = local_node(other)
                        {
                            b.connect(local_here, PortRef::new(n, there.port))
                                .map_err(err)?;
                        }
                    }
                    other => {
                        let neighbour = self.area_of_node(other).ok_or_else(unknown)?;
                        let neighbour_name = self
                            .area(neighbour)
                            .map(|a| a.name.clone())
                            .unwrap_or_default();
                        let count = per_neighbour.entry(neighbour).or_insert(0);
                        *count = count.saturating_add(1);
                        let mut name = if *count == 1 {
                            format!(">{neighbour_name}")
                        } else {
                            format!(">{neighbour_name}/{count}")
                        };
                        while b.lookup(&name).is_some() {
                            name.push('\'');
                        }
                        let initial = area < neighbour;
                        let lt = b
                            .add_terminal(name, TerminalKind::Border { initial })
                            .map_err(err)?;
                        b.connect(local_here, PortRef::new(NodeRef::Terminal(lt), Port::End))
                            .map_err(err)?;
                        terminals.push(None);
                        borders.push(CutBorder {
                            terminal: lt,
                            neighbour,
                            inside: here,
                            outside: there,
                        });
                    }
                }
            }
        }

        // Signale und Bahnübergänge auf den Gleisen des Bereichs.
        let mut signals = Vec::new();
        for (id, s) in self.signals() {
            let Some(&track) = track_map.get(&s.track) else {
                continue;
            };
            let local = b
                .add_signal(s.name.clone(), track, s.position, s.direction, s.kind)
                .map_err(err)?;
            b.set_signal_system(local, s.system).map_err(err)?;
            b.set_signal_auto(local, s.auto).map_err(err)?;
            signals.push(id);
        }
        let mut tvds = Vec::new();
        for (id, t) in self.tvds() {
            if self.area_of_tvd(id) != Some(area) {
                continue;
            }
            let members: Vec<TvdMember> = t
                .members
                .iter()
                .filter_map(|m| match *m {
                    TvdMember::Track(x) => track_map.get(&x).map(|l| TvdMember::Track(*l)),
                    TvdMember::Point(x) => point_map.get(&x).map(|l| TvdMember::Point(*l)),
                })
                .collect();
            b.add_tvd(t.name.clone(), &members).map_err(err)?;
            tvds.push(id);
        }
        let mut lxs = Vec::new();
        for (id, l) in self.level_crossings() {
            let Some(&track) = track_map.get(&l.track) else {
                continue;
            };
            b.add_level_crossing(l.name.clone(), track, l.position)
                .map_err(err)?;
            lxs.push(id);
        }

        for c in self.crossings() {
            let legs = c.legs.map(|(t, at)| track_map.get(&t).map(|l| (*l, at)));
            if let [Some(x), Some(y)] = legs {
                b.add_crossing(c.name.clone(), [x, y]).map_err(err)?;
            }
        }

        let topology = b.build()?.topology;
        Ok(AreaCut {
            area,
            topology,
            tracks,
            points,
            terminals,
            signals,
            tvds,
            lxs,
            borders,
        })
    }
}
