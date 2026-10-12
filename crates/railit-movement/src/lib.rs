//! Fahrerlaubnisse von rail-it: Signalbegriffe der Systeme Ks und H/V sowie
//! Movement Authorities für ETCS Level 2 (siehe [`authority`]).
//!
//! Der Sicherheitskern entscheidet nur **ob** ein Signal Fahrt zeigen darf (Halt/Fahrt) und
//! ob das Ersatzsignal leuchtet. Welcher **Begriff** dabei erscheint, hängt vom Signalsystem,
//! von der Geschwindigkeit der Fahrstraße und vom Zustand des nächsten Hauptsignals ab. Das
//! berechnet dieses Crate als reine Funktion aus Plan und Kernzustand (Konzept,
//! Abschnitt 12):
//!
//! | Signal        | Ks                                   | H/V                          |
//! |---------------|--------------------------------------|------------------------------|
//! | Hauptsignal   | Hp0, Ks1, Ks1 blinkend, Ks2; Zs3     | Hp0, Hp1, Hp2; Zs3           |
//! | Vorsignal     | Ks1, Ks1 blinkend, Ks2; Zs3v         | Vr0, Vr1, Vr2                |
//! | Kombiniert    | wie Hauptsignal (Mehrabschnitt)      | Hp und Vr am selben Mast     |
//! | Ersatzsignal  | Zs1 zusätzlich zu Hp0                | Zs1 zusätzlich zu Hp0        |
//!
//! Regeln:
//!
//! - Ein Hauptsignal zeigt nur dann einen Fahrtbegriff, wenn der Kern Fahrt befohlen hat;
//!   sonst Hp0.
//! - Erwartet wird „Fahrt“ nur, wenn das nächste Hauptsignal (Ziel der Fahrstraße) Fahrt
//!   befohlen hat; an einer Netzgrenze oder einem Prellbock wird „Halt erwarten“ angezeigt.
//! - Eine Geschwindigkeitsbeschränkung der Fahrstraße (abzweigende Weichen) erscheint im
//!   Ks-System als Zs3, im H/V-System als Hp2 (40 km/h) bzw. Hp2 mit Zs3. Am vorherigen
//!   Signal wird sie mit Zs3v und blinkendem Ks1 bzw. mit Vr2 angekündigt.
//!
//! Die Begriffe sind Anzeigeinformation und nicht sicherheitsrelevant für die
//! Fahrwegsicherung: Ein Fahrtbegriff setzt immer die Fahrt des Kerns voraus.

#![no_std]
#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::float_arithmetic,
    clippy::as_conversions
)]

use core::fmt;

mod etcs;

pub use etcs::{MA_SECTIONS, MaEnd, MaSection, MovementAuthority, authority};

use railit_kernel::{Aspect, RoutePhase, RouteState, SignalState};
use railit_plan::{PlanView, RouteEnd, RouteView};
use railit_types::{SignalId, SignalKind, SignalSystem, Speed};

/// Begriff am Hauptsignalschirm.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum MainAspect {
    /// Halt.
    Hp0,
    /// H/V: Fahrt.
    Hp1,
    /// H/V: Langsamfahrt (40 km/h, sofern kein Zs3 anderes zeigt).
    Hp2,
    /// Ks: Fahrt, nächstes Hauptsignal zeigt Fahrt.
    Ks1,
    /// Ks: Fahrt, Halt erwarten.
    Ks2,
    /// Rangierhalt.
    Sh0,
}

/// Begriff am Vorsignal(schirm).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum DistantAspect {
    /// H/V: Halt erwarten.
    Vr0,
    /// H/V: Fahrt erwarten.
    Vr1,
    /// H/V: Langsamfahrt erwarten.
    Vr2,
    /// Ks-Vorsignal: Fahrt erwarten.
    Ks1,
    /// Ks-Vorsignal: Halt erwarten.
    Ks2,
}

impl DistantAspect {
    /// Kündigt der Begriff eine Fahrt am nächsten Hauptsignal an?
    pub const fn expects_proceed(self) -> bool {
        matches!(self, Self::Vr1 | Self::Vr2 | Self::Ks1)
    }
}

/// Signalbild eines Signals: Schirmbegriffe und Zusatzsignale.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Picture {
    /// Begriff am Hauptsignalschirm (fehlt bei Vorsignalen und ETCS-Tafeln).
    pub main: Option<MainAspect>,
    /// Begriff am Vorsignalschirm (Vorsignal, H/V-Vorsignal am Hauptsignalmast). Fehlt, wenn
    /// dunkel.
    pub distant: Option<DistantAspect>,
    /// Ks1 blinkt (Geschwindigkeitsbeschränkung am nächsten Signal, mit Zs3v).
    pub flashing: bool,
    /// Geschwindigkeitsanzeiger Zs3 (Kennziffer × 10 km/h).
    pub zs3: Option<u8>,
    /// Geschwindigkeitsvoranzeiger Zs3v (Kennziffer × 10 km/h).
    pub zs3v: Option<u8>,
    /// Ersatzsignal Zs1.
    pub zs1: bool,
}

impl Picture {
    /// Erlaubt das Bild die Vorbeifahrt (Fahrtbegriff oder Zs1)?
    pub const fn permits_passing(&self) -> bool {
        self.zs1
            || matches!(
                self.main,
                Some(MainAspect::Hp1 | MainAspect::Hp2 | MainAspect::Ks1 | MainAspect::Ks2)
            )
    }
}

impl fmt::Display for Picture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = 0u8;
        let mut part = |f: &mut fmt::Formatter<'_>, args: fmt::Arguments<'_>| {
            if parts > 0 {
                f.write_str(" ")?;
            }
            parts = parts.saturating_add(1);
            f.write_fmt(args)
        };
        let flash = if self.flashing { " blinkend" } else { "" };
        if let Some(main) = self.main {
            match main {
                MainAspect::Ks1 => part(f, format_args!("Ks1{flash}"))?,
                other => part(f, format_args!("{other:?}"))?,
            }
        }
        if let Some(distant) = self.distant {
            match distant {
                DistantAspect::Ks1 => part(f, format_args!("Ks1{flash}"))?,
                other => part(f, format_args!("{other:?}"))?,
            }
        }
        if let Some(v) = self.zs3 {
            part(f, format_args!("Zs3({v})"))?;
        }
        if let Some(v) = self.zs3v {
            part(f, format_args!("Zs3v({v})"))?;
        }
        if self.zs1 {
            part(f, format_args!("Zs1"))?;
        }
        if parts == 0 {
            f.write_str("dunkel")?;
        }
        Ok(())
    }
}

/// Kennziffer für Zs3/Zs3v (1 bis 16), falls die Geschwindigkeit beschränkt ist.
fn indicator(speed: Option<Speed>) -> Option<u8> {
    let kmh = speed?.as_kmh();
    let digit = kmh.checked_div(10).unwrap_or(0).clamp(1, 16);
    u8::try_from(digit).ok()
}

/// H/V-Langsamfahrt ohne Zs3.
const HP2_KMH: u16 = 40;

/// Signalbilder aus Plan und Kernzustand.
#[derive(Copy, Clone, Debug)]
pub struct Pictures<'a, 'p> {
    plan: &'a PlanView<'p>,
    signals: &'a [SignalState],
    routes: &'a [RouteState],
}

/// Zustand eines Hauptsignals, wie ihn der Kern befohlen hat.
#[derive(Copy, Clone, Debug)]
struct MainState<'p> {
    proceed: bool,
    route: Option<RouteView<'p>>,
}

impl<'a, 'p> Pictures<'a, 'p> {
    /// Sicht auf Plan und Kernzustand (z. B. aus `OwnedState::signals()` und
    /// `OwnedState::routes()`).
    pub const fn new(
        plan: &'a PlanView<'p>,
        signals: &'a [SignalState],
        routes: &'a [RouteState],
    ) -> Self {
        Self {
            plan,
            signals,
            routes,
        }
    }

    /// Fahrstraße ab `signal`, die gerade die Fahrerlaubnis hat.
    fn proceed_route(&self, signal: SignalId) -> Option<RouteView<'p>> {
        self.plan.routes().find(|r| {
            r.start() == signal
                && r.id()
                    .index()
                    .and_then(|i| self.routes.get(i))
                    .is_some_and(|s| s.phase() == RoutePhase::Proceed)
        })
    }

    fn main_state(&self, signal: SignalId) -> MainState<'p> {
        let proceed = signal
            .index()
            .and_then(|i| self.signals.get(i))
            .is_some_and(|s| s.commanded() == Aspect::Proceed);
        MainState {
            proceed,
            route: if proceed {
                self.proceed_route(signal)
            } else {
                None
            },
        }
    }

    /// Zustand des nächsten Hauptsignals hinter der Fahrstraße. An Netzgrenze und
    /// Prellbock gibt es keins, hinter einer Bereichsgrenze ist es nicht bekannt; dann gilt
    /// „Halt erwarten“.
    fn next_main(&self, route: &RouteView<'_>) -> MainState<'p> {
        match route.end() {
            RouteEnd::Signal(next) => self.main_state(next),
            RouteEnd::Boundary | RouteEnd::BufferStop | RouteEnd::Border(_) => MainState {
                proceed: false,
                route: None,
            },
        }
    }

    /// Signalbild von `signal`; `None`, wenn das Signal unbekannt ist.
    pub fn picture(&self, signal: SignalId) -> Option<Picture> {
        let kind = self.plan.signal_kind(signal)?;
        let system = self.plan.signal_system(signal)?;
        let state = self.signals.get(signal.index()?)?;
        Some(match kind {
            SignalKind::Main | SignalKind::Combined => {
                self.main_picture(signal, kind, system, state.substitute())
            }
            SignalKind::Distant => self.distant_picture(signal, system),
            SignalKind::Shunting => Picture {
                main: Some(MainAspect::Sh0),
                ..Picture::default()
            },
            SignalKind::EtcsStopMarker => Picture::default(),
        })
    }

    fn main_picture(
        &self,
        signal: SignalId,
        kind: SignalKind,
        system: SignalSystem,
        zs1: bool,
    ) -> Picture {
        let own = self.main_state(signal);
        let Some(route) = own.route.filter(|_| own.proceed) else {
            return Picture {
                main: Some(MainAspect::Hp0),
                zs1,
                ..Picture::default()
            };
        };
        let speed = route.speed();
        let next = self.next_main(&route);
        let next_speed = next.route.and_then(|r| r.speed());
        match system {
            SignalSystem::Ks => {
                let zs3v = if next.proceed {
                    indicator(next_speed).filter(|v| indicator(speed).is_none_or(|own| *v < own))
                } else {
                    None
                };
                Picture {
                    main: Some(if next.proceed {
                        MainAspect::Ks1
                    } else {
                        MainAspect::Ks2
                    }),
                    flashing: zs3v.is_some(),
                    zs3: indicator(speed),
                    zs3v,
                    ..Picture::default()
                }
            }
            SignalSystem::Hv => {
                let (main, zs3) = match speed {
                    None => (MainAspect::Hp1, None),
                    Some(v) if v.as_kmh() == HP2_KMH => (MainAspect::Hp2, None),
                    Some(v) => (MainAspect::Hp2, indicator(Some(v))),
                };
                let distant = (kind == SignalKind::Combined).then(|| hv_distant(&next));
                Picture {
                    main: Some(main),
                    distant,
                    zs3,
                    ..Picture::default()
                }
            }
        }
    }

    fn distant_picture(&self, signal: SignalId, system: SignalSystem) -> Picture {
        let Some(main) = self.plan.signal_announces(signal) else {
            return Picture::default();
        };
        let next = self.main_state(main);
        match system {
            SignalSystem::Ks => {
                let zs3v = if next.proceed {
                    indicator(next.route.and_then(|r| r.speed()))
                } else {
                    None
                };
                Picture {
                    distant: Some(if next.proceed {
                        DistantAspect::Ks1
                    } else {
                        DistantAspect::Ks2
                    }),
                    flashing: zs3v.is_some(),
                    zs3v,
                    ..Picture::default()
                }
            }
            SignalSystem::Hv => Picture {
                distant: Some(hv_distant(&next)),
                ..Picture::default()
            },
        }
    }
}

fn hv_distant(next: &MainState<'_>) -> DistantAspect {
    if !next.proceed {
        DistantAspect::Vr0
    } else if next.route.and_then(|r| r.speed()).is_some() {
        DistantAspect::Vr2
    } else {
        DistantAspect::Vr1
    }
}
