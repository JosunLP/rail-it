//! Eingaben, Befehle und Ereignisse des Kerns.

use railit_types::{LxId, PointId, PointPosition, RouteId, SignalId, TvdId};

/// Meldung einer Weiche.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum PointReport {
    /// Überwachte Endlage links.
    Left,
    /// Überwachte Endlage rechts.
    Right,
    /// Keine Endlage (Weiche läuft um oder wurde aufgefahren).
    NoEndPosition,
    /// Weiche meldet eine Störung.
    Disturbed,
}

impl PointReport {
    /// Überwachte Endlage, falls vorhanden.
    pub const fn position(self) -> Option<PointPosition> {
        match self {
            Self::Left => Some(PointPosition::Left),
            Self::Right => Some(PointPosition::Right),
            Self::NoEndPosition | Self::Disturbed => None,
        }
    }
}

impl From<PointPosition> for PointReport {
    fn from(pos: PointPosition) -> Self {
        match pos {
            PointPosition::Left => Self::Left,
            PointPosition::Right => Self::Right,
        }
    }
}

/// Meldung eines Gleisfreimeldeabschnitts.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum Occupancy {
    /// Frei.
    Clear,
    /// Besetzt.
    Occupied,
    /// Gestört (z. B. Achszähler nach Störung). Wird wie „besetzt“ behandelt.
    Disturbed,
}

/// Meldung eines Signals.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum SignalReport {
    /// Zeigt Halt.
    Stop,
    /// Zeigt einen Fahrtbegriff.
    Proceed,
    /// Defekt (z. B. Lampenfehler); zeigt sicher dunkel bzw. Halt.
    Defective,
}

/// Meldung eines Bahnübergangs.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum LxReport {
    /// Geöffnet (Straßenverkehr frei).
    Open,
    /// Schließvorgang läuft.
    Closing,
    /// Geschlossen und gesichert.
    Closed,
    /// Gestört.
    Disturbed,
}

/// Bedienerkennung für zählpflichtige Hilfshandlungen.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct OperatorId(pub u32);

/// Kennung einer angeforderten Hilfshandlung.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct AuxId(pub u32);

/// Zählpflichtige Hilfshandlung (Konzept, Abschnitt 10.6).
///
/// Hilfshandlungen heben Schutzmechanismen gezielt auf. Sie werden nur ausgeführt, wenn ein
/// **zweiter, anderer** Bediener sie innerhalb der Bestätigungsfrist bestätigt; jede
/// Ausführung erhöht das Zählwerk.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum AuxOperation {
    /// Fahrstraßenhilfsauflösung: löst eine Fahrstraße samt Durchrutschweg sofort auf.
    ReleaseRoute(RouteId),
    /// Achszählgrundstellung: setzt einen gestörten, unverschlossenen Abschnitt zurück.
    ResetTvd(TvdId),
    /// Ersatzsignal (Zs1) für eine festgelegte Fahrstraße, deren Signal nicht Fahrt zeigen
    /// kann.
    Substitute(RouteId),
    /// Umstellen einer unverschlossenen Weiche in einem gestörten Abschnitt.
    ForcePoint(PointId, PointPosition),
}

/// Signalbegriff, den der Kern befiehlt.
///
/// Phase 1 unterscheidet nur Halt und Fahrt. Die Abbildung auf Ks-/H/V-Begriffe und
/// ETCS-Fahrerlaubnisse folgt mit `railit-movement`.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum Aspect {
    /// Halt.
    #[default]
    Stop,
    /// Fahrt.
    Proceed,
}

/// Kennung einer Bedienanforderung; wird in der Antwort zurückgegeben.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct RequestId(pub u64);

/// Bedienanforderung.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum Request {
    /// Fahrstraße einstellen.
    SetRoute(RouteId),
    /// Fahrstraße zurücknehmen.
    CancelRoute(RouteId),
    /// Einzelne, nicht verschlossene Weiche umstellen.
    MovePoint(PointId, PointPosition),
    /// Hilfshandlung anfordern (erster Bediener).
    AuxRequest {
        /// Hilfshandlung.
        op: AuxOperation,
        /// Anfordernder Bediener.
        operator: OperatorId,
    },
    /// Hilfshandlung bestätigen (zweiter Bediener).
    AuxConfirm {
        /// Kennung aus [`Event::AuxPending`].
        aux: AuxId,
        /// Bestätigender Bediener.
        operator: OperatorId,
    },
}

/// Eingabe eines Zyklus.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum Input {
    /// Meldung einer Weiche.
    Point {
        /// Weiche.
        point: PointId,
        /// Meldung.
        report: PointReport,
    },
    /// Meldung eines Gleisfreimeldeabschnitts.
    Tvd {
        /// Abschnitt.
        tvd: TvdId,
        /// Meldung.
        occupancy: Occupancy,
    },
    /// Meldung eines Signals.
    Signal {
        /// Signal.
        signal: SignalId,
        /// Meldung.
        report: SignalReport,
    },
    /// Meldung eines Bahnübergangs.
    LevelCrossing {
        /// Bahnübergang.
        lx: LxId,
        /// Meldung.
        report: LxReport,
    },
    /// Bedienanforderung.
    Request {
        /// Kennung für die Antwort.
        id: RequestId,
        /// Anforderung.
        request: Request,
    },
}

/// Stellbefehl an ein Feldelement.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum Command {
    /// Weiche umstellen.
    MovePoint {
        /// Weiche.
        point: PointId,
        /// Ziellage.
        position: PointPosition,
    },
    /// Signalbegriff stellen.
    SetSignal {
        /// Signal.
        signal: SignalId,
        /// Begriff.
        aspect: Aspect,
    },
    /// Ersatzsignal (Zs1) ein- oder ausschalten.
    SetSubstitute {
        /// Signal.
        signal: SignalId,
        /// Ein oder aus.
        on: bool,
    },
    /// Bahnübergang schließen oder öffnen.
    SetLevelCrossing {
        /// Bahnübergang.
        lx: LxId,
        /// Schließen (`true`) oder öffnen.
        close: bool,
    },
    /// Gleisfreimeldeabschnitt zurücksetzen (Achszählgrundstellung).
    ResetTvd(TvdId),
}

/// Phase einer Fahrstraße (Konzept, Abschnitt 10.2).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum RoutePhase {
    /// Nicht eingestellt.
    #[default]
    Idle,
    /// Elemente reserviert, Weichen laufen in die geforderte Lage.
    Setting,
    /// Festgelegt: alle Weichen überwacht in Lage, Elemente verschlossen.
    Locked,
    /// Fahrerlaubnis erteilt: Startsignal zeigt Fahrt.
    Proceed,
    /// Ein Zug befährt die Fahrstraße; Abschnitte werden hinter ihm aufgelöst.
    Occupied,
    /// Der Zug hat das Ziel erreicht; der Durchrutschweg bleibt bis zu seiner Auflösung
    /// festgelegt.
    Arrived,
    /// Zurückgenommen, Zeitverschluss läuft.
    Cancelling,
    /// Signal ist wegen einer Störung auf Halt gefallen; nur Rücknahme möglich.
    Halted,
}

/// Grund für die Ablehnung einer Anforderung.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum Rejection {
    /// Das angesprochene Element existiert im Plan nicht.
    UnknownElement,
    /// Die Fahrstraße ist bereits eingestellt.
    RouteActive,
    /// Die Fahrstraße ist nicht eingestellt.
    RouteNotActive,
    /// Eine feindliche Fahrstraße ist eingestellt.
    Conflict,
    /// Ein Gleisfreimeldeabschnitt ist nicht frei (besetzt, gestört oder ohne Meldung).
    TvdNotClear(TvdId),
    /// Die Weiche ist durch eine Fahrstraße verschlossen.
    PointLocked(PointId),
    /// Die Weiche liegt in einem nicht freien Abschnitt und darf nicht umgestellt werden.
    PointNotOperable(PointId),
    /// Ein Zug befährt die Fahrstraße; eine Rücknahme ist nicht möglich.
    TrainInRoute,
    /// Der Kern ist im sicheren Zustand.
    SafeState,
    /// Das Startsignal wird als Flankenschutz in Halt gehalten.
    SignalHeld(SignalId),
    /// Alle Plätze für offene Hilfshandlungen sind belegt.
    AuxBusy,
    /// Die Hilfshandlung ist unbekannt oder bereits erledigt.
    AuxUnknown,
    /// Die Bestätigungsfrist ist abgelaufen.
    AuxExpired,
    /// Anforderung und Bestätigung stammen vom selben Bediener.
    SameOperator,
    /// Die Voraussetzungen der Hilfshandlung sind nicht erfüllt.
    AuxNotApplicable,
}

/// Störungs- und Gefahrenmeldung.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum Alarm {
    /// Weichen haben ihre Lage nicht rechtzeitig erreicht; Einstellung abgebrochen.
    SettingTimeout(RouteId),
    /// Fahrtbegriff wegen einer Störung zurückgenommen (Signalhaltfall).
    SignalDropped(RouteId),
    /// Ein Zug ist in eine zurückgenommene Fahrstraße eingefahren.
    TrainEnteredCancelledRoute(RouteId),
    /// Ein Signal meldet einen Defekt.
    SignalDefective(SignalId),
    /// Ein Signal meldet Fahrt, obwohl Halt befohlen ist.
    SignalMismatch(SignalId),
    /// Eine Meldung betrifft ein Element, das es im Plan nicht gibt.
    UnknownElement,
    /// Ein Abschnitt wurde besetzt, ohne dass eine Fahrt dorthin erlaubt war
    /// (z. B. Haltüberfahrung, Fahrt in den Durchrutschweg, Fahrzeug ohne Fahrstraße).
    UnauthorizedOccupancy(TvdId),
    /// Ein Zug ist an einem Halt zeigenden Signal vorbeigefahren.
    SignalPassedAtDanger(RouteId),
}

/// Ereignis eines Zyklus.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum Event {
    /// Anforderung angenommen.
    Accepted(RequestId),
    /// Anforderung abgelehnt.
    Rejected(RequestId, Rejection),
    /// Phasenwechsel einer Fahrstraße.
    RoutePhase(RouteId, RoutePhase),
    /// Störungs- oder Gefahrenmeldung.
    Alarm(Alarm),
    /// Hilfshandlung angefordert; wartet auf Bestätigung durch einen zweiten Bediener.
    AuxPending {
        /// Kennung für die Bestätigung.
        aux: AuxId,
        /// Hilfshandlung.
        op: AuxOperation,
        /// Anfordernder Bediener.
        operator: OperatorId,
    },
    /// Hilfshandlung ausgeführt.
    AuxExecuted {
        /// Kennung.
        aux: AuxId,
        /// Hilfshandlung.
        op: AuxOperation,
        /// Stand des Zählwerks nach der Ausführung.
        counter: u64,
    },
    /// Hilfshandlung verfallen (nicht rechtzeitig bestätigt).
    AuxExpired(AuxId),
}

/// Grund für den Übergang in den sicheren Zustand.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum SafeStateReason {
    /// Der Zustandsspeicher passt nicht zum Plan.
    StateMismatch,
    /// Interner Zugriff außerhalb der Grenzen (darf nach Planprüfung nie auftreten).
    InternalIndex,
    /// Ein Element ist verschlossen, ohne zu einer eingestellten Fahrstraße zu gehören.
    OrphanLock,
    /// Ein Element einer eingestellten Fahrstraße ist nicht von ihr verschlossen.
    MissingLock(RouteId),
    /// Zwei feindliche Fahrstraßen sind gleichzeitig eingestellt.
    ConflictingRoutes(RouteId, RouteId),
    /// Ein Signal soll Fahrt zeigen, ohne dass alle Bedingungen erfüllt sind.
    UnsafeProceed(SignalId),
    /// Stellbefehl an eine verschlossene Weiche.
    LockedPointCommanded(PointId),
    /// Stellbefehl an eine Weiche in einem nicht freien Abschnitt.
    PointMovedUnderOccupancy(PointId),
    /// Haltezähler oder Lagenverschluss stimmen nicht mit den eingestellten Fahrstraßen
    /// überein.
    HoldMismatch,
    /// Ein Bahnübergang soll öffnen, obwohl eine Fahrstraße ihn braucht.
    UnsafeLevelCrossing(LxId),
    /// Ersatzsignal ohne passende festgelegte Fahrstraße.
    UnsafeSubstitute(SignalId),
}

/// Empfänger der Ausgaben eines Zyklus.
pub trait Outputs {
    /// Stellbefehl.
    fn command(&mut self, command: Command);
    /// Ereignis.
    fn event(&mut self, event: Event);
}
