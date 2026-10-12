//! Zyklische Kopplung von Kernen und Feld.
//!
//! Ein Netz besteht aus einem oder mehreren **Stellbereichen**, je mit einem eigenen Kern
//! (Konzept, Abschnitt 3, Prinzip 7). Die Runtime
//!
//! 1. schreibt das Feld fort und verteilt seine Meldungen auf die Bereiche (die Feldebene
//!    arbeitet mit den IDs des Gesamtnetzes, jeder Kern mit den IDs seines Teilnetzes),
//! 2. führt jeden Kern einen Zyklus aus,
//! 3. gibt die Stellbefehle ans Feld und stellt Grenzmeldungen im **nächsten** Zyklus dem
//!    Nachbarkern zu.
//!
//! Die Kerne teilen keinen Zustand; sie sprechen nur über Grenzmeldungen miteinander.

use std::io::Write;

use railit_journal::{Header, JournalWriter};
use railit_kernel::{
    AuxOperation, BorderMessage, Command, Event, Input, Interlocking, OwnedState, Request,
    RequestId, SafeStateReason, VecOutputs,
};
use railit_plan::PlanView;
use railit_rules::GenericRules;
use railit_topology::{AreaCut, Topology};
use railit_types::{BorderId, Duration, LxId, PointId, RouteId, SignalId, Tick, TvdId};

use crate::directory::Directory;
use crate::field::FieldAdapter;
use crate::names::{NamedAux, NamedEvent, Snapshot};

/// Ereignis eines Kerns mit seinem Bereich.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct AreaEvent {
    /// Index des Bereichs.
    pub area: usize,
    /// Ereignis.
    pub event: Event,
}

/// Anforderung an den Kern eines Bereichs.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Addressed {
    /// Index des Bereichs.
    pub area: usize,
    /// Kennung der Anforderung.
    pub id: RequestId,
    /// Anforderung.
    pub request: Request,
}

/// Ergebnis eines Zyklus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CycleOutput {
    /// Zeitpunkt des Zyklus.
    pub now: Tick,
    /// Ereignisse aller Kerne.
    pub events: Vec<AreaEvent>,
    /// Stellbefehle ans Feld (IDs des Gesamtnetzes).
    pub commands: Vec<Command>,
    /// Erster Bereich, dessen Kern im sicheren Zustand ist, mit Grund.
    pub safe_state: Option<(usize, SafeStateReason)>,
}

/// Abbildung lokaler IDs eines Bereichs auf das Gesamtnetz (Index = lokale ID).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct AreaMap {
    points: Vec<PointId>,
    signals: Vec<SignalId>,
    tvds: Vec<TvdId>,
    lxs: Vec<LxId>,
}

impl AreaMap {
    fn identity(plan: &PlanView<'_>) -> Self {
        Self {
            points: (0..plan.point_count())
                .filter_map(PointId::from_index)
                .collect(),
            signals: (0..plan.signal_count())
                .filter_map(SignalId::from_index)
                .collect(),
            tvds: (0..plan.tvd_count())
                .filter_map(TvdId::from_index)
                .collect(),
            lxs: (0..plan.lx_count()).filter_map(LxId::from_index).collect(),
        }
    }

    fn from_cut(cut: &AreaCut) -> Self {
        Self {
            points: cut.points.clone(),
            signals: cut.signals.clone(),
            tvds: cut.tvds.clone(),
            lxs: cut.lxs.clone(),
        }
    }
}

/// Umkehrabbildung: globale ID → (Bereich, lokale ID).
type Reverse = Vec<Option<(usize, u32)>>;

fn reverse<T: Copy>(
    areas: &[Area],
    get: impl Fn(&AreaMap) -> &Vec<T>,
    index: impl Fn(T) -> Option<usize>,
    len: usize,
) -> Reverse {
    let mut out = vec![None; len];
    for (a, area) in areas.iter().enumerate() {
        for (local, global) in get(&area.map).iter().enumerate() {
            if let (Some(slot), Ok(local)) = (
                index(*global).and_then(|g| out.get_mut(g)),
                u32::try_from(local),
            ) {
                *slot = Some((a, local));
            }
        }
    }
    out
}

fn lookup(rev: &Reverse, global: Option<usize>) -> Option<(usize, u32)> {
    rev.get(global?).copied().flatten()
}

/// Ein Stellbereich mit seinem Kern.
struct Area {
    name: String,
    kernel: Interlocking<'static, GenericRules>,
    state: OwnedState,
    map: AreaMap,
    /// Je Grenze: Bereich und Grenze auf der anderen Seite.
    peers: Vec<Option<(usize, BorderId)>>,
    /// Grenzmeldungen, die im nächsten Zyklus zugestellt werden.
    inbox: Vec<Option<BorderMessage>>,
    out: VecOutputs,
    inputs: Vec<Input>,
    journal: Option<JournalWriter<Box<dyn Write + Send>>>,
    journal_error: Option<String>,
}

impl Area {
    fn new(name: String, plan: PlanView<'static>, rules: GenericRules, map: AreaMap) -> Self {
        Self {
            name,
            kernel: Interlocking::new(plan, rules),
            state: OwnedState::new(&plan),
            map,
            peers: vec![None; plan.border_count()],
            inbox: vec![None; plan.border_count()],
            out: VecOutputs::default(),
            inputs: Vec::new(),
            journal: None,
            journal_error: None,
        }
    }
}

/// Runtime: ein Kern je Stellbereich, ein Feld, ein fester Zyklus.
pub struct Runtime<F> {
    network: String,
    areas: Vec<Area>,
    directory: Directory,
    field: F,
    now: Tick,
    cycle: Duration,
    points: Reverse,
    signals: Reverse,
    tvds: Reverse,
    lxs: Reverse,
    field_inputs: Vec<Input>,
}

impl<F: FieldAdapter> Runtime<F> {
    /// Runtime mit einem einzigen Kern für ein nicht aufgeteiltes Netz.
    pub fn new(plan: PlanView<'static>, rules: GenericRules, field: F, cycle: Duration) -> Self {
        let name = plan.network_name().to_string();
        let area = Area::new(name.clone(), plan, rules, AreaMap::identity(&plan));
        Self::assemble(name, vec![area], field, cycle)
    }

    /// Runtime für ein Netz mit Stellbereichen: je Bereich Plan und Teilnetz
    /// (siehe `railit_planning::compile_areas`).
    ///
    /// # Errors
    /// Beschreibung, wenn ein Plan nicht zu seinem Teilnetz passt oder sich Grenzen nicht
    /// paaren lassen.
    pub fn for_network(
        net: &Topology,
        areas: Vec<(PlanView<'static>, AreaCut)>,
        rules: GenericRules,
        field: F,
        cycle: Duration,
    ) -> Result<Self, String> {
        let mut built = Vec::with_capacity(areas.len());
        let mut cuts = Vec::with_capacity(areas.len());
        for (plan, cut) in areas {
            let name = net
                .area(cut.area)
                .map(|a| a.name.clone())
                .ok_or("unbekannter Stellbereich")?;
            crate::check_correspondence(&plan, &cut.topology)
                .map_err(|e| format!("Stellbereich „{name}“: {e}"))?;
            if plan.border_count() != cut.borders.len() {
                return Err(format!(
                    "Stellbereich „{name}“: Anzahl der Grenzen in Plan und Netz verschieden"
                ));
            }
            for (i, b) in cut.borders.iter().enumerate() {
                let id = BorderId::from_index(i).ok_or("zu viele Grenzen")?;
                let in_net = cut.topology.terminal(b.terminal).map(|t| t.name.as_str());
                if plan.border_name(id) != in_net {
                    return Err(format!("Stellbereich „{name}“: Grenze {i} heißt anders"));
                }
            }
            built.push(Area::new(name, plan, rules, AreaMap::from_cut(&cut)));
            cuts.push(cut);
        }
        // Grenzen paaren: Innen- und Außenport sind auf beiden Seiten vertauscht.
        for (a, cut) in cuts.iter().enumerate() {
            for (i, border) in cut.borders.iter().enumerate() {
                let peer = cuts.iter().enumerate().find_map(|(b, other)| {
                    other
                        .borders
                        .iter()
                        .position(|o| o.inside == border.outside && o.outside == border.inside)
                        .and_then(BorderId::from_index)
                        .map(|j| (b, j))
                });
                let Some(peer) = peer else {
                    return Err(format!(
                        "Grenze {i} von „{}“ hat keinen Gegenpart",
                        built.get(a).map_or("?", |x| x.name.as_str())
                    ));
                };
                if let Some(slot) = built.get_mut(a).and_then(|x| x.peers.get_mut(i)) {
                    *slot = Some(peer);
                }
            }
        }
        Ok(Self::assemble(net.name().to_string(), built, field, cycle))
    }

    fn assemble(network: String, areas: Vec<Area>, field: F, cycle: Duration) -> Self {
        let max = |f: &dyn Fn(&AreaMap) -> Option<usize>| {
            areas
                .iter()
                .filter_map(|a| f(&a.map))
                .max()
                .map_or(0, |m| m.saturating_add(1))
        };
        let points = reverse(
            &areas,
            |m| &m.points,
            PointId::index,
            max(&|m| m.points.iter().filter_map(|p| p.index()).max()),
        );
        let signals = reverse(
            &areas,
            |m| &m.signals,
            SignalId::index,
            max(&|m| m.signals.iter().filter_map(|p| p.index()).max()),
        );
        let tvds = reverse(
            &areas,
            |m| &m.tvds,
            TvdId::index,
            max(&|m| m.tvds.iter().filter_map(|p| p.index()).max()),
        );
        let lxs = reverse(
            &areas,
            |m| &m.lxs,
            LxId::index,
            max(&|m| m.lxs.iter().filter_map(|p| p.index()).max()),
        );
        let directory = Directory::new(
            areas
                .iter()
                .map(|a| (a.name.clone(), *a.kernel.plan()))
                .collect(),
        );
        Self {
            network,
            areas,
            directory,
            field,
            now: Tick::ZERO,
            cycle,
            points,
            signals,
            tvds,
            lxs,
            field_inputs: Vec::new(),
        }
    }

    /// Beginnt das Journal eines Bereichs (REQ-JRN-001): Ab dem nächsten Zyklus werden
    /// Eingaben, Ausgaben-Hash und alle `checkpoint_every` Zyklen der Zustands-Hash
    /// geschrieben. Das Journal muss vor dem ersten Zyklus beginnen, damit es sich abspielen
    /// lässt.
    ///
    /// # Errors
    /// Unbekannter Bereich oder Schreibfehler beim Kopf.
    pub fn start_journal(
        &mut self,
        area: usize,
        out: Box<dyn Write + Send>,
        checkpoint_every: u64,
    ) -> std::io::Result<()> {
        let cycle = self.cycle;
        let a = self
            .areas
            .get_mut(area)
            .ok_or_else(|| std::io::Error::other("unbekannter Stellbereich"))?;
        let header = Header::new(a.kernel.plan(), a.kernel.rules(), cycle, checkpoint_every);
        let mut journal = JournalWriter::new(out, header)?;
        journal.flush()?;
        a.journal = Some(journal);
        a.journal_error = None;
        Ok(())
    }

    /// Fehler, an dem ein Journal abgebrochen ist (erster Bereich mit Fehler). Der Betrieb
    /// läuft weiter; das Journal ist ab diesem Zyklus unvollständig.
    pub fn journal_error(&self) -> Option<&str> {
        self.areas.iter().find_map(|a| a.journal_error.as_deref())
    }

    /// Hash der letzten Journalzeile eines Bereichs, falls dort ein Journal läuft.
    pub fn journal_head(&self, area: usize) -> Option<String> {
        self.areas
            .get(area)?
            .journal
            .as_ref()
            .map(JournalWriter::head)
    }

    /// Ein Zyklus mit Anforderungen an den ersten (bzw. einzigen) Bereich.
    pub fn cycle(&mut self, requests: &[(RequestId, Request)]) -> CycleOutput {
        let addressed: Vec<Addressed> = requests
            .iter()
            .map(|(id, request)| Addressed {
                area: 0,
                id: *id,
                request: *request,
            })
            .collect();
        self.cycle_addressed(&addressed)
    }

    /// Führt einen Zyklus aus: Feld fortschreiben, Meldungen und Anforderungen an die Kerne,
    /// Stellbefehle ans Feld, Grenzmeldungen an die Nachbarn.
    pub fn cycle_addressed(&mut self, requests: &[Addressed]) -> CycleOutput {
        self.now = self.now.checked_add(self.cycle).unwrap_or(self.now);
        self.field.advance(self.now);
        self.field_inputs.clear();
        self.field.reports(&mut self.field_inputs);

        self.distribute(requests);

        let mut events = Vec::new();
        let mut commands = Vec::new();
        let mut mail = Vec::new();
        let mut safe_state = None;
        let now = self.now;
        for (index, area) in self.areas.iter_mut().enumerate() {
            let result = area
                .kernel
                .step(&mut area.state.view(), now, &area.inputs, &mut area.out);
            if let (Err(reason), None) = (result, safe_state) {
                safe_state = Some((index, reason));
            }
            events.extend(area.out.events.iter().map(|event| AreaEvent {
                area: index,
                event: *event,
            }));
            for c in &area.out.commands {
                match *c {
                    Command::Border { border, message } => {
                        if let Some(Some(peer)) = border.index().and_then(|i| area.peers.get(i)) {
                            mail.push((*peer, message));
                        }
                    }
                    other => {
                        if let Some(global) = globalize(&area.map, other) {
                            commands.push(global);
                        }
                    }
                }
            }
            if let Some(journal) = &mut area.journal {
                let written = journal
                    .record(now, &area.inputs, &area.out, &area.state)
                    .and_then(|()| journal.flush());
                if let Err(e) = written {
                    area.journal_error = Some(e.to_string());
                    area.journal = None;
                }
            }
        }
        for ((area, border), message) in mail {
            if let Some(slot) = self
                .areas
                .get_mut(area)
                .and_then(|a| border.index().and_then(|i| a.inbox.get_mut(i)))
            {
                *slot = Some(message);
            }
        }
        self.field.apply(&commands);

        CycleOutput {
            now,
            events,
            commands,
            safe_state,
        }
    }

    /// Verteilt Feldmeldungen, Grenzmeldungen des letzten Zyklus und Anforderungen auf die
    /// Eingaben der Bereiche.
    fn distribute(&mut self, requests: &[Addressed]) {
        for area in &mut self.areas {
            area.inputs.clear();
            area.out.clear();
        }
        // Feldmeldungen auf die Bereiche verteilen.
        for input in &self.field_inputs {
            let (target, local) = match *input {
                Input::Point { point, report } => {
                    let Some((a, l)) = lookup(&self.points, point.index()) else {
                        continue;
                    };
                    (
                        a,
                        Input::Point {
                            point: PointId::new(l),
                            report,
                        },
                    )
                }
                Input::Tvd { tvd, occupancy } => {
                    let Some((a, l)) = lookup(&self.tvds, tvd.index()) else {
                        continue;
                    };
                    (
                        a,
                        Input::Tvd {
                            tvd: TvdId::new(l),
                            occupancy,
                        },
                    )
                }
                Input::Signal { signal, report } => {
                    let Some((a, l)) = lookup(&self.signals, signal.index()) else {
                        continue;
                    };
                    (
                        a,
                        Input::Signal {
                            signal: SignalId::new(l),
                            report,
                        },
                    )
                }
                Input::LevelCrossing { lx, report } => {
                    let Some((a, l)) = lookup(&self.lxs, lx.index()) else {
                        continue;
                    };
                    (
                        a,
                        Input::LevelCrossing {
                            lx: LxId::new(l),
                            report,
                        },
                    )
                }
                Input::Border { .. } | Input::Request { .. } => continue,
            };
            if let Some(area) = self.areas.get_mut(target) {
                area.inputs.push(local);
            }
        }
        // Grenzmeldungen aus dem letzten Zyklus und Anforderungen.
        for area in &mut self.areas {
            for (i, slot) in area.inbox.iter_mut().enumerate() {
                if let (Some(message), Some(border)) = (slot.take(), BorderId::from_index(i)) {
                    area.inputs.push(Input::Border { border, message });
                }
            }
        }
        for r in requests {
            if let Some(area) = self.areas.get_mut(r.area) {
                area.inputs.push(Input::Request {
                    id: r.id,
                    request: r.request,
                });
            }
        }
    }

    /// Name des Netzes.
    pub fn network_name(&self) -> &str {
        &self.network
    }

    /// Anzahl der Stellbereiche (mindestens 1).
    pub fn area_count(&self) -> usize {
        self.areas.len()
    }

    /// Name eines Bereichs.
    pub fn area_name(&self, area: usize) -> Option<&str> {
        self.areas.get(area).map(|a| a.name.as_str())
    }

    /// Index des Bereichs mit dem Namen `name`.
    pub fn find_area(&self, name: &str) -> Option<usize> {
        self.areas.iter().position(|a| a.name == name)
    }

    /// Plan eines Bereichs.
    pub fn plan_of(&self, area: usize) -> Option<&PlanView<'static>> {
        self.areas.get(area).map(|a| a.kernel.plan())
    }

    /// Kernzustand eines Bereichs.
    pub fn state_of(&self, area: usize) -> Option<&OwnedState> {
        self.areas.get(area).map(|a| &a.state)
    }

    /// Plan des ersten (bzw. einzigen) Bereichs.
    pub fn plan(&self) -> &PlanView<'static> {
        // Eine Runtime hat immer mindestens einen Bereich.
        self.areas.first().map_or_else(
            || unreachable!("Runtime ohne Stellbereich"),
            |a| a.kernel.plan(),
        )
    }

    /// Kernzustand des ersten (bzw. einzigen) Bereichs.
    pub fn state(&self) -> &OwnedState {
        self.areas
            .first()
            .map_or_else(|| unreachable!("Runtime ohne Stellbereich"), |a| &a.state)
    }

    /// Namensauflösung über alle Bereiche.
    pub fn directory(&self) -> &Directory {
        &self.directory
    }

    /// Fahrstraße mit dem Namen `name` in irgendeinem Bereich.
    pub fn find_route(&self, name: &str) -> Option<(usize, RouteId)> {
        self.directory.find_route(name)
    }

    /// Weiche mit dem Namen `name`.
    pub fn find_point(&self, name: &str) -> Option<(usize, PointId)> {
        self.directory.find_point(name)
    }

    /// Signal mit dem Namen `name`.
    pub fn find_signal(&self, name: &str) -> Option<(usize, SignalId)> {
        self.directory.find_signal(name)
    }

    /// Gleisfreimeldeabschnitt mit dem Namen `name`.
    pub fn find_tvd(&self, name: &str) -> Option<(usize, TvdId)> {
        self.directory.find_tvd(name)
    }

    /// Bahnübergang mit dem Namen `name`.
    pub fn find_lx(&self, name: &str) -> Option<(usize, LxId)> {
        self.directory.find_lx(name)
    }

    /// Grenze mit der Adresse `Bereich>Nachbar`, siehe [`Directory::find_border`].
    pub fn find_border(&self, address: &str) -> Option<(usize, BorderId)> {
        self.directory.find_border(address)
    }

    /// Löst eine benannte Hilfshandlung auf, siehe [`Directory::resolve_aux`].
    ///
    /// # Errors
    /// Beschreibung, wenn das Element in keinem Bereich existiert.
    pub fn resolve_aux(&self, aux: &NamedAux) -> Result<(usize, AuxOperation), String> {
        self.directory.resolve_aux(aux)
    }

    /// Die Feldebene.
    pub fn field(&self) -> &F {
        &self.field
    }

    /// Die Feldebene (veränderlich, z. B. für Simulationsbefehle).
    pub fn field_mut(&mut self) -> &mut F {
        &mut self.field
    }

    /// Zeit des letzten Zyklus.
    pub fn now(&self) -> Tick {
        self.now
    }

    /// Zykluszeit.
    pub fn cycle_time(&self) -> Duration {
        self.cycle
    }

    /// Zustandsbild aller Bereiche mit fachlichen Namen.
    pub fn snapshot(&self) -> Snapshot {
        let areas: Vec<(&str, &PlanView<'static>, &OwnedState)> = self
            .areas
            .iter()
            .map(|a| (a.name.as_str(), a.kernel.plan(), &a.state))
            .collect();
        Snapshot::network(&self.network, self.now, &areas)
    }

    /// Ereignis mit fachlichen Namen.
    pub fn name_event(&self, event: &AreaEvent) -> NamedEvent {
        let area = self.areas.get(event.area);
        let plan = area.map_or_else(|| self.plan(), |a| a.kernel.plan());
        let prefix = if self.areas.len() > 1 {
            area.map_or("", |a| a.name.as_str())
        } else {
            ""
        };
        NamedEvent::new(plan, prefix, &event.event)
    }
}

/// Stellbefehl mit lokalen IDs → Stellbefehl mit IDs des Gesamtnetzes.
fn globalize(map: &AreaMap, command: Command) -> Option<Command> {
    let point = |p: PointId| p.index().and_then(|i| map.points.get(i)).copied();
    let signal = |s: SignalId| s.index().and_then(|i| map.signals.get(i)).copied();
    Some(match command {
        Command::MovePoint { point: p, position } => Command::MovePoint {
            point: point(p)?,
            position,
        },
        Command::SetSignal { signal: s, aspect } => Command::SetSignal {
            signal: signal(s)?,
            aspect,
        },
        Command::SetSubstitute { signal: s, on } => Command::SetSubstitute {
            signal: signal(s)?,
            on,
        },
        Command::SetLevelCrossing { lx, close } => Command::SetLevelCrossing {
            lx: lx.index().and_then(|i| map.lxs.get(i)).copied()?,
            close,
        },
        Command::ResetTvd(t) => {
            Command::ResetTvd(t.index().and_then(|i| map.tvds.get(i)).copied()?)
        }
        Command::Border { .. } => return None,
    })
}
