//! Feldrechner über das Netz (REQ-COM-008) mit Umschalter (REQ-SVC-013).
//!
//! ```text
//!  Hauptrechner ──RaSTA/UDP──┐
//!                            ├─▶ Feldrechner: Umschalter → Objektsteuerungen → Feld (Simulator)
//!  Bereitschaftsrechner ─────┘
//! ```
//!
//! Die Objektsteuerungen laufen in einem eigenen Prozess ([`FieldServer`], `railit-field`);
//! jedes Stellwerk verbindet sich je Objektsteuerung über RaSTA ([`FieldClient`],
//! [`UdpField`]). Beide sind reine Zustandsmaschinen ohne I/O und ohne Uhr; [`UdpField`]
//! legt die Übertragung über UDP darum.
//!
//! **Umschalter.** Stellen darf immer nur ein Stellwerk, das *führende*. Der Feldrechner nimmt
//! PDUs nur vom führenden Stellwerk an; die anderer Stellwerke verwirft er, deren
//! Verbindungen kommen nicht zustande. Ein anderes Stellwerk kann erst führen, wenn alle
//! Verbindungen des bisherigen seit der Umschaltzeit abgebaut sind – nach dem Ausfall des
//! Rechners oder der Strecke zu ihm (RaSTA baut nach `t_max` ohne Meldung ab). Dann hat die
//! Rückfallebene der Objektsteuerungen die Signale bereits auf Halt gestellt. Der erste
//! Verbindungsaufbau eines zugelassenen Stellwerks danach macht es zum führenden. Damit stellen
//! nie zwei Stellwerke zugleich, auch nicht bei getrennter Verbindung zwischen ihnen.

use std::collections::HashMap;
use std::net::{SocketAddr, UdpSocket};
use std::time::Instant;

use railit_kernel::{Aspect, Command, Input};
use railit_rasta::{Config, Connection, Received, Role, State};
use railit_sim::Simulator;
use railit_topology::Topology;
use railit_types::Tick;
use serde::Serialize;

use crate::sci::{Message, Protocol, Telegram};
use crate::{
    Element, Elements, INTERLOCKING_ID, LinkFaults, LinkStatus, command_of, input_of, layout,
    message_of, rasta_now, status_of,
};

/// RaSTA-Kennung des Hauptrechners (Vorgabe).
pub const PRIMARY_ID: u32 = 0x0A00_0001;
/// RaSTA-Kennung des Bereitschaftsrechners (Vorgabe).
pub const STANDBY_ID: u32 = 0x0A00_0002;
/// Höchstlänge einer PDU (Empfangspuffer).
pub const MAX_PDU: usize = railit_rasta::MAX_PDU;
/// Vorgabe der Umschaltzeit in Millisekunden.
pub const SWITCH_MS: u64 = 500;

fn drain(conn: &mut Connection) -> Vec<Vec<u8>> {
    let mut buf = [0u8; MAX_PDU];
    let mut out = Vec::new();
    while let Some(n) = conn.poll_transmit(&mut buf) {
        if let Some(pdu) = buf.get(..n) {
            out.push(pdu.to_vec());
        }
    }
    out
}

/// Verbindung einer Objektsteuerung zu einem Stellwerk.
struct ServerConn {
    interlocking: u32,
    conn: Connection,
    /// Zuletzt gemeldete Zustände.
    sent: HashMap<Element, Message>,
    was_up: bool,
}

struct ServerOc {
    name: String,
    protocol: Protocol,
    elements: Elements,
    id: u32,
    conns: Vec<ServerConn>,
    bad_telegrams: u32,
}

fn server_conn(oc: u32, interlocking: u32) -> ServerConn {
    let mut cfg = Config::new(oc, interlocking, Role::Server);
    cfg.initial_sn = 0x1000;
    ServerConn {
        interlocking,
        conn: Connection::new(cfg),
        sent: HashMap::new(),
        was_up: false,
    }
}

/// Zugelassenes Stellwerk.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct InterlockingStatus {
    /// RaSTA-Kennung.
    pub id: u32,
    /// Zuletzt gesehene Adresse.
    pub address: Option<String>,
    /// Führt (stellt) gerade.
    pub leading: bool,
    /// Objektsteuerungen mit stehender Verbindung.
    pub connected: usize,
}

/// Zustand des Feldrechners.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FieldStatus {
    /// Führendes Stellwerk.
    pub leader: Option<u32>,
    /// Zugelassene Stellwerke.
    pub interlockings: Vec<InterlockingStatus>,
    /// Objektsteuerungen.
    pub controllers: usize,
    /// Wechsel des führenden Stellwerks.
    pub switches: u32,
    /// Verworfene PDUs (unbekannte Absender, nicht führende Stellwerke).
    pub rejected: u64,
    /// Seit wann (ms, Uhr des Feldrechners) das führende Stellwerk ohne Verbindung ist.
    pub leader_lost_ms: Option<u64>,
}

/// Feldrechner: Simulator, Objektsteuerungen und Umschalter.
pub struct FieldServer {
    sim: Simulator,
    ocs: Vec<ServerOc>,
    interlockings: Vec<(u32, Option<SocketAddr>)>,
    leader: Option<u32>,
    lost_at: Option<u64>,
    switch_ms: u64,
    switches: u32,
    rejected: u64,
    now: u64,
}

impl FieldServer {
    /// Feldrechner vor `sim` für die Stellwerke `interlockings` (RaSTA-Kennungen).
    ///
    /// # Errors
    /// Ungültige Elementnamen (siehe [`crate::EulynxField::new`]) oder keine Stellwerke.
    pub fn new(sim: Simulator, interlockings: &[u32], switch_ms: u64) -> Result<Self, String> {
        if interlockings.is_empty() {
            return Err("Feldrechner ohne zugelassenes Stellwerk".into());
        }
        let ocs = layout(sim.topology())?
            .into_iter()
            .map(|oc| ServerOc {
                conns: interlockings
                    .iter()
                    .map(|i| server_conn(oc.id, *i))
                    .collect(),
                name: oc.name,
                protocol: oc.protocol,
                elements: oc.elements,
                id: oc.id,
                bad_telegrams: 0,
            })
            .collect();
        Ok(Self {
            sim,
            ocs,
            interlockings: interlockings.iter().map(|i| (*i, None)).collect(),
            leader: None,
            lost_at: None,
            switch_ms,
            switches: 0,
            rejected: 0,
            now: 0,
        })
    }

    /// Der Simulator hinter den Objektsteuerungen.
    pub const fn simulator(&self) -> &Simulator {
        &self.sim
    }

    /// Der Simulator hinter den Objektsteuerungen.
    pub const fn simulator_mut(&mut self) -> &mut Simulator {
        &mut self.sim
    }

    /// Führendes Stellwerk.
    pub const fn leader(&self) -> Option<u32> {
        self.leader
    }

    /// Nimmt eine PDU von `from` an. Nur das führende Stellwerk – oder, wenn keines führt,
    /// ein zugelassenes – wird gehört; seine Stellbefehle wirken sofort.
    pub fn receive(&mut self, from: SocketAddr, pdu: &[u8]) {
        let Some((receiver, sender)) = railit_rasta::addresses(pdu) else {
            self.rejected = self.rejected.saturating_add(1);
            return;
        };
        let Some(slot) = self.interlockings.iter_mut().find(|(id, _)| *id == sender) else {
            self.rejected = self.rejected.saturating_add(1);
            return;
        };
        if self.leader.is_some_and(|leader| leader != sender) {
            self.rejected = self.rejected.saturating_add(1);
            return;
        }
        slot.1 = Some(from);
        let t = rasta_now(self.now);
        let sim = &mut self.sim;
        let Some(oc) = self.ocs.iter_mut().find(|o| o.id == receiver) else {
            self.rejected = self.rejected.saturating_add(1);
            return;
        };
        let Some(c) = oc.conns.iter_mut().find(|c| c.interlocking == sender) else {
            return;
        };
        let mut telegrams = Vec::new();
        c.conn
            .receive(t, pdu, &mut |data| telegrams.push(data.to_vec()));
        if self.leader.is_none() {
            // Führend wird ein Stellwerk erst mit einem echten Verbindungsaufbau.
            if c.conn.state() == State::Closed {
                return;
            }
            self.leader = Some(sender);
            self.lost_at = None;
            self.switches = self.switches.saturating_add(1);
        }
        for raw in telegrams {
            let command = Telegram::decode(&raw).ok().and_then(|tg| {
                let element = oc
                    .elements
                    .iter()
                    .find(|(n, _)| *n == tg.receiver)
                    .map(|(_, e)| *e)?;
                command_of(element, tg.message)
            });
            match command {
                Some(cmd) => sim.apply(&[cmd]),
                None => oc.bad_telegrams = oc.bad_telegrams.saturating_add(1),
            }
        }
    }

    /// Schreibt das Feld bis `now` fort, überwacht die Verbindungen, meldet Zustands-
    /// änderungen und liefert die zu sendenden PDUs mit ihrem Ziel.
    pub fn tick(&mut self, now: Tick) -> Vec<(SocketAddr, Vec<u8>)> {
        self.now = now.as_millis();
        if now > self.sim.now() {
            self.sim.advance(now);
        }
        let t = rasta_now(self.now);
        let mut out = Vec::new();
        let sim = &mut self.sim;
        for oc in &mut self.ocs {
            for c in &mut oc.conns {
                c.conn.tick(t);
                let up = c.conn.is_up();
                if up && !c.was_up {
                    c.sent.clear();
                }
                // Rückfallebene: ohne Verbindung zum Stellwerk zeigen die Signale Halt, ein
                // Ersatzsignal erlischt.
                if c.was_up && !up {
                    fallback(sim, &oc.elements);
                }
                c.was_up = up;
                if up {
                    for (name, e) in &oc.elements {
                        let Some(m) = status_of(sim, *e) else {
                            continue;
                        };
                        if c.sent.get(e) == Some(&m) {
                            continue;
                        }
                        let tg = Telegram {
                            sender: name.clone(),
                            receiver: INTERLOCKING_ID.into(),
                            message: m,
                        };
                        if let Ok(raw) = tg.encode()
                            && c.conn.send(t, &raw).is_ok()
                        {
                            c.sent.insert(*e, m);
                        }
                    }
                } else if c.conn.state() == State::Closed {
                    c.sent.clear();
                }
                let to = self
                    .interlockings
                    .iter()
                    .find(|(id, _)| *id == c.interlocking)
                    .and_then(|(_, a)| *a);
                for pdu in drain(&mut c.conn) {
                    if let Some(to) = to {
                        out.push((to, pdu));
                    }
                }
            }
        }
        self.supervise_leader();
        out
    }

    /// Gibt die Führung frei, wenn alle Verbindungen des führenden Stellwerks seit der
    /// Umschaltzeit abgebaut sind.
    fn supervise_leader(&mut self) {
        let Some(leader) = self.leader else {
            return;
        };
        let alive = self.ocs.iter().any(|oc| {
            oc.conns
                .iter()
                .any(|c| c.interlocking == leader && c.conn.state() != State::Closed)
        });
        if alive {
            self.lost_at = None;
            return;
        }
        let since = *self.lost_at.get_or_insert(self.now);
        if self.now.saturating_sub(since) >= self.switch_ms {
            self.leader = None;
            self.lost_at = None;
            for oc in &mut self.ocs {
                for c in &mut oc.conns {
                    if c.interlocking == leader {
                        *c = server_conn(oc.id, leader);
                    }
                }
            }
        }
    }

    /// Zustand für Anzeige und Diagnose.
    pub fn status(&self) -> FieldStatus {
        FieldStatus {
            leader: self.leader,
            interlockings: self
                .interlockings
                .iter()
                .map(|(id, addr)| InterlockingStatus {
                    id: *id,
                    address: addr.map(|a| a.to_string()),
                    leading: self.leader == Some(*id),
                    connected: self
                        .ocs
                        .iter()
                        .filter(|oc| {
                            oc.conns
                                .iter()
                                .any(|c| c.interlocking == *id && c.conn.is_up())
                        })
                        .count(),
                })
                .collect(),
            controllers: self.ocs.len(),
            switches: self.switches,
            rejected: self.rejected,
            leader_lost_ms: self.lost_at,
        }
    }

    /// Verbindungen der Objektsteuerungen zum führenden Stellwerk.
    pub fn links(&self) -> Vec<LinkStatus> {
        self.ocs
            .iter()
            .map(|oc| {
                let conn = oc
                    .conns
                    .iter()
                    .find(|c| Some(c.interlocking) == self.leader)
                    .map(|c| &c.conn);
                LinkStatus {
                    controller: oc.name.clone(),
                    interface: format!("{:?}", oc.protocol),
                    elements: oc.elements.len(),
                    up: conn.is_some_and(Connection::is_up),
                    discarded: conn.map_or(0, Connection::discarded),
                    bad_telegrams: oc.bad_telegrams,
                    faults: LinkFaults::default(),
                }
            })
            .collect()
    }
}

/// Rückfallebene einer Objektsteuerung: Signale auf Halt, Ersatzsignale aus.
fn fallback(sim: &mut Simulator, elements: &Elements) {
    for (_, e) in elements {
        if let Element::Signal(signal) = *e {
            sim.apply(&[
                Command::SetSignal {
                    signal,
                    aspect: Aspect::Stop,
                },
                Command::SetSubstitute { signal, on: false },
            ]);
        }
    }
}

struct ClientOc {
    name: String,
    protocol: Protocol,
    elements: Elements,
    id: u32,
    conn: Connection,
    status: HashMap<Element, Message>,
    bad_telegrams: u32,
}

/// Stellwerksseite der Anbindung an einen Feldrechner: je Objektsteuerung eine
/// RaSTA-Verbindung, die sich nach einem Abbau selbsttätig neu aufbaut. Meldungen gibt sie nur
/// von Objektsteuerungen mit stehender Verbindung weiter.
pub struct FieldClient {
    id: u32,
    ocs: Vec<ClientOc>,
}

impl FieldClient {
    /// Anbindung des Stellwerks `id` an die Objektsteuerungen des Netzes `net`.
    ///
    /// # Errors
    /// Ungültige Elementnamen.
    pub fn new(net: &Topology, id: u32) -> Result<Self, String> {
        let ocs = layout(net)?
            .into_iter()
            .map(|oc| ClientOc {
                conn: Connection::new(Config::new(id, oc.id, Role::Client)),
                name: oc.name,
                protocol: oc.protocol,
                elements: oc.elements,
                id: oc.id,
                status: HashMap::new(),
                bad_telegrams: 0,
            })
            .collect();
        Ok(Self { id, ocs })
    }

    /// RaSTA-Kennung dieses Stellwerks.
    pub const fn id(&self) -> u32 {
        self.id
    }

    /// Nimmt eine PDU des Feldrechners an.
    pub fn receive(&mut self, now_ms: u64, pdu: &[u8]) {
        let Some((receiver, sender)) = railit_rasta::addresses(pdu) else {
            return;
        };
        if receiver != self.id {
            return;
        }
        let Some(oc) = self.ocs.iter_mut().find(|o| o.id == sender) else {
            return;
        };
        let mut telegrams = Vec::new();
        let r = oc
            .conn
            .receive(rasta_now(now_ms), pdu, &mut |d| telegrams.push(d.to_vec()));
        if matches!(r, Received::Disconnected(_)) {
            oc.status.clear();
        }
        for raw in telegrams {
            let found = Telegram::decode(&raw).ok().and_then(|tg| {
                oc.elements
                    .iter()
                    .find(|(n, _)| *n == tg.sender)
                    .map(|(_, e)| (*e, tg.message))
            });
            match found {
                Some((e, m)) => {
                    oc.status.insert(e, m);
                }
                None => oc.bad_telegrams = oc.bad_telegrams.saturating_add(1),
            }
        }
    }

    /// Überwacht die Verbindungen, baut abgebaute neu auf und liefert die zu sendenden PDUs.
    pub fn tick(&mut self, now_ms: u64) -> Vec<Vec<u8>> {
        let t = rasta_now(now_ms);
        let mut out = Vec::new();
        for oc in &mut self.ocs {
            if oc.conn.state() == State::Closed {
                oc.status.clear();
                oc.conn.open(t);
            }
            oc.conn.tick(t);
            if !oc.conn.is_up() {
                oc.status.clear();
            }
            out.extend(drain(&mut oc.conn));
        }
        out
    }

    /// Meldungen für den Kern.
    pub fn reports(&self) -> Vec<Input> {
        self.ocs
            .iter()
            .filter(|oc| oc.conn.is_up())
            .flat_map(|oc| {
                oc.elements
                    .iter()
                    .filter_map(|(_, e)| oc.status.get(e).and_then(|m| input_of(*e, *m)))
            })
            .collect()
    }

    /// Stellbefehle als Telegramme; liefert die zu sendenden PDUs.
    pub fn apply(&mut self, now_ms: u64, commands: &[Command]) -> Vec<Vec<u8>> {
        let t = rasta_now(now_ms);
        for cmd in commands {
            let Some((element, message)) = message_of(*cmd) else {
                continue;
            };
            let Some(oc) = self
                .ocs
                .iter_mut()
                .find(|oc| oc.elements.iter().any(|(_, e)| *e == element))
            else {
                continue;
            };
            let Some(name) = oc
                .elements
                .iter()
                .find(|(_, e)| *e == element)
                .map(|(n, _)| n.clone())
            else {
                continue;
            };
            let tg = Telegram {
                sender: INTERLOCKING_ID.into(),
                receiver: name,
                message,
            };
            // Ohne Verbindung geht der Befehl verloren; der Kern bemerkt es an den Meldungen.
            if let Ok(raw) = tg.encode() {
                let _ = oc.conn.send(t, &raw);
            }
        }
        self.ocs
            .iter_mut()
            .flat_map(|oc| drain(&mut oc.conn))
            .collect()
    }

    /// Stehen alle Verbindungen?
    pub fn connected(&self) -> bool {
        self.ocs.iter().all(|oc| oc.conn.is_up())
    }

    /// Zustand der Verbindungen.
    pub fn links(&self) -> Vec<LinkStatus> {
        self.ocs
            .iter()
            .map(|oc| LinkStatus {
                controller: oc.name.clone(),
                interface: format!("{:?}", oc.protocol),
                elements: oc.elements.len(),
                up: oc.conn.is_up(),
                discarded: oc.conn.discarded(),
                bad_telegrams: oc.bad_telegrams,
                faults: LinkFaults::default(),
            })
            .collect()
    }
}

/// [`FieldClient`] über UDP mit der Wanduhr als RaSTA-Zeit.
pub struct UdpField {
    client: FieldClient,
    socket: UdpSocket,
    clock: Instant,
}

impl UdpField {
    /// Verbindet das Stellwerk `id` mit dem Feldrechner unter `field`.
    ///
    /// # Errors
    /// Ungültige Elementnamen oder Socketfehler.
    pub fn connect(net: &Topology, field: SocketAddr, id: u32) -> Result<Self, String> {
        let local: SocketAddr = if field.is_ipv4() {
            ([0, 0, 0, 0], 0).into()
        } else {
            ([0u16; 8], 0).into()
        };
        let socket = UdpSocket::bind(local).map_err(|e| format!("UDP: {e}"))?;
        socket
            .connect(field)
            .map_err(|e| format!("Feldrechner {field}: {e}"))?;
        socket
            .set_nonblocking(true)
            .map_err(|e| format!("UDP: {e}"))?;
        Ok(Self {
            client: FieldClient::new(net, id)?,
            socket,
            clock: Instant::now(),
        })
    }

    fn ms(&self) -> u64 {
        u64::try_from(self.clock.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn send(&self, pdus: Vec<Vec<u8>>) {
        for pdu in pdus {
            // Verlust ist für RaSTA ein gewöhnlicher Fall.
            let _ = self.socket.send(&pdu);
        }
    }

    /// Holt eingegangene PDUs ab und überwacht die Verbindungen.
    pub fn advance(&mut self) {
        let mut buf = [0u8; MAX_PDU];
        let now = self.ms();
        while let Ok(n) = self.socket.recv(&mut buf) {
            if let Some(pdu) = buf.get(..n) {
                self.client.receive(now, pdu);
            }
        }
        let out = self.client.tick(now);
        self.send(out);
    }

    /// Meldungen für den Kern.
    pub fn reports(&self) -> Vec<Input> {
        self.client.reports()
    }

    /// Stellbefehle an die Objektsteuerungen.
    pub fn apply(&mut self, commands: &[Command]) {
        let now = self.ms();
        let out = self.client.apply(now, commands);
        self.send(out);
    }

    /// Die Anbindung ohne Übertragung.
    pub const fn client(&self) -> &FieldClient {
        &self.client
    }
}
