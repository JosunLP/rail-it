//! Monotone Zeit des Stellwerkszyklus.
//!
//! Der Kern liest nie selbst eine Uhr. Die Runtime übergibt pro Zyklus einen [`Tick`];
//! damit sind Zyklen deterministisch und beliebig wiederholbar (Replay).

use core::fmt;

/// Zeitdauer in Millisekunden.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct Duration(u64);

impl Duration {
    /// Dauer null.
    pub const ZERO: Self = Self(0);

    /// Erzeugt eine Dauer aus Millisekunden.
    pub const fn from_millis(ms: u64) -> Self {
        Self(ms)
    }

    /// Erzeugt eine Dauer aus Sekunden; `None` bei Überlauf.
    pub const fn from_secs(s: u64) -> Option<Self> {
        match s.checked_mul(1000) {
            Some(ms) => Some(Self(ms)),
            None => None,
        }
    }

    /// Wert in Millisekunden.
    pub const fn as_millis(self) -> u64 {
        self.0
    }
}

impl fmt::Display for Duration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ms", self.0)
    }
}

/// Zeitpunkt: Millisekunden seit Start des Systems (monoton).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct Tick(u64);

impl Tick {
    /// Systemstart.
    pub const ZERO: Self = Self(0);

    /// Erzeugt einen Zeitpunkt aus Millisekunden seit Systemstart.
    pub const fn from_millis(ms: u64) -> Self {
        Self(ms)
    }

    /// Millisekunden seit Systemstart.
    pub const fn as_millis(self) -> u64 {
        self.0
    }

    /// Zeitpunkt nach Ablauf von `d`; `None` bei Überlauf.
    pub const fn checked_add(self, d: Duration) -> Option<Self> {
        match self.0.checked_add(d.0) {
            Some(v) => Some(Self(v)),
            None => None,
        }
    }

    /// Vergangene Zeit seit `earlier`; null, falls `earlier` in der Zukunft liegt.
    ///
    /// Die Sättigung ist hier die sichere Wahl: Eine scheinbar negative Dauer verlängert
    /// laufende Zeitverschlüsse, statt sie vorzeitig ablaufen zu lassen.
    pub const fn saturating_since(self, earlier: Self) -> Duration {
        Duration(self.0.saturating_sub(earlier.0))
    }
}

impl fmt::Display for Tick {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "t+{} ms", self.0)
    }
}
