//! Aufbau und Validierung eines Netzes.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use railit_types::{
    AreaId, Length, LxId, PointId, PointPosition, SignalId, Speed, StationId, TerminalId, TrackId,
    TvdId,
};

use crate::error::{Report, TopologyError, Warning};
use crate::model::{
    Area, Coupling, Crossing, DEFAULT_DIVERGING_SPEED, Direction, ElementRef, LevelCrossing,
    NodeRef, Platform, Point, Port, PortRef, Signal, SignalKind, SignalSystem, Slip, SlipKind,
    Station, Terminal, TerminalKind, Track, TvdMember, TvdSection,
};
use crate::topology::{Links, Topology};

/// Namen der Weichen und der inneren Gleise einer Kreuzungsweiche.
fn slip_names(name: &str, kind: SlipKind) -> (Vec<String>, Vec<String>) {
    let (letters, tracks): (&[&str], usize) = match kind {
        SlipKind::Double => (&["a", "b", "c", "d"], 4),
        SlipKind::Single => (&["a", "d"], 3),
    };
    (
        letters.iter().map(|l| format!("{name}{l}")).collect(),
        (1..=tracks).map(|i| format!("{name}.{i}")).collect(),
    )
}

/// Aufbau einer Kreuzungsweiche: (Weiche, Strang, Gleis, Gleisport). Weichen und Gleise in
/// der Reihenfolge von [`slip_names`]; links gerade, rechts in den Bogen. Gleise: `a`–`c`,
/// `b`–`d`, Bogen `a`–`d`, Bogen `b`–`c`.
fn slip_links(kind: SlipKind) -> &'static [(usize, Port, usize, Port)] {
    match kind {
        SlipKind::Double => &[
            (0, Port::Left, 0, Port::A),
            (0, Port::Right, 2, Port::A),
            (1, Port::Left, 1, Port::A),
            (1, Port::Right, 3, Port::A),
            (2, Port::Left, 0, Port::B),
            (2, Port::Right, 3, Port::B),
            (3, Port::Left, 1, Port::B),
            (3, Port::Right, 2, Port::B),
        ],
        SlipKind::Single => &[
            (0, Port::Left, 0, Port::A),
            (0, Port::Right, 2, Port::A),
            (1, Port::Left, 1, Port::B),
            (1, Port::Right, 2, Port::B),
        ],
    }
}

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
    crossings: Vec<Crossing>,
    couplings: Vec<Coupling>,
    slips: Vec<Slip>,
    links: Links,
    names: BTreeMap<String, ElementRef>,
    areas: Vec<Area>,
    stations: Vec<Station>,
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
        if self.names.contains_key(name)
            || self.crossings.iter().any(|c| c.name == name)
            || self.slips.iter().any(|x| x.name == name)
        {
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
            auto: false,
        });
        Ok(id)
    }

    /// Kennzeichnet ein Signal als Selbstblocksignal (siehe [`Signal::auto`]).
    ///
    /// # Errors
    /// Unbekanntes Signal.
    pub fn set_signal_auto(&mut self, signal: SignalId, auto: bool) -> Result<(), TopologyError> {
        let s = signal.index().and_then(|i| self.signals.get_mut(i)).ok_or(
            TopologyError::UnknownId {
                kind: "Signal",
                raw: signal.raw(),
            },
        )?;
        s.auto = auto;
        Ok(())
    }

    /// Fügt einen Stellbereich hinzu (REQ-TOP-011). Sind Stellbereiche festgelegt, muss
    /// jeder Gleisfreimeldeabschnitt genau einem angehören.
    ///
    /// # Errors
    /// Doppelter Name, unbekannte Abschnitte oder zu viele Bereiche.
    pub fn add_area(
        &mut self,
        name: impl Into<String>,
        tvds: &[TvdId],
    ) -> Result<AreaId, TopologyError> {
        let name = name.into();
        if self.areas.iter().any(|a| a.name == name) {
            return Err(TopologyError::DuplicateName { name });
        }
        for t in tvds {
            if t.index().and_then(|i| self.tvds.get(i)).is_none() {
                return Err(TopologyError::UnknownId {
                    kind: "Gleisfreimeldeabschnitt",
                    raw: t.raw(),
                });
            }
        }
        let mut tvds = tvds.to_vec();
        tvds.sort_unstable();
        tvds.dedup();
        let id = AreaId::from_index(self.areas.len()).ok_or(TopologyError::TooManyElements {
            kind: "Stellbereich",
        })?;
        self.areas.push(Area { name, tvds });
        Ok(id)
    }

    /// Fügt eine Betriebsstelle mit ihren Bahnsteiggleisen hinzu (REQ-TOP-012).
    ///
    /// # Errors
    /// Doppelter Name, unbekannte Signale oder zu viele Betriebsstellen.
    pub fn add_station(
        &mut self,
        name: impl Into<String>,
        platforms: &[(String, Vec<SignalId>)],
    ) -> Result<StationId, TopologyError> {
        let name = name.into();
        if self.stations.iter().any(|s| s.name == name) {
            return Err(TopologyError::DuplicateName { name });
        }
        let mut out = Vec::new();
        for (platform, signals) in platforms {
            for s in signals {
                if s.index().and_then(|i| self.signals.get(i)).is_none() {
                    return Err(TopologyError::UnknownId {
                        kind: "Signal",
                        raw: s.raw(),
                    });
                }
            }
            out.push(Platform {
                name: platform.clone(),
                signals: signals.clone(),
            });
        }
        let id =
            StationId::from_index(self.stations.len()).ok_or(TopologyError::TooManyElements {
                kind: "Betriebsstelle",
            })?;
        self.stations.push(Station {
            name,
            platforms: out,
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

    /// Fügt eine Kreuzung hinzu: die Kreuzungsstellen auf zwei Gleisen (Position ab Port `a`).
    ///
    /// # Errors
    /// Doppelter Name, unbekanntes Gleis oder Position außerhalb des Gleises. Dass beide
    /// Stellen im selben Gleisfreimeldeabschnitt liegen, prüft [`Self::build`].
    pub fn add_crossing(
        &mut self,
        name: impl Into<String>,
        legs: [(TrackId, Length); 2],
    ) -> Result<(), TopologyError> {
        let name = name.into();
        self.check_name(&name)?;
        for (track, position) in legs {
            let t = self.track(track)?;
            if position > t.length {
                return Err(TopologyError::PositionOutsideTrack {
                    object: name,
                    track: t.name.clone(),
                    position,
                    length: t.length,
                });
            }
        }
        self.crossings.push(Crossing { name, legs });
        Ok(())
    }

    /// Koppelt zwei Weichen an einen gemeinsamen Antrieb (REQ-TOP-017, siehe [`Coupling`]).
    ///
    /// # Errors
    /// Unbekannte Weiche, dieselbe Weiche zweimal oder eine Weiche, die schon gekoppelt ist.
    pub fn couple_points(
        &mut self,
        first: PointId,
        second: PointId,
        inverted: bool,
    ) -> Result<(), TopologyError> {
        let a = self.point(first)?.name.clone();
        let b = self.point(second)?.name.clone();
        let reason = if first == second {
            Some("mit sich selbst gekoppelt")
        } else if self
            .couplings
            .iter()
            .any(|c| c.points.contains(&first) || c.points.contains(&second))
        {
            Some("bereits mit einer anderen Weiche gekoppelt")
        } else {
            None
        };
        if let Some(reason) = reason {
            return Err(TopologyError::InvalidCoupling {
                points: [a, b],
                reason,
            });
        }
        self.couplings.push(Coupling {
            points: [first, second],
            inverted,
        });
        Ok(())
    }

    /// Fügt eine Kreuzungsweiche hinzu (REQ-TOP-017, siehe [`Slip`]): die Weichen
    /// `<Name>a` … `<Name>d` (EKW: `a`, `d`), die inneren Gleise `<Name>.1` … `<Name>.4` mit
    /// der Länge `length`, die Kreuzung `<Name>.x` in ihrer Mitte, die Kopplungen und den
    /// Gleisfreimeldeabschnitt `<Name>`. Geliefert werden die Ports der Enden `a` … `d`.
    ///
    /// # Errors
    /// Doppelter Name, Länge unter 2 mm oder zu viele Elemente.
    pub fn add_slip(
        &mut self,
        name: impl Into<String>,
        kind: SlipKind,
        length: Length,
        curve_speed: Speed,
    ) -> Result<[PortRef; 4], TopologyError> {
        let name = name.into();
        let (point_names, track_names) = slip_names(&name, kind);
        let crossing = format!("{name}.x");
        // Erst alle Namen prüfen, damit ein Fehler kein halbes Element hinterlässt.
        for n in core::iter::once(&name)
            .chain(&point_names)
            .chain(&track_names)
            .chain(core::iter::once(&crossing))
        {
            self.check_name(n)?;
        }
        let half = Length::from_mm(length.as_mm() / 2);
        if half == Length::ZERO {
            return Err(TopologyError::ZeroLength {
                track: track_names.first().cloned().unwrap_or_default(),
            });
        }
        let mut tracks = Vec::new();
        for n in &track_names {
            tracks.push(self.add_track(n.clone(), length)?);
        }
        let mut points = Vec::new();
        for n in &point_names {
            let p = self.add_point(n.clone())?;
            self.set_point_diverging(p, PointPosition::Right, curve_speed)?;
            points.push(p);
        }
        for &(p, pp, t, tp) in slip_links(kind) {
            let (Some(&p), Some(&t)) = (points.get(p), tracks.get(t)) else {
                return Err(TopologyError::TooManyElements { kind: "Weiche" });
            };
            self.connect(
                PortRef::new(NodeRef::Point(p), pp),
                PortRef::new(NodeRef::Track(t), tp),
            )?;
        }
        let (Some(&s1), Some(&s2)) = (tracks.first(), tracks.get(1)) else {
            return Err(TopologyError::TooManyElements { kind: "Gleis" });
        };
        self.add_crossing(crossing, [(s1, half), (s2, half)])?;
        let members: Vec<TvdMember> = tracks
            .iter()
            .map(|t| TvdMember::Track(*t))
            .chain(points.iter().map(|p| TvdMember::Point(*p)))
            .collect();
        self.add_tvd(name.clone(), &members)?;
        self.recognize_slip(&name, kind)?;
        self.slip_ends(&name)
            .ok_or_else(|| TopologyError::InvalidSlip {
                slip: name,
                reason: "nicht angelegt".into(),
            })
    }

    /// Erkennt eine Kreuzungsweiche aus bereits angelegten Elementen nach den Namen von
    /// [`Self::add_slip`] (Import aus railML), prüft ihren Aufbau, koppelt die Weichen und
    /// übernimmt sie.
    ///
    /// # Errors
    /// [`TopologyError::InvalidSlip`], wenn ein Element fehlt oder anders verbunden ist.
    pub fn recognize_slip(&mut self, name: &str, kind: SlipKind) -> Result<(), TopologyError> {
        let invalid = |reason: String| TopologyError::InvalidSlip {
            slip: name.into(),
            reason,
        };
        let (point_names, track_names) = slip_names(name, kind);
        if kind == SlipKind::Single {
            for extra in [format!("{name}b"), format!("{name}c"), format!("{name}.4")] {
                if self.lookup(&extra).is_some() {
                    return Err(invalid(format!(
                        "„{extra}“ gehört nicht zu einer einfachen Kreuzungsweiche"
                    )));
                }
            }
        }
        let mut points = Vec::new();
        for n in point_names {
            match self.lookup(&n) {
                Some(ElementRef::Point(p)) => points.push(p),
                _ => return Err(invalid(format!("Weiche „{n}“ fehlt"))),
            }
        }
        let mut tracks = Vec::new();
        for n in track_names {
            match self.lookup(&n) {
                Some(ElementRef::Track(t)) => tracks.push(t),
                _ => return Err(invalid(format!("Gleis „{n}“ fehlt"))),
            }
        }
        let point_port = |i: usize, port: Port| {
            points
                .get(i)
                .map(|p| PortRef::new(NodeRef::Point(*p), port))
                .ok_or_else(|| invalid("unvollständig".into()))
        };
        let track_port = |i: usize, port: Port| {
            tracks
                .get(i)
                .map(|t| PortRef::new(NodeRef::Track(*t), port))
                .ok_or_else(|| invalid("unvollständig".into()))
        };
        for &(p, pp, t, tp) in slip_links(kind) {
            let here = point_port(p, pp)?;
            if self.links.slot(here).copied().flatten() != Some(track_port(t, tp)?) {
                return Err(invalid(format!(
                    "Weiche „{}“: Strang {pp} ist nicht wie bei einer Kreuzungsweiche verbunden",
                    self.node_name(here.node).unwrap_or_default(),
                )));
            }
        }
        let straight = [track_port(0, Port::A)?.node, track_port(1, Port::A)?.node];
        let crossing = format!("{name}.x");
        let crossed = self.crossings.iter().any(|c| {
            let legs = c.legs.map(|(t, _)| NodeRef::Track(t));
            c.name == crossing && (legs == straight || legs == [straight[1], straight[0]])
        });
        if !crossed {
            return Err(invalid(format!(
                "Kreuzung „{crossing}“ der geraden Gleise fehlt"
            )));
        }
        let ends = if kind == SlipKind::Double {
            [
                point_port(0, Port::Tip)?,
                point_port(1, Port::Tip)?,
                point_port(2, Port::Tip)?,
                point_port(3, Port::Tip)?,
            ]
        } else {
            [
                point_port(0, Port::Tip)?,
                track_port(1, Port::A)?,
                track_port(0, Port::B)?,
                point_port(1, Port::Tip)?,
            ]
        };
        match points.as_slice() {
            [a, b, c, d] => {
                self.couple_points(*a, *b, true)?;
                self.couple_points(*c, *d, true)?;
            }
            [a, d] => self.couple_points(*a, *d, false)?,
            _ => {}
        }
        self.slips.push(Slip {
            name: name.into(),
            kind,
            ends,
            points,
            tracks,
        });
        Ok(())
    }

    /// Übernimmt eine Kreuzungsweiche, deren Elemente bereits angelegt sind (Teilnetz).
    pub(crate) fn record_slip(&mut self, slip: Slip) {
        self.slips.push(slip);
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

    /// Ports der Enden `a` … `d` der Kreuzungsweiche `name`, sofern bereits hinzugefügt.
    pub fn slip_ends(&self, name: &str) -> Option<[PortRef; 4]> {
        self.slips.iter().find(|s| s.name == name).map(|s| s.ends)
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

        self.check_crossings(&track_tvds, &mut errors);

        let tvd_area = self.check_areas_and_stations(&mut errors);
        self.check_couplings(&point_tvds, &tvd_area, &mut errors);

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
            crossings: self.crossings,
            couplings: self.couplings,
            slips: self.slips,
            links: self.links,
            names: self.names,
            track_tvd: single(track_tvds),
            point_tvd: single(point_tvds),
            areas: self.areas,
            stations: self.stations,
            tvd_area,
        };
        Ok(Validated { topology, warnings })
    }

    /// REQ-TOP-016: Die Kreuzungsstellen einer Kreuzung liegen auf verschiedenen Gleisen im
    /// selben Abschnitt.
    fn check_crossings(&self, track_tvds: &[Vec<TvdId>], errors: &mut Vec<TopologyError>) {
        for c in &self.crossings {
            let [(a, _), (b, _)] = c.legs;
            let tvd = |t: TrackId| {
                t.index()
                    .and_then(|i| track_tvds.get(i))
                    .and_then(|l| l.first().copied())
            };
            if a == b || tvd(a).is_none() || tvd(a) != tvd(b) {
                let name = |t: TrackId| {
                    tvd(t)
                        .and_then(TvdId::index)
                        .and_then(|i| self.tvds.get(i))
                        .map_or_else(
                            || {
                                format!(
                                    "Gleis „{}“",
                                    t.index()
                                        .and_then(|i| self.tracks.get(i))
                                        .map_or("?", |x| x.name.as_str())
                                )
                            },
                            |x| x.name.clone(),
                        )
                };
                errors.push(TopologyError::CrossingTvd {
                    crossing: c.name.clone(),
                    tvds: if a == b {
                        vec![format!(
                            "beide auf Gleis „{}“",
                            a.index()
                                .and_then(|i| self.tracks.get(i))
                                .map_or("?", |x| x.name.as_str())
                        )]
                    } else {
                        vec![name(a), name(b)]
                    },
                });
            }
        }
    }

    /// REQ-TOP-017: Gekoppelte Weichen gehören demselben Stellbereich an; ein Antrieb hängt
    /// an genau einem Sicherheitskern.
    fn check_couplings(
        &self,
        point_tvds: &[Vec<TvdId>],
        tvd_area: &[Option<AreaId>],
        errors: &mut Vec<TopologyError>,
    ) {
        if self.areas.is_empty() {
            return;
        }
        let area = |p: PointId| {
            p.index()
                .and_then(|i| point_tvds.get(i))
                .and_then(|l| l.first())
                .and_then(|t| t.index())
                .and_then(|i| tvd_area.get(i).copied().flatten())
        };
        for c in &self.couplings {
            let [a, b] = c.points;
            if area(a) != area(b) {
                let name = |p: PointId| {
                    p.index()
                        .and_then(|i| self.points.get(i))
                        .map(|x| x.name.clone())
                        .unwrap_or_default()
                };
                errors.push(TopologyError::InvalidCoupling {
                    points: [name(a), name(b)],
                    reason: "in verschiedenen Stellbereichen",
                });
            }
        }
    }

    /// REQ-TOP-011/012: Stellbereiche und Bahnsteige. Liefert den Bereich je Abschnitt.
    fn check_areas_and_stations(&self, errors: &mut Vec<TopologyError>) -> Vec<Option<AreaId>> {
        // REQ-TOP-011: Sind Stellbereiche festgelegt, gehört jeder Abschnitt genau einem an.
        let mut tvd_area: Vec<Option<AreaId>> = vec![None; self.tvds.len()];
        if !self.areas.is_empty() {
            let mut tvd_areas: Vec<Vec<AreaId>> = vec![Vec::new(); self.tvds.len()];
            for (index, area) in self.areas.iter().enumerate() {
                let Some(area_id) = AreaId::from_index(index) else {
                    continue;
                };
                if area.tvds.is_empty() {
                    errors.push(TopologyError::EmptyArea {
                        area: area.name.clone(),
                    });
                }
                for t in &area.tvds {
                    if let Some(list) = t.index().and_then(|i| tvd_areas.get_mut(i)) {
                        list.push(area_id);
                    }
                }
            }
            for ((tvd, areas), slot) in self.tvds.iter().zip(&tvd_areas).zip(&mut tvd_area) {
                match areas.as_slice() {
                    [] => errors.push(TopologyError::TvdNotInArea {
                        tvd: tvd.name.clone(),
                    }),
                    [single] => *slot = Some(*single),
                    many => errors.push(TopologyError::TvdInMultipleAreas {
                        tvd: tvd.name.clone(),
                        areas: many
                            .iter()
                            .filter_map(|a| a.index().and_then(|i| self.areas.get(i)))
                            .map(|a| a.name.clone())
                            .collect(),
                    }),
                }
            }
        }

        // REQ-TOP-012: Bahnsteige verweisen auf Signale, an denen Fahrstraßen enden.
        for station in &self.stations {
            for platform in &station.platforms {
                for s in &platform.signals {
                    if let Some(signal) = s.index().and_then(|i| self.signals.get(i))
                        && !signal.kind.delimits_routes()
                    {
                        errors.push(TopologyError::PlatformSignal {
                            station: station.name.clone(),
                            platform: platform.name.clone(),
                            signal: signal.name.clone(),
                        });
                    }
                }
            }
        }

        tvd_area
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
