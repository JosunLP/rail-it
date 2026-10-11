//! Signalarten und Signalsysteme.

/// Art eines Signals.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum SignalKind {
    /// Hauptsignal (H/V: Hp; Ks: Ks-Hauptsignal).
    Main,
    /// Vorsignal.
    Distant,
    /// Mehrabschnittssignal bzw. Hauptsignal mit Vorsignal am Mast.
    Combined,
    /// Rangiersignal (Sh).
    Shunting,
    /// ETCS-Halt-Tafel (Ne 14) als Start- bzw. Zielpunkt im ETCS-Betrieb.
    EtcsStopMarker,
}

impl SignalKind {
    /// Kodierung im Plan-Image.
    pub const fn code(self) -> u32 {
        match self {
            Self::Main => 0,
            Self::Distant => 1,
            Self::Combined => 2,
            Self::Shunting => 3,
            Self::EtcsStopMarker => 4,
        }
    }

    /// Aus der Kodierung im Plan-Image.
    pub const fn from_code(code: u32) -> Option<Self> {
        match code {
            0 => Some(Self::Main),
            1 => Some(Self::Distant),
            2 => Some(Self::Combined),
            3 => Some(Self::Shunting),
            4 => Some(Self::EtcsStopMarker),
            _ => None,
        }
    }

    /// Beginnen und enden Fahrstraßen an dieser Signalart?
    pub const fn delimits_routes(self) -> bool {
        matches!(self, Self::Main | Self::Combined | Self::EtcsStopMarker)
    }
}

/// Signalsystem, in dem ein Signal seine Begriffe zeigt.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum SignalSystem {
    /// Ks-Signalsystem (Ril 301).
    #[default]
    Ks,
    /// H/V-Signalsystem (Hp und Vr).
    Hv,
}

impl SignalSystem {
    /// Kodierung im Plan-Image.
    pub const fn code(self) -> u32 {
        match self {
            Self::Ks => 0,
            Self::Hv => 1,
        }
    }

    /// Aus der Kodierung im Plan-Image.
    pub const fn from_code(code: u32) -> Option<Self> {
        match code {
            0 => Some(Self::Ks),
            1 => Some(Self::Hv),
            _ => None,
        }
    }
}
