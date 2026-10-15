//! Feldanbindung von rail-it über EULYNX SCI und RaSTA (Konzept, Abschnitt 12).
//!
//! ```text
//!  Kern ──Stellbefehle──▶ EulynxField ──SCI──▶ RaSTA ──Strecke──▶ RaSTA ──SCI──▶ Objektsteuerung ──▶ Feld
//!       ◀───Meldungen────             ◀──SCI── RaSTA ◀─Strecke─── RaSTA ◀──SCI──   (je Elementart)    (Simulator)
//! ```
//!
//! Jede Objektsteuerung (OC) ist über eine eigene RaSTA-Verbindung angebunden. Die Strecke
//! dazwischen lässt sich stören: Verlust, Verfälschung, Verzögerung, Unterbrechung. Das
//! Stellwerk gibt dem Kern nur Meldungen von Objektsteuerungen weiter, deren Verbindung steht;
//! fällt eine Verbindung aus, fehlen die Meldungen, der Kern behandelt die Elemente nach der
//! Meldezeit als gestört (REQ-IXL-009) und baut die Verbindung selbsttätig neu auf.
//!
//! Die Objektsteuerungen senden den Zustand ihrer Elemente bei jeder Änderung und nach jedem
//! Verbindungsaufbau vollständig.

pub mod ils;
pub mod net;
pub mod redundant;
pub mod sci;

use std::collections::{HashMap, VecDeque};

use railit_kernel::{Aspect, Command, Input, PointReport};
use railit_rasta::{Config, Connection, MAX_PDU, Received, Role, State};
use railit_sim::Simulator;
use railit_topology::ElementRef;
use railit_types::{LxId, PointId, SignalId, Tick, TvdId};
use serde::Serialize;

use crate::sci::{Message, Protocol, Telegram};

/// Kennung des Stellwerks in den Telegrammen.
pub const INTERLOCKING_ID: &str = "STW";
/// RaSTA-Kennung des Stellwerks.
const RASTA_IXL: u32 = 0x0A00_0001;

/// Störungen einer Übertragungsstrecke.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LinkFaults {
    /// Strecke unterbrochen.
    pub down: bool,
    /// Anteil verlorener PDUs in Prozent.
    pub loss_percent: u8,
    /// Anteil verfälschter PDUs in Prozent (ein Bit gekippt).
    pub corrupt_percent: u8,
    /// Laufzeit in Millisekunden.
    pub delay_ms: u32,
}

/// Übertragungsstrecke in eine Richtung, deterministisch gestört.
#[derive(Clone, Debug)]
struct Link {
    queue: VecDeque<(u64, Vec<u8>)>,
    faults: LinkFaults,
    seed: u64,
}

impl Link {
    fn new(seed: u64) -> Self {
        Self {
            queue: VecDeque::new(),
            faults: LinkFaults::default(),
            seed: seed | 1,
        }
    }

    fn random(&mut self) -> u64 {
        // xorshift64
        let mut x = self.seed;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.seed = x;
        x
    }

    fn send(&mut self, now: u64, mut pdu: Vec<u8>) {
        if self.faults.down || self.random() % 100 < u64::from(self.faults.loss_percent) {
            return;
        }
        if self.random() % 100 < u64::from(self.faults.corrupt_percent) && !pdu.is_empty() {
            let bit = usize::try_from(self.random() % (pdu.len() as u64 * 8)).unwrap_or(0);
            if let Some(b) = pdu.get_mut(bit / 8) {
                *b ^= 1 << (bit % 8);
            }
        }
        self.queue
            .push_back((now + u64::from(self.faults.delay_ms), pdu));
    }

    fn due(&mut self, now: u64) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        while self.queue.front().is_some_and(|(at, _)| *at <= now) {
            if let Some((_, pdu)) = self.queue.pop_front() {
                out.push(pdu);
            }
        }
        out
    }
}

/// Feldelement.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Element {
    Point(PointId),
    Signal(SignalId),
    Tvd(TvdId),
    Lx(LxId),
}

/// Angebundene Elemente einer Objektsteuerung mit ihren SCI-Kennungen.
pub(crate) type Elements = Vec<(String, Element)>;

/// Antriebe gekoppelter Weichen (REQ-TOP-017): Zwei gekoppelte Weichen hängen an **einem**
/// Feldelement, benannt nach der ersten Weiche der Kopplung. Stellbefehle an die zweite gehen
/// in gekoppelter Lage an dieses Element; seine Lagemeldung gilt für beide.
#[derive(Clone, Debug, Default)]
pub(crate) struct Drives {
    /// Zweite Weiche → (Antrieb = erste Weiche, gegenläufig).
    secondary: HashMap<PointId, (PointId, bool)>,
    /// Erste Weiche → (zweite Weiche, gegenläufig).
    primary: HashMap<PointId, (PointId, bool)>,
}

const fn flip(pos: railit_types::PointPosition, inverted: bool) -> railit_types::PointPosition {
    if inverted { pos.other() } else { pos }
}

impl Drives {
    pub(crate) fn new(net: &railit_topology::Topology) -> Self {
        let mut d = Self::default();
        for c in net.couplings() {
            let [a, b] = c.points;
            d.secondary.insert(b, (a, c.inverted));
            d.primary.insert(a, (b, c.inverted));
        }
        d
    }

    /// Hat die Weiche kein eigenes Feldelement?
    pub(crate) fn is_secondary(&self, point: PointId) -> bool {
        self.secondary.contains_key(&point)
    }

    /// Stellbefehl als Element und SCI-Nachricht; Befehle an die zweite Weiche einer Kopplung
    /// gehen an den gemeinsamen Antrieb.
    pub(crate) fn message_of(&self, cmd: Command) -> Option<(Element, Message)> {
        match cmd {
            Command::MovePoint { point, position } => {
                let (drive, pos) = self
                    .secondary
                    .get(&point)
                    .map_or((point, position), |&(a, inv)| (a, flip(position, inv)));
                Some((Element::Point(drive), Message::MovePoint(pos)))
            }
            other => message_of(other),
        }
    }

    /// Meldungen für den Kern aus dem Zustand eines Elements; die Lage eines Antriebs gilt
    /// für beide gekoppelten Weichen.
    pub(crate) fn inputs_of(&self, e: Element, m: Message) -> impl Iterator<Item = Input> {
        let partner = match (e, m) {
            (Element::Point(p), Message::PointPosition(report)) => {
                self.primary.get(&p).map(|&(b, inv)| Input::Point {
                    point: b,
                    report: match report {
                        PointReport::Left => {
                            PointReport::from(flip(railit_types::PointPosition::Left, inv))
                        }
                        PointReport::Right => {
                            PointReport::from(flip(railit_types::PointPosition::Right, inv))
                        }
                        other => other,
                    },
                })
            }
            _ => None,
        };
        input_of(e, m).into_iter().chain(partner)
    }
}

/// Objektsteuerung eines Netzes: je Elementart eine.
pub(crate) struct OcLayout {
    pub(crate) name: String,
    pub(crate) protocol: Protocol,
    pub(crate) elements: Elements,
    /// RaSTA-Kennung.
    pub(crate) id: u32,
}

/// Objektsteuerungen eines Netzes: je Elementart eine (ohne Elemente: keine).
///
/// # Errors
/// Beschreibung, wenn ein Elementname keine gültige SCI-Kennung ist (höchstens 20 Byte
/// UTF-8 ohne `_`, das als Füllzeichen dient).
pub(crate) fn layout(net: &railit_topology::Topology) -> Result<Vec<OcLayout>, String> {
    let drives = Drives::new(net);
    let groups: [(&str, Protocol, Elements); 4] = [
        (
            "OC-Weichen",
            Protocol::P,
            net.points()
                .filter(|(id, _)| !drives.is_secondary(*id))
                .map(|(id, p)| (p.name.clone(), Element::Point(id)))
                .collect(),
        ),
        (
            "OC-Signale",
            Protocol::Ls,
            net.signals()
                .map(|(id, s)| (s.name.clone(), Element::Signal(id)))
                .collect(),
        ),
        (
            "OC-Gleisfrei",
            Protocol::Tds,
            net.tvds()
                .map(|(id, t)| (t.name.clone(), Element::Tvd(id)))
                .collect(),
        ),
        (
            "OC-BUe",
            Protocol::Lx,
            net.level_crossings()
                .map(|(id, l)| (l.name.clone(), Element::Lx(id)))
                .collect(),
        ),
    ];
    let mut out = Vec::new();
    for (i, (name, protocol, elements)) in groups.into_iter().enumerate() {
        if elements.is_empty() {
            continue;
        }
        for (n, _) in &elements {
            if n.len() > sci::ID_LEN || n.contains('_') {
                return Err(format!("„{n}“ ist keine gültige SCI-Kennung"));
            }
        }
        out.push(OcLayout {
            name: name.to_string(),
            protocol,
            elements,
            id: 0x0B00_0000 + u32::try_from(i).unwrap_or(0) + 1,
        });
    }
    Ok(out)
}

/// Objektsteuerungen eines Stellbereichs (ohne Stellbereiche: des ganzen Netzes).
pub(crate) type OcGroup = (Option<String>, Vec<OcLayout>);

/// Objektsteuerungen je Stellbereich: ohne Stellbereiche eine Gruppe mit allen
/// Objektsteuerungen (wie [`layout`]); sonst je Bereich und Elementart eine Objektsteuerung
/// (`OC-Weichen-Adorf`, RaSTA-Kennung `0x0B00_0000 + 16·(Bereich + 1) + Art`).
pub(crate) fn area_layout(net: &railit_topology::Topology) -> Result<Vec<OcGroup>, String> {
    let all = layout(net)?;
    if net.areas().len() == 0 {
        return Ok(vec![(None, all)]);
    }
    let mut groups = Vec::new();
    for (g, (id, area)) in net.areas().enumerate() {
        let cut = net
            .area_cut(id)
            .map_err(|e| format!("Stellbereich „{}“: {e}", area.name))?;
        let mine = |e: &Element| match *e {
            Element::Point(p) => cut.points.contains(&p),
            Element::Signal(s) => cut.signals.contains(&s),
            Element::Tvd(t) => cut.tvds.contains(&t),
            Element::Lx(l) => cut.lxs.contains(&l),
        };
        let base = u32::try_from(g)
            .ok()
            .and_then(|g| g.checked_add(1))
            .and_then(|g| g.checked_mul(16))
            .ok_or("zu viele Stellbereiche")?;
        let ocs = all
            .iter()
            .filter_map(|oc| {
                let elements: Elements = oc
                    .elements
                    .iter()
                    .filter(|(_, e)| mine(e))
                    .cloned()
                    .collect();
                (!elements.is_empty()).then(|| OcLayout {
                    name: format!("{}-{}", oc.name, area.name),
                    protocol: oc.protocol,
                    elements,
                    id: oc.id.wrapping_add(base),
                })
            })
            .collect();
        groups.push((Some(area.name.clone()), ocs));
    }
    Ok(groups)
}

/// Objektsteuerung mit ihrer Verbindung zum Stellwerk.
struct ObjectController {
    name: String,
    protocol: Protocol,
    elements: Elements,
    /// Stellwerksseite der Verbindung.
    ixl: Connection,
    /// Seite der Objektsteuerung.
    oc: Connection,
    down: Link,
    up: Link,
    /// Zuletzt an das Stellwerk gemeldete Zustände (Seite der Objektsteuerung).
    sent: HashMap<Element, Message>,
    /// Zuletzt empfangene Zustände (Stellwerksseite).
    status: HashMap<Element, Message>,
    /// Telegramme, die nicht gelesen werden konnten.
    bad_telegrams: u32,
}

/// Zustand einer Verbindung zu einer Objektsteuerung.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LinkStatus {
    /// Objektsteuerung.
    pub controller: String,
    /// Teilschnittstelle.
    pub interface: String,
    /// Anzahl angebundener Elemente.
    pub elements: usize,
    /// Verbindung steht.
    pub up: bool,
    /// Vom Stellwerk verworfene PDUs (Verfälschung, Wiederholung, …).
    pub discarded: u32,
    /// Nicht lesbare Telegramme.
    pub bad_telegrams: u32,
    /// Eingestellte Störungen.
    pub faults: LinkFaults,
}

/// Feldebene über EULYNX SCI und RaSTA mit simulierten Objektsteuerungen.
pub struct EulynxField {
    sim: Simulator,
    controllers: Vec<ObjectController>,
    drives: Drives,
    now: u64,
}

pub(crate) fn status_of(sim: &Simulator, e: Element) -> Option<Message> {
    Some(match e {
        Element::Point(p) => Message::PointPosition(sim.point_report(p)?),
        Element::Signal(s) => Message::SignalStatus(sim.signal_report(s)?),
        Element::Tvd(t) => Message::Occupancy(sim.occupancy(t)?),
        Element::Lx(l) => Message::LxStatus(sim.lx_report(l)?),
    })
}

pub(crate) fn input_of(e: Element, m: Message) -> Option<Input> {
    Some(match (e, m) {
        (Element::Point(point), Message::PointPosition(report)) => Input::Point { point, report },
        (Element::Signal(signal), Message::SignalStatus(report)) => {
            Input::Signal { signal, report }
        }
        (Element::Tvd(tvd), Message::Occupancy(occupancy)) => Input::Tvd { tvd, occupancy },
        (Element::Lx(lx), Message::LxStatus(report)) => Input::LevelCrossing { lx, report },
        _ => return None,
    })
}

pub(crate) fn command_of(e: Element, m: Message) -> Option<Command> {
    Some(match (e, m) {
        (Element::Point(point), Message::MovePoint(position)) => {
            Command::MovePoint { point, position }
        }
        (Element::Signal(signal), Message::IndicateAspect(proceed)) => Command::SetSignal {
            signal,
            aspect: if proceed {
                Aspect::Proceed
            } else {
                Aspect::Stop
            },
        },
        (Element::Signal(signal), Message::Substitute(on)) => Command::SetSubstitute { signal, on },
        (Element::Tvd(tvd), Message::ResetTvps) => Command::ResetTvd(tvd),
        (Element::Lx(lx), Message::LxCommand(close)) => Command::SetLevelCrossing { lx, close },
        _ => return None,
    })
}

/// Stellbefehl als Element und SCI-Nachricht (Grenzmeldungen gehen nicht ans Feld).
pub(crate) fn message_of(cmd: Command) -> Option<(Element, Message)> {
    Some(match cmd {
        Command::MovePoint { point, position } => {
            (Element::Point(point), Message::MovePoint(position))
        }
        Command::SetSignal { signal, aspect } => (
            Element::Signal(signal),
            Message::IndicateAspect(aspect == Aspect::Proceed),
        ),
        Command::SetSubstitute { signal, on } => (Element::Signal(signal), Message::Substitute(on)),
        Command::SetLevelCrossing { lx, close } => (Element::Lx(lx), Message::LxCommand(close)),
        Command::ResetTvd(tvd) => (Element::Tvd(tvd), Message::ResetTvps),
        Command::Border { .. } => return None,
    })
}

pub(crate) fn rasta_now(now: u64) -> u32 {
    u32::try_from(now % (1 << 32)).unwrap_or(0)
}

impl EulynxField {
    /// Feldebene vor `sim`: je Elementart eine Objektsteuerung.
    ///
    /// # Errors
    /// Beschreibung, wenn ein Elementname keine gültige SCI-Kennung ist (höchstens 20 Byte
    /// UTF-8 ohne `_`, das als Füllzeichen dient).
    pub fn new(sim: Simulator) -> Result<Self, String> {
        let mut controllers = Vec::new();
        for oc in layout(sim.topology())? {
            let mut oc_cfg = Config::new(oc.id, RASTA_IXL, Role::Server);
            oc_cfg.initial_sn = 0x1000;
            let seed = u64::from(oc.id);
            controllers.push(ObjectController {
                name: oc.name,
                protocol: oc.protocol,
                elements: oc.elements,
                ixl: Connection::new(Config::new(RASTA_IXL, oc.id, Role::Client)),
                oc: Connection::new(oc_cfg),
                down: Link::new(seed.wrapping_mul(31)),
                up: Link::new(seed.wrapping_mul(17)),
                sent: HashMap::new(),
                status: HashMap::new(),
                bad_telegrams: 0,
            });
        }
        let drives = Drives::new(sim.topology());
        Ok(Self {
            sim,
            controllers,
            drives,
            now: 0,
        })
    }

    /// Der Simulator hinter den Objektsteuerungen.
    pub fn simulator(&self) -> &Simulator {
        &self.sim
    }

    /// Der Simulator hinter den Objektsteuerungen.
    pub fn simulator_mut(&mut self) -> &mut Simulator {
        &mut self.sim
    }

    /// Störungen der Strecke zu einer Objektsteuerung setzen (beide Richtungen).
    ///
    /// # Errors
    /// Beschreibung, wenn es die Objektsteuerung nicht gibt.
    pub fn set_link_faults(&mut self, controller: &str, faults: LinkFaults) -> Result<(), String> {
        let oc = self
            .controllers
            .iter_mut()
            .find(|c| c.name == controller)
            .ok_or_else(|| format!("Objektsteuerung „{controller}“ nicht gefunden"))?;
        oc.down.faults = faults;
        oc.up.faults = faults;
        Ok(())
    }

    /// Zustand aller Verbindungen.
    pub fn links(&self) -> Vec<LinkStatus> {
        self.controllers
            .iter()
            .map(|c| LinkStatus {
                controller: c.name.clone(),
                interface: format!("{:?}", c.protocol),
                elements: c.elements.len(),
                up: c.ixl.is_up(),
                discarded: c.ixl.discarded(),
                bad_telegrams: c.bad_telegrams,
                faults: c.down.faults,
            })
            .collect()
    }

    /// Schreibt das Feld bis `now` fort und tauscht die Telegramme aus.
    pub fn advance(&mut self, now: Tick) {
        self.now = now.as_millis();
        self.sim.advance(now);
        let t = rasta_now(self.now);
        let now = self.now;
        let sim = &mut self.sim;
        for c in &mut self.controllers {
            // Stellwerk: Verbindung (wieder) aufbauen.
            if c.ixl.state() == State::Closed {
                c.status.clear();
                c.ixl.open(t);
            }
            c.ixl.tick(t);
            flush(&mut c.ixl, &mut c.down, now);

            // Objektsteuerung: Befehle annehmen und ausführen.
            let was_up = c.oc.is_up();
            for pdu in c.down.due(now) {
                let mut telegrams = Vec::new();
                c.oc.receive(t, &pdu, &mut |data| telegrams.push(data.to_vec()));
                for raw in telegrams {
                    match Telegram::decode(&raw) {
                        Ok(tg) => {
                            let element = c
                                .elements
                                .iter()
                                .find(|(n, _)| *n == tg.receiver)
                                .map(|(_, e)| *e);
                            if let Some(cmd) = element.and_then(|e| command_of(e, tg.message)) {
                                sim.apply(&[cmd]);
                            } else {
                                c.bad_telegrams = c.bad_telegrams.saturating_add(1);
                            }
                        }
                        Err(_) => c.bad_telegrams = c.bad_telegrams.saturating_add(1),
                    }
                }
            }
            c.oc.tick(t);
            if c.oc.is_up() && !was_up {
                c.sent.clear();
            }
            // Rückfallebene der Objektsteuerung: ohne Verbindung zum Stellwerk zeigen ihre
            // Signale Halt, ein Ersatzsignal erlischt.
            if was_up && !c.oc.is_up() {
                for (_, e) in &c.elements {
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
            // Objektsteuerung: Zustandsänderungen melden.
            if c.oc.is_up() {
                for (name, e) in &c.elements {
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
                        && c.oc.send(t, &raw).is_ok()
                    {
                        c.sent.insert(*e, m);
                    }
                }
            } else if c.oc.state() == State::Closed {
                c.sent.clear();
            }
            flush(&mut c.oc, &mut c.up, now);

            c.take_reports(t, now);
        }
    }

    /// Meldungen für den Kern: nur von Objektsteuerungen mit stehender Verbindung.
    pub fn reports(&self) -> Vec<Input> {
        self.controllers
            .iter()
            .filter(|c| c.ixl.is_up())
            .flat_map(|c| {
                c.elements.iter().flat_map(|(_, e)| {
                    c.status
                        .get(e)
                        .into_iter()
                        .flat_map(|m| self.drives.inputs_of(*e, *m))
                })
            })
            .collect()
    }

    /// Stellbefehle des Kerns als Telegramme an die Objektsteuerungen.
    pub fn apply(&mut self, commands: &[Command]) {
        let t = rasta_now(self.now);
        for cmd in commands {
            let Some((element, message)) = self.drives.message_of(*cmd) else {
                continue;
            };
            let Some(c) = self
                .controllers
                .iter_mut()
                .find(|c| c.elements.iter().any(|(_, e)| *e == element))
            else {
                continue;
            };
            let Some(name) = c
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
                let _ = c.ixl.send(t, &raw);
            }
        }
        let now = self.now;
        for c in &mut self.controllers {
            flush(&mut c.ixl, &mut c.down, now);
        }
    }
}

impl ObjectController {
    /// Stellwerksseite: übernimmt die eingetroffenen Meldungen der Objektsteuerung.
    fn take_reports(&mut self, t: u32, now: u64) {
        for pdu in self.up.due(now) {
            let mut telegrams = Vec::new();
            let r = self
                .ixl
                .receive(t, &pdu, &mut |data| telegrams.push(data.to_vec()));
            if matches!(r, Received::Disconnected(_)) {
                self.status.clear();
            }
            for raw in telegrams {
                let element = Telegram::decode(&raw).ok().and_then(|tg| {
                    self.elements
                        .iter()
                        .find(|(n, _)| *n == tg.sender)
                        .map(|(_, e)| (*e, tg.message))
                });
                match element {
                    Some((e, m)) => {
                        self.status.insert(e, m);
                    }
                    None => self.bad_telegrams = self.bad_telegrams.saturating_add(1),
                }
            }
        }
        if !self.ixl.is_up() {
            self.status.clear();
        }
        flush(&mut self.ixl, &mut self.down, now);
    }
}

fn flush(conn: &mut Connection, link: &mut Link, now: u64) {
    let mut buf = [0u8; MAX_PDU];
    while let Some(n) = conn.poll_transmit(&mut buf) {
        if let Some(pdu) = buf.get(..n) {
            link.send(now, pdu.to_vec());
        }
    }
}

/// Gibt Elementart und Name zu einem Element des Netzes (für Diagnosen).
pub fn element_interface(sim: &Simulator, name: &str) -> Option<&'static str> {
    Some(match sim.topology().lookup(name)? {
        ElementRef::Point(_) => "SCI-P",
        ElementRef::Signal(_) => "SCI-LS",
        ElementRef::Tvd(_) => "SCI-TDS",
        ElementRef::LevelCrossing(_) => "SCI-LX",
        _ => return None,
    })
}
