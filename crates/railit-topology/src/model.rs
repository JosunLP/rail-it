//! Elemente, Ports und Referenzen des Netzmodells.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use railit_types::{AreaId, Length, LxId, PointId, SignalId, Speed, TerminalId, TrackId, TvdId};
pub use railit_types::{PointPosition, SignalKind, SignalSystem};

/// Gleisabschnitt mit den Ports `a` und `b`.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Track {
    /// Fachlicher Name, z. B. `"1"` oder `"W-Einfahrt"`.
    pub name: String,
    /// Länge von Port `a` bis Port `b`.
    pub length: Length,
}

/// Einfache Weiche mit den Ports `tip` (Spitze), `left` und `right`.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Point {
    /// Fachlicher Name, z. B. `"W1"`.
    pub name: String,
    /// Abzweigender Strang, falls bekannt. Der andere Strang ist der gerade.
    pub diverging: Option<PointPosition>,
    /// Zulässige Geschwindigkeit im abzweigenden Strang.
    pub diverging_speed: Speed,
}

/// Geschwindigkeit im abzweigenden Strang, wenn nichts anderes angegeben ist.
pub const DEFAULT_DIVERGING_SPEED: Speed = Speed::from_kmh(40);

/// Art eines Netzendes.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum TerminalKind {
    /// Prellbock: Das Gleis endet physisch.
    BufferStop,
    /// Grenze des modellierten Netzes; das Gleis geht außerhalb weiter.
    Boundary,
    /// Grenze zu einem benachbarten Stellbereich. Fahrten über die Grenze sichert der
    /// Erlaubniswechsel mit dem Nachbarn (Streckenblock).
    Border {
        /// Dieser Bereich übernimmt beim Anlauf die Erlaubnis für die Grenze.
        initial: bool,
    },
}

impl TerminalKind {
    /// Ist das Netzende eine Grenze zu einem anderen Stellbereich?
    pub const fn is_border(self) -> bool {
        matches!(self, Self::Border { .. })
    }
}

/// Netzende mit dem einzigen Port `end`.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Terminal {
    /// Fachlicher Name.
    pub name: String,
    /// Prellbock oder Netzgrenze.
    pub kind: TerminalKind,
}

/// Wirkrichtung eines Punktobjekts relativ zu seinem Gleisabschnitt.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum Direction {
    /// Gilt für Fahrten von Port `a` nach Port `b`.
    AToB,
    /// Gilt für Fahrten von Port `b` nach Port `a`.
    BToA,
}

/// Signal als Punktobjekt auf einem Gleisabschnitt.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Signal {
    /// Fachlicher Name, z. B. `"A"` oder `"N1"`.
    pub name: String,
    /// Gleisabschnitt, auf dem das Signal steht.
    pub track: TrackId,
    /// Position gemessen ab Port `a` des Gleisabschnitts.
    pub position: Length,
    /// Fahrtrichtung, für die das Signal gilt.
    pub direction: Direction,
    /// Signalart.
    pub kind: SignalKind,
    /// Signalsystem (Ks oder H/V).
    pub system: SignalSystem,
    /// Selbstblocksignal: Die Zuglenkung stellt Fahrstraßen ab diesem Signal selbsttätig,
    /// sobald sich ein Zug nähert.
    pub auto: bool,
}

/// Bahnübergang als Punktobjekt auf einem Gleisabschnitt.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct LevelCrossing {
    /// Fachlicher Name, z. B. `"BÜ1"`.
    pub name: String,
    /// Gleisabschnitt, den der Bahnübergang kreuzt.
    pub track: TrackId,
    /// Position gemessen ab Port `a` des Gleisabschnitts.
    pub position: Length,
}

/// Stellbereich: Gleisfreimeldeabschnitte, die genau ein Sicherheitskern steuert.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Area {
    /// Fachlicher Name, z. B. `"Adorf"`.
    pub name: String,
    /// Zugehörige Gleisfreimeldeabschnitte.
    pub tvds: Vec<TvdId>,
}

/// Bahnsteiggleis einer Betriebsstelle.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Platform {
    /// Name, z. B. `"1"`.
    pub name: String,
    /// Signale, vor denen Züge an diesem Bahnsteig halten (je Fahrtrichtung eines).
    pub signals: Vec<SignalId>,
}

/// Betriebsstelle (Bahnhof oder Haltepunkt).
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Station {
    /// Fachlicher Name, z. B. `"Adorf"`.
    pub name: String,
    /// Bahnsteiggleise.
    pub platforms: Vec<Platform>,
}

/// Verbindung zweier Stellbereiche über ein Paar verbundener Ports.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AreaLink {
    /// Bereich auf der einen Seite.
    pub a: AreaId,
    /// Port auf der Seite von `a`.
    pub a_port: PortRef,
    /// Bereich auf der anderen Seite.
    pub b: AreaId,
    /// Port auf der Seite von `b`.
    pub b_port: PortRef,
}

/// Element, das einem Gleisfreimeldeabschnitt angehören kann.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum TvdMember {
    /// Gleisabschnitt.
    Track(TrackId),
    /// Weiche.
    Point(PointId),
}

/// Gleisfreimeldeabschnitt (Train Vacancy Detection).
///
/// In Phase 0 umfasst ein Abschnitt ganze Gleise und Weichen. Abschnittsgrenzen innerhalb
/// eines Gleises werden durch Teilen des Gleises modelliert.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TvdSection {
    /// Fachlicher Name.
    pub name: String,
    /// Zugehörige Gleise und Weichen.
    pub members: Vec<TvdMember>,
}

/// Knoten des Netzgraphen.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum NodeRef {
    /// Gleisabschnitt.
    Track(TrackId),
    /// Weiche.
    Point(PointId),
    /// Netzende.
    Terminal(TerminalId),
}

impl NodeRef {
    /// Gültige Ports dieser Knotenart.
    pub const fn ports(self) -> &'static [Port] {
        match self {
            Self::Track(_) => &[Port::A, Port::B],
            Self::Point(_) => &[Port::Tip, Port::Left, Port::Right],
            Self::Terminal(_) => &[Port::End],
        }
    }

    /// Position von `port` in [`Self::ports`]; `None`, wenn der Port hier nicht existiert.
    pub(crate) fn slot(self, port: Port) -> Option<usize> {
        self.ports().iter().position(|p| *p == port)
    }
}

/// Anschluss eines Knotens.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum Port {
    /// Anfang eines Gleisabschnitts.
    A,
    /// Ende eines Gleisabschnitts.
    B,
    /// Weichenspitze.
    Tip,
    /// Linker Strang einer Weiche (von der Spitze aus gesehen).
    Left,
    /// Rechter Strang einer Weiche (von der Spitze aus gesehen).
    Right,
    /// Einziger Port eines Netzendes.
    End,
}

impl Port {
    /// Name des Ports, wie er in Netzbeschreibungen verwendet wird.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::A => "a",
            Self::B => "b",
            Self::Tip => "tip",
            Self::Left => "left",
            Self::Right => "right",
            Self::End => "end",
        }
    }

    /// Port aus seinem Namen (siehe [`Self::as_str`]).
    pub fn parse(s: &str) -> Option<Self> {
        [
            Self::A,
            Self::B,
            Self::Tip,
            Self::Left,
            Self::Right,
            Self::End,
        ]
        .into_iter()
        .find(|p| p.as_str() == s)
    }
}

impl fmt::Display for Port {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Ein bestimmter Port eines bestimmten Knotens.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PortRef {
    /// Knoten.
    pub node: NodeRef,
    /// Port des Knotens.
    pub port: Port,
}

impl PortRef {
    /// Port `port` von Knoten `node`.
    pub const fn new(node: NodeRef, port: Port) -> Self {
        Self { node, port }
    }
}

/// Beliebiges benanntes Element des Netzes.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum ElementRef {
    /// Gleisabschnitt.
    Track(TrackId),
    /// Weiche.
    Point(PointId),
    /// Netzende.
    Terminal(TerminalId),
    /// Signal.
    Signal(SignalId),
    /// Gleisfreimeldeabschnitt.
    Tvd(TvdId),
    /// Bahnübergang.
    LevelCrossing(LxId),
}

impl From<NodeRef> for ElementRef {
    fn from(node: NodeRef) -> Self {
        match node {
            NodeRef::Track(id) => Self::Track(id),
            NodeRef::Point(id) => Self::Point(id),
            NodeRef::Terminal(id) => Self::Terminal(id),
        }
    }
}

impl From<TvdMember> for ElementRef {
    fn from(member: TvdMember) -> Self {
        match member {
            TvdMember::Track(id) => Self::Track(id),
            TvdMember::Point(id) => Self::Point(id),
        }
    }
}
