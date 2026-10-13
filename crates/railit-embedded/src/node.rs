//! Der Knoten: Kern in redundanten Kanälen auf dem sicheren Rechner.

use railit_kernel::{Input, Interlocking, KernelState, Occupancy};
use railit_rasta::{Connection, Received};
use railit_rules::RuleSet;
use railit_safety::{
    ChannelOutputs, Channels, ConfigError, FixedOutputs, MAX_CHANNELS, SafetyEvent, Status, Trip,
    Verdict, Watchdog, state_digest,
};
use railit_types::{Tick, TvdId};

use crate::wire::{CycleEnd, HostMsg, NodeMsg, Packer, WireError, decode};

/// Füllwert für den Eingangspuffer (wird nie gelesen).
const FILLER: Input = Input::Tvd {
    tvd: TvdId::new(0),
    occupancy: Occupancy::Disturbed,
};

/// Teile des Zustandsabgleichs je Zyklus (begrenzt die Sendelast).
pub const SYNC_PARTS_PER_TICK: usize = 24;

/// Laufender Zustandsabgleich.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct SyncCursor {
    /// `SyncBegin` steht noch aus: Zyklus und Digest.
    begin: Option<(u64, [u8; 8])>,
    next: usize,
    total: usize,
}

/// Zähler und Digest für `End`.
#[derive(Copy, Clone, Debug)]
struct Counts {
    inputs: u16,
    dropped: u16,
    digest: [u8; 8],
}

/// Erste 8 Bytes eines Digests.
fn short(digest: &[u8]) -> [u8; 8] {
    let mut out = [0u8; 8];
    for (o, d) in out.iter_mut().zip(digest) {
        *o = *d;
    }
    out
}

/// Parameter eines Knotens.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct NodeConfig {
    /// RaSTA-Verbindung zum Host (der Knoten ist Server).
    pub rasta: railit_rasta::Config,
    /// Frist für die Rechenzeit eines Zyklus in Mikrosekunden.
    pub deadline_us: u64,
    /// Höchster Abstand zweier Zyklusbeginne in Mikrosekunden.
    pub gap_limit_us: u64,
}

/// Fehler beim Aufbau eines Knotens.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum NodeError {
    /// Die Kanäle passen nicht zusammen.
    Channels(ConfigError),
}

/// Zähler für Diagnose.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct NodeStats {
    /// Ausgeführte Zyklen.
    pub cycles: u64,
    /// Verworfene oder unlesbare Nachrichten des Hosts.
    pub protocol_errors: u32,
    /// Verbindungsabbrüche (Zeitüberschreitung, Fehler der Gegenstelle, Sendepuffer voll).
    pub link_losses: u32,
    /// Zyklen, in denen Eingaben verloren gingen.
    pub input_overflows: u32,
}

/// Zusammenfassung eines Zyklus.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct TickReport {
    /// Ergebnis der Sicherheitsschicht.
    pub verdict: Verdict,
    /// Die Ausgaben wurden an den Host gesendet.
    pub sent: bool,
}

/// Rechenkern des sicheren Rechners: Kern in bis zu drei Kanälen mit Voter und Watchdog,
/// verbunden mit dem Host über eine RaSTA-Verbindung.
///
/// - `I`: Höchstzahl der Eingaben je Zyklus,
/// - `C`, `E`: Höchstzahl der Stellbefehle bzw. Ereignisse je Zyklus und Kanal.
///
/// Ablauf: Der Aufrufer reicht empfangene PDUs mit [`Node::receive`] herein, ruft
/// [`Node::tick`] aus dem Timer-Interrupt im festen Zyklus auf und holt zu sendende PDUs mit
/// [`Node::poll_transmit`] ab. Der Knoten rechnet in jedem Fall: Bleiben die Meldungen des Hosts
/// aus, behandelt der Kern die Elemente nach der Meldezeit als gestört und nimmt die Fahrt
/// zurück.
pub struct Node<'p, 's, R, const I: usize, const C: usize, const E: usize> {
    channels: Channels<'p, R, KernelState<'s>, FixedOutputs<C, E>>,
    out: FixedOutputs<C, E>,
    inputs: [Input; I],
    input_len: usize,
    dropped: u16,
    reset: bool,
    reintegrate: Option<u8>,
    link: Connection,
    watchdog: Watchdog,
    stats: NodeStats,
    sync: Option<KernelState<'s>>,
    sync_requested: bool,
    sync_cursor: Option<SyncCursor>,
}

impl<'p, 's, R: RuleSet, const I: usize, const C: usize, const E: usize> Node<'p, 's, R, I, C, E> {
    /// Knoten mit einem Kern und den Zustandsspeichern der Kanäle (lückenlos ab Kanal 0).
    ///
    /// # Errors
    /// [`NodeError::Channels`], wenn kein Kanal angegeben ist oder die Kanäle nicht
    /// zusammenpassen.
    pub fn new(
        kernel: Interlocking<'p, R>,
        states: [Option<KernelState<'s>>; MAX_CHANNELS],
        cfg: NodeConfig,
    ) -> Result<Self, NodeError> {
        let channels = Channels::new(kernel, states.map(|s| s.map(|s| (s, FixedOutputs::new()))))
            .map_err(NodeError::Channels)?;
        Ok(Self {
            channels,
            out: FixedOutputs::new(),
            inputs: [FILLER; I],
            input_len: 0,
            dropped: 0,
            reset: false,
            reintegrate: None,
            link: Connection::new(cfg.rasta),
            watchdog: Watchdog::new(cfg.deadline_us, cfg.gap_limit_us),
            stats: NodeStats::default(),
            sync: None,
            sync_requested: false,
            sync_cursor: None,
        })
    }

    /// Abgleichspeicher (gleiche Größe wie die Kanäle) für [`HostMsg::Sync`] (REQ-EMB-006): Der Zustand wird
    /// am Ende eines Zyklus hineinkopiert und von dort in Teilen übertragen, während der Kern
    /// weiterrechnet. Ohne Abgleichspeicher bleibt `Sync` ohne Antwort.
    #[must_use]
    pub fn with_sync_buffer(mut self, buffer: KernelState<'s>) -> Self {
        self.sync = Some(buffer);
        self
    }

    /// Zustand der Sicherheitsschicht.
    pub fn status(&self) -> Status {
        self.channels.status()
    }

    /// Zähler für Diagnose.
    pub const fn stats(&self) -> NodeStats {
        self.stats
    }

    /// Verbindung zum Host steht.
    pub const fn link_up(&self) -> bool {
        self.link.is_up()
    }

    /// Kernzustand des ersten mitwählenden Kanals.
    pub fn state(&self) -> Option<&KernelState<'s>> {
        self.channels.primary()
    }

    /// Verarbeitet eine PDU des Hosts. `now_ms` ist die RaSTA-Zeit des Knotens.
    pub fn receive(&mut self, now_ms: u32, pdu: &[u8]) {
        let Self {
            channels,
            inputs,
            input_len,
            dropped,
            reset,
            reintegrate,
            link,
            stats,
            sync_requested,
            sync_cursor,
            ..
        } = self;
        let mut malformed = false;
        let result = link.receive(now_ms, pdu, &mut |payload| {
            if malformed {
                return;
            }
            let decoded = decode::<HostMsg>(payload, &mut |msg| match msg {
                HostMsg::Input(input) => {
                    if let Some(slot) = inputs.get_mut(*input_len) {
                        *slot = input;
                        *input_len = input_len.saturating_add(1);
                    } else {
                        *dropped = dropped.saturating_add(1);
                    }
                }
                HostMsg::Reset => *reset = true,
                HostMsg::Reintegrate(channel) => *reintegrate = Some(channel),
                HostMsg::InjectFault(channel, fault) => {
                    channels.inject(usize::from(channel), fault);
                }
                HostMsg::Sync => *sync_requested = true,
            });
            malformed |= decoded.is_err();
        });
        match result {
            Received::Discarded(_) => {
                stats.protocol_errors = stats.protocol_errors.saturating_add(1);
            }
            Received::Disconnected(_) => {
                stats.link_losses = stats.link_losses.saturating_add(1);
                *sync_requested = false;
                *sync_cursor = None;
            }
            Received::Nothing | Received::Delivered => {}
        }
        if malformed {
            // Ein unlesbares Telegramm hinter gültigem Sicherheitscode: Gegenstelle fehlerhaft.
            stats.protocol_errors = stats.protocol_errors.saturating_add(1);
            stats.link_losses = stats.link_losses.saturating_add(1);
            link.close(now_ms);
        }
    }

    /// Führt einen Zyklus aus (Aufruf aus dem Timer-Interrupt). `start_us` ist der Beginn
    /// laut monotoner Uhr, `clock` liefert die Uhrzeit am Ende für den Watchdog.
    pub fn tick(&mut self, start_us: u64, clock: &mut dyn FnMut() -> u64) -> TickReport {
        let now_ms = start_us.checked_div(1000).unwrap_or(0);
        let rasta_ms = u32::try_from(now_ms & u64::from(u32::MAX)).unwrap_or(0);
        if self.link.tick(rasta_ms).is_some() {
            self.stats.link_losses = self.stats.link_losses.saturating_add(1);
            self.sync_requested = false;
            self.sync_cursor = None;
        }
        if core::mem::take(&mut self.reset) {
            self.channels.reset();
            self.watchdog.restart();
        }
        if let Some(channel) = self.reintegrate.take() {
            // Ein Fehlschlag (z. B. Kanal wählt schon mit) ändert nichts.
            let _ = self.channels.reintegrate(usize::from(channel));
        }

        let dropped = core::mem::take(&mut self.dropped);
        if dropped > 0 {
            self.stats.input_overflows = self.stats.input_overflows.saturating_add(1);
        }
        let len = core::mem::take(&mut self.input_len);
        let inputs = self.inputs.get(..len).unwrap_or(&[]);
        self.out.clear();
        let mut verdict = self
            .channels
            .step(Tick::from_millis(now_ms), inputs, &mut self.out);
        self.stats.cycles = self.stats.cycles.saturating_add(1);

        let end_us = clock();
        let mut watchdog_trip = None;
        if verdict.trip.is_none()
            && let Err(trip) = self.watchdog.check(start_us, end_us)
        {
            self.channels.trip(trip);
            verdict.trip = Some(trip);
            watchdog_trip = Some(trip);
        }

        let digest = self
            .channels
            .primary()
            .map_or([0; 8], |s| short(&state_digest(s).0));
        if core::mem::take(&mut self.sync_requested)
            && let (Some(buffer), Some(primary)) = (self.sync.as_mut(), self.channels.primary())
            && buffer.copy_from(primary).is_ok()
        {
            self.sync_cursor = Some(SyncCursor {
                begin: Some((self.stats.cycles, digest)),
                next: 0,
                total: buffer.part_count(),
            });
        }

        let counts = Counts {
            inputs: u16::try_from(len).unwrap_or(u16::MAX),
            dropped,
            digest,
        };
        let sent = self.link.is_up()
            && self
                .send_outputs(rasta_ms, now_ms, &verdict, watchdog_trip, counts)
                .is_ok();
        if self.link.is_up() && !sent {
            // Sendepuffer voll: Verbindung abbauen; der Host erkennt den Ausfall.
            self.link.close(rasta_ms);
            self.stats.link_losses = self.stats.link_losses.saturating_add(1);
            self.sync_cursor = None;
        }
        TickReport { verdict, sent }
    }

    fn send_outputs(
        &mut self,
        rasta_ms: u32,
        now_ms: u64,
        verdict: &Verdict,
        watchdog_trip: Option<Trip>,
        counts: Counts,
    ) -> Result<(), WireError> {
        let Self {
            channels,
            out,
            link,
            stats,
            sync,
            sync_cursor,
            ..
        } = self;
        let mut packer = Packer::new();
        let mut flush = |payload: &[u8]| link.send(rasta_ms, payload).is_ok();
        packer.push(
            &NodeMsg::Begin {
                cycle: stats.cycles,
                now_ms,
            },
            &mut flush,
        )?;
        let mut commands: u16 = 0;
        for c in out.commands() {
            packer.push(&NodeMsg::Command(c), &mut flush)?;
            commands = commands.saturating_add(1);
        }
        let mut events: u16 = 0;
        for e in out.events() {
            packer.push(&NodeMsg::Event(e), &mut flush)?;
            events = events.saturating_add(1);
        }
        for s in verdict
            .events()
            .chain(watchdog_trip.map(SafetyEvent::Tripped))
        {
            packer.push(&NodeMsg::Safety(s), &mut flush)?;
        }
        let status = channels.status();
        packer.push(
            &NodeMsg::End(CycleEnd {
                commands,
                events,
                kernel: verdict.kernel.err(),
                trip: verdict.trip,
                active: status.active,
                channels: u8::try_from(status.redundancy.channels()).unwrap_or(u8::MAX),
                inputs_lost: counts.dropped > 0,
                inputs: counts.inputs,
                dropped: counts.dropped,
                digest: counts.digest,
            }),
            &mut flush,
        )?;
        if let (Some(cursor), Some(buffer)) = (sync_cursor.as_mut(), sync.as_ref()) {
            if let Some((cycle, digest)) = cursor.begin.take() {
                packer.push(
                    &NodeMsg::SyncBegin {
                        cycle,
                        parts: u32::try_from(cursor.total).unwrap_or(u32::MAX),
                        digest,
                        plan: short(&channels.kernel().plan().digest()),
                    },
                    &mut flush,
                )?;
            }
            let end = cursor
                .next
                .saturating_add(SYNC_PARTS_PER_TICK)
                .min(cursor.total);
            while cursor.next < end {
                if let Some(part) = buffer.part(cursor.next) {
                    packer.push(&NodeMsg::Sync(part), &mut flush)?;
                }
                cursor.next = cursor.next.saturating_add(1);
            }
            if cursor.next >= cursor.total {
                *sync_cursor = None;
            }
        }
        packer.flush(&mut flush)
    }

    /// Nächste zu sendende PDU in `buf` (mindestens `railit_rasta::MAX_PDU` Bytes).
    pub fn poll_transmit(&mut self, buf: &mut [u8]) -> Option<usize> {
        self.link.poll_transmit(buf)
    }
}
