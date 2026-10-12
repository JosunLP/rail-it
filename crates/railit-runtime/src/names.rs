//! Zustand und Ereignisse mit fachlichen Namen statt IDs.

use railit_kernel::{
    Alarm, Aspect, AuxOperation, Event, LxReport, Occupancy, OwnedState, PointReport, Rejection,
    RoutePhase, SignalReport,
};
use railit_plan::{PlanView, RouteEnd};
use railit_types::{LxId, PointId, PointPosition, RouteId, SignalId, Tick, TvdId};
use serde::{Deserialize, Serialize};

fn point_name(plan: &PlanView<'_>, id: PointId) -> String {
    plan.point_name(id).unwrap_or("?").to_string()
}

fn signal_name(plan: &PlanView<'_>, id: SignalId) -> String {
    plan.signal_name(id).unwrap_or("?").to_string()
}

fn tvd_name(plan: &PlanView<'_>, id: TvdId) -> String {
    plan.tvd_name(id).unwrap_or("?").to_string()
}

fn route_name(plan: &PlanView<'_>, id: RouteId) -> String {
    plan.route_name(id).unwrap_or("?").to_string()
}

fn lx_name(plan: &PlanView<'_>, id: LxId) -> String {
    plan.lx_name(id).unwrap_or("?").to_string()
}

/// Art einer Hilfshandlung.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuxKind {
    /// Fahrstraßenhilfsauflösung (Element: Fahrstraße).
    ReleaseRoute,
    /// Achszählgrundstellung (Element: Abschnitt).
    ResetTvd,
    /// Ersatzsignal (Element: Fahrstraße).
    Substitute,
    /// Weiche im gestörten Abschnitt umstellen (Element: Weiche, dazu die Lage).
    ForcePoint,
}

/// Hilfshandlung mit fachlichen Namen.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamedAux {
    /// Art.
    pub kind: AuxKind,
    /// Betroffenes Element.
    pub element: String,
    /// Ziellage (nur bei `force_point`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<PointPosition>,
}

impl NamedAux {
    /// Übersetzt eine Hilfshandlung des Kerns.
    pub fn new(plan: &PlanView<'_>, op: AuxOperation) -> Self {
        let (kind, element, position) = match op {
            AuxOperation::ReleaseRoute(r) => (AuxKind::ReleaseRoute, route_name(plan, r), None),
            AuxOperation::ResetTvd(t) => (AuxKind::ResetTvd, tvd_name(plan, t), None),
            AuxOperation::Substitute(r) => (AuxKind::Substitute, route_name(plan, r), None),
            AuxOperation::ForcePoint(p, pos) => {
                (AuxKind::ForcePoint, point_name(plan, p), Some(pos))
            }
        };
        Self {
            kind,
            element,
            position,
        }
    }

    /// Löst die Namen gegen den Plan auf.
    ///
    /// # Errors
    /// Beschreibung, wenn das Element unbekannt ist oder die Lage fehlt.
    pub fn resolve(&self, plan: &PlanView<'_>) -> Result<AuxOperation, String> {
        let unknown = || format!("unbekanntes Element „{}“", self.element);
        Ok(match self.kind {
            AuxKind::ReleaseRoute => {
                AuxOperation::ReleaseRoute(plan.find_route(&self.element).ok_or_else(unknown)?)
            }
            AuxKind::ResetTvd => {
                AuxOperation::ResetTvd(plan.find_tvd(&self.element).ok_or_else(unknown)?)
            }
            AuxKind::Substitute => {
                AuxOperation::Substitute(plan.find_route(&self.element).ok_or_else(unknown)?)
            }
            AuxKind::ForcePoint => AuxOperation::ForcePoint(
                plan.find_point(&self.element).ok_or_else(unknown)?,
                self.position
                    .ok_or_else(|| "force_point braucht eine Lage (position)".to_string())?,
            ),
        })
    }
}

/// Ereignis mit Namen; Grundlage für API, CLI, Szenarien und Protokoll.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum NamedEvent {
    /// Anforderung angenommen.
    Accepted {
        /// Kennung der Anforderung.
        request: u64,
    },
    /// Anforderung abgelehnt.
    Rejected {
        /// Kennung der Anforderung.
        request: u64,
        /// Grund (z. B. `conflict`, `tvd_not_clear`).
        reason: String,
        /// Betroffenes Element, falls vorhanden.
        element: Option<String>,
    },
    /// Phasenwechsel einer Fahrstraße.
    RoutePhase {
        /// Fahrstraße.
        route: String,
        /// Neue Phase.
        phase: RoutePhase,
    },
    /// Störungs- oder Gefahrenmeldung.
    Alarm {
        /// Art (z. B. `signal_dropped`).
        alarm: String,
        /// Betroffenes Element, falls vorhanden.
        element: Option<String>,
    },
    /// Hilfshandlung angefordert, wartet auf Bestätigung.
    AuxPending {
        /// Kennung für die Bestätigung.
        aux: u32,
        /// Hilfshandlung.
        op: NamedAux,
        /// Anfordernder Bediener.
        operator: u32,
    },
    /// Hilfshandlung ausgeführt.
    AuxExecuted {
        /// Kennung.
        aux: u32,
        /// Hilfshandlung.
        op: NamedAux,
        /// Zählwerk nach der Ausführung.
        counter: u64,
    },
    /// Hilfshandlung verfallen.
    AuxExpired {
        /// Kennung.
        aux: u32,
    },
}

impl NamedEvent {
    /// Übersetzt ein Kernereignis.
    pub fn new(plan: &PlanView<'_>, event: &Event) -> Self {
        match *event {
            Event::Accepted(id) => Self::Accepted { request: id.0 },
            Event::Rejected(id, reason) => {
                let (reason, element) = match reason {
                    Rejection::UnknownElement => ("unknown_element", None),
                    Rejection::RouteActive => ("route_active", None),
                    Rejection::RouteNotActive => ("route_not_active", None),
                    Rejection::Conflict => ("conflict", None),
                    Rejection::TvdNotClear(t) => ("tvd_not_clear", Some(tvd_name(plan, t))),
                    Rejection::PointLocked(p) => ("point_locked", Some(point_name(plan, p))),
                    Rejection::PointNotOperable(p) => {
                        ("point_not_operable", Some(point_name(plan, p)))
                    }
                    Rejection::TrainInRoute => ("train_in_route", None),
                    Rejection::SafeState => ("safe_state", None),
                    Rejection::SignalHeld(s) => ("signal_held", Some(signal_name(plan, s))),
                    Rejection::AuxBusy => ("aux_busy", None),
                    Rejection::AuxUnknown => ("aux_unknown", None),
                    Rejection::AuxExpired => ("aux_expired", None),
                    Rejection::SameOperator => ("same_operator", None),
                    Rejection::AuxNotApplicable => ("aux_not_applicable", None),
                };
                Self::Rejected {
                    request: id.0,
                    reason: reason.into(),
                    element,
                }
            }
            Event::RoutePhase(r, phase) => Self::RoutePhase {
                route: route_name(plan, r),
                phase,
            },
            Event::Alarm(alarm) => {
                let (alarm, element) = match alarm {
                    Alarm::SettingTimeout(r) => ("setting_timeout", Some(route_name(plan, r))),
                    Alarm::SignalDropped(r) => ("signal_dropped", Some(route_name(plan, r))),
                    Alarm::TrainEnteredCancelledRoute(r) => {
                        ("train_entered_cancelled_route", Some(route_name(plan, r)))
                    }
                    Alarm::SignalDefective(s) => ("signal_defective", Some(signal_name(plan, s))),
                    Alarm::SignalMismatch(s) => ("signal_mismatch", Some(signal_name(plan, s))),
                    Alarm::UnknownElement => ("unknown_element", None),
                    Alarm::UnauthorizedOccupancy(t) => {
                        ("unauthorized_occupancy", Some(tvd_name(plan, t)))
                    }
                    Alarm::SignalPassedAtDanger(r) => {
                        ("signal_passed_at_danger", Some(route_name(plan, r)))
                    }
                };
                Self::Alarm {
                    alarm: alarm.into(),
                    element,
                }
            }
            Event::AuxPending { aux, op, operator } => Self::AuxPending {
                aux: aux.0,
                op: NamedAux::new(plan, op),
                operator: operator.0,
            },
            Event::AuxExecuted { aux, op, counter } => Self::AuxExecuted {
                aux: aux.0,
                op: NamedAux::new(plan, op),
                counter,
            },
            Event::AuxExpired(aux) => Self::AuxExpired { aux: aux.0 },
        }
    }
}

/// Zustand einer Weiche.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PointSnapshot {
    /// Name.
    pub name: String,
    /// Letzte Meldung.
    pub report: Option<PointReport>,
    /// Zuletzt befohlene Lage.
    pub commanded: Option<PointPosition>,
    /// Fahrstraße, die die Weiche im Fahrweg verschlossen hat.
    pub locked_by: Option<String>,
    /// Verschlossene Lage (Fahrweg, Durchrutschweg oder Flankenschutz).
    pub locked_position: Option<PointPosition>,
    /// Anzahl geteilter Verschlüsse (Durchrutschweg, Flankenschutz).
    pub holds: u8,
}

/// Zustand eines Signals.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SignalSnapshot {
    /// Name.
    pub name: String,
    /// Befohlener Begriff.
    pub commanded: Aspect,
    /// Letzte Meldung.
    pub report: Option<SignalReport>,
    /// Ersatzsignal (Zs1) eingeschaltet.
    pub substitute: bool,
    /// Anzahl der Fahrstraßen, die das Signal als Flankenschutz in Halt halten.
    pub flank_holds: u8,
}

/// Zustand eines Bahnübergangs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LxSnapshot {
    /// Name.
    pub name: String,
    /// Letzte Meldung.
    pub report: Option<LxReport>,
    /// Schließen befohlen.
    pub close_commanded: bool,
    /// Anzahl der Fahrstraßen, die ihn geschlossen halten.
    pub holds: u8,
}

/// Offene Hilfshandlung.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AuxSnapshot {
    /// Kennung.
    pub aux: u32,
    /// Hilfshandlung.
    pub op: NamedAux,
    /// Anfordernder Bediener.
    pub operator: u32,
    /// Zeitpunkt der Anforderung in Millisekunden.
    pub since_ms: u64,
}

/// Zustand eines Gleisfreimeldeabschnitts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TvdSnapshot {
    /// Name.
    pub name: String,
    /// Letzte Meldung.
    pub occupancy: Option<Occupancy>,
    /// Fahrstraße, die den Abschnitt im Fahrweg verschlossen hat.
    pub locked_by: Option<String>,
    /// Fahrstraße, deren Durchrutschweg den Abschnitt verschlossen hat.
    pub overlap_of: Option<String>,
}

/// Zustand einer Fahrstraße.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RouteSnapshot {
    /// Name.
    pub name: String,
    /// Startsignal.
    pub start: String,
    /// Ziel (Signal oder Netzende).
    pub end: String,
    /// Phase.
    pub phase: RoutePhase,
}

/// Zustandsbild des Stellwerks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Snapshot {
    /// Netz.
    pub network: String,
    /// Zeit seit Start in Millisekunden.
    pub time_ms: u64,
    /// Zyklusnummer.
    pub cycle: u64,
    /// Grund des sicheren Zustands, falls aktiv.
    pub safe_state: Option<String>,
    /// Weichen.
    pub points: Vec<PointSnapshot>,
    /// Signale.
    pub signals: Vec<SignalSnapshot>,
    /// Gleisfreimeldeabschnitte.
    pub tvds: Vec<TvdSnapshot>,
    /// Fahrstraßen.
    pub routes: Vec<RouteSnapshot>,
    /// Bahnübergänge.
    pub level_crossings: Vec<LxSnapshot>,
    /// Offene Hilfshandlungen.
    pub pending_aux: Vec<AuxSnapshot>,
    /// Zählwerk der ausgeführten Hilfshandlungen.
    pub aux_counter: u64,
}

impl Snapshot {
    /// Zustandsbild aus Plan und Kernzustand.
    pub fn new(plan: &PlanView<'_>, state: &OwnedState, now: Tick) -> Self {
        let owner = |r: Option<RouteId>| r.map(|r| route_name(plan, r));
        Self {
            network: plan.network_name().to_string(),
            time_ms: now.as_millis(),
            cycle: state.meta().cycle(),
            safe_state: state.meta().safe_state().map(|r| format!("{r:?}")),
            points: state
                .points()
                .iter()
                .enumerate()
                .filter_map(|(i, p)| Some((PointId::from_index(i)?, p)))
                .map(|(id, p)| PointSnapshot {
                    name: point_name(plan, id),
                    report: p.report(),
                    commanded: p.commanded(),
                    locked_by: owner(p.owner()),
                    locked_position: p.locked_position(),
                    holds: p.holds(),
                })
                .collect(),
            signals: state
                .signals()
                .iter()
                .enumerate()
                .filter_map(|(i, s)| Some((SignalId::from_index(i)?, s)))
                .map(|(id, s)| SignalSnapshot {
                    name: signal_name(plan, id),
                    commanded: s.commanded(),
                    report: s.report(),
                    substitute: s.substitute(),
                    flank_holds: s.stop_holds(),
                })
                .collect(),
            tvds: state
                .tvds()
                .iter()
                .enumerate()
                .filter_map(|(i, t)| Some((TvdId::from_index(i)?, t)))
                .map(|(id, t)| TvdSnapshot {
                    name: tvd_name(plan, id),
                    occupancy: t.occupancy(),
                    locked_by: owner(t.owner()),
                    overlap_of: owner(t.overlap_owner()),
                })
                .collect(),
            routes: plan
                .routes()
                .map(|r| RouteSnapshot {
                    name: r.name().to_string(),
                    start: signal_name(plan, r.start()),
                    end: match r.end() {
                        RouteEnd::Signal(s) => signal_name(plan, s),
                        RouteEnd::Boundary => "Netzgrenze".into(),
                        RouteEnd::BufferStop => "Prellbock".into(),
                    },
                    phase: r
                        .id()
                        .index()
                        .and_then(|i| state.routes().get(i))
                        .map(railit_kernel::RouteState::phase)
                        .unwrap_or_default(),
                })
                .collect(),
            level_crossings: state
                .level_crossings()
                .iter()
                .enumerate()
                .filter_map(|(i, l)| Some((LxId::from_index(i)?, l)))
                .map(|(id, l)| LxSnapshot {
                    name: lx_name(plan, id),
                    report: l.report(),
                    close_commanded: l.closed_commanded(),
                    holds: l.holds(),
                })
                .collect(),
            pending_aux: state
                .pending_aux()
                .map(|a| AuxSnapshot {
                    aux: a.id().0,
                    op: NamedAux::new(plan, a.op()),
                    operator: a.operator().0,
                    since_ms: a.since().as_millis(),
                })
                .collect(),
            aux_counter: state.meta().aux_counter(),
        }
    }
}
