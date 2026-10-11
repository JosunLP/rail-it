//! Aufbau und Validierung eines Netzes.

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use railit_types::{
    Length, LxId, PointId, PointPosition, SignalId, Speed, TerminalId, TrackId, TvdId,
};

use crate::error::{Report, TopologyError, Warning};
use crate::model::{
    DEFAULT_DIVERGING_SPEED, Direction, ElementRef, LevelCrossing, NodeRef, Point, PortRef, Signal,
    SignalKind, SignalSystem, Terminal, TerminalKind, Track, TvdMember, TvdSection,
};
use crate::topology::{Links, Topology};

/// Erfolgreich validiertes Netz samt Warnungen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Validated {
    /// Das Netz.
    pub topology: Topology,
    /// Auffälligkeiten, die keine Fehler sind.
    pub warnings: Vec<Warning>,
}

/// Schrittweiser Aufbau eines Netzes.
///
/// Lokale Fehler (doppelte Namen, ungültige Ports, …) werden sofort beim jeweiligen Aufruf
/// gemeldet; das Element wird dann nicht übernommen. Globale Regeln (offene Ports,
/// Gleisfreimeldung) prüft [`Self::build`].
///
/// ```
/// use railit_topology::*;
///
/// let mut b = TopologyBuilder::new("Mini");
/// let t = b.add_track("1", Length::from_mm(100_000))?;
/// let w = b.add_terminal("W", TerminalKind::Boundary)?;
/// let e = b.add_terminal("E", TerminalKind::BufferStop)?;
/// b.connect(PortRef::new(NodeRef::Terminal(w), Port::End), PortRef::new(NodeRef::Track(t), Port::A))?;
/// b.connect(PortRef::new(NodeRef::Track(t), Port::B), PortRef::new(NodeRef::Terminal(e), Port::End))?;
/// b.add_tvd("T1", &[TvdMember::Track(t)])?;
/// let net = b.build().map_err(|r| r.errors[0].clone())?.topology;
/// assert_eq!(net.tracks().len(), 1);
/// # Ok::<(), TopologyError>(())
/// ```
#[derive(Clone, Debug, Default)]
pub struct TopologyBuilder {
    name: String,
    tracks: Vec<Track>,
    points: Vec<Point>,
    terminals: Vec<Terminal>,
    signals: Vec<Signal>,
    tvds: Vec<TvdSection>,
    level_crossings: Vec<LevelCrossing>,
    links: Links,
    names: BTreeMap<String, ElementRef>,
}

impl TopologyBuilder {
    /// Leeres Netz mit dem Namen `name`.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    fn check_name(&self, name: &str) -> Result<(), TopologyError> {
        if self.names.contains_key(name) {
            return Err(TopologyError::DuplicateName {
                name: name.to_string(),
            });
        }
        Ok(())
    }

    /// Fügt einen Gleisabschnitt hinzu.
    ///
    /// # Errors
    /// Doppelter Name, Länge null oder zu viele Gleise.
    pub fn add_track(
        &mut self,
        name: impl Into<String>,
        length: Length,
    ) -> Result<TrackId, TopologyError> {
        let name = name.into();
        self.check_name(&name)?;
        if length == Length::ZERO {
            return Err(TopologyError::ZeroLength { track: name });
        }
        let id = TrackId::from_index(self.tracks.len())
            .ok_or(TopologyError::TooManyElements { kind: "Gleis" })?;
        self.names.insert(name.clone(), ElementRef::Track(id));
        self.tracks.push(Track { name, length });
        self.links.tracks.push([None; 2]);
        Ok(id)
    }

    /// Fügt eine Weiche hinzu.
    ///
    /// # Errors
    /// Doppelter Name oder zu viele Weichen.
    pub fn add_point(&mut self, name: impl Into<String>) -> Result<PointId, TopologyError> {
        let name = name.into();
        self.check_name(&name)?;
        let id = PointId::from_index(self.points.len())
            .ok_or(TopologyError::TooManyElements { kind: "Weiche" })?;
        self.names.insert(name.clone(), ElementRef::Point(id));
        self.points.push(Point {
            name,
            diverging: None,
            diverging_speed: DEFAULT_DIVERGING_SPEED,
        });
        self.links.points.push([None; 3]);
        Ok(id)
    }

    /// Fügt ein Netzende hinzu.
    ///
    /// # Errors
    /// Doppelter Name oder zu viele Netzenden.
    pub fn add_terminal(
        &mut self,
        name: impl Into<String>,
        kind: TerminalKind,
    ) -> Result<TerminalId, TopologyError> {
        let name = name.into();
        self.check_name(&name)?;
        let id = TerminalId::from_index(self.terminals.len())
            .ok_or(TopologyError::TooManyElements { kind: "Netzende" })?;
        self.names.insert(name.clone(), ElementRef::Terminal(id));
        self.terminals.push(Terminal { name, kind });
        self.links.terminals.push([None; 1]);
        Ok(id)
    }

    /// Fügt ein Signal auf Gleisabschnitt `track` hinzu.
    ///
    /// # Errors
    /// Doppelter Name, unbekanntes Gleis, Position außerhalb des Gleises oder zu viele Signale.
    pub fn add_signal(
        &mut self,
        name: impl Into<String>,
        track: TrackId,
        position: Length,
        direction: Direction,
        kind: SignalKind,
    ) -> Result<SignalId, TopologyError> {
        let name = name.into();
        self.check_name(&name)?;
        let t = self.track(track)?;
        if position > t.length {
            return Err(TopologyError::PositionOutsideTrack {
                object: name,
                track: t.name.clone(),
                position,
                length: t.length,
            });
        }
        let id = SignalId::from_index(self.signals.len())
            .ok_or(TopologyError::TooManyElements { kind: "Signal" })?;
        self.names.insert(name.clone(), ElementRef::Signal(id));
        self.signals.push(Signal {
            name,
            track,
            position,
            direction,
            kind,
            system: SignalSystem::Ks,
        });
        Ok(id)
    }

    /// Legt den abzweigenden Strang einer Weiche und die Geschwindigkeit darin fest.
    ///
    /// # Errors
    /// Unbekannte Weiche.
    pub fn set_point_diverging(
        &mut self,
        point: PointId,
        diverging: PointPosition,
        speed: Speed,
    ) -> Result<(), TopologyError> {
        let p =
            point
                .index()
                .and_then(|i| self.points.get_mut(i))
                .ok_or(TopologyError::UnknownId {
                    kind: "Weiche",
                    raw: point.raw(),
                })?;
        p.diverging = Some(diverging);
        p.diverging_speed = speed;
        Ok(())
    }

    /// Legt das Signalsystem eines Signals fest (Standard: Ks).
    ///
    /// # Errors
    /// Unbekanntes Signal.
    pub fn set_signal_system(
        &mut self,
        signal: SignalId,
        system: SignalSystem,
    ) -> Result<(), TopologyError> {
        let s = signal.index().and_then(|i| self.signals.get_mut(i)).ok_or(
            TopologyError::UnknownId {
                kind: "Signal",
                raw: signal.raw(),
            },
        )?;
        s.system = system;
        Ok(())
    }

    /// Fügt einen Bahnübergang auf Gleisabschnitt `track` hinzu.
    ///
    /// # Errors
    /// Doppelter Name, unbekanntes Gleis, Position außerhalb des Gleises oder zu viele
    /// Bahnübergänge.
    pub fn add_level_crossing(
        &mut self,
        name: impl Into<String>,
        track: TrackId,
        position: Length,
    ) -> Result<LxId, TopologyError> {
        let name = name.into();
        self.check_name(&name)?;
        let t = self.track(track)?;
        if position > t.length {
            return Err(TopologyError::PositionOutsideTrack {
                object: name,
                track: t.name.clone(),
                position,
                length: t.length,
            });
        }
        let id =
            LxId::from_index(self.level_crossings.len()).ok_or(TopologyError::TooManyElements {
                kind: "Bahnübergang",
            })?;
        self.names
            .insert(name.clone(), ElementRef::LevelCrossing(id));
        self.level_crossings.push(LevelCrossing {
            name,
            track,
            position,
        });
        Ok(id)
    }

    /// Fügt einen Gleisfreimeldeabschnitt hinzu.
    ///
    /// # Errors
    /// Doppelter Name, keine Elemente, unbekannte Elemente oder zu viele Abschnitte.
    pub fn add_tvd(
        &mut self,
        name: impl Into<String>,
        members: &[TvdMember],
    ) -> Result<TvdId, TopologyError> {
        let name = name.into();
        self.check_name(&name)?;
        if members.is_empty() {
            return Err(TopologyError::EmptyTvd { tvd: name });
        }
        for member in members {
            match *member {
                TvdMember::Track(id) => {
                    self.track(id)?;
                }
                TvdMember::Point(id) => {
                    self.point(id)?;
                }
            }
        }
        let mut members = members.to_vec();
        members.sort_unstable();
        members.dedup();
        let id = TvdId::from_index(self.tvds.len()).ok_or(TopologyError::TooManyElements {
            kind: "Gleisfreimeldeabschnitt",
        })?;
        self.names.insert(name.clone(), ElementRef::Tvd(id));
        self.tvds.push(TvdSection { name, members });
        Ok(id)
    }

    /// Verbindet zwei Ports.
    ///
    /// # Errors
    /// Unbekannter Knoten, ungültiger Port, Selbstverbindung oder bereits verbundener Port.
    pub fn connect(&mut self, a: PortRef, b: PortRef) -> Result<(), TopologyError> {
        for end in [a, b] {
            let element = self.node_name(end.node)?;
            match self.links.slot(end) {
                None => {
                    return Err(TopologyError::InvalidPort {
                        element,
                        port: end.port,
                    });
                }
                Some(Some(_)) => {
                    return Err(TopologyError::PortAlreadyConnected {
                        element,
                        port: end.port,
                    });
                }
                Some(None) => {}
            }
        }
        if a == b {
            return Err(TopologyError::SelfConnection {
                element: self.node_name(a.node)?,
                port: a.port,
            });
        }
        // Beide Slots existieren und sind frei (oben geprüft).
        if let Some(slot) = self.links.slot_mut(a) {
            *slot = Some(b);
        }
        if let Some(slot) = self.links.slot_mut(b) {
            *slot = Some(a);
        }
        Ok(())
    }

    /// Element mit dem fachlichen Namen `name`, sofern bereits hinzugefügt.
    pub fn lookup(&self, name: &str) -> Option<ElementRef> {
        self.names.get(name).copied()
    }

    /// Prüft alle globalen Regeln und erzeugt das unveränderliche Netz.
    ///
    /// # Errors
    /// [`Report`] mit **allen** gefundenen Fehlern (REQ-TOP-009).
    pub fn build(self) -> Result<Validated, Report> {
        let mut errors = Vec::new();

        // REQ-TOP-002: Jeder Port ist verbunden.
        for node in self.links.nodes() {
            for &port in node.ports() {
                if let Some(None) = self.links.slot(PortRef::new(node, port)) {
                    let element = self.node_name(node).unwrap_or_default();
                    errors.push(TopologyError::OpenPort { element, port });
                }
            }
        }

        // REQ-TOP-006: Jedes Gleis und jede Weiche in genau einem Gleisfreimeldeabschnitt.
        let mut track_tvds: Vec<Vec<TvdId>> = vec![Vec::new(); self.tracks.len()];
        let mut point_tvds: Vec<Vec<TvdId>> = vec![Vec::new(); self.points.len()];
        for (index, tvd) in self.tvds.iter().enumerate() {
            let Some(tvd_id) = TvdId::from_index(index) else {
                continue;
            };
            for member in &tvd.members {
                let list = match *member {
                    TvdMember::Track(id) => id.index().and_then(|i| track_tvds.get_mut(i)),
                    TvdMember::Point(id) => id.index().and_then(|i| point_tvds.get_mut(i)),
                };
                if let Some(list) = list {
                    list.push(tvd_id);
                }
            }
        }
        let names_of = |ids: &[TvdId]| -> Vec<String> {
            ids.iter()
                .filter_map(|id| id.index().and_then(|i| self.tvds.get(i)))
                .map(|t| t.name.clone())
                .collect()
        };
        let mut check_membership = |element: &str, tvds: &[TvdId]| match tvds {
            [] => errors.push(TopologyError::NotInTvd {
                element: element.to_string(),
            }),
            [_] => {}
            _ => errors.push(TopologyError::InMultipleTvds {
                element: element.to_string(),
                tvds: names_of(tvds),
            }),
        };
        for (track, tvds) in self.tracks.iter().zip(&track_tvds) {
            check_membership(&track.name, tvds);
        }
        for (point, tvds) in self.points.iter().zip(&point_tvds) {
            check_membership(&point.name, tvds);
        }

        // REQ-TOP-010: Zusammenhang (nur Warnung).
        let mut warnings = Vec::new();
        let components = self.links.components();
        if components > 1 {
            warnings.push(Warning::Disconnected { components });
        }

        if !errors.is_empty() {
            return Err(Report { errors, warnings });
        }

        let single = |tvds: Vec<Vec<TvdId>>| -> Vec<Option<TvdId>> {
            tvds.into_iter().map(|t| t.first().copied()).collect()
        };
        let topology = Topology {
            name: self.name,
            tracks: self.tracks,
            points: self.points,
            terminals: self.terminals,
            signals: self.signals,
            tvds: self.tvds,
            level_crossings: self.level_crossings,
            links: self.links,
            names: self.names,
            track_tvd: single(track_tvds),
            point_tvd: single(point_tvds),
        };
        Ok(Validated { topology, warnings })
    }

    fn track(&self, id: TrackId) -> Result<&Track, TopologyError> {
        id.index()
            .and_then(|i| self.tracks.get(i))
            .ok_or(TopologyError::UnknownId {
                kind: "Gleis",
                raw: id.raw(),
            })
    }

    fn point(&self, id: PointId) -> Result<&Point, TopologyError> {
        id.index()
            .and_then(|i| self.points.get(i))
            .ok_or(TopologyError::UnknownId {
                kind: "Weiche",
                raw: id.raw(),
            })
    }

    fn terminal(&self, id: TerminalId) -> Result<&Terminal, TopologyError> {
        id.index()
            .and_then(|i| self.terminals.get(i))
            .ok_or(TopologyError::UnknownId {
                kind: "Netzende",
                raw: id.raw(),
            })
    }

    fn node_name(&self, node: NodeRef) -> Result<String, TopologyError> {
        Ok(match node {
            NodeRef::Track(id) => self.track(id)?.name.clone(),
            NodeRef::Point(id) => self.point(id)?.name.clone(),
            NodeRef::Terminal(id) => self.terminal(id)?.name.clone(),
        })
    }
}
