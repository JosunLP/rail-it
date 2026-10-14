//! Nachbarstellwerke über EULYNX SCI-ILS und RaSTA (Konzept, Abschnitt 10.4 und 12).
//!
//! Zwei Stellbereiche, die aneinandergrenzen, tauschen ihre Grenzmeldungen
//! ([`BorderMessage`]) über **eine** RaSTA-Verbindung je Paar von Stellwerken aus. Der Bereich
//! mit dem kleineren Index baut sie auf (Client), der andere nimmt sie an (Server). Die Strecke
//! dazwischen lässt sich in beide Richtungen stören wie die Strecke zu einer Objektsteuerung
//! ([`LinkFaults`]).
//!
//! ```text
//!  Kern A ──Grenzmeldung──▶ SCI-ILS ──▶ RaSTA ──Strecke──▶ RaSTA ──▶ SCI-ILS ──▶ Kern B
//!         ◀────────────────         ◀── RaSTA ◀─Strecke─── RaSTA ◀──          ◀──
//! ```
//!
//! **Telegramm** (vereinfacht nach Baseline 4, wie [`crate::sci`]):
//!
//! ```text
//! Protokoll 0x60 · Nachrichtentyp 0x0001 (big-endian) · Absender 20 Byte · Empfänger 20 Byte
//! · Grenze u8 · Merkmale u8 · laufende Nummer u32 (big-endian)
//! ```
//!
//! Absender und Empfänger sind die Namen der Stellbereiche. Die **Grenze** ist die Nummer
//! der Grenze innerhalb des Paars: die Grenzen zwischen beiden Bereichen in der Reihenfolge
//! des Bereichs mit dem kleineren Index. So brauchen beide Seiten keine gemeinsamen Namen für
//! ihre Grenzen. **Merkmale:** Bit 0 hält die Erlaubnis, Bit 1 Angebot, Bit 2
//! Annäherungsabschnitt frei, Bit 3 Anforderung, Bit 4 ausschließlich verschlossen; die
//! übrigen Bits müssen 0 sein.
//!
//! Zugestellt wird nur, was über eine stehende Verbindung **neu** eingetroffen ist: Bleiben
//! die Meldungen eines Nachbarn aus – weil die Verbindung gestört ist oder sein Kern nicht
//! mehr rechnet, während die RaSTA-Lebenszeichen weiterlaufen –, bemerkt der Kern das am
//! Alter der letzten Meldung und behandelt die Grenze als gestört (REQ-IXL-017, REQ-COM-008).
//! RaSTA liefert jede Meldung höchstens einmal und in der gesendeten Reihenfolge; die
//! laufende Nummer im Erlaubnisprotokoll bleibt dennoch die eigentliche Sicherung gegen alte
//! Angebote.

use std::fmt;

use railit_kernel::BorderMessage;
use railit_rasta::{Config, Connection, Received, Role, State};
use railit_types::{BorderId, Tick};
use serde::Serialize;

use crate::sci::ID_LEN;
use crate::{Link, LinkFaults, flush, rasta_now};

/// Teilschnittstelle SCI-ILS.
pub const PROTOCOL: u8 = 0x60;
/// Nachrichtentyp „Grenzzustand“.
pub const BORDER_STATUS: u16 = 0x0001;
/// Länge eines Telegramms.
pub const TELEGRAM_LEN: usize = 3 + 2 * ID_LEN + 6;
/// RaSTA-Kennungen der Stellwerke: diese Basis plus Index des Bereichs.
const RASTA_BASE: u32 = 0x0C00_0000;

const HOLDING: u8 = 1 << 0;
const OFFER: u8 = 1 << 1;
const APPROACH_CLEAR: u8 = 1 << 2;
const REQUEST: u8 = 1 << 3;
const EXCLUSIVE: u8 = 1 << 4;
const ALL_FLAGS: u8 = HOLDING | OFFER | APPROACH_CLEAR | REQUEST | EXCLUSIVE;

/// Telegramm zwischen zwei Stellwerken.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IlsTelegram {
    /// Absender (Name des Stellbereichs).
    pub sender: String,
    /// Empfänger (Name des Stellbereichs).
    pub receiver: String,
    /// Nummer der Grenze innerhalb des Paars.
    pub border: u8,
    /// Grenzmeldung.
    pub message: BorderMessage,
}

/// Telegramm nicht lesbar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IlsError {
    /// Länge falsch.
    Length,
    /// Unbekannte Teilschnittstelle, Nachricht oder gesetzte Reservebits.
    Unknown,
    /// Kennung zu lang oder nicht UTF-8.
    Identifier,
}

impl fmt::Display for IlsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Length => "Telegrammlänge falsch",
            Self::Unknown => "unbekannte Nachricht",
            Self::Identifier => "Kennung ungültig",
        })
    }
}

impl std::error::Error for IlsError {}

/// Ob `name` als Kennung in SCI-Telegrammen taugt (höchstens 20 Byte UTF-8 ohne `_`).
pub fn valid_identifier(name: &str) -> bool {
    !name.is_empty() && name.len() <= ID_LEN && !name.contains('_')
}

fn put_id(out: &mut Vec<u8>, id: &str) -> Result<(), IlsError> {
    if !valid_identifier(id) {
        return Err(IlsError::Identifier);
    }
    out.extend_from_slice(id.as_bytes());
    out.resize(out.len() + (ID_LEN - id.len()), b'_');
    Ok(())
}

fn get_id(raw: &[u8]) -> Result<String, IlsError> {
    let text = std::str::from_utf8(raw).map_err(|_| IlsError::Identifier)?;
    let id = text.trim_end_matches('_');
    if !valid_identifier(id) {
        return Err(IlsError::Identifier);
    }
    Ok(id.to_string())
}

const fn flags_of(m: &BorderMessage) -> u8 {
    let mut f = 0;
    if m.holding {
        f |= HOLDING;
    }
    if m.offer {
        f |= OFFER;
    }
    if m.approach_clear {
        f |= APPROACH_CLEAR;
    }
    if m.request {
        f |= REQUEST;
    }
    if m.exclusive {
        f |= EXCLUSIVE;
    }
    f
}

impl IlsTelegram {
    /// Kodiert das Telegramm.
    ///
    /// # Errors
    /// [`IlsError::Identifier`] bei zu langen oder unzulässigen Kennungen.
    pub fn encode(&self) -> Result<Vec<u8>, IlsError> {
        let mut out = Vec::with_capacity(TELEGRAM_LEN);
        out.push(PROTOCOL);
        out.extend_from_slice(&BORDER_STATUS.to_be_bytes());
        put_id(&mut out, &self.sender)?;
        put_id(&mut out, &self.receiver)?;
        out.push(self.border);
        out.push(flags_of(&self.message));
        out.extend_from_slice(&self.message.seq.to_be_bytes());
        Ok(out)
    }

    /// Liest ein Telegramm.
    ///
    /// # Errors
    /// [`IlsError`], wenn Länge, Teilschnittstelle, Nachricht, Merkmale oder Kennungen
    /// ungültig sind.
    pub fn decode(raw: &[u8]) -> Result<Self, IlsError> {
        if raw.len() != TELEGRAM_LEN {
            return Err(IlsError::Length);
        }
        let byte = |i: usize| raw.get(i).copied().ok_or(IlsError::Length);
        if byte(0)? != PROTOCOL || u16::from_be_bytes([byte(1)?, byte(2)?]) != BORDER_STATUS {
            return Err(IlsError::Unknown);
        }
        let sender = get_id(raw.get(3..3 + ID_LEN).ok_or(IlsError::Length)?)?;
        let receiver = get_id(
            raw.get(3 + ID_LEN..3 + 2 * ID_LEN)
                .ok_or(IlsError::Length)?,
        )?;
        let at = 3 + 2 * ID_LEN;
        let border = byte(at)?;
        let flags = byte(at + 1)?;
        if flags & !ALL_FLAGS != 0 {
            return Err(IlsError::Unknown);
        }
        let seq = u32::from_be_bytes([byte(at + 2)?, byte(at + 3)?, byte(at + 4)?, byte(at + 5)?]);
        Ok(Self {
            sender,
            receiver,
            border,
            message: BorderMessage {
                holding: flags & HOLDING != 0,
                seq,
                offer: flags & OFFER != 0,
                approach_clear: flags & APPROACH_CLEAR != 0,
                request: flags & REQUEST != 0,
                exclusive: flags & EXCLUSIVE != 0,
            },
        })
    }
}

/// Ende einer Grenze: Bereich (Index) und Grenze aus dessen Sicht.
pub type Endpoint = (usize, BorderId);

/// Zustand einer Verbindung zwischen zwei Stellwerken.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct IlsStatus {
    /// Name der Verbindung (`A-B`, Bereich mit dem kleineren Index zuerst).
    pub link: String,
    /// Die beiden Stellbereiche.
    pub areas: [String; 2],
    /// Anzahl der Grenzen über diese Verbindung.
    pub borders: usize,
    /// Verbindung steht (auf beiden Seiten).
    pub up: bool,
    /// Von beiden Seiten verworfene PDUs (Verfälschung, Wiederholung, …).
    pub discarded: u32,
    /// Nicht lesbare oder nicht zuzuordnende Telegramme.
    pub bad_telegrams: u32,
    /// Zugestellte Grenzmeldungen.
    pub delivered: u64,
    /// Eingestellte Störungen.
    pub faults: LinkFaults,
    /// Bereich, der in einem anderen Prozess rechnet (über UDP angebunden).
    pub remote: Option<String>,
}

/// Stellbereich an einer SCI-ILS-Verbindung.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IlsArea {
    /// Name (SCI-Kennung).
    pub name: String,
    /// Index im Gesamtnetz: bestimmt RaSTA-Kennung und Rolle (der kleinere baut auf).
    pub global: usize,
    /// Index in der Runtime, wenn der Bereich in diesem Prozess rechnet.
    pub local: Option<usize>,
}

/// Paar benachbarter Stellbereiche mit allen Grenzen dazwischen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IlsPair {
    /// Die beiden Bereiche, der mit dem kleineren Index im Gesamtnetz zuerst.
    pub areas: [IlsArea; 2],
    /// Grenzen: Grenze aus Sicht des ersten und des zweiten Bereichs. Die Reihenfolge ist die
    /// des ersten Bereichs; der Index ist die Nummer der Grenze im Telegramm.
    pub borders: Vec<(BorderId, BorderId)>,
}

/// Eine Seite der Verbindung.
struct Side {
    area: IlsArea,
    conn: Connection,
    /// Strecke von dieser Seite zur anderen.
    out: Link,
}

impl Side {
    const fn is_local(&self) -> bool {
        self.area.local.is_some()
    }
}

/// Verbindung zwischen zwei Stellwerken mit allen Grenzen dazwischen.
struct IlsLink {
    name: String,
    /// Client (kleinerer Index) und Server. Rechnet eine Seite in einem anderen Prozess,
    /// bleibt ihre Verbindung ungenutzt; ihre PDUs kommen über [`IlsNet::receive_pdu`].
    /// Je Seite eigens angelegt: Die RaSTA-Puffer sind groß.
    sides: [Box<Side>; 2],
    /// Grenzen: Grenze auf Seite 0 und auf Seite 1, Index = Nummer im Telegramm.
    borders: Vec<(BorderId, BorderId)>,
    /// Aus dem Netz eingegangene PDUs für die lokale Seite.
    inbound: Vec<Vec<u8>>,
    bad_telegrams: u32,
    delivered: u64,
}

/// SCI-ILS zwischen Stellbereichen: Verbindungen zwischen zwei Bereichen dieses Prozesses
/// laufen über eine störbare Strecke im Speicher, Verbindungen zu Bereichen anderer Prozesse
/// über das Netz ([`Self::take_outgoing`], [`Self::receive_pdu`], [`UdpIls`]).
pub struct IlsNet {
    links: Vec<IlsLink>,
    /// PDUs an Bereiche anderer Prozesse: Name des Empfängerbereichs und PDU.
    outgoing: Vec<(String, Vec<u8>)>,
    now: u64,
}

/// RaSTA-Kennung des Stellwerks eines Bereichs.
pub fn rasta_id(global: usize) -> u32 {
    RASTA_BASE.wrapping_add(u32::try_from(global).unwrap_or(u32::MAX))
}

impl IlsNet {
    /// Verbindungen für ein Netz, dessen Bereiche alle in diesem Prozess rechnen: `areas`
    /// sind die Namen der Stellbereiche, `peers` je Bereich und Grenze das Gegenstück (siehe
    /// Runtime).
    ///
    /// # Errors
    /// Beschreibung, wenn ein Bereichsname keine gültige Kennung ist, Grenzen sich nicht
    /// paaren oder zwischen zwei Bereichen mehr als 256 Grenzen liegen.
    pub fn new(areas: &[String], peers: &[Vec<Option<Endpoint>>]) -> Result<Self, String> {
        let area = |i: usize| -> Result<IlsArea, String> {
            Ok(IlsArea {
                name: areas.get(i).ok_or("unbekannter Bereich")?.clone(),
                global: i,
                local: Some(i),
            })
        };
        let mut pairs: Vec<IlsPair> = Vec::new();
        for (a, borders) in peers.iter().enumerate() {
            for (i, peer) in borders.iter().enumerate() {
                let here = BorderId::from_index(i).ok_or("zu viele Grenzen")?;
                let Some((b, j)) = *peer else {
                    return Err(format!("Grenze {i} von Bereich {a} hat keinen Gegenpart"));
                };
                let back = peers
                    .get(b)
                    .and_then(|p| j.index().and_then(|k| p.get(k)))
                    .copied()
                    .flatten();
                if b == a || back != Some((a, here)) {
                    return Err(format!("Grenze {i} von Bereich {a} ist nicht gepaart"));
                }
                // Jede Grenze nur einmal: von der Seite mit dem kleineren Index.
                if b < a {
                    continue;
                }
                match pairs
                    .iter_mut()
                    .find(|p| p.areas[0].global == a && p.areas[1].global == b)
                {
                    Some(p) => p.borders.push((here, j)),
                    None => pairs.push(IlsPair {
                        areas: [area(a)?, area(b)?],
                        borders: vec![(here, j)],
                    }),
                }
            }
        }
        Self::from_pairs(pairs)
    }

    /// Verbindungen für die Paare `pairs`; Paare ohne Bereich in diesem Prozess entfallen.
    ///
    /// # Errors
    /// Beschreibung, wenn ein Bereichsname keine gültige Kennung ist, die Bereiche eines
    /// Paars nicht nach ihrem Index geordnet sind oder zwischen zwei Bereichen mehr als 256
    /// Grenzen liegen.
    pub fn from_pairs(pairs: Vec<IlsPair>) -> Result<Self, String> {
        let mut links = Vec::new();
        for pair in pairs {
            for a in &pair.areas {
                if !valid_identifier(&a.name) {
                    return Err(format!(
                        "Stellbereich „{}“: keine gültige SCI-Kennung (höchstens 20 Byte, ohne „_“)",
                        a.name
                    ));
                }
            }
            let [a, b] = pair.areas;
            if a.global >= b.global {
                return Err(format!(
                    "Verbindung {}-{}: Bereiche nicht geordnet",
                    a.name, b.name
                ));
            }
            if a.local.is_none() && b.local.is_none() {
                continue;
            }
            if pair.borders.is_empty() || pair.borders.len() > usize::from(u8::MAX) + 1 {
                return Err(format!(
                    "Verbindung {}-{}: {} Grenzen",
                    a.name,
                    b.name,
                    pair.borders.len()
                ));
            }
            let mut link = IlsLink::new(a, b);
            link.borders = pair.borders;
            links.push(link);
        }
        Ok(Self {
            links,
            outgoing: Vec::new(),
            now: 0,
        })
    }

    /// Schickt eine Grenzmeldung des Bereichs `from` (Bereich dieses Prozesses und Grenze aus
    /// seiner Sicht) an den Nachbarn. Steht die Verbindung nicht, geht sie verloren; der
    /// Nachbar bemerkt es am Alter seiner letzten Meldung.
    pub fn send_from(&mut self, from: Endpoint, message: BorderMessage) {
        let t = rasta_now(self.now);
        for link in &mut self.links {
            let Some((side, number)) = link.locate(from) else {
                continue;
            };
            let to = 1 - side;
            let tg = IlsTelegram {
                sender: link.sides[side].area.name.clone(),
                receiver: link.sides[to].area.name.clone(),
                border: number,
                message,
            };
            if let Ok(raw) = tg.encode() {
                let _ = link.sides[side].conn.send(t, &raw);
            }
            return;
        }
    }

    /// Gibt die gesendeten PDUs auf die Strecken (nach allen [`Self::send_from`] eines Zyklus).
    pub fn flush(&mut self) {
        let now = self.now;
        for link in &mut self.links {
            for side in link.sides.iter_mut().filter(|s| s.is_local()) {
                flush(&mut side.conn, &mut side.out, now);
            }
            link.forward(now, &mut self.outgoing);
        }
    }

    /// Nimmt eine PDU aus dem Netz an; zugeordnet wird sie über die RaSTA-Kennungen von
    /// Empfänger und Absender. Fremde PDUs werden verworfen.
    pub fn receive_pdu(&mut self, pdu: &[u8]) {
        let Some((receiver, sender)) = railit_rasta::addresses(pdu) else {
            return;
        };
        for link in &mut self.links {
            let [a, b] = &link.sides;
            let fits = |near: &Side, far: &Side| {
                near.is_local()
                    && !far.is_local()
                    && rasta_id(near.area.global) == receiver
                    && rasta_id(far.area.global) == sender
            };
            if fits(a, b) || fits(b, a) {
                link.inbound.push(pdu.to_vec());
                return;
            }
        }
    }

    /// PDUs an Bereiche anderer Prozesse seit dem letzten Aufruf: Name des Empfängerbereichs
    /// und PDU.
    pub fn take_outgoing(&mut self) -> Vec<(String, Vec<u8>)> {
        std::mem::take(&mut self.outgoing)
    }

    /// Schreibt die Verbindungen bis `now` fort: Auf- und Abbau, Lebenszeichen, Empfang.
    /// Liefert die neu eingetroffenen Grenzmeldungen mit ihrem Empfänger.
    pub fn advance(&mut self, now: Tick) -> Vec<(Endpoint, BorderMessage)> {
        self.now = now.as_millis();
        let t = rasta_now(self.now);
        let now = self.now;
        let mut delivered = Vec::new();
        for link in &mut self.links {
            if link.sides[0].is_local() && link.sides[0].conn.state() == State::Closed {
                link.sides[0].conn.open(t);
            }
            for to in 0..2 {
                if !link.sides[to].is_local() {
                    continue;
                }
                let from = 1 - to;
                let pdus = if link.sides[from].is_local() {
                    link.sides[from].out.due(now)
                } else {
                    std::mem::take(&mut link.inbound)
                };
                for pdu in pdus {
                    let mut telegrams = Vec::new();
                    let r = link.sides[to]
                        .conn
                        .receive(t, &pdu, &mut |data| telegrams.push(data.to_vec()));
                    if matches!(r, Received::Disconnected(_)) {
                        continue;
                    }
                    for raw in telegrams {
                        match link.accept(to, &raw) {
                            Some(m) => {
                                link.delivered = link.delivered.saturating_add(1);
                                delivered.push(m);
                            }
                            None => link.bad_telegrams = link.bad_telegrams.saturating_add(1),
                        }
                    }
                }
            }
            for side in link.sides.iter_mut().filter(|s| s.is_local()) {
                side.conn.tick(t);
                flush(&mut side.conn, &mut side.out, now);
            }
            link.forward(now, &mut self.outgoing);
        }
        delivered
    }

    /// Störungen der Verbindung `link` setzen (beide Richtungen; zu einem anderen Prozess nur
    /// die abgehende).
    ///
    /// # Errors
    /// Beschreibung, wenn es die Verbindung nicht gibt.
    pub fn set_faults(&mut self, link: &str, faults: LinkFaults) -> Result<(), String> {
        let l = self
            .links
            .iter_mut()
            .find(|l| l.name == link)
            .ok_or_else(|| format!("SCI-ILS-Verbindung „{link}“ nicht gefunden"))?;
        for side in &mut l.sides {
            side.out.faults = faults;
        }
        Ok(())
    }

    /// Zustand aller Verbindungen (aus Sicht der Bereiche dieses Prozesses).
    pub fn links(&self) -> Vec<IlsStatus> {
        self.links
            .iter()
            .map(|l| {
                let local = || l.sides.iter().filter(|s| s.is_local());
                IlsStatus {
                    link: l.name.clone(),
                    areas: [l.sides[0].area.name.clone(), l.sides[1].area.name.clone()],
                    borders: l.borders.len(),
                    up: local().all(|s| s.conn.is_up()),
                    discarded: local()
                        .map(|s| s.conn.discarded())
                        .fold(0, u32::saturating_add),
                    bad_telegrams: l.bad_telegrams,
                    delivered: l.delivered,
                    faults: local().next().map(|s| s.out.faults).unwrap_or_default(),
                    remote: l
                        .sides
                        .iter()
                        .find(|s| !s.is_local())
                        .map(|s| s.area.name.clone()),
                }
            })
            .collect()
    }
}

impl IlsLink {
    fn new(a: IlsArea, b: IlsArea) -> Self {
        let (ia, ib) = (rasta_id(a.global), rasta_id(b.global));
        let mut server = Config::new(ib, ia, Role::Server);
        server.initial_sn = 0x2000;
        let side = |area: IlsArea, cfg, seed: u64| {
            Box::new(Side {
                area,
                conn: Connection::new(cfg),
                out: Link::new(seed),
            })
        };
        Self {
            name: format!("{}-{}", a.name, b.name),
            sides: [
                side(
                    a,
                    Config::new(ia, ib, Role::Client),
                    u64::from(ia).wrapping_mul(29),
                ),
                side(b, server, u64::from(ib).wrapping_mul(13)),
            ],
            borders: Vec::new(),
            inbound: Vec::new(),
            bad_telegrams: 0,
            delivered: 0,
        }
    }

    /// Seite des lokalen Absenders `from` und Nummer seiner Grenze.
    fn locate(&self, from: Endpoint) -> Option<(usize, u8)> {
        let (area, border) = from;
        let side = self.sides.iter().position(|s| s.area.local == Some(area))?;
        let k = self.borders.iter().position(|(b0, b1)| {
            if side == 0 {
                *b0 == border
            } else {
                *b1 == border
            }
        })?;
        Some((side, u8::try_from(k).ok()?))
    }

    /// Fällige PDUs der lokalen Seite an einen Bereich in einem anderen Prozess.
    fn forward(&mut self, now: u64, outgoing: &mut Vec<(String, Vec<u8>)>) {
        for i in 0..2 {
            let far = 1 - i;
            if self.sides[i].is_local() && !self.sides[far].is_local() {
                let to = self.sides[far].area.name.clone();
                outgoing.extend(
                    self.sides[i]
                        .out
                        .due(now)
                        .into_iter()
                        .map(|p| (to.clone(), p)),
                );
            }
        }
    }

    /// Empfang auf Seite `to`: Telegramm prüfen und dem Empfänger zuordnen.
    fn accept(&self, to: usize, raw: &[u8]) -> Option<(Endpoint, BorderMessage)> {
        let tg = IlsTelegram::decode(raw).ok()?;
        let from = 1 - to;
        if tg.sender != self.sides[from].area.name || tg.receiver != self.sides[to].area.name {
            return None;
        }
        let (b0, b1) = *self.borders.get(usize::from(tg.border))?;
        let border = if to == 0 { b0 } else { b1 };
        Some(((self.sides[to].area.local?, border), tg.message))
    }
}

/// [`IlsNet`] mit Übertragung über UDP: je Bereich eines anderen Prozesses eine Adresse.
pub struct UdpIls {
    socket: std::net::UdpSocket,
    peers: Vec<(String, std::net::SocketAddr)>,
    /// Datagramme, die keinem Bereich zugeordnet werden konnten oder zu lang waren.
    rejected: u64,
}

impl UdpIls {
    /// Lauscht auf `listen`; `peers` nennt je Bereich eines anderen Prozesses seine Adresse.
    ///
    /// # Errors
    /// Socketfehler.
    pub fn bind(
        listen: std::net::SocketAddr,
        peers: Vec<(String, std::net::SocketAddr)>,
    ) -> Result<Self, String> {
        let socket =
            std::net::UdpSocket::bind(listen).map_err(|e| format!("SCI-ILS {listen}: {e}"))?;
        socket
            .set_nonblocking(true)
            .map_err(|e| format!("SCI-ILS: {e}"))?;
        Ok(Self {
            socket,
            peers,
            rejected: 0,
        })
    }

    /// Adresse, auf der gelauscht wird.
    ///
    /// # Errors
    /// Socketfehler.
    pub fn local_addr(&self) -> Result<std::net::SocketAddr, String> {
        self.socket.local_addr().map_err(|e| e.to_string())
    }

    /// Eingegangene Datagramme an `net` übergeben. Angenommen werden nur Datagramme von den
    /// Adressen der Nachbarn.
    pub fn receive(&mut self, net: &mut IlsNet) {
        let mut buf = [0u8; railit_rasta::MAX_PDU + 1];
        while let Ok((n, from)) = self.socket.recv_from(&mut buf) {
            let known = self.peers.iter().any(|(_, a)| *a == from);
            match buf.get(..n) {
                Some(pdu) if known && n <= railit_rasta::MAX_PDU => net.receive_pdu(pdu),
                _ => self.rejected = self.rejected.saturating_add(1),
            }
        }
    }

    /// Abgehende PDUs von `net` versenden.
    pub fn send(&mut self, net: &mut IlsNet) {
        for (area, pdu) in net.take_outgoing() {
            if let Some((_, addr)) = self.peers.iter().find(|(n, _)| *n == area) {
                // Verlust ist für RaSTA ein gewöhnlicher Fall.
                let _ = self.socket.send_to(&pdu, addr);
            }
        }
    }

    /// Verworfene Datagramme.
    pub const fn rejected(&self) -> u64 {
        self.rejected
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(seq: u32) -> BorderMessage {
        BorderMessage {
            holding: seq.is_multiple_of(2),
            seq,
            offer: seq.is_multiple_of(3),
            approach_clear: true,
            request: false,
            exclusive: seq.is_multiple_of(5),
        }
    }

    #[test]
    fn telegram_roundtrip_and_rejects() {
        for seq in [0, 1, 2, 3, 15, 30, u32::MAX] {
            let t = IlsTelegram {
                sender: "Adorf".into(),
                receiver: "Strecke".into(),
                border: 3,
                message: msg(seq),
            };
            let raw = t.encode().unwrap();
            assert_eq!(raw.len(), TELEGRAM_LEN);
            assert_eq!(IlsTelegram::decode(&raw).unwrap(), t);
        }
        let mut raw = IlsTelegram {
            sender: "A".into(),
            receiver: "B".into(),
            border: 0,
            message: msg(1),
        }
        .encode()
        .unwrap();
        assert_eq!(IlsTelegram::decode(&raw[..10]), Err(IlsError::Length));
        let flags = 3 + 2 * ID_LEN + 1;
        raw[flags] |= 0x80;
        assert_eq!(IlsTelegram::decode(&raw), Err(IlsError::Unknown));
        raw[flags] &= 0x7F;
        raw[0] = 0x20;
        assert_eq!(IlsTelegram::decode(&raw), Err(IlsError::Unknown));
        let bad = IlsTelegram {
            sender: "zu_lang".into(),
            receiver: "B".into(),
            border: 0,
            message: msg(1),
        };
        assert_eq!(bad.encode(), Err(IlsError::Identifier));
    }

    fn b(i: usize) -> BorderId {
        BorderId::from_index(i).unwrap()
    }

    /// Drei Bereiche in einer Reihe: 0 –(Grenze 0/0)– 1 –(Grenze 1/0)– 2.
    fn line() -> IlsNet {
        let areas = ["West".to_string(), "Mitte".into(), "Ost".into()];
        let peers = vec![
            vec![Some((1, b(0)))],
            vec![Some((0, b(0))), Some((2, b(0)))],
            vec![Some((1, b(1)))],
        ];
        IlsNet::new(&areas, &peers).unwrap()
    }

    fn run(net: &mut IlsNet, from_ms: u64, to_ms: u64) -> Vec<(Endpoint, BorderMessage)> {
        let mut got = Vec::new();
        let mut t = from_ms;
        while t <= to_ms {
            got.extend(net.advance(Tick::from_millis(t)));
            // West → Mitte, Mitte → West, Mitte → Ost (Empfänger und seine Grenze).
            net.send_from((0, b(0)), msg(u32::try_from(t).unwrap()));
            net.send_from((1, b(0)), msg(7));
            net.send_from((1, b(1)), msg(9));
            net.flush();
            t += 100;
        }
        got
    }

    #[test]
    fn delivers_to_the_right_border_once_up() {
        let mut net = line();
        let names: Vec<String> = net.links().into_iter().map(|l| l.link).collect();
        assert_eq!(names, ["West-Mitte", "Mitte-Ost"]);
        let got = run(&mut net, 0, 2000);
        assert!(net.links().iter().all(|l| l.up), "{:?}", net.links());
        assert!(got.contains(&((0, b(0)), msg(7))));
        assert!(got.contains(&((2, b(0)), msg(9))));
        assert!(got.iter().any(|((a, g), _)| *a == 1 && *g == b(0)));
        // Ost hat Mitte nichts geschickt.
        assert!(!got.iter().any(|((a, g), _)| *a == 1 && *g == b(1)));
        assert!(net.links().iter().all(|l| l.bad_telegrams == 0));
    }

    #[test]
    fn interrupted_link_delivers_nothing_and_recovers() {
        let mut net = line();
        run(&mut net, 0, 1000);
        net.set_faults(
            "West-Mitte",
            LinkFaults {
                down: true,
                ..LinkFaults::default()
            },
        )
        .unwrap();
        let during = run(&mut net, 1100, 4000);
        // Höchstens was vor der Unterbrechung schon unterwegs war.
        let late = during
            .iter()
            .filter(|((a, g), _)| (*a == 0) || (*a == 1 && *g == b(0)))
            .count();
        assert!(late <= 2, "{late} Meldungen trotz Unterbrechung");
        assert!(!net.links()[0].up);
        assert!(net.links()[1].up);
        net.set_faults("West-Mitte", LinkFaults::default()).unwrap();
        let after = run(&mut net, 4100, 7000);
        assert!(net.links()[0].up);
        assert!(after.iter().any(|((a, _), _)| *a == 0));
        assert!(net.set_faults("Nord-Süd", LinkFaults::default()).is_err());
    }

    #[test]
    fn corruption_is_never_delivered() {
        let mut net = line();
        net.set_faults(
            "Mitte-Ost",
            LinkFaults {
                corrupt_percent: 40,
                loss_percent: 10,
                ..LinkFaults::default()
            },
        )
        .unwrap();
        let got = run(&mut net, 0, 20_000);
        let mut to_ost = 0;
        for ((a, g), m) in &got {
            assert!(!(*a == 1 && *g == b(1)), "Ost hat Mitte nichts geschickt");
            if *a == 2 {
                assert_eq!((*g, *m), (b(0), msg(9)));
                to_ost += 1;
            }
        }
        assert!(to_ost > 10, "nur {to_ost} Meldungen trotz Wiederholung");
        assert!(net.links()[1].discarded > 0);
    }

    /// West und Mitte rechnen in einem Prozess, Ost in einem anderen; die PDUs zwischen
    /// beiden gehen über das „Netz“ (hier eine Liste, teils verloren).
    #[test]
    fn split_processes_exchange_over_the_network() {
        let area = |name: &str, global, local| IlsArea {
            name: name.into(),
            global,
            local,
        };
        let pairs = |here: [Option<usize>; 3]| {
            vec![
                IlsPair {
                    areas: [area("West", 0, here[0]), area("Mitte", 1, here[1])],
                    borders: vec![(b(0), b(0))],
                },
                IlsPair {
                    areas: [area("Mitte", 1, here[1]), area("Ost", 2, here[2])],
                    borders: vec![(b(1), b(0))],
                },
            ]
        };
        let mut one = IlsNet::from_pairs(pairs([Some(0), Some(1), None])).unwrap();
        let mut two = IlsNet::from_pairs(pairs([None, None, Some(0)])).unwrap();
        assert_eq!(
            two.links().len(),
            1,
            "West-Mitte betrifft den zweiten Prozess nicht"
        );
        assert_eq!(one.links()[1].remote.as_deref(), Some("Ost"));
        let (mut to_ost, mut to_mitte, mut sent, mut last) = (0, 0, 0u32, None);
        for k in 0..400u32 {
            let now = Tick::from_millis(u64::from(k) * 100);
            for ((a, g), m) in one.advance(now) {
                if (a, g) == (1, b(1)) {
                    assert_eq!(m, msg(5));
                    to_mitte += 1;
                }
            }
            // RaSTA liefert lückenlos in Sendereihenfolge, Verlorenes wiederholt.
            for ((a, g), m) in two.advance(now) {
                assert_eq!((a, g), (0, b(0)));
                assert_eq!(m, msg(last.map_or(m.seq, |l: u32| l + 1)));
                assert!(m.seq < k);
                last = Some(m.seq);
                to_ost += 1;
            }
            one.send_from((1, b(1)), msg(k));
            two.send_from((0, b(0)), msg(5));
            one.flush();
            two.flush();
            // Jede siebte PDU geht verloren; fremde PDUs werden verworfen.
            for (dest, pdu) in one.take_outgoing() {
                assert_eq!(dest, "Ost");
                sent += 1;
                if sent % 7 != 3 {
                    two.receive_pdu(&pdu);
                }
                one.receive_pdu(&pdu);
            }
            for (dest, pdu) in two.take_outgoing() {
                assert_eq!(dest, "Mitte");
                one.receive_pdu(&pdu);
            }
        }
        assert!(one.links()[1].up && two.links()[0].up);
        assert!(to_ost > 300 && to_mitte > 300, "{to_ost} {to_mitte}");
        assert_eq!(one.links()[1].bad_telegrams, 0);
    }

    #[test]
    fn rejects_bad_names_and_unpaired_borders() {
        let peers = vec![vec![Some((1, b(0)))], vec![Some((0, b(0)))]];
        assert!(IlsNet::new(&["A_B".into(), "C".into()], &peers).is_err());
        let unpaired = vec![vec![Some((1, b(0)))], vec![None]];
        assert!(IlsNet::new(&["A".into(), "C".into()], &unpaired).is_err());
    }
}
