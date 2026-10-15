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
//! - Leuchtet an einem Halt zeigenden Signal das Ersatzsignal (Zs1), fährt er vorbei.
//! - Mit PZB-Fahrzeuggerät überwacht ihn die PZB 90 (REQ-MOV-007): Am 1000-Hz-Magneten
//!   („Halt erwarten“) muss der Triebfahrzeugführer binnen 4 s die Wachsamkeitstaste drücken,
//!   danach überwacht eine Kurve der Zugart die Geschwindigkeit; 250 m vor dem Halt zeigenden
//!   Hauptsignal prüft der 500-Hz-Magnet die Geschwindigkeit und eine zweite Kurve; der
//!   2000-Hz-Magnet am Halt zeigenden Hauptsignal bremst immer. Eine Verletzung löst die
//!   Zwangsbremsung aus; der Zug steht nach dem Bremsweg v²/2a.
//! - Bahnübergänge schließen mit einer Schließzeit; befährt ein Zug einen nicht
//!   geschlossenen Bahnübergang, meldet der Simulator das.

use std::collections::{HashSet, VecDeque};
use std::fmt;

use railit_kernel::{Aspect, Command, Input, LxReport, Occupancy, PointReport, SignalReport};
use railit_topology::{
    Direction, NodeRef, Port, PortRef, SignalKind, TerminalKind, Topology, TvdMember,
};
use railit_types::{
    Duration, Length, LxId, PointId, PointPosition, SignalId, Speed, Tick, TrackId, TvdId,
};
use serde::{Deserialize, Serialize};

/// Parameter des Simulators.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct SimConfig {
    /// Umstellzeit einer Weiche.
    pub point_move_time: Duration,
    /// Länge, mit der eine Weiche bei Zugfahrten gerechnet wird.
    pub point_length: Length,
    /// Schließzeit eines Bahnübergangs (Gelblicht, Rotlicht, Schranken).
    pub lx_close_time: Duration,
    /// Verzögerung bei einer Zwangsbremsung durch die PZB in mm/s².
    pub pzb_deceleration_mm_s2: u32,
    /// Zugart der PZB 90 (bestimmt die Überwachungskurven).
    pub pzb_class: PzbClass,
    /// Der Triebfahrzeugführer, der Halt missachtet, betätigt am 1000-Hz-Magneten die
    /// Wachsamkeitstaste (sonst Zwangsbremsung nach 4 s).
    pub pzb_acknowledge: bool,
}

/// Zugart der PZB 90 (REQ-MOV-007).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PzbClass {
    /// Obere Zugart (Vmax 165 km/h).
    O,
    /// Mittlere Zugart (Vmax 125 km/h).
    #[default]
    M,
    /// Untere Zugart (Vmax 105 km/h).
    U,
}

/// Überwachungswerte einer Zugart: Höchstgeschwindigkeit, Ziel und Dauer der
/// 1000-Hz-Kurve, Prüfgeschwindigkeit am 500-Hz-Magneten und Ziel der 500-Hz-Kurve.
struct PzbCurves {
    v_max: u64,
    v_1000: u64,
    t_1000_ms: u64,
    v_500_check: u64,
    v_500: u64,
}

impl PzbClass {
    const fn curves(self) -> PzbCurves {
        match self {
            Self::O => PzbCurves {
                v_max: 165,
                v_1000: 85,
                t_1000_ms: 23_000,
                v_500_check: 65,
                v_500: 45,
            },
            Self::M => PzbCurves {
                v_max: 125,
                v_1000: 70,
                t_1000_ms: 29_000,
                v_500_check: 50,
                v_500: 35,
            },
            Self::U => PzbCurves {
                v_max: 105,
                v_1000: 55,
                t_1000_ms: 38_000,
                v_500_check: 40,
                v_500: 25,
            },
        }
    }
}

/// Wirkweg der 1000-Hz-Überwachung (mm).
const PZB_1000_RANGE: u64 = 1_250_000;
/// Abstand des 500-Hz-Magneten vor dem Hauptsignal und Wirkweg seiner Überwachung (mm).
const PZB_500_AHEAD: u64 = 250_000;
/// Weg, auf dem die 500-Hz-Kurve auf ihr Ziel fällt (mm).
const PZB_500_CURVE: u64 = 153_000;
/// Frist für die Wachsamkeitstaste (ms).
const PZB_ACK_MS: u64 = 4_000;

impl Default for SimConfig {
    fn default() -> Self {
        Self {
            point_move_time: Duration::from_millis(4_000),
            point_length: Length::from_mm(30_000),
            lx_close_time: Duration::from_millis(8_000),
            pzb_deceleration_mm_s2: 1_000,
            pzb_class: PzbClass::M,
            pzb_acknowledge: true,
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
    /// ETCS: Notbremsung (Trip), wartet auf eine neue Fahrerlaubnis.
    Tripped,
    /// PZB: nach einer Zwangsbremsung zum Stehen gekommen.
    Braked,
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
    /// Zug mit ETCS-Fahrzeuggerät (Level 2): fährt nach der Fahrerlaubnis des RBC statt nach
    /// den Signalbegriffen.
    pub etcs: bool,
    /// Zug mit PZB-Fahrzeuggerät: Überfährt er einen wirksamen Gleismagneten, wird er
    /// zwangsgebremst (siehe [`Simulator`]).
    pub pzb: bool,
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
    /// Bahnübergang meldet Störung.
    LxDisturbed(LxId),
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
    /// Zug ist am Ersatzsignal (Zs1) vorbeigefahren.
    SubstitutePassed {
        /// Zug.
        train: String,
        /// Signal.
        signal: String,
    },
    /// Zug hat einen nicht geschlossenen Bahnübergang befahren.
    LevelCrossingNotClosed {
        /// Zug.
        train: String,
        /// Bahnübergang.
        lx: String,
    },
    /// ETCS: Notbremsung, weil die Fahrerlaubnis entzogen wurde.
    EmergencyBrake {
        /// Zug.
        train: String,
    },
    /// Zwei Züge stehen zugleich auf beiden Kreuzungsstellen einer Kreuzung: Zusammenstoß.
    CrossingCollision {
        /// Kreuzung.
        crossing: String,
        /// Zug auf der ersten Kreuzungsstelle.
        train: String,
        /// Zug auf der zweiten Kreuzungsstelle.
        other: String,
    },
    /// PZB: Zwangsbremsung an einem wirksamen Gleismagneten.
    PzbBrake {
        /// Zug.
        train: String,
        /// Signal mit dem Gleismagneten.
        signal: String,
        /// Frequenz des Gleismagneten (1000 Hz am Vorsignal, 500 Hz 250 m vor dem
        /// Hauptsignal, 2000 Hz am Hauptsignal).
        frequency_hz: u16,
        /// Auslöser: `magnet` (2000 Hz), `wachsamkeit` (Taste nicht betätigt) oder `kurve`
        /// (Überwachungskurve überschritten).
        cause: String,
        /// Geschwindigkeit bei Beginn der Bremsung.
        speed_kmh: u16,
    },
    /// PZB: Nach der Zwangsbremsung steht der Zug.
    PzbStopped {
        /// Zug.
        train: String,
        /// Bremsweg in Metern (ganzzahlig gerundet).
        braking_distance_m: u32,
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
    /// Zug mit ETCS-Fahrzeuggerät.
    pub etcs: bool,
    /// Zug mit PZB-Fahrzeuggerät.
    #[serde(default)]
    pub pzb: bool,
    /// Zuletzt passiertes Signal bzw. Signal, vor dem der Zug steht.
    pub reference: Option<String>,
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
    substitute: bool,
}

#[derive(Clone, Debug)]
struct SimLx {
    closed: bool,
    closing_until: Option<Tick>,
    disturbed: bool,
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
    /// Bezugspunkt: zuletzt passiertes Signal (`true`) bzw. Signal, vor dem der Zug steht.
    reference: (SignalId, bool),
    /// ETCS-Fahrzeuggerät mit den Signalen, die die Fahrerlaubnis freigibt (leer: keine
    /// Fahrerlaubnis); `None` bei Zügen ohne ETCS.
    cab: Option<HashSet<SignalId>>,
    /// PZB-Fahrzeuggerät.
    pzb: bool,
    /// Zwangsbremsung: aktuelle Geschwindigkeit in mm/s und bisheriger Bremsweg in mm.
    braking: Option<(u64, u64)>,
    /// Zurückgelegter Weg der Spitze (mm).
    odometer: u64,
    /// Wirksamer 1000-Hz-Magnet: Zeitpunkt (ms), Wegstand und Signal.
    pzb_1000: Option<(u64, u64, SignalId)>,
    /// Wirksamer 500-Hz-Magnet: Wegstand und Hauptsignal.
    pzb_500: Option<(u64, SignalId)>,
}

/// Simulierte Feldebene.
#[derive(Clone, Debug)]
pub struct Simulator {
    net: Topology,
    cfg: SimConfig,
    now: Tick,
    points: Vec<SimPoint>,
    signals: Vec<SimSignal>,
    lxs: Vec<SimLx>,
    tvd_faults: Vec<Option<Occupancy>>,
    trains: Vec<Train>,
    events: Vec<SimEvent>,
    /// Bereits gemeldete Zusammenstöße an Kreuzungen: Kreuzung und beide Züge.
    collisions: HashSet<(String, String, String)>,
}

const fn delimits(kind: SignalKind) -> bool {
    matches!(
        kind,
        SignalKind::Main | SignalKind::Combined | SignalKind::EtcsStopMarker
    )
}

/// Ob der Zug die Stelle `pos` (mm ab Port `a`) auf Gleis `track` überdeckt.
fn covers(t: &Train, track: TrackId, pos: u64) -> bool {
    if t.state == TrainState::Left {
        return false;
    }
    let mut remaining = t.length;
    for (k, seg) in t.path.iter().rev().enumerate() {
        let hi = if k == 0 { t.head.min(seg.len) } else { seg.len };
        let lo = hi.saturating_sub(remaining);
        if seg.node == NodeRef::Track(track) {
            let p = if travel(seg.entry) == Direction::AToB {
                pos
            } else {
                seg.len.saturating_sub(pos)
            };
            if (lo..=hi).contains(&p) {
                return true;
            }
        }
        remaining = remaining.saturating_sub(hi);
        if remaining == 0 {
            break;
        }
    }
    false
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
                defective: false,
                substitute: false
            };
            net.signals().len()
        ];
        let lxs = vec![
            SimLx {
                closed: false,
                closing_until: None,
                disturbed: false
            };
            net.level_crossings().len()
        ];
        let tvd_faults = vec![None; net.tvds().len()];
        Self {
            net,
            cfg,
            now: Tick::ZERO,
            points,
            signals,
            lxs,
            tvd_faults,
            trains: Vec::new(),
            events: Vec::new(),
            collisions: HashSet::new(),
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

    /// Leuchtet das Ersatzsignal?
    pub fn substitute(&self, signal: SignalId) -> bool {
        signal
            .index()
            .and_then(|i| self.signals.get(i))
            .is_some_and(|s| s.substitute)
    }

    /// Meldung eines Bahnübergangs.
    pub fn lx_report(&self, lx: LxId) -> Option<LxReport> {
        let l = self.lxs.get(lx.index()?)?;
        Some(if l.disturbed {
            LxReport::Disturbed
        } else if l.closing_until.is_some() {
            LxReport::Closing
        } else if l.closed {
            LxReport::Closed
        } else {
            LxReport::Open
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
        for (id, _) in self.net.level_crossings() {
            if let Some(report) = self.lx_report(id) {
                inputs.push(Input::LevelCrossing { lx: id, report });
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
                    // Gekoppelte Weichen hängen an einem Antrieb und laufen gemeinsam um
                    // (REQ-TOP-017).
                    let partner = self.net.coupled(point, position);
                    for (point, position) in core::iter::once((point, position)).chain(partner) {
                        if let Some(p) = point.index().and_then(|i| self.points.get_mut(i))
                            && !p.disturbed
                            && (p.position != position || p.moving.is_some())
                        {
                            p.moving = Some((position, until));
                        }
                    }
                }
                Command::SetSignal { signal, aspect } => {
                    if let Some(s) = signal.index().and_then(|i| self.signals.get_mut(i)) {
                        s.aspect = aspect;
                    }
                }
                Command::SetSubstitute { signal, on } => {
                    if let Some(s) = signal.index().and_then(|i| self.signals.get_mut(i))
                        && !s.defective
                    {
                        s.substitute = on;
                    }
                }
                Command::SetLevelCrossing { lx, close } => {
                    let until = self
                        .now
                        .checked_add(self.cfg.lx_close_time)
                        .unwrap_or(self.now);
                    if let Some(l) = lx.index().and_then(|i| self.lxs.get_mut(i))
                        && !l.disturbed
                    {
                        if close && !l.closed && l.closing_until.is_none() {
                            l.closing_until = Some(until);
                        } else if !close {
                            l.closed = false;
                            l.closing_until = None;
                        }
                    }
                }
                // Grenzmeldungen stellt die Runtime dem Nachbarkern zu.
                Command::Border { .. } => {}
                Command::ResetTvd(tvd) => {
                    // Achszählgrundstellung: Eine Störung des Zählpunkts wird aufgehoben.
                    if let Some(f) = tvd.index().and_then(|i| self.tvd_faults.get_mut(i))
                        && *f == Some(Occupancy::Disturbed)
                    {
                        *f = None;
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
            Fault::LxDisturbed(l) => {
                if let Some(l) = l.index().and_then(|i| self.lxs.get_mut(i)) {
                    l.disturbed = on;
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
            reference: (spec.signal, false),
            cab: spec.etcs.then(HashSet::new),
            pzb: spec.pzb,
            braking: None,
            odometer: 0,
            pzb_1000: None,
            pzb_500: None,
        });
        Ok(())
    }

    /// Positionsmeldungen der Züge mit ETCS-Fahrzeuggerät: (Zug, Bezugspunkt, passiert).
    pub fn etcs_reports(&self) -> Vec<(String, SignalId, bool)> {
        self.trains
            .iter()
            .filter(|t| t.cab.is_some() && t.state != TrainState::Left)
            .map(|t| (t.name.clone(), t.reference.0, t.reference.1))
            .collect()
    }

    /// Übergibt einem ETCS-Fahrzeuggerät die Signale, die seine Fahrerlaubnis freigibt; eine
    /// leere Liste: vor dem nächsten Signal halten. `None` ist die Notbremsung des RBC: Ein
    /// fahrender Zug hält sofort (Trip). Eine neue Fahrerlaubnis hebt den Trip auf.
    pub fn set_cab_authority(&mut self, train: &str, authority: Option<Vec<SignalId>>) {
        let Some(t) = self.trains.iter_mut().find(|t| t.name == train) else {
            return;
        };
        let Some(cab) = &mut t.cab else {
            return;
        };
        let has = authority.is_some();
        *cab = authority.unwrap_or_default().into_iter().collect();
        if !has && t.state == TrainState::Running {
            t.state = TrainState::Tripped;
            self.events.push(SimEvent::EmergencyBrake {
                train: t.name.clone(),
            });
        } else if has && t.state == TrainState::Tripped {
            t.state = TrainState::Waiting;
        }
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
        let speed_kmh = match t.braking {
            Some((v, _)) => u16::try_from(v * 36 / 10_000).unwrap_or(u16::MAX),
            None if t.state == TrainState::Braked => 0,
            None => t.speed.as_kmh(),
        };
        TrainInfo {
            name: t.name.clone(),
            state: t.state,
            speed_kmh,
            head_element,
            head_offset_m,
            occupied,
            etcs: t.cab.is_some(),
            pzb: t.pzb,
            reference: self.net.signal(t.reference.0).map(|s| s.name.clone()),
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
        for l in &mut self.lxs {
            if let Some(until) = l.closing_until
                && now >= until
            {
                l.closed = true;
                l.closing_until = None;
            }
        }
        let mut trains = std::mem::take(&mut self.trains);
        for t in &mut trains {
            self.move_train(t, dt);
        }
        trains.retain(|t| t.state != TrainState::Left);
        self.trains = trains;
        self.check_crossings();
    }

    /// Meldet Züge, die zugleich beide Kreuzungsstellen einer Kreuzung belegen.
    fn check_crossings(&mut self) {
        let mut found = Vec::new();
        for c in self.net.crossings() {
            let [(ta, pa), (tb, pb)] = c.legs;
            let on = |track: TrackId, pos: Length| {
                self.trains
                    .iter()
                    .filter(move |t| covers(t, track, u64::from(pos.as_mm())))
                    .map(|t| t.name.clone())
            };
            for a in on(ta, pa) {
                for b in on(tb, pb) {
                    if a != b {
                        found.push((c.name.clone(), a.clone(), b));
                    }
                }
            }
        }
        for key in found {
            if self.collisions.insert(key.clone()) {
                let (crossing, train, other) = key;
                self.events.push(SimEvent::CrossingCollision {
                    crossing,
                    train,
                    other,
                });
            }
        }
    }

    /// Nächstes Halt zeigendes Hauptsignal im aktuellen Element ab Position `head`.
    fn stop_ahead(
        &self,
        seg: &Seg,
        head: u64,
        cab: Option<&HashSet<SignalId>>,
    ) -> Option<(u64, SignalId)> {
        let NodeRef::Track(track) = seg.node else {
            return None;
        };
        let dir = travel(seg.entry);
        self.net
            .signals_on(track)
            .filter(|(id, s)| {
                s.direction == dir
                    && delimits(s.kind)
                    && match cab {
                        // ETCS: nur die Fahrerlaubnis zählt, nicht der Signalbegriff.
                        Some(authority) => !authority.contains(id),
                        None => {
                            self.signal_report(*id) != Some(SignalReport::Proceed)
                                && !self.substitute(*id)
                        }
                    }
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
            TrainState::Arrived
                | TrainState::Left
                | TrainState::Derailed
                | TrainState::Tripped
                | TrainState::Braked
        ) {
            return;
        }
        self.pzb_supervise(t);
        let mut remaining = self.step_distance(t, dt);

        loop {
            let Some(seg) = t.path.back().cloned() else {
                return;
            };
            let stop = if t.ignore_signals {
                None
            } else {
                self.stop_ahead(&seg, t.head, t.cab.as_ref())
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
                // Anfahren am Signal: Ist es das Ersatzsignal, das die Fahrt erlaubt?
                if let Some(s) = self.passing(t, &seg, t.head, t.head) {
                    t.reference = (s, true);
                }
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
                && let Some((offset, signal)) = self.stop_ahead(&seg, t.head, None)
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
            self.pzb_check(t, &seg, &mut remaining);
            let advance_by = remaining.min(limit.saturating_sub(t.head));
            if let Some(s) = self.passing(t, &seg, t.head, t.head.saturating_add(advance_by)) {
                t.reference = (s, true);
            }
            t.head += advance_by;
            t.odometer += advance_by;
            remaining -= advance_by;

            // Signal erreicht (auch am Gleisende) oder Strecke dieses Zyklus verbraucht.
            if t.head < seg.len || stop.is_some_and(|(offset, _)| offset == t.head) {
                continue;
            }
            // Hauptsignale stehen am Gleisende: Mit dem Übergang ins nächste Element ist der
            // Zug an ihnen vorbei.
            let at_end = self.signal_at_end(&seg);
            if !self.enter_next(t, &seg) {
                break;
            }
            if let Some(s) = at_end {
                t.reference = (s, true);
            }
        }
        if let Some((0, distance)) = t.braking
            && matches!(t.state, TrainState::Running | TrainState::Waiting)
        {
            t.braking = None;
            t.state = TrainState::Braked;
            self.events.push(SimEvent::PzbStopped {
                train: t.name.clone(),
                braking_distance_m: u32::try_from((distance + 500) / 1000).unwrap_or(u32::MAX),
            });
        }
        self.trim_tail(t);
    }

    /// Weg der Zugspitze in `dt` ms; bei Zwangsbremsung gleichmäßig verzögert bis zum
    /// Stillstand.
    fn step_distance(&self, t: &mut Train, dt: u64) -> u64 {
        match t.braking {
            Some((v, distance)) => {
                let a = u64::from(self.cfg.pzb_deceleration_mm_s2);
                let v1 = v.saturating_sub(a * dt / 1000);
                let step = (v + v1) * dt / 2000;
                t.braking = Some((v1, distance + step));
                step
            }
            None => u64::from(t.speed.as_kmh()) * dt * 10 / 36,
        }
    }

    /// PZB: Erreicht die Zugspitze einen wirksamen Gleismagneten, gilt er ab dort
    /// (REQ-MOV-007): 2000 Hz bremst sofort, 1000 Hz und 500 Hz beginnen ihre Überwachung.
    fn pzb_check(&mut self, t: &mut Train, seg: &Seg, remaining: &mut u64) {
        if !(t.pzb && t.ignore_signals && t.braking.is_none()) {
            return;
        }
        // 500 Hz: 250 m vor dem nächsten Hauptsignal, wenn es Halt zeigt.
        if let NodeRef::Track(track) = seg.node
            && let Some((signal, d)) = self.main_signal_ahead(track, travel(seg.entry), t.head)
            && d <= PZB_500_AHEAD
            && self.signal_report(signal) != Some(SignalReport::Proceed)
            && !self.substitute(signal)
            && t.pzb_500.is_none_or(|(_, s)| s != signal)
        {
            t.pzb_500 = Some((t.odometer, signal));
        }
        let Some((offset, signal, hz)) = self.pzb_magnet(seg, t.head, *remaining) else {
            return;
        };
        let delta = offset.saturating_sub(t.head);
        if let Some(s) = self.passing(t, seg, t.head, t.head.saturating_add(delta)) {
            t.reference = (s, true);
        }
        t.head = t.head.saturating_add(delta);
        t.odometer = t.odometer.saturating_add(delta);
        *remaining = remaining.saturating_sub(delta);
        if hz == 2000 {
            self.pzb_brake(t, signal, 2000, "magnet");
        } else if t.pzb_1000.is_none_or(|(_, _, s)| s != signal) {
            t.pzb_1000 = Some((self.now.as_millis(), t.odometer, signal));
        }
    }

    /// Überwachung der PZB 90: Wachsamkeitstaste und Kurven der Zugart.
    fn pzb_supervise(&mut self, t: &mut Train) {
        if !(t.pzb && t.ignore_signals && t.braking.is_none()) {
            return;
        }
        let c = self.cfg.pzb_class.curves();
        let v = u64::from(t.speed.as_kmh());
        if let Some((since, odo, signal)) = t.pzb_1000 {
            let elapsed = self.now.as_millis().saturating_sub(since);
            let limit = c.v_max - (c.v_max - c.v_1000) * elapsed.min(c.t_1000_ms) / c.t_1000_ms;
            if t.odometer.saturating_sub(odo) > PZB_1000_RANGE {
                t.pzb_1000 = None;
            } else if !self.cfg.pzb_acknowledge && elapsed >= PZB_ACK_MS {
                self.pzb_brake(t, signal, 1000, "wachsamkeit");
                return;
            } else if v > limit {
                self.pzb_brake(t, signal, 1000, "kurve");
                return;
            }
        }
        if let Some((odo, signal)) = t.pzb_500 {
            let s = t.odometer.saturating_sub(odo);
            let limit =
                c.v_500_check - (c.v_500_check - c.v_500) * s.min(PZB_500_CURVE) / PZB_500_CURVE;
            if s > PZB_500_AHEAD {
                t.pzb_500 = None;
            } else if v > limit {
                self.pzb_brake(t, signal, 500, "kurve");
            }
        }
    }

    fn pzb_brake(&mut self, t: &mut Train, signal: SignalId, hz: u16, cause: &str) {
        let v = u64::from(t.speed.as_kmh()) * 10_000 / 36;
        t.braking = Some((v, 0));
        self.events.push(SimEvent::PzbBrake {
            train: t.name.clone(),
            signal: self
                .net
                .signal(signal)
                .map(|s| s.name.clone())
                .unwrap_or_default(),
            frequency_hz: hz,
            cause: cause.into(),
            speed_kmh: t.speed.as_kmh(),
        });
    }

    /// REQ-MOV-007: Nächster wirksamer PZB-Gleismagnet im Element `seg` zwischen `head` und
    /// `head + ahead`: Position, Signal und Frequenz.
    ///
    /// - 2000 Hz am Hauptsignal, solange es Halt zeigt (ohne Ersatzsignal),
    /// - 1000 Hz am Vorsignal bzw. am Mehrabschnittssignal mit Fahrt, wenn das angekündigte
    ///   Hauptsignal Halt zeigt („Halt erwarten“).
    ///
    /// Wirksam wird ein Magnet nur für Züge, deren Triebfahrzeugführer Halt zeigende Signale
    /// überfährt (`ignore_signals`); wer die Signale beachtet, bremst rechtzeitig selbst. Der
    /// 500-Hz-Magnet 250 m vor dem Hauptsignal steht nicht im Netz; er gilt dort als
    /// vorhanden, wenn das Hauptsignal Halt zeigt.
    fn pzb_magnet(&self, seg: &Seg, head: u64, ahead: u64) -> Option<(u64, SignalId, u16)> {
        let NodeRef::Track(track) = seg.node else {
            return None;
        };
        let dir = travel(seg.entry);
        let stop = |id: SignalId| {
            self.signal_report(id) != Some(SignalReport::Proceed) && !self.substitute(id)
        };
        self.net
            .signals_on(track)
            .filter(|(_, s)| s.direction == dir)
            .filter_map(|(id, s)| {
                let pos = u64::from(s.position.as_mm());
                let offset = if dir == Direction::AToB {
                    pos
                } else {
                    seg.len.saturating_sub(pos)
                };
                if offset < head || offset > head.saturating_add(ahead) {
                    return None;
                }
                let warn = || {
                    self.main_signal_ahead(track, dir, offset)
                        .is_some_and(|(next, _)| stop(next))
                };
                let hz = match s.kind {
                    SignalKind::Main | SignalKind::Combined if stop(id) => 2000,
                    SignalKind::Combined | SignalKind::Distant if warn() => 1000,
                    _ => return None,
                };
                Some((offset, id, hz))
            })
            .min_by_key(|(offset, _, _)| *offset)
    }

    /// Nächstes Hauptsignal in Fahrtrichtung hinter Position `offset` (mm in Fahrtrichtung)
    /// auf `track`, über die aktuellen Weichenlagen hinweg; mit seiner Entfernung.
    fn main_signal_ahead(
        &self,
        track: TrackId,
        dir: Direction,
        offset: u64,
    ) -> Option<(SignalId, u64)> {
        let (mut track, mut dir, mut after) = (track, dir, Some(offset));
        let mut distance = 0u64;
        for _ in 0..64 {
            let len = u64::from(self.net.track(track)?.length.as_mm());
            let along = |pos: Length| {
                let pos = u64::from(pos.as_mm());
                if dir == Direction::AToB {
                    pos
                } else {
                    len.saturating_sub(pos)
                }
            };
            let start = after.unwrap_or(0);
            if let Some((id, o)) = self
                .net
                .signals_on(track)
                .filter(|(_, s)| s.direction == dir && delimits(s.kind))
                .map(|(id, s)| (id, along(s.position)))
                .filter(|(_, o)| after.is_none_or(|a| *o > a))
                .min_by_key(|(_, o)| *o)
            {
                return Some((id, distance + o.saturating_sub(start)));
            }
            distance += len.saturating_sub(start);
            // Weiter über die Weichen in ihrer aktuellen Lage.
            let exit = if dir == Direction::AToB {
                Port::B
            } else {
                Port::A
            };
            let mut port = PortRef::new(NodeRef::Track(track), exit);
            let next = loop {
                let entry = self.net.linked(port)?;
                match entry.node {
                    NodeRef::Track(t) => break (t, entry.port),
                    NodeRef::Point(p) => {
                        let pos = self.points.get(p.index()?)?.position;
                        let out = match entry.port {
                            Port::Tip if pos == PointPosition::Left => Port::Left,
                            Port::Tip => Port::Right,
                            _ => Port::Tip,
                        };
                        distance += u64::from(self.cfg.point_length.as_mm());
                        port = PortRef::new(entry.node, out);
                    }
                    NodeRef::Terminal(_) => return None,
                }
            };
            track = next.0;
            dir = travel(next.1);
            after = None;
        }
        None
    }

    /// Meldet, was die Zugspitze zwischen `from` (ausschließlich) und `to` (einschließlich)
    /// passiert: Bahnübergänge, die nicht geschlossen sind, und Ersatzsignale. Mit
    /// `from == to` (Anfahren aus dem Stand) zählt ein Signal genau an dieser Stelle.
    /// Gibt das zuletzt passierte Hauptsignal bzw. die zuletzt passierte Halt-Tafel zurück.
    fn passing(&mut self, t: &Train, seg: &Seg, from: u64, to: u64) -> Option<SignalId> {
        let NodeRef::Track(track) = seg.node else {
            return None;
        };
        let dir = travel(seg.entry);
        let offset = |pos: Length| {
            let pos = u64::from(pos.as_mm());
            if dir == Direction::AToB {
                pos
            } else {
                seg.len.saturating_sub(pos)
            }
        };
        let within = |o: u64| (o > from && o <= to) || (from == to && o == from);
        let mut events = Vec::new();
        let mut passed: Option<(u64, SignalId)> = None;
        for (id, l) in self.net.level_crossings_on(track) {
            let o = offset(l.position);
            if from < to && within(o) && self.lx_report(id) != Some(LxReport::Closed) {
                events.push(SimEvent::LevelCrossingNotClosed {
                    train: t.name.clone(),
                    lx: l.name.clone(),
                });
            }
        }
        // Am Signal steht die Spitze genau an dessen Position; vorbeigefahren ist sie, sobald
        // sie sich von dort weiterbewegt.
        for (id, s) in self.net.signals_on(track) {
            let o = offset(s.position);
            // Die Spitze steht am Signal und fährt an bzw. ist darüber hinweggefahren.
            let crossed = (o >= from && o < to) || (from == to && o == from);
            if s.direction == dir
                && delimits(s.kind)
                && crossed
                && passed.is_none_or(|(p, _)| o >= p)
            {
                passed = Some((o, id));
            }
            if s.direction == dir
                && delimits(s.kind)
                && within(o)
                && self.substitute(id)
                && self.signal_report(id) != Some(SignalReport::Proceed)
            {
                events.push(SimEvent::SubstitutePassed {
                    train: t.name.clone(),
                    signal: s.name.clone(),
                });
            }
        }
        self.events.extend(events);
        passed.map(|(_, id)| id)
    }

    /// Hauptsignal bzw. Halt-Tafel am Ende des Elements in Fahrtrichtung.
    fn signal_at_end(&self, seg: &Seg) -> Option<SignalId> {
        let NodeRef::Track(track) = seg.node else {
            return None;
        };
        let dir = travel(seg.entry);
        self.net
            .signals_on(track)
            .find(|(_, s)| {
                let pos = u64::from(s.position.as_mm());
                let offset = if dir == Direction::AToB {
                    pos
                } else {
                    seg.len.saturating_sub(pos)
                };
                s.direction == dir && delimits(s.kind) && offset == seg.len
            })
            .map(|(id, _)| id)
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
