//! RaSTA-Sicherheits- und Wiederholschicht (DIN VDE V 0831-200), vereinfacht.
//!
//! RaSTA sichert die Übertragung zwischen Stellwerk und Feldelementen (EULYNX SCI) nach
//! EN 50159 gegen Verfälschung, Wiederholung, Vertauschung, Verlust, Verzögerung und
//! Einfügung:
//!
//! | Bedrohung | Maßnahme |
//! |-----------|----------|
//! | Verfälschung | Sicherheitscode (MD4 mit vereinbarten Anfangswerten, 8 Byte) |
//! | Wiederholung, Vertauschung, Verlust | Datenindex, Quittierung, Wiederholanforderung |
//! | Verzögerung | Zeitstempel und bestätigter Zeitstempel, Höchstalter `t_max` |
//! | Einfügung, Maskerade | Absender- und Empfängerkennung, Sicherheitscode |
//! | Ausfall | Lebenszeichen (Heartbeat) alle `t_h`, Abbau nach `t_max` ohne Meldung |
//!
//! **PDU** (little-endian): Länge u16, Typ u16, Empfänger u32, Absender u32,
//! Sequenznummer u32, bestätigte Sequenznummer u32, Zeitstempel u32, bestätigter
//! Zeitstempel u32, bestätigter Datenindex u32, nächster eigener Datenindex u32, Nutzdaten, Sicherheitscode (erste 8 Byte von
//! MD4 über alles davor). Datennachrichten tragen vor den Nutzdaten deren Länge (u16) und
//! ihren Datenindex (u32).
//!
//! **Wiederholung** (REQ-COM-003, REQ-COM-007): Datennachrichten sind fortlaufend nummeriert
//! (Datenindex). Der Empfänger
//! liefert sie nur lückenlos in dieser Reihenfolge; ältere Indizes sind Doppel und werden
//! verworfen. Jede PDU quittiert den nächsten erwarteten Index. Fehlt eine Nachricht, fordert
//! der Empfänger die Wiederholung an (wiederholt alle `t_h / 3`, solange die Lücke besteht) –
//! auch wenn die letzte Nachricht fehlt: Jede PDU nennt den nächsten Datenindex ihres Senders;
//! der Sender schickt dann alle unquittierten Nachrichten mit ihrem ursprünglichen Index
//! erneut. Eine Wiederholung ist damit beliebig oft möglich, ohne dass etwas doppelt geliefert
//! wird. Lebendüberwachung und Zeitstempelbestätigung folgen jeder authentischen,
//! rechtzeitigen PDU.
//!
//! Die Verbindung ist eine reine Zustandsmaschine ohne I/O und ohne Uhr: Der Aufrufer reicht
//! empfangene PDUs und die aktuelle Zeit herein und holt zu sendende PDUs ab
//! ([`Connection::poll_transmit`]). Alle Puffer haben feste Größe.

#![no_std]
#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::float_arithmetic,
    clippy::as_conversions
)]
#![cfg_attr(
    test,
    allow(
        clippy::indexing_slicing,
        clippy::arithmetic_side_effects,
        clippy::unwrap_used,
        clippy::as_conversions
    )
)]

pub mod md4;
#[cfg(kani)]
mod proofs;
pub mod redundancy;

/// Länge des Kopfs in Byte.
pub const HEADER_LEN: usize = 36;
/// Länge des Sicherheitscodes in Byte.
pub const SAFETY_CODE_LEN: usize = 8;
/// Höchstlänge der Nutzdaten einer Datennachricht.
pub const MAX_PAYLOAD: usize = 256;
/// Kopf einer Datennachricht vor den Nutzdaten: Länge (u16) und Datenindex (u32).
pub const DATA_HEADER_LEN: usize = 6;
/// Höchstlänge einer PDU.
pub const MAX_PDU: usize = HEADER_LEN + DATA_HEADER_LEN + MAX_PAYLOAD + SAFETY_CODE_LEN;
/// Anzahl unquittierter Nachrichten, die für Wiederholungen gehalten werden (`N_sendmax`).
pub const SEND_BUFFER: usize = 16;
/// Länge der Sendewarteschlange.
pub const QUEUE: usize = 32;

/// Empfänger- und Absenderkennung einer PDU, ungeprüft: zum Verteilen eingehender PDUs auf
/// Verbindungen. Ob die PDU echt ist, prüft erst [`Connection::receive`].
pub fn addresses(pdu: &[u8]) -> Option<(u32, u32)> {
    let receiver: [u8; 4] = pdu.get(4..8)?.try_into().ok()?;
    let sender: [u8; 4] = pdu.get(8..12)?.try_into().ok()?;
    Some((u32::from_le_bytes(receiver), u32::from_le_bytes(sender)))
}

/// Nachrichtentypen.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum Kind {
    /// Verbindungsanforderung.
    ConnReq = 6200,
    /// Verbindungsantwort.
    ConnResp = 6201,
    /// Wiederholanforderung.
    RetrReq = 6212,
    /// Wiederholantwort.
    RetrResp = 6213,
    /// Verbindungsabbau.
    DiscReq = 6216,
    /// Lebenszeichen.
    Heartbeat = 6220,
    /// Daten.
    Data = 6240,
    /// Wiederholte Daten.
    RetrData = 6241,
}

impl Kind {
    const fn from_code(code: u16) -> Option<Self> {
        Some(match code {
            6200 => Self::ConnReq,
            6201 => Self::ConnResp,
            6212 => Self::RetrReq,
            6213 => Self::RetrResp,
            6216 => Self::DiscReq,
            6220 => Self::Heartbeat,
            6240 => Self::Data,
            6241 => Self::RetrData,
            _ => return None,
        })
    }

    const fn code(self) -> u16 {
        match self {
            Self::ConnReq => 6200,
            Self::ConnResp => 6201,
            Self::RetrReq => 6212,
            Self::RetrResp => 6213,
            Self::DiscReq => 6216,
            Self::Heartbeat => 6220,
            Self::Data => 6240,
            Self::RetrData => 6241,
        }
    }
}

/// Rolle beim Verbindungsaufbau.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Role {
    /// Baut die Verbindung auf (Stellwerk).
    Client,
    /// Nimmt die Verbindung an (Objektsteuerung).
    Server,
}

/// Parameter einer Verbindung.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Config {
    /// Eigene Kennung.
    pub local: u32,
    /// Kennung der Gegenstelle.
    pub peer: u32,
    /// Rolle.
    pub role: Role,
    /// Höchstalter einer Nachricht und Höchstdauer ohne Meldung (ms).
    pub t_max_ms: u32,
    /// Abstand der Lebenszeichen (ms).
    pub t_h_ms: u32,
    /// Anfangswerte des MD4 (vereinbart je Netz).
    pub md4_iv: [u32; 4],
    /// Anfängliche eigene Sequenznummer.
    pub initial_sn: u32,
}

impl Config {
    /// Vorgaben nach DIN VDE V 0831-200: `t_max` 1800 ms, `t_h` 300 ms, Standard-MD4.
    pub const fn new(local: u32, peer: u32, role: Role) -> Self {
        Self {
            local,
            peer,
            role,
            t_max_ms: 1800,
            t_h_ms: 300,
            md4_iv: md4::STANDARD_IV,
            initial_sn: 0,
        }
    }
}

/// Zustand der Verbindung.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// Geschlossen.
    Closed,
    /// Verbindungsanforderung gesendet bzw. Antwort gesendet, Bestätigung ausstehend.
    Opening,
    /// Verbunden; Daten werden übertragen.
    Up,
    /// Lücke erkannt, Wiederholung angefordert.
    RetransmissionRequested,
}

/// Grund für Verwerfen einer PDU oder für den Abbau.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Fault {
    /// Länge oder Aufbau ungültig.
    Malformed,
    /// Sicherheitscode falsch (Verfälschung).
    SafetyCode,
    /// Absender oder Empfänger falsch (Einfügung, Maskerade).
    Address,
    /// Daten bereits empfangen (Wiederholung) oder außer der Reihe.
    Duplicate,
    /// Nachricht zu alt oder bestätigter Zeitstempel unplausibel (Verzögerung).
    Timeliness,
    /// Quittierte Sequenznummer oder quittierter Datenindex unplausibel.
    Confirmation,
    /// Zu lange keine Meldung der Gegenstelle.
    Timeout,
    /// Wiederholung nicht möglich (Nachricht nicht mehr im Puffer).
    RetransmissionFailed,
    /// Die Gegenstelle hat die Verbindung abgebaut.
    PeerDisconnected,
    /// Nachricht passt nicht zum Zustand.
    UnexpectedMessage,
    /// Nutzdaten zu lang oder Warteschlange voll.
    Overflow,
}

/// Ergebnis der Verarbeitung einer empfangenen PDU.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Received {
    /// Keine Nutzdaten (Lebenszeichen, Verbindungsaufbau, Quittung).
    Nothing,
    /// Nutzdaten wurden an `deliver` übergeben.
    Delivered,
    /// PDU verworfen, Verbindung besteht weiter.
    Discarded(Fault),
    /// Verbindung abgebaut.
    Disconnected(Fault),
}

#[derive(Copy, Clone)]
struct Pdu {
    len: usize,
    bytes: [u8; MAX_PDU],
}

const EMPTY_PDU: Pdu = Pdu {
    len: 0,
    bytes: [0; MAX_PDU],
};

impl Pdu {
    fn as_slice(&self) -> &[u8] {
        self.bytes.get(..self.len).unwrap_or(&[])
    }
}

/// Ring fester Größe.
struct Ring<const N: usize> {
    items: [Pdu; N],
    head: usize,
    len: usize,
}

impl<const N: usize> Ring<N> {
    const fn new() -> Self {
        Self {
            items: [EMPTY_PDU; N],
            head: 0,
            len: 0,
        }
    }

    fn push(&mut self, pdu: &Pdu) -> bool {
        if self.len >= N {
            return false;
        }
        let Some(index) = self
            .head
            .checked_add(self.len)
            .and_then(|i| i.checked_rem(N))
        else {
            return false;
        };
        if let Some(slot) = self.items.get_mut(index) {
            *slot = *pdu;
            self.len = self.len.saturating_add(1);
            return true;
        }
        false
    }

    fn pop(&mut self) -> Option<Pdu> {
        if self.len == 0 {
            return None;
        }
        let pdu = self.items.get(self.head).copied()?;
        self.head = self.head.checked_add(1).and_then(|i| i.checked_rem(N))?;
        self.len = self.len.saturating_sub(1);
        Some(pdu)
    }

    fn get(&self, i: usize) -> Option<&Pdu> {
        if i >= self.len {
            return None;
        }
        self.items
            .get(self.head.checked_add(i).and_then(|x| x.checked_rem(N))?)
    }

    fn clear(&mut self) {
        self.head = 0;
        self.len = 0;
    }
}

fn read_u16(b: &[u8], off: usize) -> Option<u16> {
    let raw = b.get(off..off.checked_add(2)?)?;
    Some(u16::from_le_bytes([*raw.first()?, *raw.get(1)?]))
}

fn read_u32(b: &[u8], off: usize) -> Option<u32> {
    let raw = b.get(off..off.checked_add(4)?)?;
    Some(u32::from_le_bytes([
        *raw.first()?,
        *raw.get(1)?,
        *raw.get(2)?,
        *raw.get(3)?,
    ]))
}

/// Gelesener Kopf einer PDU.
#[derive(Copy, Clone, Debug)]
struct Header {
    kind: Kind,
    receiver: u32,
    sender: u32,
    sn: u32,
    cs: u32,
    ts: u32,
    cts: u32,
    /// Nächster erwarteter Datenindex der Gegenstelle (Quittung der eigenen Daten).
    ca: u32,
    /// Nächster Datenindex der Gegenstelle (zeigt Lücken auch ohne folgende Daten).
    dt: u32,
}

/// Eine RaSTA-Verbindung (Sicherheits- und Wiederholschicht).
pub struct Connection {
    cfg: Config,
    state: State,
    /// Nächste eigene Sequenznummer.
    sn_t: u32,
    /// Höchste empfangene Sequenznummer der Gegenstelle plus eins.
    sn_r: u32,
    /// Letzter Zeitstempel der Gegenstelle (wird als bestätigter Zeitstempel zurückgesendet).
    ts_r: u32,
    /// Letzter von der Gegenstelle bestätigter eigener Zeitstempel.
    cts_r: u32,
    /// Index der nächsten eigenen Datennachricht.
    data_tx: u32,
    /// Nächster erwarteter Datenindex der Gegenstelle.
    data_rx: u32,
    /// Von der Gegenstelle quittiert: alle eigenen Daten vor diesem Index.
    data_acked: u32,
    /// Zeitpunkt der letzten gültigen Nachricht der Gegenstelle.
    last_rx: u32,
    /// Zeitpunkt der letzten gesendeten Nachricht.
    last_tx: u32,
    /// Unquittierte gesendete Datennachrichten (für Wiederholungen).
    sent: Ring<SEND_BUFFER>,
    /// Zu sendende PDUs.
    out: Ring<QUEUE>,
    /// Zähler verworfener Nachrichten (Diagnose).
    discarded: u32,
    /// Zeitpunkt der letzten Wiederholanforderung.
    retr_requested_at: u32,
    /// Zeitpunkt und quittierter Index der letzten Wiederholung (begrenzt Wiederholungen).
    retransmitted: Option<(u32, u32)>,
}

/// Liegt `a` (zyklisch) vor `b`?
const fn before(a: u32, b: u32) -> bool {
    let d = b.wrapping_sub(a);
    d != 0 && d <= u32::MAX / 2
}

impl Connection {
    /// Neue, geschlossene Verbindung.
    pub const fn new(cfg: Config) -> Self {
        Self {
            cfg,
            state: State::Closed,
            sn_t: cfg.initial_sn,
            sn_r: 0,
            ts_r: 0,
            cts_r: 0,
            data_tx: 0,
            data_rx: 0,
            data_acked: 0,
            last_rx: 0,
            last_tx: 0,
            sent: Ring::new(),
            out: Ring::new(),
            discarded: 0,
            retr_requested_at: 0,
            retransmitted: None,
        }
    }

    /// Zustand.
    pub const fn state(&self) -> State {
        self.state
    }

    /// Anzahl verworfener Nachrichten seit dem Start.
    pub const fn discarded(&self) -> u32 {
        self.discarded
    }

    /// Ist die Verbindung für Daten bereit?
    pub const fn is_up(&self) -> bool {
        matches!(self.state, State::Up | State::RetransmissionRequested)
    }

    /// Baut die Verbindung auf (nur Client).
    pub fn open(&mut self, now: u32) {
        if self.cfg.role != Role::Client || self.state != State::Closed {
            return;
        }
        self.reset();
        self.state = State::Opening;
        self.last_rx = now;
        self.emit(now, Kind::ConnReq, &conn_payload());
    }

    /// Baut die Verbindung ab und meldet es der Gegenstelle.
    pub fn close(&mut self, now: u32) {
        if self.state != State::Closed {
            self.emit(now, Kind::DiscReq, &[]);
        }
        self.state = State::Closed;
    }

    fn reset(&mut self) {
        self.sn_t = self.cfg.initial_sn;
        self.sn_r = 0;
        self.ts_r = 0;
        self.cts_r = 0;
        self.data_tx = 0;
        self.data_rx = 0;
        self.data_acked = 0;
        self.retransmitted = None;
        self.sent.clear();
    }

    /// Sendet Nutzdaten (nur im Zustand `Up`).
    ///
    /// # Errors
    /// [`Fault::UnexpectedMessage`], wenn die Verbindung nicht steht, [`Fault::Overflow`] bei
    /// zu langen Nutzdaten oder vollem Puffer.
    pub fn send(&mut self, now: u32, data: &[u8]) -> Result<(), Fault> {
        if !self.is_up() {
            return Err(Fault::UnexpectedMessage);
        }
        if data.len() > MAX_PAYLOAD || self.sent.len >= SEND_BUFFER {
            return Err(Fault::Overflow);
        }
        let mut payload = [0u8; MAX_PAYLOAD + DATA_HEADER_LEN];
        let len = u16::try_from(data.len()).map_err(|_| Fault::Overflow)?;
        let n = data
            .len()
            .checked_add(DATA_HEADER_LEN)
            .ok_or(Fault::Overflow)?;
        payload
            .get_mut(..2)
            .ok_or(Fault::Overflow)?
            .copy_from_slice(&len.to_le_bytes());
        payload
            .get_mut(2..DATA_HEADER_LEN)
            .ok_or(Fault::Overflow)?
            .copy_from_slice(&self.data_tx.to_le_bytes());
        payload
            .get_mut(DATA_HEADER_LEN..n)
            .ok_or(Fault::Overflow)?
            .copy_from_slice(data);
        let pdu = self
            .emit(now, Kind::Data, payload.get(..n).unwrap_or(&[]))
            .ok_or(Fault::Overflow)?;
        if !self.sent.push(&pdu) {
            return Err(Fault::Overflow);
        }
        self.data_tx = self.data_tx.wrapping_add(1);
        Ok(())
    }

    /// Nächste zu sendende PDU in `buf`; gibt ihre Länge zurück.
    pub fn poll_transmit(&mut self, buf: &mut [u8]) -> Option<usize> {
        let pdu = self.out.pop()?;
        let dst = buf.get_mut(..pdu.len)?;
        dst.copy_from_slice(pdu.as_slice());
        Some(pdu.len)
    }

    /// Zeitüberwachung und Lebenszeichen; regelmäßig aufrufen.
    pub fn tick(&mut self, now: u32) -> Option<Fault> {
        if self.state == State::Closed {
            return None;
        }
        if now.wrapping_sub(self.last_rx) > self.cfg.t_max_ms {
            self.disconnect(now);
            return Some(Fault::Timeout);
        }
        if self.is_up() && now.wrapping_sub(self.last_tx) >= self.cfg.t_h_ms {
            self.emit(now, Kind::Heartbeat, &[]);
        }
        self.repeat_retransmission_request(now);
        None
    }

    fn disconnect(&mut self, now: u32) {
        self.emit(now, Kind::DiscReq, &[]);
        self.state = State::Closed;
    }

    /// Erzeugt eine PDU mit dem nächsten eigenen Zähler und stellt sie in die Warteschlange.
    fn emit(&mut self, now: u32, kind: Kind, payload: &[u8]) -> Option<Pdu> {
        let sn = self.sn_t;
        self.sn_t = self.sn_t.wrapping_add(1);
        let pdu = self.build(kind, sn, now, payload)?;
        self.last_tx = now;
        if !self.out.push(&pdu) {
            self.discarded = self.discarded.saturating_add(1);
        }
        Some(pdu)
    }

    fn build(&self, kind: Kind, sn: u32, now: u32, payload: &[u8]) -> Option<Pdu> {
        let mut pdu = EMPTY_PDU;
        let body = HEADER_LEN.checked_add(payload.len())?;
        let total = body.checked_add(SAFETY_CODE_LEN)?;
        if total > MAX_PDU {
            return None;
        }
        let fields: [(usize, &[u8]); 11] = [
            (0, &u16::try_from(total).ok()?.to_le_bytes()),
            (2, &kind.code().to_le_bytes()),
            (4, &self.cfg.peer.to_le_bytes()),
            (8, &self.cfg.local.to_le_bytes()),
            (12, &sn.to_le_bytes()),
            (16, &self.sn_r.wrapping_sub(1).to_le_bytes()),
            (20, &now.to_le_bytes()),
            (24, &self.ts_r.to_le_bytes()),
            (28, &self.data_rx.to_le_bytes()),
            (32, &self.data_tx.to_le_bytes()),
            (HEADER_LEN, payload),
        ];
        for (off, bytes) in fields {
            pdu.bytes
                .get_mut(off..off.checked_add(bytes.len())?)?
                .copy_from_slice(bytes);
        }
        let code = md4::md4(self.cfg.md4_iv, pdu.bytes.get(..body)?);
        pdu.bytes
            .get_mut(body..total)?
            .copy_from_slice(code.get(..SAFETY_CODE_LEN)?);
        pdu.len = total;
        Some(pdu)
    }

    fn parse(&self, pdu: &[u8]) -> Result<(Header, usize), Fault> {
        let total = usize::from(read_u16(pdu, 0).ok_or(Fault::Malformed)?);
        if total != pdu.len() || total < HEADER_LEN.saturating_add(SAFETY_CODE_LEN) {
            return Err(Fault::Malformed);
        }
        let body = total.checked_sub(SAFETY_CODE_LEN).ok_or(Fault::Malformed)?;
        let code = md4::md4(self.cfg.md4_iv, pdu.get(..body).ok_or(Fault::Malformed)?);
        if pdu.get(body..total) != code.get(..SAFETY_CODE_LEN) {
            return Err(Fault::SafetyCode);
        }
        let r = |off| read_u32(pdu, off).ok_or(Fault::Malformed);
        let header = Header {
            kind: Kind::from_code(read_u16(pdu, 2).ok_or(Fault::Malformed)?)
                .ok_or(Fault::Malformed)?,
            receiver: r(4)?,
            sender: r(8)?,
            sn: r(12)?,
            cs: r(16)?,
            ts: r(20)?,
            cts: r(24)?,
            ca: r(28)?,
            dt: r(32)?,
        };
        if header.receiver != self.cfg.local || header.sender != self.cfg.peer {
            return Err(Fault::Address);
        }
        Ok((header, body))
    }

    /// Verarbeitet eine empfangene PDU. Nutzdaten gehen in Reihenfolge an `deliver`.
    pub fn receive(&mut self, now: u32, pdu: &[u8], deliver: &mut dyn FnMut(&[u8])) -> Received {
        let (h, body) = match self.parse(pdu) {
            Ok(v) => v,
            Err(fault) => {
                self.discarded = self.discarded.saturating_add(1);
                return Received::Discarded(fault);
            }
        };
        if h.kind == Kind::DiscReq {
            self.state = State::Closed;
            return Received::Disconnected(Fault::PeerDisconnected);
        }
        match (self.state, h.kind) {
            (State::Closed, Kind::ConnReq) if self.cfg.role == Role::Server => {
                self.reset();
                self.sn_r = h.sn.wrapping_add(1);
                self.ts_r = h.ts;
                self.last_rx = now;
                self.state = State::Opening;
                self.emit(now, Kind::ConnResp, &conn_payload());
                Received::Nothing
            }
            (State::Opening, Kind::ConnResp) if self.cfg.role == Role::Client => {
                if h.cs != self.sn_t.wrapping_sub(1) {
                    self.disconnect(now);
                    return Received::Disconnected(Fault::Confirmation);
                }
                self.sn_r = h.sn.wrapping_add(1);
                self.ts_r = h.ts;
                self.cts_r = h.cts;
                self.last_rx = now;
                self.state = State::Up;
                self.emit(now, Kind::Heartbeat, &[]);
                Received::Nothing
            }
            (State::Closed, _) => {
                self.discarded = self.discarded.saturating_add(1);
                Received::Discarded(Fault::UnexpectedMessage)
            }
            // Der Client wartet nach der Anforderung nur auf die Antwort.
            (State::Opening, _) if self.cfg.role == Role::Client => {
                self.discarded = self.discarded.saturating_add(1);
                Received::Discarded(Fault::UnexpectedMessage)
            }
            _ => self.receive_up(now, h, pdu, body, deliver),
        }
    }

    fn receive_up(
        &mut self,
        now: u32,
        h: Header,
        pdu: &[u8],
        body: usize,
        deliver: &mut dyn FnMut(&[u8]),
    ) -> Received {
        // Zeitstempel: Die Gegenstelle bestätigt einen eigenen Zeitstempel; er darf nicht älter
        // als t_max sein (Verzögerung) und keinen künftigen Zeitpunkt bestätigen.
        if now.wrapping_sub(h.cts) > self.cfg.t_max_ms {
            self.disconnect(now);
            return Received::Disconnected(Fault::Timeliness);
        }
        // Quittungen: nur bereits gesendete Sequenznummern und Datenindizes.
        if before(self.sn_t.wrapping_sub(1), h.cs) || before(self.data_tx, h.ca) {
            self.disconnect(now);
            return Received::Disconnected(Fault::Confirmation);
        }
        // Authentisch und rechtzeitig: Die Gegenstelle lebt. Ihr Zeitstempel wird bestätigt
        // (nie rückwärts), ihre Quittung übernommen.
        self.last_rx = now;
        if !before(h.ts, self.ts_r) {
            self.ts_r = h.ts;
        }
        if !before(h.cts, self.cts_r) {
            self.cts_r = h.cts;
        }
        if !before(h.sn, self.sn_r) {
            self.sn_r = h.sn.wrapping_add(1);
        }
        self.acknowledge(h.ca);
        if self.state == State::Opening {
            self.state = State::Up;
        }
        match h.kind {
            Kind::Data | Kind::RetrData => self.receive_data(now, pdu, body, deliver),
            // Die Gegenstelle hat mehr gesendet, als hier ankam: Lücke ohne folgende Daten.
            _ if before(self.data_rx, h.dt) && h.kind != Kind::RetrReq => {
                self.gap(now);
                Received::Nothing
            }
            Kind::RetrReq => {
                if self.retransmit(now, h.ca) {
                    if before(self.data_rx, h.dt) {
                        self.gap(now);
                    }
                    Received::Nothing
                } else {
                    self.disconnect(now);
                    Received::Disconnected(Fault::RetransmissionFailed)
                }
            }
            _ => Received::Nothing,
        }
    }

    /// Datennachricht: nur lückenlos in Reihenfolge liefern.
    fn receive_data(
        &mut self,
        now: u32,
        pdu: &[u8],
        body: usize,
        deliver: &mut dyn FnMut(&[u8]),
    ) -> Received {
        let (Some(len), Some(index)) = (
            read_u16(pdu, HEADER_LEN).map(usize::from),
            read_u32(pdu, HEADER_LEN.saturating_add(2)),
        ) else {
            return Received::Discarded(Fault::Malformed);
        };
        if index != self.data_rx {
            self.discarded = self.discarded.saturating_add(1);
            if before(self.data_rx, index) {
                self.gap(now);
            }
            return Received::Discarded(Fault::Duplicate);
        }
        let start = HEADER_LEN.saturating_add(DATA_HEADER_LEN);
        match start
            .checked_add(len)
            .filter(|end| *end <= body)
            .and_then(|end| pdu.get(start..end))
        {
            Some(data) => {
                self.data_rx = self.data_rx.wrapping_add(1);
                self.state = State::Up;
                deliver(data);
                Received::Delivered
            }
            None => Received::Discarded(Fault::Malformed),
        }
    }

    /// Lücke erkannt: Wiederholung anfordern (danach alle `t_h / 3`, solange sie besteht).
    fn gap(&mut self, now: u32) {
        if self.state == State::RetransmissionRequested {
            self.repeat_retransmission_request(now);
        } else {
            self.state = State::RetransmissionRequested;
            self.retr_requested_at = now;
            self.emit(now, Kind::RetrReq, &[]);
        }
    }

    /// Wiederholt eine Wiederholanforderung, die verloren gegangen sein kann (alle `t_h / 3`,
    /// damit eine verlorene Anforderung die Verbindung nicht bis an `t_max` bringt).
    fn repeat_retransmission_request(&mut self, now: u32) {
        if self.state == State::RetransmissionRequested
            && now.wrapping_sub(self.retr_requested_at) >= self.cfg.t_h_ms / 3
        {
            self.retr_requested_at = now;
            self.emit(now, Kind::RetrReq, &[]);
        }
    }

    /// Datenindex einer gespeicherten Datennachricht.
    fn index_of(pdu: &Pdu) -> u32 {
        read_u32(pdu.as_slice(), HEADER_LEN.saturating_add(2)).unwrap_or(0)
    }

    /// Entfernt quittierte Nachrichten (Index vor `ca`) aus dem Wiederholpuffer.
    fn acknowledge(&mut self, ca: u32) {
        if before(self.data_acked, ca) {
            self.data_acked = ca;
        }
        while let Some(first) = self.sent.get(0) {
            if !before(Self::index_of(first), self.data_acked) {
                break;
            }
            self.sent.pop();
        }
    }

    /// Beantwortet eine Wiederholanforderung: alle unquittierten Datennachrichten mit ihrem
    /// ursprünglichen Index und neuen Sequenznummern. `false`, wenn die Gegenstelle eine
    /// Nachricht vermisst, die nicht mehr im Puffer steht (kann nur bei Fehlern der
    /// Gegenstelle auftreten: quittiert wird erst nach Empfang).
    fn retransmit(&mut self, now: u32, ca: u32) -> bool {
        let first = self.sent.get(0).map_or(self.data_tx, Self::index_of);
        if before(ca, first) {
            return false;
        }
        // Auf wiederholte Anforderungen nicht öfter als alle t_h / 3 antworten.
        if let Some((at, acked)) = self.retransmitted
            && acked == ca
            && now.wrapping_sub(at) < self.cfg.t_h_ms / 3
        {
            return true;
        }
        self.retransmitted = Some((now, ca));
        for i in 0..self.sent.len {
            let Some(p) = self.sent.get(i).copied() else {
                break;
            };
            let payload = p
                .as_slice()
                .get(HEADER_LEN..p.len.saturating_sub(SAFETY_CODE_LEN))
                .unwrap_or(&[]);
            let mut copy = [0u8; MAX_PAYLOAD + DATA_HEADER_LEN];
            let len = payload.len().min(copy.len());
            if let (Some(dst), Some(src)) = (copy.get_mut(..len), payload.get(..len)) {
                dst.copy_from_slice(src);
            }
            self.emit(now, Kind::RetrData, copy.get(..len).unwrap_or(&[]));
        }
        true
    }
}

/// Nutzdaten von Verbindungsanforderung und -antwort: Protokollversion "0303", `N_sendmax`,
/// Reserve.
fn conn_payload() -> [u8; 14] {
    let mut p = [0u8; 14];
    if let Some(dst) = p.get_mut(..4) {
        dst.copy_from_slice(b"0303");
    }
    let n = u16::try_from(SEND_BUFFER).unwrap_or(u16::MAX).to_le_bytes();
    if let Some(dst) = p.get_mut(4..6) {
        dst.copy_from_slice(&n);
    }
    p
}
