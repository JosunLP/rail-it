//! Zustand und Ereignisse mit fachlichen Namen statt IDs.

use railit_kernel::{
    Alarm, Aspect, Event, Occupancy, OwnedState, PointReport, Rejection, RoutePhase, SignalReport,
};
use railit_plan::{PlanView, RouteEnd};
use railit_types::{PointId, PointPosition, RouteId, SignalId, Tick, TvdId};
use serde::Serialize;

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
                };
                Self::Alarm {
                    alarm: alarm.into(),
                    element,
                }
            }
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
    /// Verschließende Fahrstraße.
    pub locked_by: Option<String>,
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
}

/// Zustand eines Gleisfreimeldeabschnitts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TvdSnapshot {
    /// Name.
    pub name: String,
    /// Letzte Meldung.
    pub occupancy: Option<Occupancy>,
    /// Verschließende Fahrstraße.
    pub locked_by: Option<String>,
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
        }
    }
}
