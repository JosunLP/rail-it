//! Fahrstraßengraph des ganzen Netzes über alle Stellbereiche.
//!
//! Knoten sind Hauptsignale (mit ihrem Namen, der im ganzen Netz eindeutig ist), Kanten die
//! Fahrstraßen. Eine Fahrstraße, die an einer Bereichsgrenze endet, führt zum Einfahrsignal
//! des Nachbarbereichs an dieser Grenze.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

use railit_plan::{PlanView, RouteEnd};
use railit_topology::Topology;
use railit_types::{BorderId, RouteId};

/// Wohin eine Fahrstraße führt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RouteNext {
    /// Zum Zielsignal (bei einer Bereichsgrenze: Einfahrsignal des Nachbarn).
    Signal(String),
    /// Aus dem Netz hinaus.
    Leave,
    /// An einen Prellbock.
    BufferStop,
}

/// Fahrstraße im Graphen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpsRoute {
    /// Stellbereich (Index wie in der Runtime).
    pub area: usize,
    /// Kennung im Plan des Bereichs.
    pub id: RouteId,
    /// Name.
    pub name: String,
    /// Startsignal.
    pub start: String,
    /// Ziel.
    pub next: RouteNext,
    /// Grenze, an der die Fahrstraße endet; der Bereich muss deren Erlaubnis halten.
    pub border: Option<BorderId>,
}

/// Bahnsteig einer Betriebsstelle mit den Signalen, vor denen Züge halten.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlatformInfo {
    /// Name des Bahnsteiggleises.
    pub name: String,
    /// Haltsignale (je Fahrtrichtung eines).
    pub signals: Vec<String>,
}

/// Betriebsstelle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StationInfo {
    /// Name.
    pub name: String,
    /// Bahnsteiggleise.
    pub platforms: Vec<PlatformInfo>,
}

/// Fahrstraßengraph mit Betriebsstellen und Selbstblocksignalen.
#[derive(Clone, Debug, Default)]
pub struct OpsNet {
    areas: Vec<String>,
    border_names: HashMap<(usize, BorderId), String>,
    routes: Vec<OpsRoute>,
    by_kernel: HashMap<(usize, RouteId), usize>,
    from_signal: BTreeMap<String, Vec<usize>>,
    signals: BTreeSet<String>,
    stations: Vec<StationInfo>,
    auto: BTreeSet<String>,
}

impl OpsNet {
    /// Baut den Graphen aus den Plänen der Stellbereiche (in der Reihenfolge der Runtime),
    /// der Paarung der Grenzen und dem Netz (Betriebsstellen, Selbstblocksignale).
    pub fn new(
        areas: &[(&str, &PlanView<'_>)],
        peer: impl Fn(usize, BorderId) -> Option<(usize, BorderId)>,
        topology: &Topology,
    ) -> Self {
        let mut graph = Self {
            areas: areas.iter().map(|(n, _)| (*n).to_string()).collect(),
            ..Self::default()
        };
        for (area, (_, plan)) in areas.iter().enumerate() {
            for b in (0..plan.border_count()).filter_map(BorderId::from_index) {
                if let Some(name) = plan.border_name(b) {
                    graph.border_names.insert((area, b), name.to_string());
                }
            }
            for s in (0..plan.signal_count()).filter_map(railit_types::SignalId::from_index) {
                if let Some(name) = plan.signal_name(s) {
                    graph.signals.insert(name.to_string());
                }
            }
            for r in plan.routes() {
                let Some(start) = plan.signal_name(r.start()) else {
                    continue;
                };
                let (next, border) = match r.end() {
                    RouteEnd::Signal(s) => (
                        plan.signal_name(s)
                            .map_or(RouteNext::Leave, |n| RouteNext::Signal(n.to_string())),
                        None,
                    ),
                    RouteEnd::Boundary => (RouteNext::Leave, None),
                    RouteEnd::BufferStop => (RouteNext::BufferStop, None),
                    RouteEnd::Border(b) => {
                        let entry = peer(area, b).and_then(|(pa, pb)| {
                            let (_, p) = areas.get(pa)?;
                            p.signal_name(p.border_entry_signal(pb)?)
                        });
                        (
                            entry.map_or(RouteNext::Leave, |n| RouteNext::Signal(n.to_string())),
                            Some(b),
                        )
                    }
                };
                let index = graph.routes.len();
                graph.by_kernel.insert((area, r.id()), index);
                graph
                    .from_signal
                    .entry(start.to_string())
                    .or_default()
                    .push(index);
                graph.routes.push(OpsRoute {
                    area,
                    id: r.id(),
                    name: r.name().to_string(),
                    start: start.to_string(),
                    next,
                    border,
                });
            }
        }
        let signal_name = |id| topology.signal(id).map(|s| s.name.clone());
        graph.stations = topology
            .stations()
            .map(|(_, s)| StationInfo {
                name: s.name.clone(),
                platforms: s
                    .platforms
                    .iter()
                    .map(|p| PlatformInfo {
                        name: p.name.clone(),
                        signals: p.signals.iter().filter_map(|&id| signal_name(id)).collect(),
                    })
                    .collect(),
            })
            .collect();
        graph.auto = topology
            .signals()
            .filter(|(_, s)| s.auto)
            .map(|(_, s)| s.name.clone())
            .collect();
        graph
    }

    /// Name eines Stellbereichs.
    pub fn area_name(&self, area: usize) -> Option<&str> {
        self.areas.get(area).map(String::as_str)
    }

    /// Adresse einer Grenze, z. B. `"Adorf>Strecke"`.
    pub fn border_address(&self, area: usize, border: BorderId) -> String {
        format!(
            "{}{}",
            self.area_name(area).unwrap_or("?"),
            self.border_names
                .get(&(area, border))
                .map_or("?", String::as_str)
        )
    }

    /// Fahrstraße nach Index.
    pub fn route(&self, index: usize) -> Option<&OpsRoute> {
        self.routes.get(index)
    }

    /// Alle Fahrstraßen.
    pub fn routes(&self) -> &[OpsRoute] {
        &self.routes
    }

    /// Index einer Fahrstraße aus Bereich und Kennung.
    pub fn find_kernel_route(&self, area: usize, id: RouteId) -> Option<usize> {
        self.by_kernel.get(&(area, id)).copied()
    }

    /// Fahrstraßen ab einem Signal.
    pub fn routes_from(&self, signal: &str) -> &[usize] {
        self.from_signal.get(signal).map_or(&[], Vec::as_slice)
    }

    /// Gibt es das Hauptsignal?
    pub fn has_signal(&self, signal: &str) -> bool {
        self.signals.contains(signal)
    }

    /// Ist das Signal ein Selbstblocksignal?
    pub fn is_auto(&self, signal: &str) -> bool {
        self.auto.contains(signal)
    }

    /// Betriebsstellen.
    pub fn stations(&self) -> &[StationInfo] {
        &self.stations
    }

    /// Haltsignale eines Bahnsteiggleises.
    pub fn platform_signals(&self, station: &str, platform: &str) -> Option<&[String]> {
        self.stations
            .iter()
            .find(|s| s.name == station)?
            .platforms
            .iter()
            .find(|p| p.name == platform)
            .map(|p| p.signals.as_slice())
    }

    /// Kürzester Fahrweg (nach Anzahl der Fahrstraßen) von `from` zu einem Ziel, das `goal`
    /// erfüllt. Ergebnis: Indizes der Fahrstraßen in Fahrtrichtung.
    pub fn path(&self, from: &str, goal: impl Fn(&RouteNext) -> bool) -> Option<Vec<usize>> {
        // Breitensuche über Signale; je Signal die Fahrstraße, über die es erreicht wurde.
        let mut reached: HashMap<&str, Option<usize>> = HashMap::new();
        let mut queue = VecDeque::new();
        reached.insert(from, None);
        queue.push_back(from);
        while let Some(signal) = queue.pop_front() {
            for &r in self.routes_from(signal) {
                let Some(route) = self.routes.get(r) else {
                    continue;
                };
                if goal(&route.next) {
                    let mut path = vec![r];
                    let mut at = signal;
                    while let Some(Some(prev)) = reached.get(at) {
                        path.push(*prev);
                        at = self.routes.get(*prev).map_or(from, |p| p.start.as_str());
                    }
                    path.reverse();
                    return Some(path);
                }
                if let RouteNext::Signal(next) = &route.next
                    && !reached.contains_key(next.as_str())
                {
                    reached.insert(next.as_str(), Some(r));
                    queue.push_back(next.as_str());
                }
            }
        }
        None
    }
}
