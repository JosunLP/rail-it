//! Gegenstelle auf dem Server: sendet Eingaben an den Knoten und sammelt seine Ausgaben je
//! Zyklus (Feature `host`).

use alloc::collections::VecDeque;
use alloc::vec::Vec;

use railit_kernel::{Command, Event, StatePart};
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

/// Vollständig übertragener Zustand des Knotens (Zustandsabgleich).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeSync {
    /// Zyklus, dessen Endzustand übertragen wurde.
    pub cycle: u64,
    /// Digest dieses Zustands.
    pub digest: [u8; 8],
    /// Erste 8 Bytes des Plan-Hashes des Knotens.
    pub plan: [u8; 8],
    /// Alle Teile in Reihenfolge.
    pub parts: Vec<StatePart>,
}

/// Empfangenes vom Knoten, in Empfangsreihenfolge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NodeEvent {
    /// Vollständiger Zyklus.
    Cycle(NodeCycle),
    /// Abgeschlossener Zustandsabgleich (nach den Zyklen, die währenddessen liefen).
    Synced(NodeSync),
}

/// Verbindung des Hosts zum Knoten (RaSTA-Client).
pub struct NodeClient {
    link: Connection,
    partial: Option<NodeCycle>,
    sync: Option<(NodeSync, usize)>,
    events: VecDeque<NodeEvent>,
    protocol_errors: u32,
}

impl NodeClient {
    /// Client für die RaSTA-Verbindung `cfg` (Rolle Client).
    pub const fn new(cfg: railit_rasta::Config) -> Self {
        Self {
            link: Connection::new(cfg),
            partial: None,
            sync: None,
            events: VecDeque::new(),
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

    /// Zustand der RaSTA-Verbindung.
    pub const fn state(&self) -> railit_rasta::State {
        self.link.state()
    }

    /// Baut die Verbindung ab (z. B. nach einem unvollständig gesendeten Zyklus); angefangene
    /// Zyklen und Zustandsabgleiche werden verworfen.
    pub fn close(&mut self, now_ms: u32) {
        self.link.close(now_ms);
        self.partial = None;
        self.sync = None;
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
            sync,
            events,
            protocol_errors,
        } = self;
        let mut errors = 0u32;
        let result = link.receive(now_ms, pdu, &mut |payload| {
            let decoded = decode::<NodeMsg>(payload, &mut |msg| {
                if !Self::accept(partial, sync, events, msg) {
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
        sync: &mut Option<(NodeSync, usize)>,
        events: &mut VecDeque<NodeEvent>,
        msg: NodeMsg,
    ) -> bool {
        match (msg, partial.as_mut()) {
            (
                NodeMsg::SyncBegin {
                    cycle,
                    parts,
                    digest,
                    plan,
                },
                None,
            ) => {
                let clean = sync.is_none();
                let total = usize::try_from(parts).unwrap_or(usize::MAX);
                let begun = NodeSync {
                    cycle,
                    digest,
                    plan,
                    parts: Vec::new(),
                };
                if total == 0 {
                    events.push_back(NodeEvent::Synced(begun));
                    *sync = None;
                } else {
                    *sync = Some((begun, total));
                }
                clean
            }
            (NodeMsg::Sync(part), None) => {
                let Some((mut s, total)) = sync.take() else {
                    return false;
                };
                s.parts.push(part);
                if s.parts.len() >= total {
                    events.push_back(NodeEvent::Synced(s));
                } else {
                    *sync = Some((s, total));
                }
                true
            }
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
                        channels: 0,
                        inputs_lost: false,
                        inputs: 0,
                        dropped: 0,
                        digest: [0; 8],
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
                    events.push_back(NodeEvent::Cycle(p));
                }
                complete
            }
            // Zustandsabgleich mitten in einem Zyklus: Widerspruch.
            (NodeMsg::SyncBegin { .. } | NodeMsg::Sync(_), Some(_)) => {
                *partial = None;
                *sync = None;
                false
            }
            (_, None) => false,
        }
    }

    /// Zeitüberwachung der Verbindung; regelmäßig aufrufen.
    pub fn tick(&mut self, now_ms: u32) -> Option<Fault> {
        let fault = self.link.tick(now_ms);
        if fault.is_some() {
            self.partial = None;
            self.sync = None;
        }
        fault
    }

    /// Nächste zu sendende PDU.
    pub fn poll_transmit(&mut self, buf: &mut [u8]) -> Option<usize> {
        self.link.poll_transmit(buf)
    }

    /// Ältester vollständig empfangener Zyklus (übergeht Zustandsabgleiche).
    pub fn take_cycle(&mut self) -> Option<NodeCycle> {
        while let Some(event) = self.events.pop_front() {
            if let NodeEvent::Cycle(c) = event {
                return Some(c);
            }
        }
        None
    }

    /// Nächstes Empfangene (Zyklus oder abgeschlossener Zustandsabgleich).
    pub fn poll(&mut self) -> Option<NodeEvent> {
        self.events.pop_front()
    }

    /// Ein Zustandsabgleich wird gerade empfangen.
    pub const fn syncing(&self) -> bool {
        self.sync.is_some()
    }
}
