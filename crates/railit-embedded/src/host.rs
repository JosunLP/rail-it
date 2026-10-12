//! Gegenstelle auf dem Server: sendet Eingaben an den Knoten und sammelt seine Ausgaben je
//! Zyklus (Feature `host`).

use alloc::collections::VecDeque;
use alloc::vec::Vec;

use railit_kernel::{Command, Event};
use railit_rasta::{Connection, Fault, Received};
use railit_safety::SafetyEvent;

use crate::wire::{CycleEnd, HostMsg, NodeMsg, Packer, decode};

/// Vollständige Ausgaben eines Zyklus des Knotens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeCycle {
    /// Zyklusnummer des Knotens.
    pub cycle: u64,
    /// Zeit des Knotens in Millisekunden.
    pub now_ms: u64,
    /// Stellbefehle.
    pub commands: Vec<Command>,
    /// Ereignisse des Kerns.
    pub events: Vec<Event>,
    /// Meldungen der Sicherheitsschicht.
    pub safety: Vec<SafetyEvent>,
    /// Abschluss mit Zustand.
    pub end: CycleEnd,
}

/// Verbindung des Hosts zum Knoten (RaSTA-Client).
pub struct NodeClient {
    link: Connection,
    partial: Option<NodeCycle>,
    cycles: VecDeque<NodeCycle>,
    protocol_errors: u32,
}

impl NodeClient {
    /// Client für die RaSTA-Verbindung `cfg` (Rolle Client).
    pub const fn new(cfg: railit_rasta::Config) -> Self {
        Self {
            link: Connection::new(cfg),
            partial: None,
            cycles: VecDeque::new(),
            protocol_errors: 0,
        }
    }

    /// Baut die Verbindung auf.
    pub fn open(&mut self, now_ms: u32) {
        self.link.open(now_ms);
    }

    /// Verbindung steht.
    pub const fn is_up(&self) -> bool {
        self.link.is_up()
    }

    /// Unvollständige oder widersprüchliche Zyklen und unlesbare Nachrichten.
    pub const fn protocol_errors(&self) -> u32 {
        self.protocol_errors
    }

    /// Sendet Nachrichten an den Knoten.
    ///
    /// # Errors
    /// [`Fault`], wenn die Verbindung nicht steht oder der Sendepuffer voll ist.
    pub fn send(&mut self, now_ms: u32, msgs: &[HostMsg]) -> Result<(), Fault> {
        if !self.link.is_up() {
            return Err(Fault::UnexpectedMessage);
        }
        let link = &mut self.link;
        let mut error = None;
        let mut flush = |payload: &[u8]| match link.send(now_ms, payload) {
            Ok(()) => true,
            Err(e) => {
                error = Some(e);
                false
            }
        };
        let mut packer = Packer::new();
        for m in msgs {
            if packer.push(m, &mut flush).is_err() {
                break;
            }
        }
        let _ = packer.flush(&mut flush);
        error.map_or(Ok(()), Err)
    }

    /// Verarbeitet eine PDU des Knotens.
    pub fn receive(&mut self, now_ms: u32, pdu: &[u8]) {
        let Self {
            link,
            partial,
            cycles,
            protocol_errors,
        } = self;
        let mut errors = 0u32;
        let result = link.receive(now_ms, pdu, &mut |payload| {
            let decoded = decode::<NodeMsg>(payload, &mut |msg| {
                if !Self::accept(partial, cycles, msg) {
                    errors = errors.saturating_add(1);
                }
            });
            if decoded.is_err() {
                errors = errors.saturating_add(1);
            }
        });
        if matches!(result, Received::Discarded(_)) {
            errors = errors.saturating_add(1);
        }
        *protocol_errors = protocol_errors.saturating_add(errors);
    }

    /// Ordnet eine Nachricht dem laufenden Zyklus zu; `false` bei Widerspruch.
    fn accept(
        partial: &mut Option<NodeCycle>,
        cycles: &mut VecDeque<NodeCycle>,
        msg: NodeMsg,
    ) -> bool {
        match (msg, partial.as_mut()) {
            (NodeMsg::Begin { cycle, now_ms }, current) => {
                let clean = current.is_none();
                *partial = Some(NodeCycle {
                    cycle,
                    now_ms,
                    commands: Vec::new(),
                    events: Vec::new(),
                    safety: Vec::new(),
                    end: CycleEnd {
                        commands: 0,
                        events: 0,
                        kernel: None,
                        trip: None,
                        active: [false; railit_safety::MAX_CHANNELS],
                        inputs_lost: false,
                    },
                });
                clean
            }
            (NodeMsg::Command(c), Some(p)) => {
                p.commands.push(c);
                true
            }
            (NodeMsg::Event(e), Some(p)) => {
                p.events.push(e);
                true
            }
            (NodeMsg::Safety(s), Some(p)) => {
                p.safety.push(s);
                true
            }
            (NodeMsg::End(end), Some(_)) => {
                let Some(mut p) = partial.take() else {
                    return false;
                };
                let complete = usize::from(end.commands) == p.commands.len()
                    && usize::from(end.events) == p.events.len();
                p.end = end;
                if complete {
                    cycles.push_back(p);
                }
                complete
            }
            (_, None) => false,
        }
    }

    /// Zeitüberwachung der Verbindung; regelmäßig aufrufen.
    pub fn tick(&mut self, now_ms: u32) -> Option<Fault> {
        let fault = self.link.tick(now_ms);
        if fault.is_some() {
            self.partial = None;
        }
        fault
    }

    /// Nächste zu sendende PDU.
    pub fn poll_transmit(&mut self, buf: &mut [u8]) -> Option<usize> {
        self.link.poll_transmit(buf)
    }

    /// Ältester vollständig empfangener Zyklus.
    pub fn take_cycle(&mut self) -> Option<NodeCycle> {
        self.cycles.pop_front()
    }
}
