//! Stellbereiche auf einem sicheren Rechner (Konzept, Abschnitt 13, Einsatzvariante „verteilt“;
//! REQ-EMB-005, REQ-EMB-006).
//!
//! Der Kern eines Bereichs rechnet dann nicht im Dienst, sondern auf einem Knoten
//! ([`railit_embedded::Node`]) mit eigenen Kanälen, Voter und Watchdog, verbunden über RaSTA.
//! Der Dienst
//!
//! 1. schickt dem Knoten je Zyklus die Eingaben des Bereichs (Feld- und Grenzmeldungen,
//!    Anforderungen),
//! 2. übernimmt die Stellbefehle, Ereignisse und Meldungen der Sicherheitsschicht des Knotens
//!    unverändert – **der Knoten ist maßgeblich**,
//! 3. führt einen **Spiegel** des Kernzustands für Anzeige, Betriebsführung und RBC (SIL 0):
//!    Er rechnet jeden Zyklus des Knotens mit denselben Eingaben nach und vergleicht Befehle,
//!    Ereignisse und den Digest des Zustands. Weicht der Spiegel ab, nach dem Start oder nach
//!    einem Verbindungsabbruch, holt er sich den Zustand des Knotens per Zustandsabgleich.
//!
//! Bleibt der Knoten aus, stellt der Dienst als Rückfallebene alle Signale des Bereichs auf
//! Halt (Befehle in sicherer Richtung). Der Knoten selbst nimmt ohne Meldungen ohnehin die
//! Fahrt zurück (REQ-EMB-002).
//!
//! Übertragungswege ([`NodeLink`]): [`LockstepLink`] lässt den Knoten in einem eigenen Thread
//! im Gleichschritt mit der Runtime rechnen (Szenarien, Tests, deterministisch, mit dem
//! vollständigen Protokoll), [`UdpLink`] verbindet sich mit einem eigenständigen Knoten
//! (`railit-node`) über UDP.

use std::collections::VecDeque;
use std::net::{SocketAddr, UdpSocket};
use std::sync::mpsc;
use std::thread;
use std::time::Instant;

use crate::runtime::JournalSlot;
use railit_embedded::wire::HostMsg;
use railit_embedded::{Node, NodeClient, NodeConfig, NodeCycle, NodeEvent, NodeSync};
use railit_kernel::{
    Aspect, Command, Event, Input, Interlocking, OwnedState, SafeStateReason, VecOutputs,
};
use railit_plan::PlanView;
use railit_rasta::{Config, MAX_PDU, Role, State};
use railit_rules::GenericRules;
use railit_safety::{Channels, Fault, MAX_CHANNELS, Redundancy, SafetyEvent, Status, state_digest};
use railit_types::{SignalId, Tick};
use serde::Serialize;

/// RaSTA-Kennung des Dienstes.
pub const HOST_ID: u32 = 0x0C00_0000;
/// RaSTA-Kennung des Knotens.
pub const NODE_ID: u32 = 0x0C00_0001;

/// Höchstzahl der Eingaben je Zyklus eines Knotens auf dem Server.
pub const NODE_INPUTS: usize = 4096;
/// Höchstzahl der Stellbefehle bzw. Ereignisse je Zyklus und Kanal.
pub const NODE_OUTPUTS: usize = 2048;

/// Knoten, wie ihn der Dienst und `railit-node` betreiben.
pub type ServerNode<'p, 's> = Node<'p, 's, GenericRules, NODE_INPUTS, NODE_OUTPUTS, NODE_OUTPUTS>;

/// Nach so vielen Zyklen des Dienstes ohne Zyklus des Knotens gilt er als ausgefallen.
const STALE_CYCLES: u32 = 10;

/// RAM-Prüfbereich des Knotens in Wörtern (16 KiB, REQ-SAF-008).
pub(crate) const RAM_TEST_WORDS: usize = 4096;
/// Je Zyklus geprüfte Bytes des Plans.
pub(crate) const PLAN_CHECK_BYTES: usize = 4096;
/// Je Zyklus geprüfte Wörter des RAM-Prüfbereichs.
const RAM_CHECK_WORDS: usize = 256;

/// Höchstzahl der Zyklen, die auf einen Zustandsabgleich warten.
const MAX_BUFFERED: usize = 600;

/// Parameter eines Knotens.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct NodeSetup {
    /// Kanäle des Knotens.
    pub redundancy: Redundancy,
    /// Watchdog: Frist für die Rechenzeit und höchster Zyklusabstand in Mikrosekunden.
    pub deadline_us: u64,
    /// Höchster Abstand zweier Zyklusbeginne in Mikrosekunden.
    pub gap_limit_us: u64,
}

impl NodeSetup {
    /// RaSTA-Parameter und Watchdog des Knotens.
    pub const fn config(&self) -> NodeConfig {
        NodeConfig {
            rasta: Config::new(NODE_ID, HOST_ID, Role::Server),
            deadline_us: self.deadline_us,
            gap_limit_us: self.gap_limit_us,
        }
    }
}

/// Baut einen Knoten für `plan` mit Speicher auf dem Heap und übergibt ihn `run`, das ihn
/// betreibt, solange es will (Thread des Gleichschritts, Hauptschleife von `railit-node`).
///
/// # Errors
/// Beschreibung, wenn die Kanäle nicht zusammenpassen.
pub fn with_node<T>(
    plan: PlanView<'static>,
    rules: GenericRules,
    setup: NodeSetup,
    run: impl FnOnce(&mut ServerNode<'static, '_>) -> T,
) -> Result<T, String> {
    let channels = setup.redundancy.channels();
    let mut memory: Vec<OwnedState> = (0..=channels).map(|_| OwnedState::new(&plan)).collect();
    let (sync, rest) = memory.split_last_mut().ok_or("kein Zustandsspeicher")?;
    let mut views = rest.iter_mut().map(OwnedState::view);
    let states = [views.next(), views.next(), views.next()];
    let mut ram = vec![0u32; RAM_TEST_WORDS];
    let node = ServerNode::new(Interlocking::new(plan, rules), states, setup.config())
        .map_err(|e| format!("Knoten: {e:?}"))?
        .with_sync_buffer(sync.view())
        .with_memory_check(
            plan.body(),
            plan.digest(),
            &mut ram,
            PLAN_CHECK_BYTES,
            RAM_CHECK_WORDS,
        );
    let mut node = Box::new(node);
    Ok(run(&mut node))
}

/// Übertragungsweg zwischen Dienst und Knoten.
pub trait NodeLink: Send {
    /// RaSTA-Zeit des Dienstes in Millisekunden zum Zyklus `now`.
    fn clock_ms(&self, now: Tick) -> u32;
    /// Gibt die PDUs des Dienstes an den Knoten und liefert, was vom Knoten eingegangen ist.
    fn exchange(&mut self, now: Tick, outgoing: Vec<Vec<u8>>) -> Vec<Vec<u8>>;
    /// Beschreibung für die Anzeige (z. B. Adresse).
    fn describe(&self) -> String;
}

/// RaSTA-Zeit aus Millisekunden (läuft nach 49 Tagen über, wie RaSTA es vorsieht).
fn rasta_ms(ms: u64) -> u32 {
    u32::try_from(ms & u64::from(u32::MAX)).unwrap_or(0)
}

/// Knoten in einem eigenen Thread, der im Gleichschritt mit der Runtime rechnet: Je Zyklus
/// gehen die PDUs des Dienstes an den Knoten, er rechnet einen Zyklus zur Simulationszeit, und
/// seine PDUs gehen zurück. Deterministisch und mit dem vollständigen Protokoll (RaSTA,
/// Binärtelegramme, Zustandsabgleich).
pub struct LockstepLink {
    to_node: Option<mpsc::Sender<(Tick, Vec<Vec<u8>>)>>,
    from_node: mpsc::Receiver<Vec<Vec<u8>>>,
    worker: Option<thread::JoinHandle<()>>,
    /// Verbindung künstlich unterbrochen (Prüfung).
    cut: bool,
}

impl LockstepLink {
    /// Startet den Knoten-Thread.
    ///
    /// # Errors
    /// Beschreibung, wenn der Thread nicht startet oder der Knoten sich nicht bauen lässt.
    pub fn spawn(
        plan: PlanView<'static>,
        rules: GenericRules,
        setup: NodeSetup,
    ) -> Result<Self, String> {
        let (to_node, inbox) = mpsc::channel::<(Tick, Vec<Vec<u8>>)>();
        let (outbox, from_node) = mpsc::channel::<Vec<Vec<u8>>>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        let worker = thread::Builder::new()
            .name(format!("knoten-{}", plan.network_name()))
            .stack_size(32 << 20)
            .spawn(move || {
                let result = with_node(plan, rules, setup, |node| {
                    let _ = ready_tx.send(Ok(()));
                    let mut buf = [0u8; MAX_PDU];
                    while let Ok((now, pdus)) = inbox.recv() {
                        let ms = now.as_millis();
                        for pdu in &pdus {
                            node.receive(rasta_ms(ms), pdu);
                        }
                        let start = ms.saturating_mul(1000);
                        node.tick(start, &mut || start);
                        let mut out = Vec::new();
                        while let Some(n) = node.poll_transmit(&mut buf) {
                            out.push(buf.get(..n).unwrap_or_default().to_vec());
                        }
                        if outbox.send(out).is_err() {
                            break;
                        }
                    }
                });
                if let Err(e) = result {
                    let _ = ready_tx.send(Err(e));
                }
            })
            .map_err(|e| format!("Knoten-Thread: {e}"))?;
        ready_rx
            .recv()
            .map_err(|_| "Knoten-Thread beendet".to_string())??;
        Ok(Self {
            to_node: Some(to_node),
            from_node,
            worker: Some(worker),
            cut: false,
        })
    }

    /// Unterbricht die Übertragung in beide Richtungen bzw. stellt sie wieder her (Prüfung).
    pub fn set_cut(&mut self, cut: bool) {
        self.cut = cut;
    }
}

impl NodeLink for LockstepLink {
    fn clock_ms(&self, now: Tick) -> u32 {
        rasta_ms(now.as_millis())
    }

    fn exchange(&mut self, now: Tick, outgoing: Vec<Vec<u8>>) -> Vec<Vec<u8>> {
        let Some(tx) = &self.to_node else {
            return Vec::new();
        };
        let outgoing = if self.cut { Vec::new() } else { outgoing };
        if tx.send((now, outgoing)).is_err() {
            return Vec::new();
        }
        let incoming = self.from_node.recv().unwrap_or_default();
        if self.cut { Vec::new() } else { incoming }
    }

    fn describe(&self) -> String {
        "im Gleichschritt".into()
    }
}

impl Drop for LockstepLink {
    fn drop(&mut self) {
        self.to_node = None;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Verbindung zu einem eigenständigen Knoten (`railit-node`) über UDP. Der Knoten rechnet nach
/// seinem eigenen Takt; RaSTA läuft auf der Wanduhr.
pub struct UdpLink {
    socket: UdpSocket,
    peer: SocketAddr,
    started: Instant,
}

impl UdpLink {
    /// Bindet einen lokalen Port und verbindet ihn mit dem Knoten unter `peer`.
    ///
    /// # Errors
    /// Fehler des Betriebssystems.
    pub fn connect(peer: SocketAddr) -> std::io::Result<Self> {
        let local: SocketAddr = if peer.is_ipv6() {
            "[::]:0".parse().map_err(std::io::Error::other)?
        } else {
            "0.0.0.0:0".parse().map_err(std::io::Error::other)?
        };
        let socket = UdpSocket::bind(local)?;
        socket.connect(peer)?;
        socket.set_nonblocking(true)?;
        Ok(Self {
            socket,
            peer,
            started: Instant::now(),
        })
    }
}

impl NodeLink for UdpLink {
    fn clock_ms(&self, _now: Tick) -> u32 {
        rasta_ms(u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX))
    }

    fn exchange(&mut self, _now: Tick, outgoing: Vec<Vec<u8>>) -> Vec<Vec<u8>> {
        for pdu in outgoing {
            // Verlorene Datagramme holt RaSTA nach.
            let _ = self.socket.send(&pdu);
        }
        let mut incoming = Vec::new();
        let mut buf = [0u8; MAX_PDU];
        while let Ok(n) = self.socket.recv(&mut buf) {
            incoming.push(buf.get(..n).unwrap_or_default().to_vec());
        }
        incoming
    }

    fn describe(&self) -> String {
        format!("udp://{}", self.peer)
    }
}

/// Zähler und Zustand der Anbindung eines Knotens (Zustandsbild).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct NodeSnapshot {
    /// Übertragungsweg.
    pub link: String,
    /// RaSTA-Verbindung steht.
    pub up: bool,
    /// Der Knoten liefert Zyklen (sonst Rückfallebene aktiv).
    pub available: bool,
    /// Der Spiegel stimmt mit dem Knoten überein.
    pub synced: bool,
    /// Empfangene Zyklen des Knotens.
    pub cycles: u64,
    /// Abgeschlossene Zustandsabgleiche.
    pub syncs: u32,
    /// Abweichungen des Spiegels vom Knoten.
    pub divergences: u32,
    /// Verbindungsabbrüche.
    pub link_losses: u32,
    /// Unvollständige oder widersprüchliche Nachrichten.
    pub protocol_errors: u32,
    /// Letzte Abweichung oder Störung in Worten.
    pub last_problem: Option<String>,
}

/// Was ein Zyklus des Dienstes von einem Knoten erhält.
#[derive(Debug, Default)]
pub(crate) struct RemoteOutput {
    /// Stellbefehle (lokale IDs, einschließlich Grenzmeldungen).
    pub commands: Vec<Command>,
    /// Ereignisse des Kerns.
    pub events: Vec<Event>,
    /// Meldungen der Sicherheitsschicht.
    pub safety: Vec<SafetyEvent>,
    /// Sicherer Zustand laut letztem Zyklus des Knotens.
    pub safe_state: Option<SafeStateReason>,
}

/// Spiegel des Kernzustands (ein Kanal, SIL 0).
pub(crate) type Mirror = Channels<'static, GenericRules, OwnedState, VecOutputs>;

/// Anbindung eines Stellbereichs an seinen Knoten.
pub(crate) struct Remote {
    link: Box<dyn NodeLink>,
    client: NodeClient,
    /// Gesendete, vom Knoten noch nicht verbrauchte Eingaben (aktuelle Verbindung).
    queue: VecDeque<Input>,
    synced: bool,
    sync_requested: bool,
    /// Zyklen, die bis zum Abschluss eines Zustandsabgleichs warten.
    buffered: Vec<NodeCycle>,
    /// Handlungen an der Sicherheitsschicht für den nächsten Versand.
    pending: Vec<HostMsg>,
    status: Status,
    plan_short: [u8; 8],
    signals: usize,
    /// Zyklen des Dienstes seit dem letzten Zyklus des Knotens.
    silent: u32,
    fallback_sent: bool,
    snapshot: NodeSnapshot,
}

fn short(digest: &[u8]) -> [u8; 8] {
    let mut out = [0u8; 8];
    for (o, d) in out.iter_mut().zip(digest) {
        *o = *d;
    }
    out
}

/// Digest des Spiegels wie im Knoten.
fn mirror_digest(mirror: &mut Mirror) -> [u8; 8] {
    mirror
        .primary_mut()
        .map_or([0; 8], |s| short(&state_digest(&s.view()).0))
}

impl Remote {
    pub(crate) fn new(
        link: Box<dyn NodeLink>,
        plan: &PlanView<'_>,
        redundancy: Redundancy,
    ) -> Self {
        let snapshot = NodeSnapshot {
            link: link.describe(),
            ..NodeSnapshot::default()
        };
        Self {
            link,
            client: NodeClient::new(Config::new(HOST_ID, NODE_ID, Role::Client)),
            queue: VecDeque::new(),
            synced: false,
            sync_requested: false,
            buffered: Vec::new(),
            pending: Vec::new(),
            status: Status {
                redundancy,
                active: [false; MAX_CHANNELS],
                trip: None,
                cycle: 0,
            },
            plan_short: short(&plan.digest()),
            signals: plan.signal_count(),
            silent: 0,
            fallback_sent: false,
            snapshot,
        }
    }

    /// Zustand der Sicherheitsschicht des Knotens laut letztem Zyklus.
    pub(crate) const fn status(&self) -> Status {
        self.status
    }

    /// Zustandsbild der Anbindung.
    pub(crate) fn snapshot(&self) -> NodeSnapshot {
        let mut s = self.snapshot.clone();
        s.up = self.client.is_up();
        s.synced = self.synced;
        s.protocol_errors = self.client.protocol_errors();
        s
    }

    /// Handlung an der Sicherheitsschicht des Knotens (mit dem nächsten Zyklus).
    pub(crate) fn safety_action(&mut self, msg: HostMsg) {
        self.pending.push(msg);
    }

    fn problem(&mut self, text: String) {
        self.snapshot.last_problem = Some(text);
    }

    /// Spiegel weicht ab oder ist unbekannt: Zustandsabgleich anfordern.
    fn desync(&mut self, why: String) {
        if self.synced {
            self.snapshot.divergences = self.snapshot.divergences.saturating_add(1);
        }
        self.synced = false;
        self.sync_requested = false;
        self.buffered.clear();
        self.problem(why);
    }

    fn link_lost(&mut self) {
        self.snapshot.link_losses = self.snapshot.link_losses.saturating_add(1);
        self.queue.clear();
        self.synced = false;
        self.sync_requested = false;
        self.buffered.clear();
        self.problem("Verbindung zum Knoten abgebrochen".into());
    }

    /// Ein Zyklus des Dienstes: Eingaben senden, Knoten empfangen, Spiegel nachführen.
    pub(crate) fn cycle(
        &mut self,
        now: Tick,
        inputs: &[Input],
        mirror: &mut Mirror,
        journal: &mut JournalSlot,
    ) -> RemoteOutput {
        let clock = self.link.clock_ms(now);
        if self.client.tick(clock).is_some() {
            self.link_lost();
        }
        if self.client.state() == State::Closed {
            self.client.open(clock);
        }
        if self.client.is_up() {
            let mut msgs = Vec::with_capacity(inputs.len().saturating_add(4));
            if !self.synced && !self.sync_requested {
                // Erste Nachricht nach dem Aufbau bzw. nach einer Abweichung.
                msgs.push(HostMsg::Sync);
                self.sync_requested = true;
            }
            msgs.append(&mut self.pending);
            msgs.extend(inputs.iter().copied().map(HostMsg::Input));
            if self.client.send(clock, &msgs).is_ok() {
                self.queue.extend(inputs.iter().copied());
            } else {
                // Teilweise gesendet: Zuordnung der Eingaben unsicher. Neu verbinden.
                self.client.close(clock);
                self.link_lost();
            }
        }
        let mut outgoing = Vec::new();
        let mut buf = [0u8; MAX_PDU];
        while let Some(n) = self.client.poll_transmit(&mut buf) {
            outgoing.push(buf.get(..n).unwrap_or_default().to_vec());
        }
        for pdu in self.link.exchange(now, outgoing) {
            self.client.receive(clock, &pdu);
        }

        let mut out = RemoteOutput::default();
        let mut got_cycle = false;
        while let Some(event) = self.client.poll() {
            match event {
                NodeEvent::Cycle(c) => {
                    got_cycle = true;
                    self.take_cycle(&c, &mut out);
                    self.mirror_cycle(c, mirror, journal);
                }
                NodeEvent::Synced(s) => self.mirror_sync(now, &s, mirror, journal, &mut out),
            }
        }
        if got_cycle {
            self.silent = 0;
            self.fallback_sent = false;
            self.snapshot.available = true;
        } else {
            self.silent = self.silent.saturating_add(1);
            if self.silent >= STALE_CYCLES {
                self.snapshot.available = false;
                if !self.fallback_sent {
                    // Rückfallebene: alle Signale des Bereichs auf Halt (sichere Richtung).
                    self.fallback_sent = true;
                    self.problem("Knoten liefert keine Zyklen: alle Signale auf Halt".into());
                    out.commands.extend(
                        (0..self.signals)
                            .filter_map(SignalId::from_index)
                            .flat_map(|signal| {
                                [
                                    Command::SetSignal {
                                        signal,
                                        aspect: Aspect::Stop,
                                    },
                                    Command::SetSubstitute { signal, on: false },
                                ]
                            }),
                    );
                }
            }
        }
        out
    }

    /// Ausgaben eines Zyklus des Knotens übernehmen (maßgeblich).
    fn take_cycle(&mut self, c: &NodeCycle, out: &mut RemoteOutput) {
        self.snapshot.cycles = self.snapshot.cycles.saturating_add(1);
        out.commands.extend_from_slice(&c.commands);
        out.events.extend_from_slice(&c.events);
        out.safety.extend_from_slice(&c.safety);
        out.safe_state = match (c.end.trip, c.end.kernel) {
            (Some(_), _) => Some(SafeStateReason::SafetyLayer),
            (None, reason) => reason,
        };
        self.status.active = c.end.active;
        if let Some(r) = Redundancy::from_channels(usize::from(c.end.channels)) {
            self.status.redundancy = r;
        }
        self.status.trip = c.end.trip;
        self.status.cycle = c.cycle;
    }

    /// Nimmt die nächsten `n` Eingaben der Warteschlange.
    fn pop(&mut self, n: u16) -> Option<Vec<Input>> {
        let n = usize::from(n);
        if self.queue.len() < n {
            return None;
        }
        Some(self.queue.drain(..n).collect())
    }

    /// Spiegel um einen Zyklus des Knotens nachführen und vergleichen.
    fn mirror_cycle(&mut self, c: NodeCycle, mirror: &mut Mirror, journal: &mut JournalSlot) {
        if !self.synced {
            if self.buffered.len() >= MAX_BUFFERED {
                // Abgleich bleibt aus: neu anfordern.
                self.desync("Zustandsabgleich bleibt aus".into());
            }
            self.buffered.push(c);
            return;
        }
        if let Err(why) = self.replay(&c, mirror, journal) {
            self.desync(why);
        }
    }

    /// Rechnet einen Zyklus des Knotens im Spiegel nach. Stimmt er, steht er mit genau den
    /// Eingaben des Knotens im Journal (REQ-JRN-007).
    fn replay(
        &mut self,
        c: &NodeCycle,
        mirror: &mut Mirror,
        journal: &mut JournalSlot,
    ) -> Result<(), String> {
        let inputs = self
            .pop(c.end.inputs)
            .ok_or_else(|| format!("Zyklus {}: Eingaben fehlen", c.cycle))?;
        let _ = self.pop(c.end.dropped);
        let mut out = VecOutputs::default();
        let _ = mirror.step(Tick::from_millis(c.now_ms), &inputs, &mut out);
        if out.commands != c.commands || out.events != c.events {
            return Err(format!(
                "Zyklus {}: Spiegel liefert andere Ausgaben als der Knoten",
                c.cycle
            ));
        }
        if mirror_digest(mirror) != c.end.digest {
            return Err(format!(
                "Zyklus {}: Zustand des Spiegels weicht vom Knoten ab",
                c.cycle
            ));
        }
        if let Some(state) = mirror.primary() {
            journal.write(|j| j.record(Tick::from_millis(c.now_ms), &inputs, &out, state));
        }
        Ok(())
    }

    /// Zustandsabgleich abgeschlossen: Spiegel einsetzen, wartende Zyklen nachrechnen.
    fn mirror_sync(
        &mut self,
        now: Tick,
        s: &NodeSync,
        mirror: &mut Mirror,
        journal: &mut JournalSlot,
        out: &mut RemoteOutput,
    ) {
        if s.plan != self.plan_short {
            self.problem("Knoten rechnet mit einem anderen Plan".into());
            // Kein neuer Abgleich: Mit anderem Plan kann der Spiegel nie stimmen.
            self.buffered.clear();
            return;
        }
        let loaded = mirror.primary_mut().is_some_and(|state| {
            let mut view = state.view();
            s.parts.len() == view.part_count() && s.parts.iter().all(|p| view.apply(*p).is_ok())
        });
        if !loaded || mirror_digest(mirror) != s.digest {
            self.desync("Zustandsabgleich unvollständig oder verfälscht".into());
            return;
        }
        let waiting = std::mem::take(&mut self.buffered);
        self.synced = true;
        self.sync_requested = false;
        self.snapshot.syncs = self.snapshot.syncs.saturating_add(1);
        if let Some(state) = mirror.primary() {
            let detail = format!("Zustandsabgleich {} mit dem Knoten", self.snapshot.syncs);
            journal.write(|j| j.resync(now, state, &detail));
        }
        for c in waiting {
            if c.cycle <= s.cycle {
                // Im übertragenen Zustand enthalten.
                let _ = self.pop(c.end.inputs);
                let _ = self.pop(c.end.dropped);
            } else if let Err(why) = self.replay(&c, mirror, journal) {
                self.desync(why);
                return;
            }
        }
        // Befehle aus Zyklen ohne Verbindung haben das Feld nie erreicht: Stellbild des
        // (digestgeprüften) Zustands nachsenden.
        let mut image = VecOutputs::default();
        let kernel = Interlocking::new(*mirror.kernel().plan(), *mirror.kernel().rules());
        if let Some(state) = mirror.primary_mut() {
            kernel.command_image(&state.view(), &mut image);
        }
        out.commands.extend(image.commands);
    }
}

/// Fehlerarten für Prüfungen am Knoten über den Dienst.
pub(crate) fn fault_msg(channel: usize, fault: Fault) -> Option<HostMsg> {
    u8::try_from(channel)
        .ok()
        .map(|c| HostMsg::InjectFault(c, fault))
}
