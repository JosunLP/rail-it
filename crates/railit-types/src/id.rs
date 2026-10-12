//! Typsichere Indizes.
//!
//! Elemente werden in Arenen (`Vec`/Slices) gehalten und über `u32`-Indizes adressiert.
//! Jede Elementart hat einen eigenen Typ, damit z. B. eine Weichen-ID nie versehentlich
//! als Signal-ID verwendet werden kann.

use core::fmt;

macro_rules! define_id {
    ($(#[$meta:meta])* $name:ident, $prefix:literal) => {
        $(#[$meta])*
        #[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
        #[cfg_attr(feature = "serde", serde(transparent))]
        pub struct $name(u32);

        impl $name {
            /// Erzeugt eine ID aus ihrem Rohwert.
            pub const fn new(raw: u32) -> Self {
                Self(raw)
            }

            /// Rohwert der ID.
            pub const fn raw(self) -> u32 {
                self.0
            }

            /// Index in die zugehörige Arena.
            ///
            /// Gibt `None` zurück, wenn der Wert auf der Zielplattform nicht in `usize` passt.
            pub fn index(self) -> Option<usize> {
                usize::try_from(self.0).ok()
            }

            /// Erzeugt eine ID aus einem Arena-Index, sofern er in `u32` passt.
            pub fn from_index(index: usize) -> Option<Self> {
                u32::try_from(index).ok().map(Self)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!($prefix, "#{}"), self.0)
            }
        }
    };
}

define_id!(
    /// Gleisabschnitt (Kante mit den Ports `A` und `B`).
    TrackId,
    "Track"
);
define_id!(
    /// Weiche (Ports `Spitze`, `Links`, `Rechts`).
    PointId,
    "Point"
);
define_id!(
    /// Netzende: Prellbock oder Netzgrenze (ein Port).
    TerminalId,
    "Terminal"
);
define_id!(
    /// Signal oder ETCS-Halt-Tafel als Punktobjekt auf einem Gleisabschnitt.
    SignalId,
    "Signal"
);
define_id!(
    /// Gleisfreimeldeabschnitt (Train Vacancy Detection).
    TvdId,
    "Tvd"
);
define_id!(
    /// Fahrstraße (Zug- oder Rangierstraße) aus der Projektierung.
    RouteId,
    "Route"
);
define_id!(
    /// Bahnübergang (BÜ) als Punktobjekt auf einem Gleisabschnitt.
    LxId,
    "Lx"
);
define_id!(
    /// Stellbereich: Teil des Netzes, den genau ein Sicherheitskern steuert.
    AreaId,
    "Area"
);
define_id!(
    /// Betriebsstelle (Bahnhof, Haltepunkt) mit Bahnsteiggleisen.
    StationId,
    "Station"
);
define_id!(
    /// Grenze eines Stellbereichs zu einem benachbarten Stellbereich.
    BorderId,
    "Border"
);
