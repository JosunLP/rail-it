//! Fahrerlaubnis für ETCS Level 2: Movement Authority aus Plan und Kernzustand (REQ-MOV-004).
//!
//! Der Kern entscheidet, welche Fahrstraßen Fahrt haben. Für ETCS werden daraus
//! Fahrerlaubnisse (Movement Authorities): ab einem Signal bzw. einer ETCS-Halt-Tafel entlang
//! der Kette aufeinanderfolgender Fahrstraßen mit Fahrt bis zum Ende der Fahrerlaubnis (`EoA`),
//! dazu der Gefahrpunkt (`EoA` plus Durchrutschweg) und das statische Geschwindigkeitsprofil.
//!
//! Regeln:
//!
//! - Ein Abschnitt gehört zur Fahrerlaubnis, wenn sein Startsignal Fahrt befohlen hat und die
//!   Fahrstraße Fahrt zeigt. Hat der Zug das Startsignal schon passiert (`passed`), zählt die
//!   erste Fahrstraße auch, solange sie befahren ist oder ihr Ziel erreicht wurde.
//! - Das `EoA` liegt am Zielsignal der letzten Fahrstraße der Kette, an einer Netzgrenze, einem
//!   Prellbock oder einer Bereichsgrenze. Der Durchrutschweg hinter dem Zielsignal ist der
//!   Abstand zum Gefahrpunkt.
//! - Ersatzsignal (Zs1) und Fahrten ohne Fahrstraße ergeben keine Fahrerlaubnis.
//! - Jede Rücknahme der Fahrt am Signal verkürzt die Fahrerlaubnis sofort.

use railit_kernel::{Aspect, RoutePhase, RouteState, SignalState};
use railit_plan::{PlanView, RouteEnd, RouteView};
use railit_types::{BorderId, Length, RouteId, SignalId, Speed};

/// Höchstzahl der Abschnitte einer Fahrerlaubnis.
pub const MA_SECTIONS: usize = 8;

/// Abschnitt einer Fahrerlaubnis: eine Fahrstraße.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MaSection {
    /// Fahrstraße.
    pub route: RouteId,
    /// Länge vom Startsignal bis zum Ziel.
    pub length: Length,
    /// Zulässige Geschwindigkeit, falls beschränkt.
    pub speed: Option<Speed>,
}

const EMPTY: MaSection = MaSection {
    route: RouteId::new(0),
    length: Length::from_mm(0),
    speed: None,
};

/// Wo die Fahrerlaubnis endet.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum MaEnd {
    /// Am Halt zeigenden (bzw. nicht erfassten) Signal.
    Signal(SignalId),
    /// An der Netzgrenze.
    Boundary,
    /// Am Prellbock.
    BufferStop,
    /// An der Grenze zum Nachbarbereich; dort setzt der Nachbar die Fahrerlaubnis fort.
    Border(BorderId),
}

/// Fahrerlaubnis ab einem Signal.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct MovementAuthority {
    start: SignalId,
    sections: [MaSection; MA_SECTIONS],
    count: usize,
    end: MaEnd,
    overlap: Length,
}

impl MovementAuthority {
    /// Signal, an dem die Fahrerlaubnis beginnt.
    pub const fn start(&self) -> SignalId {
        self.start
    }

    /// Abschnitte in Fahrtrichtung.
    pub fn sections(&self) -> &[MaSection] {
        self.sections.get(..self.count).unwrap_or(&[])
    }

    /// Ende der Fahrerlaubnis.
    pub const fn end(&self) -> MaEnd {
        self.end
    }

    /// Abstand vom Startsignal bis zum `EoA`.
    pub fn eoa(&self) -> Length {
        self.sections().iter().fold(Length::from_mm(0), |sum, s| {
            sum.checked_add(s.length).unwrap_or(sum)
        })
    }

    /// Länge des Durchrutschwegs hinter dem `EoA`.
    pub const fn overlap(&self) -> Length {
        self.overlap
    }

    /// Abstand vom Startsignal bis zum Gefahrpunkt.
    pub fn danger_point(&self) -> Length {
        let eoa = self.eoa();
        eoa.checked_add(self.overlap).unwrap_or(eoa)
    }

    /// Zulässige Geschwindigkeit im Abstand `distance` hinter dem Startsignal (`None`:
    /// unbeschränkt bzw. Streckengeschwindigkeit; hinter dem `EoA`: `Some(0)`).
    pub fn speed_at(&self, distance: Length) -> Option<Speed> {
        let mut from = Length::from_mm(0);
        for s in self.sections() {
            let to = from.checked_add(s.length).unwrap_or(from);
            if distance.as_mm() < to.as_mm() {
                return s.speed;
            }
            from = to;
        }
        Some(Speed::from_kmh(0))
    }
}

/// Fahrstraße ab `signal`, die in einer der Phasen `phases` ist.
fn active_route<'p>(
    plan: &PlanView<'p>,
    routes: &[RouteState],
    signal: SignalId,
    accept: impl Fn(RoutePhase) -> bool,
) -> Option<RouteView<'p>> {
    plan.routes().find(|r| {
        r.start() == signal
            && r.id()
                .index()
                .and_then(|i| routes.get(i))
                .is_some_and(|s| accept(s.phase()))
    })
}

fn commands_proceed(signals: &[SignalState], signal: SignalId) -> bool {
    signal
        .index()
        .and_then(|i| signals.get(i))
        .is_some_and(|s| s.commanded() == Aspect::Proceed)
}

/// Fahrerlaubnis ab `start` (REQ-MOV-004). `passed`: Der Zug hat `start` bereits passiert und
/// befindet sich in der ersten Fahrstraße. `None`, wenn keine Fahrerlaubnis besteht.
pub fn authority(
    plan: &PlanView<'_>,
    signals: &[SignalState],
    routes: &[RouteState],
    start: SignalId,
    passed: bool,
) -> Option<MovementAuthority> {
    let mut ma = MovementAuthority {
        start,
        sections: [EMPTY; MA_SECTIONS],
        count: 0,
        end: MaEnd::Signal(start),
        overlap: Length::from_mm(0),
    };
    let mut at = start;
    for slot in &mut ma.sections {
        let first = ma.count == 0;
        let route = if first && passed {
            active_route(plan, routes, at, |p| {
                matches!(
                    p,
                    RoutePhase::Proceed | RoutePhase::Occupied | RoutePhase::Arrived
                )
            })
        } else if commands_proceed(signals, at) {
            active_route(plan, routes, at, |p| p == RoutePhase::Proceed)
        } else {
            None
        };
        let Some(route) = route else {
            break;
        };
        *slot = MaSection {
            route: route.id(),
            length: route.length(),
            speed: route.speed(),
        };
        ma.count = ma.count.saturating_add(1);
        ma.overlap = route.overlap_length();
        match route.end() {
            RouteEnd::Signal(next) => {
                ma.end = MaEnd::Signal(next);
                at = next;
            }
            RouteEnd::Boundary => {
                ma.end = MaEnd::Boundary;
                break;
            }
            RouteEnd::BufferStop => {
                ma.end = MaEnd::BufferStop;
                break;
            }
            RouteEnd::Border(b) => {
                ma.end = MaEnd::Border(b);
                break;
            }
        }
    }
    (ma.count > 0).then_some(ma)
}
