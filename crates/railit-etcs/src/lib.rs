//! RBC-Funktion von rail-it für ETCS Level 2 (Konzept, Abschnitt 9).
//!
//! Das RBC (Radio Block Centre) gibt Zügen ihre Fahrerlaubnis per Funk statt über
//! Lichtsignale. Es entscheidet nichts selbst: Jede Fahrerlaubnis folgt aus den Fahrstraßen,
//! denen der Sicherheitskern Fahrt gegeben hat ([`railit_movement::authority`]).
//!
//! - **Anmeldung:** Ein Zug meldet sich mit seiner ersten Positionsmeldung an (Start of
//!   Mission) und ab, sobald keine Meldung mehr kommt (End of Mission).
//! - **Position:** Bezugspunkt ist das zuletzt passierte Signal bzw. die zuletzt passierte
//!   ETCS-Halt-Tafel (oder das Signal, vor dem der Zug steht).
//! - **Fahrerlaubnis:** ab dem Bezugspunkt entlang der Fahrstraßen mit Fahrt; an einer
//!   Bereichsgrenze setzt sie sich über den Annäherungsabschnitt des Nachbarn bis zu dessen
//!   Einfahrsignal und weiter entlang seiner Fahrstraßen fort (REQ-ETCS-002).
//! - **Verkürzung und Notbremsung:** Nimmt der Kern Fahrt zurück, wird die Fahrerlaubnis
//!   sofort verkürzt. Wird die Fahrstraße, in der ein Zug fährt, per Hilfshandlung aufgelöst
//!   oder fällt ihr Signal wegen einer Störung auf Halt, löst das RBC eine Notbremsung aus
//!   (REQ-ETCS-003).
//! - **Zugbewirkte Auflösung:** Löst sich die Fahrstraße hinter dem Zug regulär auf, setzt das
//!   RBC den Bezugspunkt auf ihr Zielsignal bzw. das Einfahrsignal des Nachbarn.

use std::collections::{BTreeMap, BTreeSet};

use railit_kernel::{RoutePhase, RouteState, SignalState};
use railit_movement::{MaEnd, authority};
use railit_plan::PlanView;
use railit_plan::RouteEnd;
use railit_types::{BorderId, RouteId, SignalId};
use serde::Serialize;

/// Zugriff des RBC auf Pläne und Kernzustände aller Stellbereiche.
pub trait RbcView {
    /// Plan eines Bereichs.
    fn plan(&self, area: usize) -> Option<&PlanView<'_>>;
    /// Signalzustände eines Bereichs.
    fn signals(&self, area: usize) -> &[SignalState];
    /// Fahrstraßenzustände eines Bereichs.
    fn routes(&self, area: usize) -> &[RouteState];
    /// Gegenstück einer Grenze im Nachbarbereich.
    fn peer(&self, area: usize, border: BorderId) -> Option<(usize, BorderId)>;
    /// Name eines Bereichs (für Anzeigen).
    fn area_name(&self, area: usize) -> String;
    /// Vorausmeldung des RBC hinter einer Grenze zu einem Bereich in einem anderen Prozess
    /// (REQ-ETCS-004); `None`, wenn keine gültige vorliegt.
    fn remote(&self, _area: usize, _border: BorderId) -> Option<&RemoteAuthority> {
        None
    }
}

/// Bereich eines Signals in einem anderen Prozess: Die Kennung des Signals in
/// [`Authority::passable`] ist dann die des Gesamtnetzes.
pub const REMOTE_AREA: usize = usize::MAX;

/// Fahrerlaubnis ab einer Grenze zu einem Bereich in einem anderen Prozess, wie das RBC
/// dort sie vorausmeldet: über den Annäherungsabschnitt bis zum Einfahrsignal und weiter
/// entlang der Fahrstraßen mit Fahrt (REQ-ETCS-004).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteAuthority {
    /// Abschnitte ab der Grenze.
    pub sections: Vec<Section>,
    /// Ende der Fahrerlaubnis.
    pub end: String,
    /// Abstand von der Grenze bis zum Ende.
    pub eoa_mm: u64,
    /// Abstand von der Grenze bis zum Gefahrpunkt.
    pub danger_point_mm: u64,
    /// Passierbare Signale (Kennungen des Gesamtnetzes).
    pub passable: Vec<u32>,
}

/// Positionsmeldung eines Zuges.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PositionReport {
    /// Zugnummer (Kennung des Fahrzeuggeräts).
    pub train: String,
    /// Stellbereich des Bezugspunkts.
    pub area: usize,
    /// Bezugspunkt: Signal bzw. ETCS-Halt-Tafel im Bereich.
    pub reference: SignalId,
    /// Der Zug hat den Bezugspunkt passiert (sonst steht er davor).
    pub passed: bool,
}

/// Abschnitt einer Fahrerlaubnis über alle Bereiche.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Section {
    /// Stellbereich.
    pub area: String,
    /// Fahrstraße bzw. `None` für den Annäherungsabschnitt hinter einer Bereichsgrenze.
    pub route: Option<String>,
    /// Länge in Millimetern.
    pub length_mm: u64,
    /// Zulässige Geschwindigkeit, falls beschränkt.
    pub speed_kmh: Option<u16>,
}

/// Fahrerlaubnis (Movement Authority) eines Zuges.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Authority {
    /// Abschnitte in Fahrtrichtung ab dem Bezugspunkt.
    pub sections: Vec<Section>,
    /// Ende der Fahrerlaubnis: Signal, `"Netzgrenze"`, `"Prellbock"` oder Grenze.
    pub end: String,
    /// Abstand vom Bezugspunkt bis zum Ende der Fahrerlaubnis (`EoA`).
    pub eoa_mm: u64,
    /// Abstand vom Bezugspunkt bis zum Gefahrpunkt.
    pub danger_point_mm: u64,
    /// Signale bzw. Halt-Tafeln, die der Zug passieren darf (Bereich, Signal).
    #[serde(skip)]
    pub passable: Vec<(usize, SignalId)>,
    /// Erste Fahrstraße (Bereich, Fahrstraße).
    #[serde(skip)]
    pub first: Option<(usize, RouteId)>,
}

/// Betriebsart des Fahrzeuggeräts.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Angemeldet, ohne Fahrerlaubnis (Stand By).
    StandBy,
    /// Volle Überwachung mit Fahrerlaubnis (Full Supervision).
    FullSupervision,
    /// Notbremsung ausgelöst (Trip).
    Trip,
}

/// Ereignis des RBC.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RbcEvent {
    /// Zug angemeldet.
    TrainRegistered {
        /// Zugnummer.
        train: String,
    },
    /// Zug abgemeldet.
    TrainDeregistered {
        /// Zugnummer.
        train: String,
    },
    /// Neue oder verlängerte Fahrerlaubnis.
    AuthorityGranted {
        /// Zugnummer.
        train: String,
        /// Ende der Fahrerlaubnis.
        end: String,
        /// Abstand bis zum `EoA`.
        eoa_mm: u64,
    },
    /// Fahrerlaubnis verkürzt.
    AuthorityShortened {
        /// Zugnummer.
        train: String,
        /// Neues Ende (leer: keine Fahrerlaubnis mehr).
        end: String,
        /// Neuer Abstand bis zum `EoA`.
        eoa_mm: u64,
    },
    /// Notbremsung: Der Zug hat keine Fahrerlaubnis mehr, obwohl er fährt.
    EmergencyStop {
        /// Zugnummer.
        train: String,
        /// Grund.
        reason: String,
    },
}

impl RbcEvent {
    /// Art des Ereignisses wie im Feld `type`.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::TrainRegistered { .. } => "train_registered",
            Self::TrainDeregistered { .. } => "train_deregistered",
            Self::AuthorityGranted { .. } => "authority_granted",
            Self::AuthorityShortened { .. } => "authority_shortened",
            Self::EmergencyStop { .. } => "emergency_stop",
        }
    }
}

#[derive(Clone, Debug)]
struct TrainRecord {
    report: PositionReport,
    authority: Option<Authority>,
    mode: Mode,
    /// Fahrstraße, in der der Zug fährt bzw. vor der er steht (Bereich, Fahrstraße).
    first: Option<(usize, RouteId)>,
    /// Diese Fahrstraße wurde per Hilfshandlung aufgelöst.
    revoked: bool,
    /// Bezugspunkt nach regulärer Auflösung der Fahrstraße hinter dem Zug (Bereich, Signal).
    virtual_ref: Option<(usize, SignalId)>,
}

/// Was nach dem Auflösen der Fahrstraße unter dem Zug gilt.
enum Released {
    /// Neuer Bezugspunkt vor diesem Signal.
    Reference(usize, SignalId),
    /// Der Zug ist über eine Grenze zu einem Bereich in einem anderen Prozess (Bereich und
    /// Grenze); dort übernimmt das andere RBC.
    Remote(usize, BorderId),
    /// Zug steht am Ende des Netzes bzw. am Prellbock.
    AtEnd,
    /// Fahrstraße vor dem Ziel aufgelöst: Notbremsung.
    Revoked,
}

fn phase(view: &dyn RbcView, area: usize, route: RouteId) -> RoutePhase {
    route
        .index()
        .and_then(|i| view.routes(area).get(i))
        .map_or(RoutePhase::Idle, RouteState::phase)
}

/// Was nach dem Ende der Fahrstraße `route` unter dem Zug gilt.
fn released(view: &dyn RbcView, area: usize, route: RouteId, revoked: bool) -> Released {
    if revoked || phase(view, area, route) == RoutePhase::Halted {
        return Released::Revoked;
    }
    let Some(end) = view
        .plan(area)
        .and_then(|p| p.route(route))
        .map(|r| r.end())
    else {
        return Released::Revoked;
    };
    match end {
        RouteEnd::Signal(t) => Released::Reference(area, t),
        RouteEnd::Border(b) => {
            let Some((pa, pb)) = view.peer(area, b) else {
                // Nachbar in einem anderen Prozess: Dessen RBC übernimmt den Zug.
                return Released::Remote(area, b);
            };
            let entry = view.plan(pa).and_then(|p| p.border_entry_signal(pb));
            entry.map_or(Released::Revoked, |e| Released::Reference(pa, e))
        }
        RouteEnd::Boundary | RouteEnd::BufferStop => Released::AtEnd,
    }
}

/// Zug im Zustandsbild des RBC.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TrainSnapshot {
    /// Zugnummer.
    pub train: String,
    /// Betriebsart.
    pub mode: Mode,
    /// Bezugspunkt (Signal bzw. Halt-Tafel).
    pub reference: String,
    /// Bezugspunkt bereits passiert.
    pub passed: bool,
    /// Fahrerlaubnis.
    pub authority: Option<Authority>,
}

/// Fahrerlaubnis für ein Fahrzeuggerät: Zug und freigegebene Signale (Bereich, Signal);
/// `None` bedeutet Notbremsung.
pub type CabAuthority = (String, Option<Vec<(usize, SignalId)>>);

/// Höchstzahl der Bereichsübergänge in einer Fahrerlaubnis (gegen Kreise im Netz).
const MAX_AREAS: usize = 16;

/// RBC für alle Stellbereiche eines Netzes.
#[derive(Clone, Debug, Default)]
pub struct Rbc {
    trains: BTreeMap<String, TrainRecord>,
}

fn end_name(view: &dyn RbcView, plan: &PlanView<'_>, area: usize, end: MaEnd) -> String {
    match end {
        MaEnd::Signal(s) => plan.signal_name(s).unwrap_or("?").to_string(),
        MaEnd::Boundary => "Netzgrenze".into(),
        MaEnd::BufferStop => "Prellbock".into(),
        MaEnd::Border(b) => format!(
            "{}{}",
            view.area_name(area),
            plan.border_name(b).unwrap_or("?")
        ),
    }
}

/// Hängt die Vorausmeldung eines RBC in einem anderen Prozess an (REQ-ETCS-004).
fn append_remote(ma: &mut Authority, remote: &RemoteAuthority) {
    ma.sections.extend(remote.sections.iter().cloned());
    ma.danger_point_mm = ma.eoa_mm.saturating_add(remote.danger_point_mm);
    ma.eoa_mm = ma.eoa_mm.saturating_add(remote.eoa_mm);
    ma.end.clone_from(&remote.end);
    ma.passable.extend(
        remote
            .passable
            .iter()
            .map(|id| (REMOTE_AREA, SignalId::new(*id))),
    );
}

/// Fahrerlaubnis eines Zuges, der schon hinter einer Grenze zu einem anderen Prozess ist.
fn remote_only(view: &dyn RbcView, area: usize, border: BorderId) -> Option<Authority> {
    let remote = view.remote(area, border)?;
    let mut ma = Authority {
        sections: Vec::new(),
        end: String::new(),
        eoa_mm: 0,
        danger_point_mm: 0,
        passable: Vec::new(),
        first: None,
    };
    append_remote(&mut ma, remote);
    (!ma.sections.is_empty()).then_some(ma)
}

/// Fahrerlaubnis ab einem Bezugspunkt über alle Bereiche (REQ-ETCS-002).
pub fn compute(
    view: &dyn RbcView,
    area: usize,
    start: SignalId,
    passed: bool,
) -> Option<Authority> {
    let mut ma = Authority {
        sections: Vec::new(),
        end: String::new(),
        eoa_mm: 0,
        danger_point_mm: 0,
        passable: Vec::new(),
        first: None,
    };
    let (mut area, mut start, mut passed) = (area, start, passed);
    for _ in 0..MAX_AREAS {
        let plan = view.plan(area)?;
        let Some(piece) = authority(plan, view.signals(area), view.routes(area), start, passed)
        else {
            // Kein Anschluss im Nachbarbereich: EoA am Einfahrsignal (bereits gesetzt).
            break;
        };
        for s in piece.sections() {
            if let Some(route) = plan.route(s.route) {
                ma.passable.push((area, route.start()));
            }
            if ma.first.is_none() {
                ma.first = Some((area, s.route));
            }
            ma.sections.push(Section {
                area: view.area_name(area),
                route: plan.route_name(s.route).map(str::to_string),
                length_mm: u64::from(s.length.as_mm()),
                speed_kmh: s.speed.map(railit_types::Speed::as_kmh),
            });
            ma.eoa_mm = ma.eoa_mm.saturating_add(u64::from(s.length.as_mm()));
        }
        ma.end = end_name(view, plan, area, piece.end());
        ma.danger_point_mm = ma.eoa_mm.saturating_add(u64::from(piece.overlap().as_mm()));
        let MaEnd::Border(border) = piece.end() else {
            break;
        };
        // Über die Grenze: Annäherungsabschnitt des Nachbarn bis zu seinem Einfahrsignal.
        let Some((peer_area, peer_border)) = view.peer(area, border) else {
            // Nachbar in einem anderen Prozess: dessen Vorausmeldung anhängen.
            if let Some(remote) = view.remote(area, border) {
                append_remote(&mut ma, remote);
            }
            break;
        };
        let Some(peer_plan) = view.plan(peer_area) else {
            break;
        };
        let (Some(entry), Some(approach), Some(overlap)) = (
            peer_plan.border_entry_signal(peer_border),
            peer_plan.border_approach_length(peer_border),
            peer_plan.border_entry_overlap(peer_border),
        ) else {
            break;
        };
        ma.sections.push(Section {
            area: view.area_name(peer_area),
            route: None,
            length_mm: u64::from(approach.as_mm()),
            speed_kmh: None,
        });
        ma.eoa_mm = ma.eoa_mm.saturating_add(u64::from(approach.as_mm()));
        ma.end = peer_plan.signal_name(entry).unwrap_or("?").to_string();
        ma.danger_point_mm = ma.eoa_mm.saturating_add(u64::from(overlap.as_mm()));
        (area, start, passed) = (peer_area, entry, false);
    }
    (!ma.sections.is_empty()).then_some(ma)
}

impl Rbc {
    /// RBC ohne angemeldete Züge.
    pub fn new() -> Self {
        Self::default()
    }

    /// Ein Zyklus: Positionsmeldungen übernehmen, Fahrerlaubnisse berechnen, Verkürzungen
    /// und Notbremsungen erkennen.
    /// `revoked`: Fahrstraßen, die in diesem Zyklus per Hilfshandlung aufgelöst wurden.
    pub fn update(
        &mut self,
        view: &dyn RbcView,
        reports: &[PositionReport],
        revoked: &[(usize, RouteId)],
    ) -> Vec<RbcEvent> {
        let mut events = Vec::new();
        let reported: BTreeSet<&str> = reports.iter().map(|r| r.train.as_str()).collect();
        let gone: Vec<String> = self
            .trains
            .keys()
            .filter(|t| !reported.contains(t.as_str()))
            .cloned()
            .collect();
        for train in gone {
            self.trains.remove(&train);
            events.push(RbcEvent::TrainDeregistered { train });
        }
        for report in reports {
            let record = self.trains.entry(report.train.clone()).or_insert_with(|| {
                events.push(RbcEvent::TrainRegistered {
                    train: report.train.clone(),
                });
                TrainRecord {
                    report: report.clone(),
                    authority: None,
                    mode: Mode::StandBy,
                    first: None,
                    revoked: false,
                    virtual_ref: None,
                }
            });
            let moved = record.report.reference != report.reference
                || record.report.area != report.area
                || record.report.passed != report.passed;
            if moved {
                record.virtual_ref = None;
            }
            record.report = report.clone();
            if record.first.is_some_and(|f| revoked.contains(&f)) {
                record.revoked = true;
            }
            Self::update_train(view, record, moved, &mut events);
        }
        events
    }

    /// Fahrerlaubnis eines angemeldeten Zuges neu berechnen.
    fn update_train(
        view: &dyn RbcView,
        record: &mut TrainRecord,
        mut moved: bool,
        events: &mut Vec<RbcEvent>,
    ) {
        let train = record.report.train.clone();
        let (area, reference, passed) = match record.virtual_ref {
            Some((a, s)) => (a, s, false),
            None => (
                record.report.area,
                record.report.reference,
                record.report.passed,
            ),
        };
        let mut new_ma = compute(view, area, reference, passed);
        let mut revoked = false;
        let mut at_end = false;
        if passed
            && let Some((first_area, route)) = record.first
            && (new_ma.is_none() || record.revoked)
        {
            match released(view, first_area, route, record.revoked) {
                Released::Reference(a, s) if new_ma.is_none() => {
                    // Der Zug ist über das Ziel hinaus: neuer Bezugspunkt, keine Verkürzung.
                    moved = moved || record.virtual_ref != Some((a, s));
                    record.virtual_ref = Some((a, s));
                    new_ma = compute(view, a, s, false);
                }
                Released::Remote(a, b) if new_ma.is_none() => {
                    // Im Annäherungsabschnitt des Nachbarn: dessen Vorausmeldung, bis der Zug
                    // dort angemeldet ist; ohne Vorausmeldung keine Fahrerlaubnis.
                    new_ma = remote_only(view, a, b);
                    at_end = new_ma.is_none();
                }
                Released::Reference(..) | Released::Remote(..) => {}
                Released::AtEnd => at_end = true,
                Released::Revoked => {
                    revoked = true;
                    new_ma = None;
                }
            }
        }
        if !revoked {
            record.revoked = false;
            if let Some(first) = new_ma.as_ref().and_then(|m| m.first) {
                record.first = Some(first);
            }
        }

        let before = record.authority.as_ref().map(|a| (a.eoa_mm, a.end.clone()));
        let after = new_ma.as_ref().map(|a| (a.eoa_mm, a.end.clone()));
        let changed = before != after;
        record.authority = new_ma;
        match &after {
            Some((eoa, end)) if changed => {
                // Kürzer bei gleichem Bezugspunkt: Fahrt zurückgenommen.
                let shorter = !moved && before.as_ref().is_some_and(|(b, _)| eoa < b);
                let _ = end;
                events.push(if shorter {
                    RbcEvent::AuthorityShortened {
                        train: train.clone(),
                        end: end.clone(),
                        eoa_mm: *eoa,
                    }
                } else {
                    RbcEvent::AuthorityGranted {
                        train: train.clone(),
                        end: end.clone(),
                        eoa_mm: *eoa,
                    }
                });
                record.mode = Mode::FullSupervision;
            }
            Some(_) => {
                if record.mode == Mode::Trip {
                    record.mode = Mode::FullSupervision;
                }
            }
            None => {
                if before.is_some() && !revoked && !moved {
                    events.push(RbcEvent::AuthorityShortened {
                        train: train.clone(),
                        end: String::new(),
                        eoa_mm: 0,
                    });
                }
                if revoked {
                    if record.mode != Mode::Trip {
                        record.mode = Mode::Trip;
                        events.push(RbcEvent::EmergencyStop {
                            train,
                            reason: "Fahrstraße unter dem Zug aufgelöst".into(),
                        });
                    }
                } else if record.mode != Mode::Trip || at_end {
                    record.mode = Mode::StandBy;
                }
                if at_end {
                    record.first = None;
                }
            }
        }
    }

    /// Fahrerlaubnis eines Zuges.
    pub fn authority(&self, train: &str) -> Option<&Authority> {
        self.trains.get(train)?.authority.as_ref()
    }

    /// Betriebsart eines Zuges.
    pub fn mode(&self, train: &str) -> Option<Mode> {
        self.trains.get(train).map(|t| t.mode)
    }

    /// Fahrerlaubnisse aller Züge als (Zug, passierbare Signale) für das Fahrzeuggerät. Eine
    /// leere Liste heißt: keine Fahrerlaubnis, vor dem nächsten Signal halten; `None` heißt
    /// Notbremsung.
    pub fn cab_authorities(&self) -> Vec<CabAuthority> {
        self.trains
            .iter()
            .map(|(name, t)| {
                let cab = (t.mode != Mode::Trip).then(|| {
                    t.authority
                        .as_ref()
                        .map(|a| a.passable.clone())
                        .unwrap_or_default()
                });
                (name.clone(), cab)
            })
            .collect()
    }

    /// Zustandsbild.
    pub fn snapshot(&self, view: &dyn RbcView) -> Vec<TrainSnapshot> {
        self.trains
            .iter()
            .map(|(name, t)| TrainSnapshot {
                train: name.clone(),
                mode: t.mode,
                reference: view
                    .plan(t.report.area)
                    .and_then(|p| p.signal_name(t.report.reference))
                    .unwrap_or("?")
                    .to_string(),
                passed: t.report.passed,
                authority: t.authority.clone(),
            })
            .collect()
    }
}
