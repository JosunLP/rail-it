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
use std::time::Instant;

use railit_etcs::{PositionReport, Rbc, RbcEvent, RbcView};
use railit_journal::{Header, JournalWriter, SafetyKind};
use railit_kernel::{
    AuxOperation, BorderMessage, Command, Event, Input, Interlocking, OwnedState, Request,
    RequestId, SafeStateReason, VecOutputs,
};
use railit_ops::{Ops, OpsConfig, OpsEvent, OpsNet, OpsView, Timetable};
use railit_plan::PlanView;
use railit_rules::GenericRules;
use railit_safety::{Channels, Fault, Redundancy, SafetyEvent, Status, Watchdog};
use railit_topology::{AreaCut, Topology};
use railit_types::{BorderId, Duration, LxId, PointId, RouteId, SignalId, Tick, TvdId};

use crate::directory::Directory;
use crate::field::FieldAdapter;
use crate::names::{NamedAux, NamedEvent, SafetySnapshot, Snapshot};
use crate::remote::{LockstepLink, NodeLink, NodeSetup, Remote};

/// Ereignis eines Kerns mit seinem Bereich.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct AreaEvent {
    /// Index des Bereichs.
    pub area: usize,
    /// Ereignis.
    pub event: Event,
}

/// Meldung der Sicherheitsschicht eines Bereichs.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct AreaSafetyEvent {
    /// Index des Bereichs.
    pub area: usize,
    /// Meldung.
    pub event: SafetyEvent,
}

/// Redundanz und Überwachung der Stellbereiche (Konzept, Abschnitt 15).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct SafetyConfig {
    /// Anzahl der Kanäle je Bereich.
    pub redundancy: Redundancy,
    /// Watchdog: Frist für die Rechenzeit eines Bereichs je Zyklus und höchster Abstand zweier
    /// Zyklen, in Mikrosekunden (Wanduhr).
    pub watchdog: Option<(u64, u64)>,
    /// Zustandsvergleich alle so viele Zyklen (Ausgaben werden immer verglichen).
    pub state_check_every: u64,
}

impl Default for SafetyConfig {
    fn default() -> Self {
        Self {
            redundancy: Redundancy::Single,
            watchdog: None,
            state_check_every: 1,
        }
    }
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
    /// Erster Bereich, dessen Kern im sicheren Zustand ist, mit Grund
    /// ([`SafeStateReason::SafetyLayer`], wenn die Sicherheitsschicht abgeschaltet hat).
    pub safe_state: Option<(usize, SafeStateReason)>,
    /// Meldungen der Sicherheitsschichten.
    pub safety: Vec<AreaSafetyEvent>,
    /// Ereignisse der Betriebsführung.
    pub ops: Vec<OpsEvent>,
    /// Ereignisse des RBC (ETCS).
    pub etcs: Vec<RbcEvent>,
}

/// Kennungen ab diesem Wert gehören zu Anforderungen der Betriebsführung.
pub const OPS_REQUEST_BASE: u64 = 1 << 62;

/// Sicht des RBC auf die Kerne.
struct RbcAreas<'a>(&'a [Area]);

impl RbcView for RbcAreas<'_> {
    fn plan(&self, area: usize) -> Option<&PlanView<'_>> {
        self.0.get(area).map(Area::plan)
    }

    fn signals(&self, area: usize) -> &[railit_kernel::SignalState] {
        self.0.get(area).map_or(&[], |a| a.state().signals())
    }

    fn routes(&self, area: usize) -> &[railit_kernel::RouteState] {
        self.0.get(area).map_or(&[], |a| a.state().routes())
    }

    fn peer(&self, area: usize, border: BorderId) -> Option<(usize, BorderId)> {
        self.0
            .get(area)
            .and_then(|a| border.index().and_then(|i| a.peers.get(i)))
            .copied()
            .flatten()
    }

    fn area_name(&self, area: usize) -> String {
        self.0.get(area).map(|a| a.name.clone()).unwrap_or_default()
    }
}

/// Sicht der Betriebsführung auf die Kerne.
struct AreasView<'a>(&'a [Area]);

impl OpsView for AreasView<'_> {
    fn route_phase(&self, area: usize, route: RouteId) -> Option<railit_kernel::RoutePhase> {
        let a = self.0.get(area)?;
        a.state()
            .routes()
            .get(route.index()?)
            .map(railit_kernel::RouteState::phase)
    }

    fn holding(&self, area: usize, border: BorderId) -> Option<bool> {
        let a = self.0.get(area)?;
        a.state()
            .borders()
            .get(border.index()?)
            .map(railit_kernel::BorderState::holding)
    }
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

/// Kanäle eines Stellbereichs.
type AreaChannels = Channels<'static, GenericRules, OwnedState, VecOutputs>;

fn build_channels(
    plan: PlanView<'static>,
    rules: GenericRules,
    cfg: &SafetyConfig,
) -> AreaChannels {
    let channel = || Some((OwnedState::new(&plan), VecOutputs::default()));
    let slots = match cfg.redundancy {
        Redundancy::Single => [channel(), None, None],
        Redundancy::TwoOutOfTwo => [channel(), channel(), None],
        Redundancy::TwoOutOfThree => [channel(), channel(), channel()],
    };
    match Channels::new(Interlocking::new(plan, rules), slots) {
        Ok(c) => c.with_state_check_every(cfg.state_check_every),
        // Frische Zustände sind gleich, die Kanäle lückenlos: kann nicht auftreten.
        Err(e) => unreachable!("Kanäle: {e:?}"),
    }
}

/// Ein Stellbereich mit seinen Kanälen.
struct Area {
    name: String,
    channels: AreaChannels,
    watchdog: Option<Watchdog>,
    map: AreaMap,
    /// Je Grenze: Bereich und Grenze auf der anderen Seite.
    peers: Vec<Option<(usize, BorderId)>>,
    /// Grenzmeldungen, die im nächsten Zyklus zugestellt werden.
    inbox: Vec<Option<BorderMessage>>,
    out: VecOutputs,
    inputs: Vec<Input>,
    journal: Option<JournalWriter<Box<dyn Write + Send>>>,
    journal_error: Option<String>,
    /// Kern auf einem sicheren Rechner; `channels` ist dann der Spiegel.
    remote: Option<Box<Remote>>,
}

impl Area {
    fn new(name: String, plan: PlanView<'static>, rules: GenericRules, map: AreaMap) -> Self {
        Self {
            name,
            channels: build_channels(plan, rules, &SafetyConfig::default()),
            watchdog: None,
            map,
            peers: vec![None; plan.border_count()],
            inbox: vec![None; plan.border_count()],
            out: VecOutputs::default(),
            inputs: Vec::new(),
            journal: None,
            journal_error: None,
            remote: None,
        }
    }

    fn plan(&self) -> &PlanView<'static> {
        self.channels.kernel().plan()
    }

    fn rules(&self) -> &GenericRules {
        self.channels.kernel().rules()
    }

    /// Zustand des ersten mitwählenden Kanals.
    fn state(&self) -> &OwnedState {
        self.channels
            .primary()
            .unwrap_or_else(|| unreachable!("Stellbereich ohne Kanal"))
    }

    /// Ein Zyklus des Bereichs: lokal in den Kanälen oder auf dem Knoten (dann mit Spiegel).
    /// Liefert den Grund eines sicheren Zustands, die Meldungen der Sicherheitsschicht und ob der
    /// Watchdog des Dienstes den Zyklus überwacht (nur lokal und nicht abgeschaltet).
    fn step(&mut self, now: Tick) -> (Option<SafeStateReason>, Vec<SafetyEvent>, bool) {
        if let Some(remote) = &mut self.remote {
            let got = remote.cycle(now, &self.inputs, &mut self.channels);
            self.out.commands = got.commands;
            self.out.events = got.events;
            return (got.safe_state, got.safety, false);
        }
        let verdict = self.channels.step(now, &self.inputs, &mut self.out);
        let reason = match (verdict.trip, verdict.kernel) {
            (Some(_), _) => Some(SafeStateReason::SafetyLayer),
            (None, Err(reason)) => Some(reason),
            (None, Ok(())) => None,
        };
        (reason, verdict.events().collect(), verdict.trip.is_none())
    }

    /// Hält eine Meldung des Voters im Journal fest.
    fn journal_safety_event(&mut self, now: Tick, event: SafetyEvent) {
        let (kind, detail) = match event {
            SafetyEvent::ChannelExcluded {
                channel,
                divergence,
            } => (
                SafetyKind::Excluded,
                format!("Kanal {channel}: {divergence:?}"),
            ),
            SafetyEvent::Tripped(trip) => (SafetyKind::Tripped, format!("{trip:?}")),
        };
        self.journal_safety(now, kind, &detail);
    }

    /// Hält eine Meldung der Sicherheitsschicht im Journal fest.
    fn journal_safety(&mut self, now: Tick, kind: SafetyKind, detail: &str) {
        if let Some(journal) = &mut self.journal {
            let written = journal
                .record_safety(now, kind, detail)
                .and_then(|()| journal.flush());
            if let Err(e) = written {
                self.journal_error = Some(e.to_string());
                self.journal = None;
            }
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
    ops: Option<Ops>,
    ops_pending: Vec<Addressed>,
    ops_next: u64,
    rbc: Rbc,
    safety: SafetyConfig,
    /// Monotone Uhr für den Watchdog.
    clock: Instant,
    /// Abstand der Zustandsabzüge in neu begonnenen Journalen (Zyklen, 0 = keine).
    snapshot_every: Option<u64>,
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
        let directory = Directory::new(areas.iter().map(|a| (a.name.clone(), *a.plan())).collect());
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
            ops: None,
            ops_pending: Vec::new(),
            ops_next: OPS_REQUEST_BASE,
            rbc: Rbc::new(),
            safety: SafetyConfig::default(),
            clock: Instant::now(),
            snapshot_every: None,
        }
    }

    /// Legt den Abstand der Zustandsabzüge für danach begonnene Journale fest (Zyklen,
    /// 0 = keine). Ohne Angabe: alle zehn Minuten Betrieb.
    pub const fn set_journal_snapshots(&mut self, every: u64) {
        self.snapshot_every = Some(every);
    }

    /// Legt Redundanz und Watchdog aller Stellbereiche fest (vor dem ersten Zyklus).
    ///
    /// # Errors
    /// Beschreibung, wenn bereits Zyklen gelaufen sind.
    pub fn set_safety(&mut self, cfg: SafetyConfig) -> Result<(), String> {
        if self.now != Tick::ZERO {
            return Err("Redundanz nur vor dem ersten Zyklus änderbar".into());
        }
        if self.areas.iter().any(|a| a.remote.is_some()) {
            return Err("Redundanz vor dem Anbinden der Knoten festlegen".into());
        }
        for area in &mut self.areas {
            area.channels = build_channels(*area.plan(), *area.rules(), &cfg);
            area.watchdog = cfg
                .watchdog
                .map(|(deadline, gap)| Watchdog::new(deadline, gap));
        }
        self.safety = cfg;
        Ok(())
    }

    /// Redundanz und Watchdog.
    pub fn safety(&self) -> SafetyConfig {
        self.safety
    }

    /// Zustand der Sicherheitsschicht eines Bereichs (beim Knoten laut letztem Zyklus).
    pub fn safety_status(&self, area: usize) -> Option<Status> {
        self.areas.get(area).map(|a| {
            a.remote
                .as_ref()
                .map_or_else(|| a.channels.status(), |r| r.status())
        })
    }

    /// Lässt den Kern eines Bereichs auf einem sicheren Rechner rechnen (vor dem ersten
    /// Zyklus). Der Dienst behält einen Spiegel für Anzeige, Betriebsführung und RBC; die
    /// Redundanz legt der Knoten fest.
    ///
    /// # Errors
    /// Unbekannter Bereich, schon gelaufene Zyklen oder laufendes Journal.
    pub fn attach_node(&mut self, area: usize, link: Box<dyn NodeLink>) -> Result<(), String> {
        if self.now != Tick::ZERO {
            return Err("Knoten nur vor dem ersten Zyklus anbinden".into());
        }
        let redundancy = self.safety.redundancy;
        let a = self.area_mut(area)?;
        if a.journal.is_some() {
            return Err("Journal und Knoten: das Journal schreibt der Knoten".into());
        }
        let plan = *a.plan();
        let rules = *a.rules();
        a.channels = build_channels(plan, rules, &SafetyConfig::default());
        a.watchdog = None;
        a.remote = Some(Box::new(Remote::new(link, &plan, redundancy)));
        Ok(())
    }

    /// Lässt alle Bereiche auf Knoten im Gleichschritt rechnen (Szenarien, Tests): das
    /// vollständige Protokoll mit RaSTA, Binärtelegrammen und Zustandsabgleich, aber
    /// deterministisch in der Simulationszeit.
    ///
    /// # Errors
    /// Siehe [`Self::attach_node`].
    pub fn use_lockstep_nodes(&mut self) -> Result<(), String> {
        let cycle_us = self.cycle.as_millis().saturating_mul(1000);
        let (deadline_us, gap_limit_us) = self
            .safety
            .watchdog
            .unwrap_or((1_000_000, cycle_us.saturating_mul(10).max(1_000_000)));
        let setup = NodeSetup {
            redundancy: self.safety.redundancy,
            deadline_us,
            gap_limit_us,
        };
        for area in 0..self.areas.len() {
            let a = self.areas.get(area).ok_or("Bereich")?;
            let link = LockstepLink::spawn(*a.plan(), *a.rules(), setup)?;
            self.attach_node(area, Box::new(link))?;
        }
        Ok(())
    }

    /// Alle Knoten sind verbunden und ihre Spiegel abgeglichen (ohne Knoten: `true`).
    pub fn nodes_synced(&self) -> bool {
        self.areas
            .iter()
            .filter_map(|a| a.remote.as_ref())
            .all(|r| r.snapshot().synced)
    }

    /// Bereiche, die auf einem Knoten rechnen.
    pub fn remote_areas(&self) -> Vec<usize> {
        (0..self.areas.len())
            .filter(|i| self.areas.get(*i).is_some_and(|a| a.remote.is_some()))
            .collect()
    }

    fn area_mut(&mut self, area: usize) -> Result<&mut Area, String> {
        self.areas
            .get_mut(area)
            .ok_or_else(|| format!("unbekannter Stellbereich {area}"))
    }

    /// Bringt einen Fehler in einen Kanal ein (Simulation, Prüfstand).
    ///
    /// # Errors
    /// Unbekannter Bereich oder Kanal.
    pub fn inject_channel_fault(
        &mut self,
        area: usize,
        channel: usize,
        fault: Fault,
    ) -> Result<(), String> {
        let a = self.area_mut(area)?;
        if let Some(remote) = &mut a.remote {
            let msg = crate::remote::fault_msg(channel, fault)
                .ok_or_else(|| format!("Kanal {channel} gibt es nicht"))?;
            remote.safety_action(msg);
            return Ok(());
        }
        if a.channels.inject(channel, fault) {
            Ok(())
        } else {
            Err(format!("Kanal {channel} gibt es nicht"))
        }
    }

    /// Nimmt einen ausgeschlossenen Kanal wieder auf (REQ-SAF-003).
    ///
    /// # Errors
    /// Beschreibung, wenn der Kanal nicht aufgenommen werden kann.
    pub fn reintegrate_channel(&mut self, area: usize, channel: usize) -> Result<(), String> {
        let now = self.now;
        let a = self.area_mut(area)?;
        if let Some(remote) = &mut a.remote {
            let channel = u8::try_from(channel).map_err(|_| format!("Kanal {channel}"))?;
            remote.safety_action(railit_embedded::wire::HostMsg::Reintegrate(channel));
            return Ok(());
        }
        a.channels
            .reintegrate(channel)
            .map_err(|e| format!("Kanal {channel} nicht aufgenommen: {e}"))?;
        a.journal_safety(now, SafetyKind::Reintegrated, &format!("Kanal {channel}"));
        Ok(())
    }

    /// Setzt den sicheren Zustand eines Bereichs zurück – den der Sicherheitsschicht wie den
    /// des Kerns: Alle Kanäle beginnen wie nach einem Neustart (REQ-SAF-004).
    ///
    /// # Errors
    /// Unbekannter Bereich.
    pub fn reset_safety(&mut self, area: usize) -> Result<(), String> {
        let now = self.now;
        let a = self.area_mut(area)?;
        if let Some(remote) = &mut a.remote {
            remote.safety_action(railit_embedded::wire::HostMsg::Reset);
            a.journal_safety(now, SafetyKind::Reset, "");
            return Ok(());
        }
        a.channels.reset();
        if let Some(w) = &mut a.watchdog {
            w.restart();
        }
        a.journal_safety(now, SafetyKind::Reset, "");
        Ok(())
    }

    /// RBC (ETCS Level 2).
    pub fn rbc(&self) -> &Rbc {
        &self.rbc
    }

    /// Zustandsbild des RBC.
    pub fn etcs_snapshot(&self) -> Vec<railit_etcs::TrainSnapshot> {
        self.rbc.snapshot(&RbcAreas(&self.areas))
    }

    /// RBC nach dem Zyklus: Positionsmeldungen der Fahrzeuggeräte übernehmen,
    /// Fahrerlaubnisse berechnen und an die Fahrzeuggeräte übergeben.
    fn run_etcs(&mut self, cycle_events: &[AreaEvent]) -> Vec<RbcEvent>
    where
        F: FieldAdapter,
    {
        let reports: Vec<PositionReport> = self
            .field
            .etcs_reports()
            .into_iter()
            .filter_map(|(train, signal, passed)| {
                let (area, local) = lookup(&self.signals, signal.index())?;
                Some(PositionReport {
                    train,
                    area,
                    reference: SignalId::new(local),
                    passed,
                })
            })
            .collect();
        if reports.is_empty() && self.rbc.cab_authorities().is_empty() {
            return Vec::new();
        }
        // Per Hilfshandlung aufgelöste Fahrstraßen: Züge darin erhalten eine Notbremsung.
        let revoked: Vec<(usize, RouteId)> = cycle_events
            .iter()
            .filter_map(|e| match e.event {
                Event::AuxExecuted {
                    op: AuxOperation::ReleaseRoute(r),
                    ..
                } => Some((e.area, r)),
                _ => None,
            })
            .collect();
        let events = self.rbc.update(&RbcAreas(&self.areas), &reports, &revoked);
        for (train, authority) in self.rbc.cab_authorities() {
            let global = authority.map(|list| {
                list.into_iter()
                    .filter_map(|(area, local)| {
                        self.areas
                            .get(area)?
                            .map
                            .signals
                            .get(local.index()?)
                            .copied()
                    })
                    .collect()
            });
            self.field.set_cab_authority(&train, global);
        }
        events
    }

    /// Schaltet die Betriebsführung ein (Zugnummernmeldung, Fahrplan, Zuglenkung,
    /// Selbstblock). `topology` ist das Gesamtnetz (Betriebsstellen, Selbstblocksignale).
    ///
    /// # Errors
    /// Abweichungen des Fahrplans vom Netz.
    pub fn enable_ops(
        &mut self,
        topology: &Topology,
        cfg: OpsConfig,
        timetable: Timetable,
    ) -> Result<(), String> {
        let plans: Vec<(&str, &PlanView<'_>)> = self
            .areas
            .iter()
            .map(|a| (a.name.as_str(), a.plan()))
            .collect();
        let peer = |area: usize, b: BorderId| {
            self.areas
                .get(area)
                .and_then(|a| b.index().and_then(|i| a.peers.get(i)))
                .copied()
                .flatten()
        };
        let net = OpsNet::new(&plans, peer, topology);
        let ops = Ops::new(net, cfg, timetable)
            .map_err(|e| format!("Fahrplan passt nicht zum Netz:\n  {}", e.join("\n  ")))?;
        self.ops = Some(ops);
        // Züge, die bei Betriebsbeginn schon im Netz stehen, vor der ersten Meldung einsetzen:
        // Ihre Besetzung ist dann Grundstellung, keine unerlaubte Besetzung.
        if self.now == Tick::ZERO {
            let events = self.run_ops(Tick::ZERO, &[]);
            if let Some(ops) = &mut self.ops {
                ops.requeue_events(events);
            }
        }
        Ok(())
    }

    /// Betriebsführung, falls eingeschaltet.
    pub fn ops(&self) -> Option<&Ops> {
        self.ops.as_ref()
    }

    /// Betriebsführung zum Disponieren, falls eingeschaltet.
    pub fn ops_mut(&mut self) -> Option<&mut Ops> {
        self.ops.as_mut()
    }

    /// Globale Kennung eines Signals.
    pub fn global_signal(&self, name: &str) -> Option<SignalId> {
        global_signal(&self.directory, &self.areas, name)
    }

    /// Betriebsführung nach dem Zyklus: Ereignisse auswerten, Anforderungen für den nächsten
    /// Zyklus und Züge für die Simulation planen.
    fn run_ops(&mut self, now: Tick, events: &[AreaEvent]) -> Vec<OpsEvent>
    where
        F: FieldAdapter,
    {
        let Some(ops) = &mut self.ops else {
            return Vec::new();
        };
        for e in events {
            ops.observe(now, e.area, &e.event);
        }
        let actions = ops.plan(now, &AreasView(&self.areas));
        for (area, request) in actions.requests {
            self.ops_next = self.ops_next.wrapping_add(1).max(OPS_REQUEST_BASE);
            self.ops_pending.push(Addressed {
                area,
                id: RequestId(self.ops_next),
                request,
            });
        }
        for spawn in actions.spawns {
            let result = global_signal(&self.directory, &self.areas, &spawn.signal)
                .ok_or_else(|| format!("Signal „{}“ nicht gefunden", spawn.signal))
                .and_then(|signal| self.field.spawn_train(&spawn, signal));
            if let Err(reason) = result {
                ops.spawn_failed(&spawn.train, reason);
            }
        }
        ops.take_events()
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
        self.open_journal(area, out, checkpoint_every, None)
    }

    /// Wie [`Self::start_journal`], aber gesiegelt: nach dem Kopf, alle `seal_every` Zyklen
    /// und mit [`Self::seal_journals`] folgt ein Siegel mit `key` (REQ-JRN-004).
    ///
    /// # Errors
    /// Wie [`Self::start_journal`].
    pub fn start_journal_sealed(
        &mut self,
        area: usize,
        out: Box<dyn Write + Send>,
        checkpoint_every: u64,
        key: railit_plan::SigningKey,
        seal_every: u64,
    ) -> std::io::Result<()> {
        self.open_journal(area, out, checkpoint_every, Some((key, seal_every)))
    }

    /// Siegelt alle gesiegelten Journale (z. B. beim Beenden). Schreibfehler brechen das
    /// Journal ab wie beim Schreiben eines Zyklus.
    pub fn seal_journals(&mut self) {
        for area in &mut self.areas {
            if let Some(journal) = &mut area.journal {
                let written = journal.seal().and_then(|()| journal.flush());
                if let Err(e) = written {
                    area.journal_error = Some(e.to_string());
                    area.journal = None;
                }
            }
        }
    }

    fn open_journal(
        &mut self,
        area: usize,
        out: Box<dyn Write + Send>,
        checkpoint_every: u64,
        sealer: Option<(railit_plan::SigningKey, u64)>,
    ) -> std::io::Result<()> {
        let cycle = self.cycle;
        let a = self
            .areas
            .get_mut(area)
            .ok_or_else(|| std::io::Error::other("unbekannter Stellbereich"))?;
        if a.remote.is_some() {
            return Err(std::io::Error::other(
                "Journal und Knoten: das Journal schreibt der Knoten",
            ));
        }
        // Zustandsabzüge, ohne Angabe alle zehn Minuten Betrieb: Abschnitte für paralleles
        // Replay.
        let snapshot_every = self.snapshot_every.unwrap_or_else(|| {
            600_000u64
                .checked_div(cycle.as_millis())
                .unwrap_or(0)
                .max(1)
        });
        let header = Header::new(a.plan(), a.rules(), cycle, checkpoint_every)
            .with_snapshots(snapshot_every);
        let mut journal = match sealer {
            Some((key, every)) => JournalWriter::sealed(out, header, key, every)?,
            None => JournalWriter::new(out, header)?,
        };
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

        let mut all = std::mem::take(&mut self.ops_pending);
        all.extend_from_slice(requests);
        self.distribute(&all);

        let mut events = Vec::new();
        let mut commands = Vec::new();
        let mut mail = Vec::new();
        let mut safe_state = None;
        let mut safety = Vec::new();
        let now = self.now;
        let clock = self.clock;
        let micros = || u64::try_from(clock.elapsed().as_micros()).unwrap_or(u64::MAX);
        for (index, area) in self.areas.iter_mut().enumerate() {
            let started = micros();
            let (reason, layer_events, watched) = area.step(now);
            let finished = micros();
            if let (Some(reason), None) = (reason, safe_state) {
                safe_state = Some((index, reason));
            }
            for event in layer_events {
                area.journal_safety_event(now, event);
                safety.push(AreaSafetyEvent { area: index, event });
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
                    .record(
                        now,
                        &area.inputs,
                        &area.out,
                        area.channels
                            .primary()
                            .unwrap_or_else(|| unreachable!("Stellbereich ohne Kanal")),
                    )
                    .and_then(|()| journal.flush());
                if let Err(e) = written {
                    area.journal_error = Some(e.to_string());
                    area.journal = None;
                }
            }
            if watched
                && let Some(Err(trip)) = area.watchdog.as_mut().map(|w| w.check(started, finished))
            {
                area.channels.trip(trip);
                area.journal_safety(now, SafetyKind::Tripped, &format!("{trip:?}"));
                safety.push(AreaSafetyEvent {
                    area: index,
                    event: SafetyEvent::Tripped(trip),
                });
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
        let ops = self.run_ops(now, &events);
        let etcs = self.run_etcs(&events);

        CycleOutput {
            now,
            events,
            commands,
            safe_state,
            safety,
            ops,
            etcs,
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
        self.areas.get(area).map(Area::plan)
    }

    /// Kernzustand eines Bereichs.
    pub fn state_of(&self, area: usize) -> Option<&OwnedState> {
        self.areas.get(area).map(Area::state)
    }

    /// Plan des ersten (bzw. einzigen) Bereichs.
    pub fn plan(&self) -> &PlanView<'static> {
        // Eine Runtime hat immer mindestens einen Bereich.
        self.areas
            .first()
            .map_or_else(|| unreachable!("Runtime ohne Stellbereich"), Area::plan)
    }

    /// Kernzustand des ersten (bzw. einzigen) Bereichs.
    pub fn state(&self) -> &OwnedState {
        self.areas
            .first()
            .map_or_else(|| unreachable!("Runtime ohne Stellbereich"), Area::state)
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
            .map(|a| (a.name.as_str(), a.plan(), a.state()))
            .collect();
        let mut snap = Snapshot::network(&self.network, self.now, &areas);
        for (area, snapshot) in self.areas.iter().zip(&mut snap.areas) {
            if let Some(remote) = &area.remote {
                snapshot.safety = SafetySnapshot::new(&remote.status());
                snapshot.safety.node = Some(remote.snapshot());
            } else {
                snapshot.safety = SafetySnapshot::new(&area.channels.status());
            }
        }
        snap.ops = self.ops.as_ref().map(Ops::snapshot);
        snap.etcs = self.etcs_snapshot();
        snap
    }

    /// Ereignis mit fachlichen Namen.
    pub fn name_event(&self, event: &AreaEvent) -> NamedEvent {
        let area = self.areas.get(event.area);
        let plan = area.map_or_else(|| self.plan(), Area::plan);
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

/// Globale Kennung eines Signals über die Namensauflösung.
fn global_signal(directory: &Directory, areas: &[Area], name: &str) -> Option<SignalId> {
    let (area, local) = directory.find_signal(name)?;
    areas.get(area)?.map.signals.get(local.index()?).copied()
}
