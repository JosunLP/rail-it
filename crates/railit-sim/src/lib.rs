//! Simulator von rail-it.
//!
//! Bildet die Feldebene nach: Weichen mit Umstellzeit, Signale, Gleisfreimeldung und Züge,
//! die sich auf dem Netzmodell bewegen. Störungen lassen sich gezielt einspeisen.
//!
//! Der Simulator ist **nicht** sicherheitsrelevant. Er ist deterministisch (keine Uhr, kein
//! Zufall) und eignet sich damit für reproduzierbare Szenarien und Tests.
//!
//! Zugmodell (Phase 1):
//!
//! - Ein Zug ist eine Kette von Netzelementen (Gleise, Weichen) mit fester Länge.
//! - Er fährt mit konstanter Geschwindigkeit und hält vor Hauptsignalen, die Halt zeigen
//!   (auch defekte Signale gelten als Halt). Mit `ignore_signals` überfährt er sie – zum
//!   Prüfen der Schutzmechanismen.
//! - An Weichen folgt er der aktuellen Lage. Läuft die Weiche um, entgleist er; fährt er
//!   von einem Strang auf eine falsch liegende Weiche, schneidet er sie auf.
//! - An einer Netzgrenze verlässt er das Netz, am Prellbock bleibt er stehen.

use std::collections::VecDeque;
use std::fmt;

use railit_kernel::{Aspect, Command, Input, Occupancy, PointReport, SignalReport};
use railit_topology::{
    Direction, NodeRef, Port, PortRef, SignalKind, TerminalKind, Topology, TvdMember,
};
use railit_types::{Duration, Length, PointId, PointPosition, SignalId, Speed, Tick, TvdId};
use serde::{Deserialize, Serialize};

/// Parameter des Simulators.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct SimConfig {
    /// Umstellzeit einer Weiche.
    pub point_move_time: Duration,
    /// Länge, mit der eine Weiche bei Zugfahrten gerechnet wird.
    pub point_length: Length,
}

impl Default for SimConfig {
    fn default() -> Self {
        Self {
            point_move_time: Duration::from_millis(4_000),
            point_length: Length::from_mm(30_000),
        }
    }
}

/// Zustand eines Zuges.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrainState {
    /// Fährt.
    Running,
    /// Wartet vor einem Halt zeigenden Signal.
    Waiting,
    /// Steht am Prellbock.
    Arrived,
    /// Hat das Netz verlassen.
    Left,
    /// Entgleist.
    Derailed,
}

/// Neuer Zug.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrainSpec {
    /// Zugnummer.
    pub name: String,
    /// Der Zug steht mit der Spitze an diesem Signal, in dessen Wirkrichtung.
    pub signal: SignalId,
    /// Zuglänge.
    pub length: Length,
    /// Geschwindigkeit.
    pub speed: Speed,
    /// Überfährt Halt zeigende Signale (Störungsfall).
    pub ignore_signals: bool,
}

/// Einspeisbare Störung.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Fault {
    /// Weiche meldet Störung.
    PointDisturbed(PointId),
    /// Gleisfreimeldeabschnitt meldet Störung.
    TvdDisturbed(TvdId),
    /// Gleisfreimeldeabschnitt meldet besetzt, ohne dass ein Zug dort ist.
    TvdOccupied(TvdId),
    /// Signal ist defekt.
    SignalDefective(SignalId),
}

/// Ereignis des Simulators.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "event")]
pub enum SimEvent {
    /// Zug hält vor einem Signal.
    TrainStopped {
        /// Zug.
        train: String,
    },
    /// Zug fährt wieder an.
    TrainStarted {
        /// Zug.
        train: String,
    },
    /// Zug hat das Netz verlassen.
    TrainLeft {
        /// Zug.
        train: String,
    },
    /// Zug steht am Prellbock.
    TrainArrived {
        /// Zug.
        train: String,
    },
    /// Zug ist an einer umlaufenden oder gestörten Weiche entgleist.
    TrainDerailed {
        /// Zug.
        train: String,
        /// Weiche.
        point: String,
    },
    /// Zug hat eine falsch liegende Weiche aufgeschnitten.
    PointTrailed {
        /// Zug.
        train: String,
        /// Weiche.
        point: String,
    },
    /// Zug hat ein Halt zeigendes Signal überfahren.
    SignalPassedAtDanger {
        /// Zug.
        train: String,
        /// Signal.
        signal: String,
    },
}

/// Fehler beim Bedienen des Simulators.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SimError {
    /// Unbekanntes Signal.
    UnknownSignal,
    /// Zugnummer ist schon vergeben.
    DuplicateTrain(String),
    /// Der Zug passt nicht auf das Gleis vor dem Signal.
    TrainTooLong,
}

impl fmt::Display for SimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownSignal => f.write_str("unbekanntes Signal"),
            Self::DuplicateTrain(n) => write!(f, "Zug „{n}“ existiert bereits"),
            Self::TrainTooLong => f.write_str("Zug passt nicht auf das Gleis vor dem Signal"),
        }
    }
}

impl std::error::Error for SimError {}

/// Öffentliche Sicht auf einen Zug.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TrainInfo {
    /// Zugnummer.
    pub name: String,
    /// Zustand.
    pub state: TrainState,
    /// Geschwindigkeit in km/h.
    pub speed_kmh: u16,
    /// Element, auf dem die Zugspitze steht.
    pub head_element: String,
    /// Position der Spitze auf diesem Element in Fahrtrichtung, in Metern.
    pub head_offset_m: f64,
    /// Belegte Gleisfreimeldeabschnitte.
    pub occupied: Vec<String>,
}

#[derive(Clone, Debug)]
struct SimPoint {
    position: PointPosition,
    moving: Option<(PointPosition, Tick)>,
    disturbed: bool,
}

#[derive(Clone, Debug)]
struct SimSignal {
    aspect: Aspect,
    defective: bool,
}

/// Ein Element auf dem Weg eines Zuges.
#[derive(Clone, Debug)]
struct Seg {
    node: NodeRef,
    entry: Port,
    exit: Port,
    len: u64,
}

/// Außerhalb des Netzes (hinter einer Netzgrenze).
const OUTSIDE: u64 = u64::MAX / 4;

#[derive(Clone, Debug)]
struct Train {
    name: String,
    length: u64,
    speed: Speed,
    ignore_signals: bool,
    state: TrainState,
    path: VecDeque<Seg>,
    /// Position der Spitze im letzten Element, in mm ab dessen Eingang.
    head: u64,
}

/// Simulierte Feldebene.
#[derive(Clone, Debug)]
pub struct Simulator {
    net: Topology,
    cfg: SimConfig,
    now: Tick,
    points: Vec<SimPoint>,
    signals: Vec<SimSignal>,
    tvd_faults: Vec<Option<Occupancy>>,
    trains: Vec<Train>,
    events: Vec<SimEvent>,
}

const fn delimits(kind: SignalKind) -> bool {
    matches!(
        kind,
        SignalKind::Main | SignalKind::Combined | SignalKind::EtcsStopMarker
    )
}

const fn travel(entry: Port) -> Direction {
    if matches!(entry, Port::A) {
        Direction::AToB
    } else {
        Direction::BToA
    }
}

impl Simulator {
    /// Simulator für `net`; alle Weichen liegen rechts, alle Signale zeigen Halt.
    pub fn new(net: Topology, cfg: SimConfig) -> Self {
        let points = vec![
            SimPoint {
                position: PointPosition::Right,
                moving: None,
                disturbed: false
            };
            net.points().len()
        ];
        let signals = vec![
            SimSignal {
                aspect: Aspect::Stop,
                defective: false
            };
            net.signals().len()
        ];
        let tvd_faults = vec![None; net.tvds().len()];
        Self {
            net,
            cfg,
            now: Tick::ZERO,
            points,
            signals,
            tvd_faults,
            trains: Vec::new(),
            events: Vec::new(),
        }
    }

    /// Das simulierte Netz.
    pub fn topology(&self) -> &Topology {
        &self.net
    }

    /// Aktuelle Simulationszeit.
    pub fn now(&self) -> Tick {
        self.now
    }

    /// Setzt die Lage einer Weiche sofort (Anfangszustand).
    pub fn set_point_position(&mut self, point: PointId, position: PointPosition) {
        if let Some(p) = point.index().and_then(|i| self.points.get_mut(i)) {
            p.position = position;
            p.moving = None;
        }
    }

    /// Meldung einer Weiche.
    pub fn point_report(&self, point: PointId) -> Option<PointReport> {
        let p = self.points.get(point.index()?)?;
        Some(if p.disturbed {
            PointReport::Disturbed
        } else if p.moving.is_some() {
            PointReport::NoEndPosition
        } else {
            p.position.into()
        })
    }

    /// Meldung eines Signals.
    pub fn signal_report(&self, signal: SignalId) -> Option<SignalReport> {
        let s = self.signals.get(signal.index()?)?;
        Some(match (s.defective, s.aspect) {
            (true, _) => SignalReport::Defective,
            (false, Aspect::Stop) => SignalReport::Stop,
            (false, Aspect::Proceed) => SignalReport::Proceed,
        })
    }

    /// Meldung eines Gleisfreimeldeabschnitts.
    pub fn occupancy(&self, tvd: TvdId) -> Option<Occupancy> {
        if let Some(fault) = *self.tvd_faults.get(tvd.index()?)? {
            return Some(fault);
        }
        let occupied = self
            .trains
            .iter()
            .filter(|t| t.state != TrainState::Left)
            .any(|t| {
                t.path
                    .iter()
                    .any(|seg| self.node_tvd(seg.node) == Some(tvd))
            });
        Some(if occupied {
            Occupancy::Occupied
        } else {
            Occupancy::Clear
        })
    }

    /// Meldungen aller Feldelemente für den Kern.
    pub fn reports(&self) -> Vec<Input> {
        let mut inputs = Vec::new();
        for (id, _) in self.net.points() {
            if let Some(report) = self.point_report(id) {
                inputs.push(Input::Point { point: id, report });
            }
        }
        for (id, _) in self.net.tvds() {
            if let Some(occupancy) = self.occupancy(id) {
                inputs.push(Input::Tvd { tvd: id, occupancy });
            }
        }
        for (id, _) in self.net.signals() {
            if let Some(report) = self.signal_report(id) {
                inputs.push(Input::Signal { signal: id, report });
            }
        }
        inputs
    }

    /// Setzt Stellbefehle des Kerns um.
    pub fn apply(&mut self, commands: &[Command]) {
        for c in commands {
            match *c {
                Command::MovePoint { point, position } => {
                    let until = self
                        .now
                        .checked_add(self.cfg.point_move_time)
                        .unwrap_or(self.now);
                    if let Some(p) = point.index().and_then(|i| self.points.get_mut(i))
                        && !p.disturbed
                        && (p.position != position || p.moving.is_some())
                    {
                        p.moving = Some((position, until));
                    }
                }
                Command::SetSignal { signal, aspect } => {
                    if let Some(s) = signal.index().and_then(|i| self.signals.get_mut(i)) {
                        s.aspect = aspect;
                    }
                }
            }
        }
    }

    /// Speist eine Störung ein.
    pub fn inject(&mut self, fault: Fault) {
        self.set_fault(fault, true);
    }

    /// Hebt eine Störung auf.
    pub fn clear(&mut self, fault: Fault) {
        self.set_fault(fault, false);
    }

    fn set_fault(&mut self, fault: Fault, on: bool) {
        match fault {
            Fault::PointDisturbed(p) => {
                if let Some(p) = p.index().and_then(|i| self.points.get_mut(i)) {
                    p.disturbed = on;
                }
            }
            Fault::SignalDefective(s) => {
                if let Some(s) = s.index().and_then(|i| self.signals.get_mut(i)) {
                    s.defective = on;
                }
            }
            Fault::TvdDisturbed(t) | Fault::TvdOccupied(t) => {
                let value = match fault {
                    Fault::TvdDisturbed(_) => Occupancy::Disturbed,
                    _ => Occupancy::Occupied,
                };
                if let Some(f) = t.index().and_then(|i| self.tvd_faults.get_mut(i)) {
                    *f = on.then_some(value);
                }
            }
        }
    }

    /// Setzt einen Zug mit der Spitze an ein Signal.
    ///
    /// # Errors
    /// [`SimError`], wenn das Signal fehlt, der Name vergeben ist oder der Zug nicht auf das
    /// Gleis vor dem Signal passt.
    pub fn add_train(&mut self, spec: TrainSpec) -> Result<(), SimError> {
        if self.trains.iter().any(|t| t.name == spec.name) {
            return Err(SimError::DuplicateTrain(spec.name));
        }
        let signal = self
            .net
            .signal(spec.signal)
            .ok_or(SimError::UnknownSignal)?;
        let track = self
            .net
            .track(signal.track)
            .ok_or(SimError::UnknownSignal)?;
        let len = u64::from(track.length.as_mm());
        let (entry, exit, head) = match signal.direction {
            Direction::AToB => (Port::A, Port::B, u64::from(signal.position.as_mm())),
            Direction::BToA => (
                Port::B,
                Port::A,
                len.saturating_sub(u64::from(signal.position.as_mm())),
            ),
        };
        let length = u64::from(spec.length.as_mm());
        if length > head {
            return Err(SimError::TrainTooLong);
        }
        let seg = Seg {
            node: NodeRef::Track(signal.track),
            entry,
            exit,
            len,
        };
        self.trains.push(Train {
            name: spec.name,
            length,
            speed: spec.speed,
            ignore_signals: spec.ignore_signals,
            state: TrainState::Running,
            path: VecDeque::from([seg]),
            head,
        });
        Ok(())
    }

    /// Entfernt einen Zug.
    pub fn remove_train(&mut self, name: &str) -> bool {
        let before = self.trains.len();
        self.trains.retain(|t| t.name != name);
        before != self.trains.len()
    }

    /// Alle Züge.
    pub fn trains(&self) -> Vec<TrainInfo> {
        self.trains.iter().map(|t| self.train_info(t)).collect()
    }

    fn train_info(&self, t: &Train) -> TrainInfo {
        let head_element = t
            .path
            .back()
            .map(|s| match s.node {
                NodeRef::Terminal(_) => "außerhalb".to_string(),
                node => self
                    .net
                    .element_name(node.into())
                    .unwrap_or("?")
                    .to_string(),
            })
            .unwrap_or_default();
        let mut occupied: Vec<String> = Vec::new();
        if t.state != TrainState::Left {
            for seg in &t.path {
                if let Some(name) = self
                    .node_tvd(seg.node)
                    .and_then(|id| self.net.tvd(id))
                    .map(|t| t.name.clone())
                    && !occupied.contains(&name)
                {
                    occupied.push(name);
                }
            }
        }
        #[allow(clippy::cast_precision_loss)]
        let head_offset_m = t.head as f64 / 1000.0;
        TrainInfo {
            name: t.name.clone(),
            state: t.state,
            speed_kmh: t.speed.as_kmh(),
            head_element,
            head_offset_m,
            occupied,
        }
    }

    /// Ereignisse seit dem letzten Abruf.
    pub fn take_events(&mut self) -> Vec<SimEvent> {
        std::mem::take(&mut self.events)
    }

    fn node_tvd(&self, node: NodeRef) -> Option<TvdId> {
        match node {
            NodeRef::Track(t) => self.net.tvd_of(TvdMember::Track(t)),
            NodeRef::Point(p) => self.net.tvd_of(TvdMember::Point(p)),
            NodeRef::Terminal(_) => None,
        }
    }

    /// Schreitet bis `now` fort: Weichen laufen, Züge fahren.
    pub fn advance(&mut self, now: Tick) {
        let dt = now.saturating_since(self.now).as_millis();
        self.now = now;
        for p in &mut self.points {
            if let Some((target, until)) = p.moving
                && now >= until
            {
                p.position = target;
                p.moving = None;
            }
        }
        let mut trains = std::mem::take(&mut self.trains);
        for t in &mut trains {
            self.move_train(t, dt);
        }
        trains.retain(|t| t.state != TrainState::Left);
        self.trains = trains;
    }

    /// Nächstes Halt zeigendes Hauptsignal im aktuellen Element ab Position `head`.
    fn stop_ahead(&self, seg: &Seg, head: u64) -> Option<(u64, SignalId)> {
        let NodeRef::Track(track) = seg.node else {
            return None;
        };
        let dir = travel(seg.entry);
        self.net
            .signals_on(track)
            .filter(|(id, s)| {
                s.direction == dir
                    && delimits(s.kind)
                    && self.signal_report(*id) != Some(SignalReport::Proceed)
            })
            .map(|(id, s)| {
                let pos = u64::from(s.position.as_mm());
                let offset = if dir == Direction::AToB {
                    pos
                } else {
                    seg.len.saturating_sub(pos)
                };
                (offset, id)
            })
            .filter(|(offset, _)| *offset >= head)
            .min_by_key(|(offset, _)| *offset)
    }

    fn move_train(&mut self, t: &mut Train, dt: u64) {
        if matches!(
            t.state,
            TrainState::Arrived | TrainState::Left | TrainState::Derailed
        ) {
            return;
        }
        let mut remaining = u64::from(t.speed.as_kmh()) * dt * 10 / 36;

        loop {
            let Some(seg) = t.path.back().cloned() else {
                return;
            };
            let stop = if t.ignore_signals {
                None
            } else {
                self.stop_ahead(&seg, t.head)
            };

            if let Some((offset, _)) = stop
                && offset == t.head
            {
                if t.state != TrainState::Waiting {
                    t.state = TrainState::Waiting;
                    self.events.push(SimEvent::TrainStopped {
                        train: t.name.clone(),
                    });
                }
                break;
            }
            if t.state == TrainState::Waiting {
                t.state = TrainState::Running;
                self.events.push(SimEvent::TrainStarted {
                    train: t.name.clone(),
                });
            }
            if remaining == 0 {
                break;
            }

            let mut limit = seg.len;
            if let Some((offset, _)) = stop {
                limit = limit.min(offset);
            }
            if t.ignore_signals
                && let Some((offset, signal)) = self.stop_ahead(&seg, t.head)
                && offset <= t.head.saturating_add(remaining)
            {
                let name = self
                    .net
                    .signal(signal)
                    .map(|s| s.name.clone())
                    .unwrap_or_default();
                self.events.push(SimEvent::SignalPassedAtDanger {
                    train: t.name.clone(),
                    signal: name,
                });
            }
            let advance_by = remaining.min(limit.saturating_sub(t.head));
            t.head += advance_by;
            remaining -= advance_by;

            // Signal erreicht (auch am Gleisende) oder Strecke dieses Zyklus verbraucht.
            if t.head < seg.len || stop.is_some_and(|(offset, _)| offset == t.head) {
                continue;
            }
            if !self.enter_next(t, &seg) {
                break;
            }
        }
        self.trim_tail(t);
    }

    /// Betritt das nächste Element. `false`, wenn der Zug nicht weiterfahren kann.
    fn enter_next(&mut self, t: &mut Train, seg: &Seg) -> bool {
        if let NodeRef::Terminal(_) = seg.node {
            return false;
        }
        let Some(next) = self.net.linked(PortRef::new(seg.node, seg.exit)) else {
            t.state = TrainState::Arrived;
            return false;
        };
        let new_seg = match next.node {
            NodeRef::Terminal(id) => {
                if self.net.terminal(id).map(|x| x.kind) == Some(TerminalKind::BufferStop) {
                    t.state = TrainState::Arrived;
                    self.events.push(SimEvent::TrainArrived {
                        train: t.name.clone(),
                    });
                    return false;
                }
                Seg {
                    node: next.node,
                    entry: Port::End,
                    exit: Port::End,
                    len: OUTSIDE,
                }
            }
            NodeRef::Track(id) => {
                let len = self
                    .net
                    .track(id)
                    .map_or(0, |x| u64::from(x.length.as_mm()));
                let exit = if next.port == Port::A {
                    Port::B
                } else {
                    Port::A
                };
                Seg {
                    node: next.node,
                    entry: next.port,
                    exit,
                    len,
                }
            }
            NodeRef::Point(id) => {
                let name = self
                    .net
                    .point(id)
                    .map(|p| p.name.clone())
                    .unwrap_or_default();
                let Some(p) = id.index().and_then(|i| self.points.get_mut(i)) else {
                    return false;
                };
                let exit = if next.port == Port::Tip {
                    if p.moving.is_some() || p.disturbed {
                        t.state = TrainState::Derailed;
                        self.events.push(SimEvent::TrainDerailed {
                            train: t.name.clone(),
                            point: name,
                        });
                        return false;
                    }
                    if p.position == PointPosition::Left {
                        Port::Left
                    } else {
                        Port::Right
                    }
                } else {
                    let required = if next.port == Port::Left {
                        PointPosition::Left
                    } else {
                        PointPosition::Right
                    };
                    if p.position != required || p.moving.is_some() {
                        // Aufgeschnitten: Weiche wird in die befahrene Lage gedrückt und meldet
                        // danach eine Störung.
                        p.position = required;
                        p.moving = None;
                        p.disturbed = true;
                        self.events.push(SimEvent::PointTrailed {
                            train: t.name.clone(),
                            point: name,
                        });
                    }
                    Port::Tip
                };
                Seg {
                    node: next.node,
                    entry: next.port,
                    exit,
                    len: u64::from(self.cfg.point_length.as_mm()),
                }
            }
        };
        t.path.push_back(new_seg);
        t.head = 0;
        true
    }

    /// Entfernt Elemente, die vollständig hinter dem Zugschluss liegen.
    fn trim_tail(&mut self, t: &mut Train) {
        let mut behind: u64 = t.path.iter().rev().skip(1).map(|s| s.len).sum::<u64>() + t.head;
        while t.path.len() > 1 {
            let front = t.path.front().map_or(0, |s| s.len);
            if behind - front >= t.length {
                behind -= front;
                t.path.pop_front();
            } else {
                break;
            }
        }
        if let Some(only) = t.path.front()
            && t.path.len() == 1
            && matches!(only.node, NodeRef::Terminal(_))
            && t.head >= t.length
        {
            t.state = TrainState::Left;
            self.events.push(SimEvent::TrainLeft {
                train: t.name.clone(),
            });
        }
    }
}
