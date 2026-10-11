//! Weichenlagen.

use core::fmt;

/// Lage einer Weiche.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum PointPosition {
    /// Spitze ↔ linker Strang.
    Left,
    /// Spitze ↔ rechter Strang.
    Right,
}

impl PointPosition {
    /// Die jeweils andere Lage.
    #[must_use]
    pub const fn other(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }

    /// Name der Lage, wie er in Netzbeschreibungen und der API verwendet wird.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
        }
    }

    /// Lage aus ihrem Namen (siehe [`Self::as_str`]).
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "left" => Some(Self::Left),
            "right" => Some(Self::Right),
            _ => None,
        }
    }
}

impl fmt::Display for PointPosition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
