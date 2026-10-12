//! Der Knoten: Kern in redundanten Kanälen auf dem sicheren Rechner.

use railit_kernel::{Input, Interlocking, KernelState, Occupancy};
use railit_rasta::{Connection, Received};
use railit_rules::RuleSet;
use railit_safety::{
    ChannelOutputs, Channels, ConfigError, FixedOutputs, MAX_CHANNELS, SafetyEvent, Status, Trip,
    Verdict, Watchdog,
};
use railit_types::{Tick, TvdId};

use crate::wire::{CycleEnd, HostMsg, NodeMsg, Packer, WireError, decode};

/// Füllwert für den Eingangspuffer (wird nie gelesen).
const FILLER: Input = Input::Tvd {
    tvd: TvdId::new(0),
    occupancy: Occupancy::Disturbed,
};

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
    inputs_lost: bool,
    reset: bool,
    reintegrate: Option<u8>,
    link: Connection,
    watchdog: Watchdog,
    stats: NodeStats,
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
            inputs_lost: false,
            reset: false,
            reintegrate: None,
            link: Connection::new(cfg.rasta),
            watchdog: Watchdog::new(cfg.deadline_us, cfg.gap_limit_us),
            stats: NodeStats::default(),
        })
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
            inputs_lost,
            reset,
            reintegrate,
            link,
            stats,
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
                        *inputs_lost = true;
                    }
                }
                HostMsg::Reset => *reset = true,
                HostMsg::Reintegrate(channel) => *reintegrate = Some(channel),
                HostMsg::InjectFault(channel, fault) => {
                    channels.inject(usize::from(channel), fault);
                }
            });
            malformed |= decoded.is_err();
        });
        match result {
            Received::Discarded(_) => {
                stats.protocol_errors = stats.protocol_errors.saturating_add(1);
            }
            Received::Disconnected(_) => {
                stats.link_losses = stats.link_losses.saturating_add(1);
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
        }
        if core::mem::take(&mut self.reset) {
            self.channels.reset();
            self.watchdog.restart();
        }
        if let Some(channel) = self.reintegrate.take() {
            // Ein Fehlschlag (z. B. Kanal wählt schon mit) ändert nichts.
            let _ = self.channels.reintegrate(usize::from(channel));
        }

        let inputs_lost = core::mem::take(&mut self.inputs_lost);
        if inputs_lost {
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

        let sent = self.link.is_up()
            && self
                .send_outputs(rasta_ms, now_ms, &verdict, watchdog_trip, inputs_lost)
                .is_ok();
        if self.link.is_up() && !sent {
            // Sendepuffer voll: Verbindung abbauen; der Host erkennt den Ausfall.
            self.link.close(rasta_ms);
            self.stats.link_losses = self.stats.link_losses.saturating_add(1);
        }
        TickReport { verdict, sent }
    }

    fn send_outputs(
        &mut self,
        rasta_ms: u32,
        now_ms: u64,
        verdict: &Verdict,
        watchdog_trip: Option<Trip>,
        inputs_lost: bool,
    ) -> Result<(), WireError> {
        let Self {
            channels,
            out,
            link,
            stats,
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
                inputs_lost,
            }),
            &mut flush,
        )?;
        packer.flush(&mut flush)
    }

    /// Nächste zu sendende PDU in `buf` (mindestens `railit_rasta::MAX_PDU` Bytes).
    pub fn poll_transmit(&mut self, buf: &mut [u8]) -> Option<usize> {
        self.link.poll_transmit(buf)
    }
}
