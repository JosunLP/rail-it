//! Zustand und Ereignisse mit fachlichen Namen statt IDs.

use railit_kernel::{
    Alarm, Aspect, AuxOperation, Command, Event, LxReport, Occupancy, OwnedState, PointReport,
    Rejection, RoutePhase, SignalReport,
};
use railit_plan::{PlanView, RouteEnd};
use railit_types::{BorderId, LxId, PointId, PointPosition, RouteId, SignalId, Tick, TvdId};
use serde::{Deserialize, Serialize};
use std::fmt;

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

fn border_name(plan: &PlanView<'_>, id: BorderId) -> String {
    plan.border_name(id).unwrap_or("?").to_string()
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
    /// Erlaubnis an einer Bereichsgrenze zurückholen (Element: Grenze).
    TakePermission,
}

impl AuxKind {
    /// Name wie in API und Szenarien.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ReleaseRoute => "release_route",
            Self::ResetTvd => "reset_tvd",
            Self::Substitute => "substitute",
            Self::ForcePoint => "force_point",
            Self::TakePermission => "take_permission",
        }
    }
}

/// Prüfcode einer offenen Hilfshandlung für die sichere Anzeige- und Bestätigungskette
/// (REQ-UI-003).
///
/// Der Bedienplatz berechnet den Code aus dem, was er anzeigt, der Dienst aus der offenen
/// Hilfshandlung des Kerns; nur bei Gleichheit wird die Bestätigung weitergegeben. Kanonischer
/// Text (UTF-8, Zeilen mit `\n` getrennt):
///
/// ```text
/// railit-aux-v1
/// <Stellbereich>
/// <Kennung>
/// <Art>            z. B. release_route
/// <Element>        z. B. A-N1
/// <Lage>           left, right oder leer
/// ```
///
/// Der Code sind die ersten drei Bytes des SHA-256 als sechs Großbuchstaben-Hexziffern.
pub fn aux_check(area: &str, aux: u32, op: &NamedAux) -> String {
    use sha2::{Digest, Sha256};
    let position = match op.position {
        Some(PointPosition::Left) => "left",
        Some(PointPosition::Right) => "right",
        None => "",
    };
    let text = format!(
        "railit-aux-v1\n{area}\n{aux}\n{}\n{}\n{position}",
        op.kind.as_str(),
        op.element
    );
    let digest = Sha256::digest(text.as_bytes());
    digest
        .iter()
        .take(3)
        .fold(String::with_capacity(6), |mut s, b| {
            use std::fmt::Write as _;
            let _ = write!(s, "{b:02X}");
            s
        })
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
        Self::in_area(plan, "", op)
    }

    /// Wie [`Self::new`]; Grenzen werden mit dem Bereichsnamen `area` adressiert
    /// (z. B. `"Adorf>Strecke"`).
    pub fn in_area(plan: &PlanView<'_>, area: &str, op: AuxOperation) -> Self {
        let (kind, element, position) = match op {
            AuxOperation::ReleaseRoute(r) => (AuxKind::ReleaseRoute, route_name(plan, r), None),
            AuxOperation::ResetTvd(t) => (AuxKind::ResetTvd, tvd_name(plan, t), None),
            AuxOperation::Substitute(r) => (AuxKind::Substitute, route_name(plan, r), None),
            AuxOperation::ForcePoint(p, pos) => {
                (AuxKind::ForcePoint, point_name(plan, p), Some(pos))
            }
            AuxOperation::TakePermission(b) => (
                AuxKind::TakePermission,
                format!("{area}{}", border_name(plan, b)),
                None,
            ),
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
            AuxKind::TakePermission => {
                AuxOperation::TakePermission(plan.find_border(&self.element).ok_or_else(unknown)?)
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
    /// Erlaubnis an einer Bereichsgrenze übernommen oder abgegeben.
    Permission {
        /// Grenze.
        border: String,
        /// Hält der Bereich die Erlaubnis jetzt?
        holding: bool,
    },
}

/// Stellbefehl mit Namen (Diagnose, Journal).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum NamedCommand {
    /// Weiche umstellen.
    MovePoint {
        /// Weiche.
        point: String,
        /// Lage.
        position: PointPosition,
    },
    /// Signalbegriff.
    SetSignal {
        /// Signal.
        signal: String,
        /// Begriff.
        aspect: Aspect,
    },
    /// Ersatzsignal ein oder aus.
    SetSubstitute {
        /// Signal.
        signal: String,
        /// Eingeschaltet?
        on: bool,
    },
    /// Bahnübergang schließen oder öffnen.
    SetLevelCrossing {
        /// Bahnübergang.
        lx: String,
        /// Schließen?
        close: bool,
    },
    /// Grundstellung eines Abschnitts (Achszähler).
    ResetTvd {
        /// Abschnitt.
        tvd: String,
    },
    /// Meldung an den Nachbarbereich.
    Border {
        /// Grenze.
        border: String,
        /// Der Bereich hält die Erlaubnis.
        holding: bool,
        /// Der Bereich bietet die Erlaubnis an.
        offer: bool,
        /// Der Bereich fordert die Erlaubnis an.
        request: bool,
    },
}

impl NamedCommand {
    /// Übersetzt einen Stellbefehl mit den Namen aus `plan`.
    pub fn new(plan: &PlanView<'_>, command: &Command) -> Self {
        let id = |name: Option<&str>, raw: Option<usize>| {
            name.map_or_else(|| format!("#{}", raw.unwrap_or(usize::MAX)), str::to_string)
        };
        match *command {
            Command::MovePoint { point, position } => Self::MovePoint {
                point: id(plan.point_name(point), point.index()),
                position,
            },
            Command::SetSignal { signal, aspect } => Self::SetSignal {
                signal: id(plan.signal_name(signal), signal.index()),
                aspect,
            },
            Command::SetSubstitute { signal, on } => Self::SetSubstitute {
                signal: id(plan.signal_name(signal), signal.index()),
                on,
            },
            Command::SetLevelCrossing { lx, close } => Self::SetLevelCrossing {
                lx: id(plan.lx_name(lx), lx.index()),
                close,
            },
            Command::ResetTvd(tvd) => Self::ResetTvd {
                tvd: id(plan.tvd_name(tvd), tvd.index()),
            },
            Command::Border { border, message } => Self::Border {
                border: border_name(plan, border),
                holding: message.holding,
                offer: message.offer,
                request: message.request,
            },
        }
    }
}

impl fmt::Display for NamedCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MovePoint { point, position } => {
                let p = match position {
                    PointPosition::Left => "links",
                    PointPosition::Right => "rechts",
                };
                write!(f, "Weiche {point} nach {p}")
            }
            Self::SetSignal { signal, aspect } => {
                let a = match aspect {
                    Aspect::Stop => "Halt",
                    Aspect::Proceed => "Fahrt",
                };
                write!(f, "Signal {signal} auf {a}")
            }
            Self::SetSubstitute { signal, on } => {
                write!(
                    f,
                    "Ersatzsignal {signal} {}",
                    if *on { "ein" } else { "aus" }
                )
            }
            Self::SetLevelCrossing { lx, close } => write!(
                f,
                "Bahnübergang {lx} {}",
                if *close { "schließen" } else { "öffnen" }
            ),
            Self::ResetTvd { tvd } => write!(f, "Grundstellung {tvd}"),
            Self::Border {
                border,
                holding,
                offer,
                request,
            } => write!(
                f,
                "Grenze {border}: hält {holding}, bietet an {offer}, fordert an {request}"
            ),
        }
    }
}

impl NamedEvent {
    /// Übersetzt ein Kernereignis. Grenzen werden mit dem Bereichsnamen `area` adressiert
    /// (leer bei Netzen ohne Stellbereiche).
    pub fn new(plan: &PlanView<'_>, area: &str, event: &Event) -> Self {
        let border_name = |b: BorderId| format!("{area}{}", border_name(plan, b));
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
                    Rejection::AuxMismatch => ("aux_mismatch", None),
                    Rejection::AuxNotApplicable => ("aux_not_applicable", None),
                    Rejection::NoPermission(b) => ("no_permission", Some(border_name(b))),
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
                    Alarm::BorderLost(b) => ("border_lost", Some(border_name(b))),
                    Alarm::PermissionConflict(b) => ("permission_conflict", Some(border_name(b))),
                };
                Self::Alarm {
                    alarm: alarm.into(),
                    element,
                }
            }
            Event::AuxPending { aux, op, operator } => Self::AuxPending {
                aux: aux.0,
                op: NamedAux::in_area(plan, area, op),
                operator: operator.0,
            },
            Event::AuxExecuted { aux, op, counter } => Self::AuxExecuted {
                aux: aux.0,
                op: NamedAux::in_area(plan, area, op),
                counter,
            },
            Event::AuxExpired(aux) => Self::AuxExpired { aux: aux.0 },
            Event::Permission { border, holding } => Self::Permission {
                border: border_name(border),
                holding,
            },
        }
    }
}

/// Zustand einer Weiche.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PointSnapshot {
    /// Stellbereich.
    pub area: String,
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
    /// Stellbereich.
    pub area: String,
    /// Name.
    pub name: String,
    /// Befohlener Begriff.
    pub commanded: Aspect,
    /// Letzte Meldung.
    pub report: Option<SignalReport>,
    /// Ersatzsignal (Zs1) eingeschaltet.
    pub substitute: bool,
    /// Signalbild nach Signalsystem, z. B. `Ks1 Zs3(4)` oder `Hp2 Vr0`.
    pub picture: String,
    /// Signalbild strukturiert.
    pub aspects: Option<railit_movement::Picture>,
    /// Anzahl der Fahrstraßen, die das Signal als Flankenschutz in Halt halten.
    pub flank_holds: u8,
}

/// Zustand eines Bahnübergangs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LxSnapshot {
    /// Stellbereich.
    pub area: String,
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
    /// Stellbereich.
    pub area: String,
    /// Kennung.
    pub aux: u32,
    /// Hilfshandlung.
    pub op: NamedAux,
    /// Anfordernder Bediener.
    pub operator: u32,
    /// Zeitpunkt der Anforderung in Millisekunden.
    pub since_ms: u64,
    /// Prüfcode für die Bestätigung, siehe [`aux_check`].
    pub check: String,
}

/// Zustand eines Gleisfreimeldeabschnitts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TvdSnapshot {
    /// Stellbereich.
    pub area: String,
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
    /// Stellbereich.
    pub area: String,
    /// Name.
    pub name: String,
    /// Startsignal.
    pub start: String,
    /// Ziel (Signal oder Netzende).
    pub end: String,
    /// Phase.
    pub phase: RoutePhase,
}

/// Zustand einer Grenze zu einem Nachbarbereich.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct BorderSnapshot {
    /// Adresse, z. B. `"Adorf>Strecke"`.
    pub name: String,
    /// Hält der Bereich die Erlaubnis?
    pub holding: bool,
    /// Laufende Nummer der Erlaubnis.
    pub seq: u32,
    /// Abgegeben und angeboten, noch nicht quittiert.
    pub offering: bool,
    /// Erlaubnis angefordert.
    pub requested: bool,
    /// Annäherungsabschnitt verschlossen (Züge des Nachbarn dürfen kommen).
    pub approach_locked: bool,
    /// Letzte Meldung des Nachbarn: hält er die Erlaubnis?
    pub peer_holding: Option<bool>,
    /// Letzte Meldung des Nachbarn: ist sein Annäherungsabschnitt frei?
    pub peer_approach_clear: Option<bool>,
    /// Letzte Meldung des Nachbarn: ist sein Annäherungsabschnitt nur für uns verschlossen?
    pub peer_exclusive: Option<bool>,
    /// Zeitpunkt der letzten Meldung des Nachbarn in Millisekunden.
    pub peer_at_ms: Option<u64>,
}

/// Zustand eines Stellbereichs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AreaSnapshot {
    /// Name.
    pub name: String,
    /// Zyklusnummer des Kerns.
    pub cycle: u64,
    /// Grund des sicheren Zustands, falls aktiv.
    pub safe_state: Option<String>,
    /// Prüfsumme des Plans (Hex, gekürzt).
    pub plan_digest: String,
    /// Grenzen zu Nachbarbereichen.
    pub borders: Vec<BorderSnapshot>,
    /// Zählwerk der Hilfshandlungen.
    pub aux_counter: u64,
    /// Redundante Kanäle und Voter.
    pub safety: SafetySnapshot,
}

/// Zustand der Sicherheitsschicht eines Stellbereichs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct SafetySnapshot {
    /// Projektierte Redundanz.
    pub redundancy: railit_safety::Redundancy,
    /// Kanäle.
    pub channels: Vec<ChannelSnapshot>,
    /// Grund der Abschaltung, falls die Schicht im sicheren Zustand ist.
    pub trip: Option<railit_safety::Trip>,
    /// Anbindung des sicheren Rechners, falls der Bereich auf einem Knoten rechnet.
    pub node: Option<crate::remote::NodeSnapshot>,
}

/// Zustand eines Kanals.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ChannelSnapshot {
    /// Nummer (0, 1, 2).
    pub channel: usize,
    /// Wählt mit (sonst ausgeschlossen).
    pub active: bool,
}

impl SafetySnapshot {
    /// Zustandsbild aus dem Status der Schicht.
    pub fn new(status: &railit_safety::Status) -> Self {
        Self {
            redundancy: status.redundancy,
            channels: status
                .active
                .iter()
                .take(status.redundancy.channels())
                .enumerate()
                .map(|(channel, active)| ChannelSnapshot {
                    channel,
                    active: *active,
                })
                .collect(),
            trip: status.trip,
            node: None,
        }
    }
}

/// Zustandsbild des Stellwerks (aller Stellbereiche).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Snapshot {
    /// Netz.
    pub network: String,
    /// Zeit seit Start in Millisekunden.
    pub time_ms: u64,
    /// Zyklusnummer (des ersten Bereichs).
    pub cycle: u64,
    /// Sicherer Zustand eines Bereichs, falls aktiv (`"Bereich: Grund"`).
    pub safe_state: Option<String>,
    /// Stellbereiche.
    pub areas: Vec<AreaSnapshot>,
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
    /// Zählwerk der ausgeführten Hilfshandlungen (Summe aller Bereiche).
    pub aux_counter: u64,
    /// Betriebsführung (Zugnummern, Fahrplanlage), falls eingeschaltet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ops: Option<railit_ops::OpsSnapshot>,
    /// Angemeldete ETCS-Züge mit Fahrerlaubnis.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub etcs: Vec<railit_etcs::TrainSnapshot>,
}

impl Snapshot {
    /// Zustandsbild eines einzelnen Kerns.
    pub fn new(plan: &PlanView<'_>, state: &OwnedState, now: Tick) -> Self {
        Self::network(
            plan.network_name(),
            now,
            &[(plan.network_name(), plan, state)],
        )
    }

    /// Zustandsbild aller Bereiche eines Netzes.
    pub fn network(network: &str, now: Tick, areas: &[(&str, &PlanView<'_>, &OwnedState)]) -> Self {
        let mut snap = Self {
            network: network.to_string(),
            time_ms: now.as_millis(),
            cycle: areas.first().map_or(0, |(_, _, s)| s.meta().cycle()),
            safe_state: None,
            areas: Vec::new(),
            points: Vec::new(),
            signals: Vec::new(),
            tvds: Vec::new(),
            routes: Vec::new(),
            level_crossings: Vec::new(),
            pending_aux: Vec::new(),
            aux_counter: 0,
            ops: None,
            etcs: Vec::new(),
        };
        let prefixed = areas.len() > 1;
        for (name, plan, state) in areas {
            snap.add_area(name, if prefixed { name } else { "" }, plan, state);
        }
        snap
    }

    #[allow(clippy::too_many_lines)]
    fn add_area(&mut self, area: &str, prefix: &str, plan: &PlanView<'_>, state: &OwnedState) {
        let owner = |r: Option<RouteId>| r.map(|r| route_name(plan, r));
        let pictures = railit_movement::Pictures::new(plan, state.signals(), state.routes());
        let area_name = area.to_string();
        if self.safe_state.is_none() {
            self.safe_state = state.meta().safe_state().map(|r| {
                if prefix.is_empty() {
                    format!("{r:?}")
                } else {
                    format!("{area}: {r:?}")
                }
            });
        }
        self.aux_counter = self.aux_counter.saturating_add(state.meta().aux_counter());
        self.areas.push(AreaSnapshot {
            name: area_name.clone(),
            cycle: state.meta().cycle(),
            safe_state: state.meta().safe_state().map(|r| format!("{r:?}")),
            safety: SafetySnapshot::default(),
            plan_digest: crate::keys::to_hex(&plan.digest())
                .get(..16)
                .unwrap_or_default()
                .to_string(),
            borders: state
                .borders()
                .iter()
                .enumerate()
                .filter_map(|(i, b)| Some((BorderId::from_index(i)?, b)))
                .map(|(id, b)| {
                    let peer = b.peer();
                    BorderSnapshot {
                        name: format!("{area}{}", border_name(plan, id)),
                        holding: b.holding(),
                        seq: b.seq(),
                        offering: b.offering(),
                        requested: b.requested(),
                        approach_locked: b.approach_locked(),
                        peer_holding: peer.map(|(m, _)| m.holding),
                        peer_approach_clear: peer.map(|(m, _)| m.approach_clear),
                        peer_exclusive: peer.map(|(m, _)| m.exclusive),
                        peer_at_ms: peer.map(|(_, t)| t.as_millis()),
                    }
                })
                .collect(),
            aux_counter: state.meta().aux_counter(),
        });
        self.points.extend(
            state
                .points()
                .iter()
                .enumerate()
                .filter_map(|(i, p)| Some((PointId::from_index(i)?, p)))
                .map(|(id, p)| PointSnapshot {
                    area: area_name.clone(),
                    name: point_name(plan, id),
                    report: p.report(),
                    commanded: p.commanded(),
                    locked_by: owner(p.owner()),
                    locked_position: p.locked_position(),
                    holds: p.holds(),
                }),
        );
        self.signals.extend(
            state
                .signals()
                .iter()
                .enumerate()
                .filter_map(|(i, s)| Some((SignalId::from_index(i)?, s)))
                .map(|(id, s)| SignalSnapshot {
                    area: area_name.clone(),
                    name: signal_name(plan, id),
                    commanded: s.commanded(),
                    report: s.report(),
                    substitute: s.substitute(),
                    picture: pictures
                        .picture(id)
                        .map(|p| p.to_string())
                        .unwrap_or_default(),
                    aspects: pictures.picture(id),
                    flank_holds: s.stop_holds(),
                }),
        );
        self.tvds.extend(
            state
                .tvds()
                .iter()
                .enumerate()
                .filter_map(|(i, t)| Some((TvdId::from_index(i)?, t)))
                .map(|(id, t)| TvdSnapshot {
                    area: area_name.clone(),
                    name: tvd_name(plan, id),
                    occupancy: t.occupancy(),
                    locked_by: owner(t.owner()),
                    overlap_of: owner(t.overlap_owner()),
                }),
        );
        self.routes.extend(plan.routes().map(|r| {
            RouteSnapshot {
                area: area_name.clone(),
                name: r.name().to_string(),
                start: signal_name(plan, r.start()),
                end: match r.end() {
                    RouteEnd::Signal(s) => signal_name(plan, s),
                    RouteEnd::Boundary => "Netzgrenze".into(),
                    RouteEnd::BufferStop => "Prellbock".into(),
                    RouteEnd::Border(b) => format!("{prefix}{}", border_name(plan, b)),
                },
                phase: r
                    .id()
                    .index()
                    .and_then(|i| state.routes().get(i))
                    .map(railit_kernel::RouteState::phase)
                    .unwrap_or_default(),
            }
        }));
        self.level_crossings.extend(
            state
                .level_crossings()
                .iter()
                .enumerate()
                .filter_map(|(i, l)| Some((LxId::from_index(i)?, l)))
                .map(|(id, l)| LxSnapshot {
                    area: area_name.clone(),
                    name: lx_name(plan, id),
                    report: l.report(),
                    close_commanded: l.closed_commanded(),
                    holds: l.holds(),
                }),
        );
        self.pending_aux.extend(state.pending_aux().map(|a| {
            let op = NamedAux::in_area(plan, prefix, a.op());
            AuxSnapshot {
                area: area_name.clone(),
                aux: a.id().0,
                check: aux_check(&area_name, a.id().0, &op),
                op,
                operator: a.operator().0,
                since_ms: a.since().as_millis(),
            }
        }));
    }
}

#[cfg(test)]
mod check_tests {
    use super::*;

    /// Prüfvektoren; dieselben prüft der Bedienplatz (`apps/railitd/ui/test/check.test.ts`).
    #[test]
    fn check_code_vectors() {
        let named = |kind, element: &str, position| NamedAux {
            kind,
            element: element.into(),
            position,
        };
        assert_eq!(
            aux_check(
                "Beispielstadt",
                0,
                &named(AuxKind::ReleaseRoute, "A-N1", None)
            ),
            "63CC78"
        );
        assert_eq!(
            aux_check(
                "Adorf",
                7,
                &named(AuxKind::ForcePoint, "W1", Some(PointPosition::Left))
            ),
            "98171F"
        );
        assert_eq!(
            aux_check("Bstadt", 12, &named(AuxKind::ResetTvd, "Ü1", None)),
            "DF8640"
        );
    }
}
