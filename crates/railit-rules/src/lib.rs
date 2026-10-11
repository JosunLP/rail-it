//! Regelwerke von rail-it.
//!
//! Der Sicherheitskern ist generisch; was zwischen Bahnen und Regelwerken abweicht, steckt
//! in einem [`RuleSet`]. Regelwerke werden **statisch** (über Generics) eingebunden – es wird
//! nie Code dynamisch in den Kern nachgeladen (Konzept, Abschnitt 11).
//!
//! Phase 1 enthält das Lehr- und Testregelwerk [`GenericRules`]. Das deutsche ESTW-Regelwerk
//! (`de-estw`) und ETCS folgen in den Phasen 2 und 4.

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

use railit_types::Duration;

/// Betriebliche Regeln, die der Kern anwendet.
pub trait RuleSet {
    /// Längste zulässige Umstellzeit einer Weiche. Erreichen die Weichen einer Fahrstraße
    /// in dieser Zeit ihre Endlage nicht, wird die Einstellung abgebrochen.
    fn point_move_timeout(&self) -> Duration;

    /// Höchstalter einer Elementmeldung. Meldet sich ein Element länger nicht, gilt es als
    /// gestört (Kommunikationsausfall ⇒ restriktiver Zustand).
    fn report_timeout(&self) -> Duration;

    /// Zeitverschluss: So lange bleibt eine Fahrstraße nach der Rücknahme festgelegt, wenn
    /// ihr Signal bereits Fahrt gezeigt hat (ein Zug könnte sich nähern).
    fn cancel_time_lock(&self) -> Duration;
}

/// Lehr- und Testregelwerk mit einstellbaren Zeiten.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct GenericRules {
    /// Siehe [`RuleSet::point_move_timeout`].
    pub point_move_timeout: Duration,
    /// Siehe [`RuleSet::report_timeout`].
    pub report_timeout: Duration,
    /// Siehe [`RuleSet::cancel_time_lock`].
    pub cancel_time_lock: Duration,
}

impl Default for GenericRules {
    /// Weichenumstellung 10 s, Meldungsalter 2 s, Zeitverschluss 60 s.
    fn default() -> Self {
        Self {
            point_move_timeout: Duration::from_millis(10_000),
            report_timeout: Duration::from_millis(2_000),
            cancel_time_lock: Duration::from_millis(60_000),
        }
    }
}

impl RuleSet for GenericRules {
    fn point_move_timeout(&self) -> Duration {
        self.point_move_timeout
    }

    fn report_timeout(&self) -> Duration {
        self.report_timeout
    }

    fn cancel_time_lock(&self) -> Duration {
        self.cancel_time_lock
    }
}
