//! Validiertes, unveränderliches Netz und Navigation darin.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use railit_types::{
    AreaId, Length, LxId, PointId, SignalId, StationId, TerminalId, TrackId, TvdId,
};

use crate::model::{
    Area, AreaLink, Coupling, Crossing, ElementRef, LevelCrossing, NodeRef, Point, PointPosition,
    Port, PortRef, Signal, Slip, Station, Terminal, Track, TvdMember, TvdSection,
};

/// Verbindungstabelle: pro Knoten und Port der verbundene Gegenport.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Links {
    pub(crate) tracks: Vec<[Option<PortRef>; 2]>,
    pub(crate) points: Vec<[Option<PortRef>; 3]>,
    pub(crate) terminals: Vec<[Option<PortRef>; 1]>,
}

impl Links {
    /// Eintrag für `port`; `None`, wenn Knoten oder Port nicht existieren.
    pub(crate) fn slot(&self, port: PortRef) -> Option<&Option<PortRef>> {
        let slot = port.node.slot(port.port)?;
        match port.node {
            NodeRef::Track(id) => self.tracks.get(id.index()?)?.get(slot),
            NodeRef::Point(id) => self.points.get(id.index()?)?.get(slot),
            NodeRef::Terminal(id) => self.terminals.get(id.index()?)?.get(slot),
        }
    }

    /// Veränderlicher Eintrag für `port`.
    pub(crate) fn slot_mut(&mut self, port: PortRef) -> Option<&mut Option<PortRef>> {
        let slot = port.node.slot(port.port)?;
        match port.node {
            NodeRef::Track(id) => self.tracks.get_mut(id.index()?)?.get_mut(slot),
            NodeRef::Point(id) => self.points.get_mut(id.index()?)?.get_mut(slot),
            NodeRef::Terminal(id) => self.terminals.get_mut(id.index()?)?.get_mut(slot),
        }
    }

    /// Alle Knoten in fester Reihenfolge: Gleise, Weichen, Netzenden.
    pub(crate) fn nodes(&self) -> impl Iterator<Item = NodeRef> + '_ {
        let tracks = (0..self.tracks.len())
            .filter_map(TrackId::from_index)
            .map(NodeRef::Track);
        let points = (0..self.points.len())
            .filter_map(PointId::from_index)
            .map(NodeRef::Point);
        let terminals = (0..self.terminals.len())
            .filter_map(TerminalId::from_index)
            .map(NodeRef::Terminal);
        tracks.chain(points).chain(terminals)
    }

    /// Fortlaufender Index eines Knotens über alle Knotenarten.
    fn flat_index(&self, node: NodeRef) -> Option<usize> {
        match node {
            NodeRef::Track(id) => id.index(),
            NodeRef::Point(id) => self.tracks.len().checked_add(id.index()?),
            NodeRef::Terminal(id) => self
                .tracks
                .len()
                .checked_add(self.points.len())?
                .checked_add(id.index()?),
        }
    }

    /// Anzahl zusammenhängender Teilnetze.
    pub(crate) fn components(&self) -> usize {
        let total = self
            .tracks
            .len()
            .saturating_add(self.points.len())
            .saturating_add(self.terminals.len());
        let mut visited = vec![false; total];
        let mut stack = Vec::new();
        let mut components = 0usize;

        for start in self.nodes() {
            let Some(flag) = self.flat_index(start).and_then(|i| visited.get_mut(i)) else {
                continue;
            };
            if *flag {
                continue;
            }
            *flag = true;
            components = components.saturating_add(1);
            stack.push(start);

            while let Some(node) = stack.pop() {
                for &port in node.ports() {
                    let Some(Some(other)) = self.slot(PortRef::new(node, port)) else {
                        continue;
                    };
                    if let Some(seen) = self.flat_index(other.node).and_then(|i| visited.get_mut(i))
                        && !*seen
                    {
                        *seen = true;
                        stack.push(other.node);
                    }
                }
            }
        }
        components
    }
}

/// Möglicher Ausgang beim Durchfahren eines Knotens, siehe [`Topology::exits`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Exit {
    /// Port, über den der Knoten verlassen wird.
    pub port: PortRef,
    /// Weichenlage, die für diesen Weg erforderlich ist.
    pub requires: Option<(PointId, PointPosition)>,
}

/// Iterator über die Ausgänge eines Knotens (höchstens zwei).
#[derive(Clone, Debug)]
pub struct Exits {
    items: [Option<Exit>; 2],
    next: usize,
}

impl Exits {
    const fn none() -> Self {
        Self {
            items: [None, None],
            next: 0,
        }
    }

    const fn one(exit: Exit) -> Self {
        Self {
            items: [Some(exit), None],
            next: 0,
        }
    }

    const fn two(first: Exit, second: Exit) -> Self {
        Self {
            items: [Some(first), Some(second)],
            next: 0,
        }
    }
}

impl Iterator for Exits {
    type Item = Exit;

    fn next(&mut self) -> Option<Exit> {
        let item = self.items.get(self.next).copied().flatten();
        if item.is_some() {
            self.next = self.next.saturating_add(1);
        }
        item
    }
}

/// Validiertes, unveränderliches Netz.
///
/// Erzeugt ausschließlich über [`crate::TopologyBuilder::build`]; damit ist jede `Topology`
/// geprüft (REQ-TOP-008).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Topology {
    pub(crate) name: String,
    pub(crate) tracks: Vec<Track>,
    pub(crate) points: Vec<Point>,
    pub(crate) terminals: Vec<Terminal>,
    pub(crate) signals: Vec<Signal>,
    pub(crate) tvds: Vec<TvdSection>,
    pub(crate) level_crossings: Vec<LevelCrossing>,
    pub(crate) crossings: Vec<Crossing>,
    pub(crate) couplings: Vec<Coupling>,
    pub(crate) slips: Vec<Slip>,
    pub(crate) links: Links,
    pub(crate) names: BTreeMap<String, ElementRef>,
    pub(crate) track_tvd: Vec<Option<TvdId>>,
    pub(crate) point_tvd: Vec<Option<TvdId>>,
    pub(crate) areas: Vec<Area>,
    pub(crate) stations: Vec<Station>,
    pub(crate) tvd_area: Vec<Option<AreaId>>,
}

impl Topology {
    /// Alle Stellbereiche (leer, wenn das Netz nicht aufgeteilt ist).
    pub fn areas(&self) -> impl ExactSizeIterator<Item = (AreaId, &Area)> {
        enumerate_ids(&self.areas)
    }

    /// Stellbereich `id`.
    pub fn area(&self, id: AreaId) -> Option<&Area> {
        self.areas.get(id.index()?)
    }

    /// Stellbereich mit dem Namen `name`.
    pub fn find_area(&self, name: &str) -> Option<AreaId> {
        self.areas().find(|(_, a)| a.name == name).map(|(id, _)| id)
    }

    /// Stellbereich eines Gleisfreimeldeabschnitts.
    pub fn area_of_tvd(&self, tvd: TvdId) -> Option<AreaId> {
        self.tvd_area.get(tvd.index()?).copied().flatten()
    }

    /// Stellbereich eines Gleises oder einer Weiche; Netzenden gehören zum Bereich des
    /// Knotens, an den sie angeschlossen sind.
    pub fn area_of_node(&self, node: NodeRef) -> Option<AreaId> {
        match node {
            NodeRef::Track(t) => self.area_of_tvd(self.tvd_of(TvdMember::Track(t))?),
            NodeRef::Point(p) => self.area_of_tvd(self.tvd_of(TvdMember::Point(p))?),
            NodeRef::Terminal(t) => {
                let inside = self.linked(PortRef::new(NodeRef::Terminal(t), Port::End))?;
                match inside.node {
                    NodeRef::Terminal(_) => None,
                    node => self.area_of_node(node),
                }
            }
        }
    }

    /// Verbindungen zwischen verschiedenen Stellbereichen, je Paar einmal (`a < b` nach
    /// Port-Reihenfolge).
    pub fn area_links(&self) -> Vec<AreaLink> {
        let mut out = Vec::new();
        for node in self.links.nodes() {
            for &port in node.ports() {
                let here = PortRef::new(node, port);
                let Some(there) = self.linked(here) else {
                    continue;
                };
                if here >= there {
                    continue;
                }
                if let (Some(a), Some(b)) =
                    (self.area_of_node(here.node), self.area_of_node(there.node))
                    && a != b
                {
                    out.push(AreaLink {
                        a,
                        a_port: here,
                        b,
                        b_port: there,
                    });
                }
            }
        }
        out
    }

    /// Alle Betriebsstellen.
    pub fn stations(&self) -> impl ExactSizeIterator<Item = (StationId, &Station)> {
        enumerate_ids(&self.stations)
    }

    /// Betriebsstelle `id`.
    pub fn station(&self, id: StationId) -> Option<&Station> {
        self.stations.get(id.index()?)
    }

    /// Betriebsstelle mit dem Namen `name`.
    pub fn find_station(&self, name: &str) -> Option<StationId> {
        self.stations()
            .find(|(_, s)| s.name == name)
            .map(|(id, _)| id)
    }
    /// Name des Netzes.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Alle Gleisabschnitte.
    pub fn tracks(&self) -> impl ExactSizeIterator<Item = (TrackId, &Track)> {
        enumerate_ids(&self.tracks)
    }

    /// Alle Weichen.
    pub fn points(&self) -> impl ExactSizeIterator<Item = (PointId, &Point)> {
        enumerate_ids(&self.points)
    }

    /// Alle Netzenden.
    pub fn terminals(&self) -> impl ExactSizeIterator<Item = (TerminalId, &Terminal)> {
        enumerate_ids(&self.terminals)
    }

    /// Alle Signale.
    pub fn signals(&self) -> impl ExactSizeIterator<Item = (SignalId, &Signal)> {
        enumerate_ids(&self.signals)
    }

    /// Alle Gleisfreimeldeabschnitte.
    pub fn tvds(&self) -> impl ExactSizeIterator<Item = (TvdId, &TvdSection)> {
        enumerate_ids(&self.tvds)
    }

    /// Alle Kreuzungen.
    pub fn crossings(&self) -> impl ExactSizeIterator<Item = &Crossing> {
        self.crossings.iter()
    }

    /// Kreuzungsstellen auf Gleis `track`: Kreuzung und Position ab Port `a`.
    pub fn crossings_on(&self, track: TrackId) -> impl Iterator<Item = (&Crossing, Length)> {
        self.crossings.iter().flat_map(move |c| {
            c.legs
                .iter()
                .filter(move |(t, _)| *t == track)
                .map(move |(_, at)| (c, *at))
        })
    }

    /// Alle Kopplungen von Weichen (REQ-TOP-017).
    pub fn couplings(&self) -> impl ExactSizeIterator<Item = &Coupling> {
        self.couplings.iter()
    }

    /// Kopplung, zu der Weiche `point` gehört.
    pub fn coupling_of(&self, point: PointId) -> Option<&Coupling> {
        self.couplings.iter().find(|c| c.points.contains(&point))
    }

    /// Partnerweiche und ihre Lage, wenn `point` in `pos` liegt.
    pub fn coupled(&self, point: PointId, pos: PointPosition) -> Option<(PointId, PointPosition)> {
        self.coupling_of(point)?.partner(point, pos)
    }

    /// Alle Kreuzungsweichen.
    pub fn slips(&self) -> impl ExactSizeIterator<Item = &Slip> {
        self.slips.iter()
    }

    /// Alle Bahnübergänge.
    pub fn level_crossings(&self) -> impl ExactSizeIterator<Item = (LxId, &LevelCrossing)> {
        enumerate_ids(&self.level_crossings)
    }

    /// Bahnübergang `id`.
    pub fn level_crossing(&self, id: LxId) -> Option<&LevelCrossing> {
        self.level_crossings.get(id.index()?)
    }

    /// Bahnübergänge auf Gleisabschnitt `track`.
    pub fn level_crossings_on(
        &self,
        track: TrackId,
    ) -> impl Iterator<Item = (LxId, &LevelCrossing)> {
        self.level_crossings()
            .filter(move |(_, l)| l.track == track)
    }

    /// Gleisabschnitt `id`.
    pub fn track(&self, id: TrackId) -> Option<&Track> {
        self.tracks.get(id.index()?)
    }

    /// Weiche `id`.
    pub fn point(&self, id: PointId) -> Option<&Point> {
        self.points.get(id.index()?)
    }

    /// Netzende `id`.
    pub fn terminal(&self, id: TerminalId) -> Option<&Terminal> {
        self.terminals.get(id.index()?)
    }

    /// Signal `id`.
    pub fn signal(&self, id: SignalId) -> Option<&Signal> {
        self.signals.get(id.index()?)
    }

    /// Gleisfreimeldeabschnitt `id`.
    pub fn tvd(&self, id: TvdId) -> Option<&TvdSection> {
        self.tvds.get(id.index()?)
    }

    /// Element mit dem fachlichen Namen `name`.
    pub fn lookup(&self, name: &str) -> Option<ElementRef> {
        self.names.get(name).copied()
    }

    /// Fachlicher Name eines Elements.
    pub fn element_name(&self, element: ElementRef) -> Option<&str> {
        let name = match element {
            ElementRef::Track(id) => &self.track(id)?.name,
            ElementRef::Point(id) => &self.point(id)?.name,
            ElementRef::Terminal(id) => &self.terminal(id)?.name,
            ElementRef::Signal(id) => &self.signal(id)?.name,
            ElementRef::Tvd(id) => &self.tvd(id)?.name,
            ElementRef::LevelCrossing(id) => &self.level_crossing(id)?.name,
        };
        Some(name)
    }

    /// Port, mit dem `port` verbunden ist.
    pub fn linked(&self, port: PortRef) -> Option<PortRef> {
        self.links.slot(port).copied().flatten()
    }

    /// Gleisfreimeldeabschnitt, zu dem ein Gleis oder eine Weiche gehört.
    pub fn tvd_of(&self, member: TvdMember) -> Option<TvdId> {
        match member {
            TvdMember::Track(id) => self.track_tvd.get(id.index()?).copied().flatten(),
            TvdMember::Point(id) => self.point_tvd.get(id.index()?).copied().flatten(),
        }
    }

    /// Signale auf Gleisabschnitt `track`.
    pub fn signals_on(&self, track: TrackId) -> impl Iterator<Item = (SignalId, &Signal)> {
        self.signals().filter(move |(_, s)| s.track == track)
    }

    /// Mögliche Ausgänge, wenn der Knoten über `entry` befahren wird (REQ-TOP-007).
    ///
    /// | Knoten  | Eingang       | Ausgänge                                      |
    /// |---------|---------------|-----------------------------------------------|
    /// | Gleis   | `a` / `b`     | `b` / `a`                                     |
    /// | Weiche  | `tip`         | `left` (Lage links), `right` (Lage rechts)    |
    /// | Weiche  | `left`        | `tip` (Lage links)                            |
    /// | Weiche  | `right`       | `tip` (Lage rechts)                           |
    /// | Netzende| `end`         | keine                                         |
    ///
    /// Für Knoten oder Ports, die es nicht gibt, ist das Ergebnis leer.
    pub fn exits(&self, entry: PortRef) -> Exits {
        if self.links.slot(entry).is_none() {
            return Exits::none();
        }
        let at = |port| PortRef::new(entry.node, port);
        match (entry.node, entry.port) {
            (NodeRef::Track(_), Port::A) => Exits::one(Exit {
                port: at(Port::B),
                requires: None,
            }),
            (NodeRef::Track(_), Port::B) => Exits::one(Exit {
                port: at(Port::A),
                requires: None,
            }),
            (NodeRef::Point(id), Port::Tip) => Exits::two(
                Exit {
                    port: at(Port::Left),
                    requires: Some((id, PointPosition::Left)),
                },
                Exit {
                    port: at(Port::Right),
                    requires: Some((id, PointPosition::Right)),
                },
            ),
            (NodeRef::Point(id), Port::Left) => Exits::one(Exit {
                port: at(Port::Tip),
                requires: Some((id, PointPosition::Left)),
            }),
            (NodeRef::Point(id), Port::Right) => Exits::one(Exit {
                port: at(Port::Tip),
                requires: Some((id, PointPosition::Right)),
            }),
            _ => Exits::none(),
        }
    }

    /// Anzahl zusammenhängender Teilnetze.
    pub fn components(&self) -> usize {
        self.links.components()
    }
}

fn enumerate_ids<T, Id>(items: &[T]) -> impl ExactSizeIterator<Item = (Id, &T)>
where
    Id: FromIndex,
{
    // Die Arenen sind beim Bau auf `u32::MAX` Einträge begrenzt, die Umwandlung gelingt immer.
    items
        .iter()
        .enumerate()
        .map(|(i, item)| (Id::from_index_saturating(i), item))
}

/// Hilfstrait, um über die verschiedenen ID-Typen generisch zu iterieren.
trait FromIndex {
    fn from_index_saturating(index: usize) -> Self;
}

macro_rules! impl_from_index {
    ($($id:ty),*) => {$(
        impl FromIndex for $id {
            fn from_index_saturating(index: usize) -> Self {
                <$id>::new(u32::try_from(index).unwrap_or(u32::MAX))
            }
        }
    )*};
}

impl_from_index!(
    TrackId, PointId, TerminalId, SignalId, TvdId, LxId, AreaId, StationId
);
