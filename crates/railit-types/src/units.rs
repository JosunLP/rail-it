//! Physikalische Größen als Ganzzahlen.
//!
//! Gleitkommazahlen sind im sicherheitsrelevanten Pfad ausgeschlossen: Sie sind nicht
//! bitgenau reproduzierbar über Plattformen und Optimierungsstufen hinweg.

use core::fmt;

/// Länge bzw. Position entlang eines Gleises in Millimetern.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct Length(u32);

impl Length {
    /// Länge null.
    pub const ZERO: Self = Self(0);

    /// Erzeugt eine Länge aus Millimetern.
    pub const fn from_mm(mm: u32) -> Self {
        Self(mm)
    }

    /// Erzeugt eine Länge aus ganzen Metern; `None` bei Überlauf.
    pub const fn from_m(m: u32) -> Option<Self> {
        match m.checked_mul(1000) {
            Some(mm) => Some(Self(mm)),
            None => None,
        }
    }

    /// Wert in Millimetern.
    pub const fn as_mm(self) -> u32 {
        self.0
    }

    /// Summe; `None` bei Überlauf.
    pub const fn checked_add(self, rhs: Self) -> Option<Self> {
        match self.0.checked_add(rhs.0) {
            Some(v) => Some(Self(v)),
            None => None,
        }
    }

    /// Differenz; `None`, wenn `rhs` größer ist.
    pub const fn checked_sub(self, rhs: Self) -> Option<Self> {
        match self.0.checked_sub(rhs.0) {
            Some(v) => Some(Self(v)),
            None => None,
        }
    }
}

impl fmt::Display for Length {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let metres = self.0.checked_div(1000).unwrap_or(0);
        let mm = self.0.checked_rem(1000).unwrap_or(0);
        if mm == 0 {
            write!(f, "{metres} m")
        } else {
            write!(f, "{metres}.{mm:03} m")
        }
    }
}

/// Geschwindigkeit in km/h.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct Speed(u16);

impl Speed {
    /// Stillstand.
    pub const ZERO: Self = Self(0);

    /// Erzeugt eine Geschwindigkeit aus km/h.
    pub const fn from_kmh(kmh: u16) -> Self {
        Self(kmh)
    }

    /// Wert in km/h.
    pub const fn as_kmh(self) -> u16 {
        self.0
    }
}

impl fmt::Display for Speed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} km/h", self.0)
    }
}
