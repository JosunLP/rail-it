//! Betriebsprotokoll von rail-it (Konzept, Abschnitt 15).
//!
//! Der Sicherheitskern ist deterministisch: Gleiche Eingaben zur gleichen Zeit liefern
//! gleiche Ausgaben und gleichen Zustand. Das Journal speichert deshalb nur die
//! **Eingaben** jedes Zyklus (Meldungen und Anforderungen) samt Zeitpunkt – daraus lässt
//! sich jeder Zustand nachträglich exakt rekonstruieren.
//!
//! Format: JSON Lines. Jede Zeile ist `{"entry": …, "prev": "<hex>", "hash": "<hex>"}`.
//!
//! - Die erste Zeile ist der **Kopf** mit Plan-Digest, Zykluszeit und Regelwerk.
//! - Danach folgt je Zyklus ein Eintrag mit Eingaben, Zeitpunkt und dem Hash der Ausgaben
//!   (Stellbefehle und Ereignisse). Alle `checkpoint_every` Zyklen kommt ein Hash des
//!   Kernzustands dazu (**Prüfpunkt**).
//! - Die Feldelemente melden in jedem Zyklus, meist unverändert. Ab Version 4 steht deshalb
//!   nur die **Änderung** der Eingaben gegenüber dem vorigen Zyklus im Journal
//!   ([`InputDelta`], REQ-JRN-005); an Prüfpunkten stehen sie vollständig. Beim Lesen werden die
//!   Eingaben wieder vollständig zusammengesetzt.
//! - Ist im Kopf `snapshot_every` gesetzt, folgt alle so vielen Zyklen (an einem Prüfpunkt,
//!   außerhalb des sicheren Zustands der Sicherheitsschicht) ein **Zustandsabzug**
//!   ([`Snapshot`], REQ-JRN-006). [`replay`] spielt die Abschnitte zwischen den Abzügen parallel ab und
//!   prüft am Ende jedes Abschnitts, dass der abgespielte Zustand genau dem nächsten Abzug
//!   gleicht – das Ergebnis ist dasselbe wie beim Abspielen am Stück (Konzept, Abschnitt 15:
//!   Replay von 24 h Betrieb in weniger als einer Minute).
//! - `hash = SHA-256(prev ‖ JSON(entry))`, `prev` ist der Hash der Vorgängerzeile (beim Kopf
//!   32 Nullbytes). Wird eine Zeile verändert, entfernt oder eingefügt, bricht die Kette
//!   (REQ-JRN-001).
//!
//! - Mit Schlüssel folgt nach dem Kopf und danach regelmäßig ein **Siegel** (Ed25519 über den
//!   Hash der vorhergehenden Zeile, siehe [`Seal`]); es bindet den Stand an das Stellwerk
//!   (REQ-JRN-004).
//!
//! [`verify`] prüft die Kette, [`replay`] spielt das Journal gegen einen Plan erneut ab und
//! vergleicht Ausgaben und Prüfpunkte (REQ-JRN-002). Der Zustands-Hash beruht auf der
//! `Hash`-Implementierung des Kernzustands; Prüfpunkte sind daher nur zwischen Programmen
//! gleicher Version und Plattform vergleichbar. Die Ausgaben-Hashes sind plattformunabhängig.

pub mod audit;
mod seal;

pub use seal::Seal;

use std::fmt;
use std::hash::{Hash, Hasher};
use std::io::{self, BufRead, Write};

use ed25519_dalek::{SigningKey, VerifyingKey};
use railit_kernel::{Command, Event, Input, Interlocking, OwnedState, VecOutputs};
use railit_plan::PlanView;
use railit_rules::GenericRules;
use railit_types::{Duration, Tick};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Kennung des Formats im Kopf.
pub const FORMAT: &str = "railit-journal";
/// Version des Formats (3: Siegel, 4: Eingaben als Änderung). Gelesen werden auch Journale
/// der Versionen 2 und 3.
pub const VERSION: u32 = 4;

type Hash32 = [u8; 32];

/// Bytes als Kleinbuchstaben-Hex.
pub fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

fn from_hex32(text: &str) -> Option<Hash32> {
    if text.len() != 64 || !text.is_ascii() {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(text.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

/// Regelwerk, wie es im Kopf steht.
#[allow(clippy::struct_field_names)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rules {
    /// Umstellzeit-Grenze.
    pub point_move_timeout_ms: u64,
    /// Höchstalter von Meldungen.
    pub report_timeout_ms: u64,
    /// Zeitverschluss.
    pub cancel_time_lock_ms: u64,
    /// Auflösezeit des Durchrutschwegs.
    pub overlap_release_time_ms: u64,
    /// Bestätigungsfrist für Hilfshandlungen.
    pub aux_confirm_timeout_ms: u64,
    /// Höchstdauer des Ersatzsignals.
    pub substitute_duration_ms: u64,
    /// Einschwingzeit des Erlaubniswechsels.
    #[serde(default = "default_settle_ms")]
    pub border_settle_time_ms: u64,
}

const fn default_settle_ms() -> u64 {
    5_000
}

impl From<&GenericRules> for Rules {
    fn from(r: &GenericRules) -> Self {
        Self {
            point_move_timeout_ms: r.point_move_timeout.as_millis(),
            report_timeout_ms: r.report_timeout.as_millis(),
            cancel_time_lock_ms: r.cancel_time_lock.as_millis(),
            overlap_release_time_ms: r.overlap_release_time.as_millis(),
            aux_confirm_timeout_ms: r.aux_confirm_timeout.as_millis(),
            substitute_duration_ms: r.substitute_duration.as_millis(),
            border_settle_time_ms: r.border_settle_time.as_millis(),
        }
    }
}

impl From<Rules> for GenericRules {
    fn from(r: Rules) -> Self {
        Self {
            point_move_timeout: Duration::from_millis(r.point_move_timeout_ms),
            report_timeout: Duration::from_millis(r.report_timeout_ms),
            cancel_time_lock: Duration::from_millis(r.cancel_time_lock_ms),
            overlap_release_time: Duration::from_millis(r.overlap_release_time_ms),
            aux_confirm_timeout: Duration::from_millis(r.aux_confirm_timeout_ms),
            substitute_duration: Duration::from_millis(r.substitute_duration_ms),
            border_settle_time: Duration::from_millis(r.border_settle_time_ms),
        }
    }
}

/// Kopf des Journals.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Header {
    /// Immer [`FORMAT`].
    pub format: String,
    /// Immer [`VERSION`].
    pub version: u32,
    /// Name des Netzes.
    pub network: String,
    /// SHA-256 des Plan-Images (Hex).
    pub plan_digest: String,
    /// Zykluszeit.
    pub cycle_ms: u64,
    /// Regelwerk.
    pub rules: Rules,
    /// Abstand der Prüfpunkte in Zyklen (0 = keine).
    pub checkpoint_every: u64,
    /// Abstand der Zustandsabzüge in Zyklen (0 = keine; ab Version 4).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub snapshot_every: u64,
}

#[allow(clippy::trivially_copy_pass_by_ref)]
const fn is_zero(n: &u64) -> bool {
    *n == 0
}

impl Header {
    /// Kopf für einen Plan und ein Regelwerk.
    pub fn new(
        plan: &PlanView<'_>,
        rules: &GenericRules,
        cycle: Duration,
        checkpoint_every: u64,
    ) -> Self {
        Self {
            format: FORMAT.into(),
            version: VERSION,
            network: plan.network_name().into(),
            plan_digest: to_hex(&plan.digest()),
            cycle_ms: cycle.as_millis(),
            rules: rules.into(),
            checkpoint_every,
            snapshot_every: 0,
        }
    }

    /// Mit Zustandsabzügen alle `every` Zyklen (für paralleles Replay).
    #[must_use]
    pub const fn with_snapshots(mut self, every: u64) -> Self {
        self.snapshot_every = every;
        self
    }
}

/// Zustandsübernahme (REQ-JRN-007): Der Zustand wurde von außen übernommen, im
/// Knotenbetrieb beim Zustandsabgleich mit dem sicheren Rechner. Das Replay rechnet ab hier
/// mit diesem Zustand weiter; Zyklen davor, die der Spiegel nicht gesehen hat (etwa während
/// einer Verbindungsunterbrechung), fehlen im Journal und sind so ausgewiesen.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resync {
    /// Zyklusnummer des übernommenen Zustands.
    pub cycle: u64,
    /// Zeitpunkt der Übernahme in Millisekunden.
    pub now_ms: u64,
    /// Anlass in Worten.
    pub detail: String,
    /// Übernommener Zustand.
    pub state: OwnedState,
}

/// Zustandsabzug des Kerns nach einem Zyklus.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    /// Zyklusnummer; der Abzug folgt unmittelbar auf den Eintrag dieses Zyklus.
    pub cycle: u64,
    /// Gesamter Zustand des Kerns.
    pub state: OwnedState,
}

/// Eintrag eines Zyklus.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cycle {
    /// Zyklusnummer des Kerns nach dem Zyklus.
    pub cycle: u64,
    /// Zeitpunkt in Millisekunden.
    pub now_ms: u64,
    /// Eingaben des Kerns. Im Journal leer, wenn [`Self::delta`] sie als Änderung zum vorigen
    /// Zyklus angibt; gelesene Zyklen enthalten sie immer vollständig.
    #[serde(default)]
    pub inputs: Vec<Input>,
    /// Eingaben als Änderung gegenüber dem vorigen Zyklus (nur im Journal, Version 4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delta: Option<InputDelta>,
    /// SHA-256 der Ausgaben (Hex), siehe [`outputs_digest`].
    pub outputs: String,
    /// Hash des Kernzustands nach dem Zyklus (nur an Prüfpunkten), siehe [`state_digest`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
}

/// Eingaben eines Zyklus als Änderung gegenüber denen des vorigen Zyklus.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputDelta {
    /// Anzahl der Eingaben.
    pub len: u32,
    /// Eingaben, die sich gegenüber dem vorigen Zyklus an ihrer Position geändert haben oder
    /// neu hinzukommen, aufsteigend nach Position.
    pub set: Vec<(u32, Input)>,
}

impl InputDelta {
    /// Änderung von `prev` nach `next`; `None`, wenn sich mehr als die Hälfte ändert (dann
    /// ist die vollständige Liste kürzer).
    fn between(prev: &[Input], next: &[Input]) -> Option<Self> {
        let len = u32::try_from(next.len()).ok()?;
        let mut set = Vec::new();
        for (i, input) in next.iter().enumerate() {
            if prev.get(i) != Some(input) {
                set.push((u32::try_from(i).ok()?, *input));
            }
        }
        (set.len().saturating_mul(2) < next.len()).then_some(Self { len, set })
    }

    /// Wendet die Änderung auf die Eingaben des vorigen Zyklus an.
    fn apply(&self, inputs: &mut Vec<Input>) -> Result<(), String> {
        let len = usize::try_from(self.len).map_err(|e| e.to_string())?;
        inputs.truncate(len);
        let mut last = None;
        for (pos, input) in &self.set {
            let i = usize::try_from(*pos).map_err(|e| e.to_string())?;
            if last.is_some_and(|l| l >= i) || i >= len {
                return Err(format!("Änderung an Position {i} ungültig"));
            }
            last = Some(i);
            if let Some(slot) = inputs.get_mut(i) {
                *slot = *input;
            } else if i == inputs.len() {
                inputs.push(*input);
            } else {
                return Err(format!("Eingabe {i} fehlt"));
            }
        }
        if inputs.len() != len {
            return Err(format!("{} statt {len} Eingaben", inputs.len()));
        }
        Ok(())
    }
}

/// Inhalt einer Journalzeile.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Entry {
    /// Kopf.
    Header(Header),
    /// Zyklus.
    Cycle(Cycle),
    /// Meldung der Sicherheitsschicht (redundante Kanäle).
    Safety(SafetyRecord),
    /// Siegel über alle vorhergehenden Zeilen.
    Seal(Seal),
    /// Zustandsabzug (ab Version 4).
    Snapshot(Box<Snapshot>),
    /// Zustandsübernahme (ab Version 4).
    Resync(Box<Resync>),
}

/// Art einer Meldung der Sicherheitsschicht.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SafetyKind {
    /// Ein Kanal wurde ausgeschlossen; die Ausgaben bleiben die der Mehrheit.
    Excluded,
    /// Ein Kanal wurde wieder aufgenommen.
    Reintegrated,
    /// Übergang in den sicheren Zustand: Ab dem folgenden Zyklus stammen die Ausgaben von
    /// der Schicht, nicht vom Kern.
    Tripped,
    /// Rücksetzen: Alle Kanäle beginnen wie nach einem Neustart.
    Reset,
    /// Meldung der Sicherheitsschicht eines Knotens des sicheren Rechners oder Handlung an
    /// ihr (nur zur Information). Im Knotenbetrieb hält das Journal die Zyklen fest, die der
    /// Spiegel nachgerechnet und geprüft hat; was der Knoten im sicheren Zustand tut, zeigt
    /// eine folgende Zustandsübernahme ([`Resync`]).
    Node,
}

/// Meldung der Sicherheitsschicht im Journal. Sie steht vor dem Zyklus, in dem sie wirkt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SafetyRecord {
    /// Zeitpunkt in Millisekunden.
    pub now_ms: u64,
    /// Art.
    pub kind: SafetyKind,
    /// Beschreibung (Kanal, Grund).
    pub detail: String,
}

/// Gelesene Zeile: Der Eintrag bleibt als Text erhalten. Gehasht wird genau dieser Text; er
/// gleicht der Kodierung beim Schreiben, ein anders formatierter Eintrag bricht die Kette.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLine<'a> {
    #[serde(borrow)]
    entry: &'a serde_json::value::RawValue,
    prev: &'a str,
    hash: &'a str,
}

fn chain(prev: &Hash32, json: &[u8]) -> Hash32 {
    let mut h = Sha256::new();
    h.update(prev);
    h.update(json);
    h.finalize().into()
}

/// SHA-256 über Stellbefehle und Ereignisse eines Zyklus (Hex).
pub fn outputs_digest(commands: &[Command], events: &[Event]) -> String {
    let json = serde_json::to_vec(&(commands, events)).unwrap_or_default();
    to_hex(&Sha256::digest(&json))
}

struct ShaHasher(Sha256);

impl Hasher for ShaHasher {
    fn finish(&self) -> u64 {
        0
    }

    fn write(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }
}

/// SHA-256 über den gesamten Kernzustand (Hex).
pub fn state_digest(state: &OwnedState) -> String {
    let mut h = ShaHasher(Sha256::new());
    state.hash(&mut h);
    to_hex(&h.0.finalize())
}

/// Siegelt ein Journal regelmäßig.
#[derive(Debug)]
struct Sealer {
    key: SigningKey,
    every: u64,
    since: u64,
}

/// Schreibt ein Journal.
#[derive(Debug)]
pub struct JournalWriter<W: Write> {
    out: W,
    prev: Hash32,
    checkpoint_every: u64,
    sealer: Option<Sealer>,
    /// Die letzte Zeile ist ein Siegel.
    sealed: bool,
    /// Eingaben des zuletzt geschriebenen Zyklus.
    last_inputs: Vec<Input>,
    /// Abstand der Zustandsabzüge.
    snapshot_every: u64,
    /// Die Sicherheitsschicht ist im sicheren Zustand (keine Abzüge).
    tripped: bool,
}

impl<W: Write> JournalWriter<W> {
    /// Beginnt ein Journal mit `header`.
    ///
    /// # Errors
    /// Schreibfehler.
    pub fn new(out: W, header: Header) -> io::Result<Self> {
        let checkpoint_every = header.checkpoint_every;
        let snapshot_every = header.snapshot_every;
        let mut w = Self {
            out,
            prev: [0; 32],
            checkpoint_every,
            sealer: None,
            sealed: false,
            last_inputs: Vec::new(),
            snapshot_every,
            tripped: false,
        };
        w.write(&Entry::Header(header))?;
        Ok(w)
    }

    /// Beginnt ein gesiegeltes Journal: Nach dem Kopf und danach alle `every` Zyklen folgt
    /// ein Siegel mit `key` (REQ-JRN-004); [`Self::seal`] siegelt zusätzlich, etwa beim
    /// Beenden.
    ///
    /// # Errors
    /// Schreibfehler.
    pub fn sealed(out: W, header: Header, key: SigningKey, every: u64) -> io::Result<Self> {
        let mut w = Self::new(out, header)?;
        w.sealer = Some(Sealer {
            key,
            every: every.max(1),
            since: 0,
        });
        w.seal()?;
        Ok(w)
    }

    /// Siegelt den bisherigen Stand (ohne Schlüssel: nichts; direkt nach einem Siegel:
    /// nichts).
    ///
    /// # Errors
    /// Schreibfehler.
    pub fn seal(&mut self) -> io::Result<()> {
        if self.sealed {
            return Ok(());
        }
        let Some(sealer) = &mut self.sealer else {
            return Ok(());
        };
        sealer.since = 0;
        let seal = seal::make(seal::SealKind::Journal, &sealer.key, &self.prev);
        self.write(&Entry::Seal(seal))?;
        self.sealed = true;
        Ok(())
    }

    /// Schreibt einen Eintrag unverändert in die Kette (für Werkzeuge, die Journale
    /// umschreiben, und für Tests). Die Kette ist danach gültig; ob der Inhalt zum Plan
    /// passt, zeigt erst [`replay`].
    ///
    /// # Errors
    /// Schreibfehler.
    pub fn append(&mut self, entry: &Entry) -> io::Result<()> {
        if let Entry::Cycle(c) = entry {
            match &c.delta {
                Some(d) => d.apply(&mut self.last_inputs).map_err(io::Error::other)?,
                None => self.last_inputs.clone_from(&c.inputs),
            }
        }
        self.write(entry)
    }

    /// Schreibt `{"entry":…,"prev":"…","hash":"…"}` – den Eintrag genau einmal kodiert;
    /// gehasht wird derselbe Text, den die Zeile enthält.
    fn write(&mut self, entry: &Entry) -> io::Result<()> {
        let json = serde_json::to_vec(entry).map_err(io::Error::other)?;
        let hash = chain(&self.prev, &json);
        let mut line = Vec::with_capacity(json.len().saturating_add(160));
        line.extend_from_slice(b"{\"entry\":");
        line.extend_from_slice(&json);
        line.extend_from_slice(b",\"prev\":\"");
        line.extend_from_slice(to_hex(&self.prev).as_bytes());
        line.extend_from_slice(b"\",\"hash\":\"");
        line.extend_from_slice(to_hex(&hash).as_bytes());
        line.extend_from_slice(b"\"}\n");
        self.out.write_all(&line)?;
        self.prev = hash;
        self.sealed = false;
        Ok(())
    }

    /// Hält einen Zyklus fest: Zeitpunkt und Eingaben, Hash der Ausgaben und – an
    /// Prüfpunkten – den Hash des Zustands nach dem Zyklus.
    ///
    /// # Errors
    /// Schreibfehler.
    pub fn record(
        &mut self,
        now: Tick,
        inputs: &[Input],
        out: &VecOutputs,
        state: &OwnedState,
    ) -> io::Result<()> {
        let cycle = state.meta().cycle();
        let snapshot =
            self.snapshot_every > 0 && !self.tripped && cycle.is_multiple_of(self.snapshot_every);
        let checkpoint =
            snapshot || (self.checkpoint_every > 0 && cycle.is_multiple_of(self.checkpoint_every));
        // An Prüfpunkten vollständig, sonst als Änderung, wenn das kürzer ist.
        let delta = if checkpoint {
            None
        } else {
            InputDelta::between(&self.last_inputs, inputs)
        };
        self.last_inputs.clear();
        self.last_inputs.extend_from_slice(inputs);
        self.write(&Entry::Cycle(Cycle {
            cycle,
            now_ms: now.as_millis(),
            inputs: if delta.is_some() {
                Vec::new()
            } else {
                inputs.to_vec()
            },
            delta,
            outputs: outputs_digest(&out.commands, &out.events),
            state: checkpoint.then(|| state_digest(state)),
        }))?;
        if snapshot {
            self.write(&Entry::Snapshot(Box::new(Snapshot {
                cycle,
                state: state.clone(),
            })))?;
        }
        let due = self.sealer.as_mut().is_some_and(|s| {
            s.since = s.since.saturating_add(1);
            s.since >= s.every
        });
        if due {
            self.seal()?;
        }
        Ok(())
    }

    /// Hält eine Zustandsübernahme fest (REQ-JRN-007): Ab hier rechnet das Replay mit
    /// `state` weiter.
    ///
    /// # Errors
    /// Schreibfehler.
    pub fn resync(&mut self, now: Tick, state: &OwnedState, detail: &str) -> io::Result<()> {
        self.tripped = false;
        // Der nächste Zyklus steht vollständig im Journal.
        self.last_inputs.clear();
        self.write(&Entry::Resync(Box::new(Resync {
            cycle: state.meta().cycle(),
            now_ms: now.as_millis(),
            detail: detail.to_string(),
            state: state.clone(),
        })))
    }

    /// Hält eine Meldung der Sicherheitsschicht fest (vor dem Zyklus, in dem sie wirkt).
    ///
    /// # Errors
    /// Schreibfehler.
    pub fn record_safety(&mut self, now: Tick, kind: SafetyKind, detail: &str) -> io::Result<()> {
        match kind {
            SafetyKind::Tripped => self.tripped = true,
            SafetyKind::Reset => self.tripped = false,
            SafetyKind::Excluded | SafetyKind::Reintegrated | SafetyKind::Node => {}
        }
        self.write(&Entry::Safety(SafetyRecord {
            now_ms: now.as_millis(),
            kind,
            detail: detail.to_string(),
        }))
    }

    /// Schreibt gepufferte Daten.
    ///
    /// # Errors
    /// Schreibfehler.
    pub fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }

    /// Gibt das Ziel zurück.
    pub fn into_inner(self) -> W {
        self.out
    }

    /// Hash der letzten Zeile (Hex); bestätigt den Stand des Journals.
    pub fn head(&self) -> String {
        to_hex(&self.prev)
    }
}

/// Fehler beim Prüfen oder Abspielen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JournalError {
    /// Lesefehler.
    Io(String),
    /// Zeile ist kein gültiger Eintrag.
    Syntax {
        /// Zeilennummer (ab 1).
        line: usize,
        /// Beschreibung.
        message: String,
    },
    /// Kopf fehlt oder hat ein unbekanntes Format.
    Format(String),
    /// Die Hashkette ist an dieser Zeile unterbrochen (Manipulation oder Beschädigung).
    Chain {
        /// Zeilennummer (ab 1).
        line: usize,
    },
    /// Der Plan passt nicht zum Journal.
    Plan(String),
    /// Ein Siegel ist ungültig (falsche Signatur, fremder Schlüssel, falscher Bezug).
    Seal {
        /// Zeilennummer (ab 1).
        line: usize,
        /// Beschreibung.
        message: String,
    },
    /// Beim Abspielen weicht das Ergebnis vom Journal ab.
    Divergence {
        /// Zyklus.
        cycle: u64,
        /// Was abweicht (`outputs`, `state`, `cycle`).
        what: String,
    },
}

impl fmt::Display for JournalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(m) => write!(f, "Lesefehler: {m}"),
            Self::Syntax { line, message } => write!(f, "Zeile {line}: {message}"),
            Self::Format(m) => write!(f, "Format: {m}"),
            Self::Chain { line } => write!(f, "Hashkette in Zeile {line} unterbrochen"),
            Self::Plan(m) => write!(f, "Plan: {m}"),
            Self::Seal { line, message } => write!(f, "Siegel in Zeile {line}: {message}"),
            Self::Divergence { cycle, what } => {
                write!(f, "Zyklus {cycle}: Abweichung beim Abspielen ({what})")
            }
        }
    }
}

impl std::error::Error for JournalError {}

/// Ergebnis von [`verify`] und [`replay`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Summary {
    /// Kopf.
    pub header: Header,
    /// Anzahl der Zyklen.
    pub cycles: u64,
    /// Anzahl der Prüfpunkte.
    pub checkpoints: u64,
    /// Anzahl der Zustandsabzüge.
    pub snapshots: u64,
    /// Anzahl der Zustandsübernahmen.
    pub resyncs: u64,
    /// Zeitpunkt des letzten Zyklus in Millisekunden.
    pub last_ms: u64,
    /// Hash der letzten vollständigen Zeile (Hex).
    pub head: String,
    /// Die letzte Zeile war unvollständig (Abbruch beim Schreiben) und wurde ignoriert.
    pub truncated: bool,
    /// Meldungen der Sicherheitsschicht.
    pub safety: Vec<SafetyRecord>,
    /// Anzahl gültiger Siegel.
    pub seals: u64,
    /// Schlüssel der Siegel (Hex), falls gesiegelt.
    pub seal_key: Option<String>,
    /// Einträge nach dem letzten Siegel (noch nicht bestätigt).
    pub unsealed: u64,
}

/// Empfänger der gelesenen Einträge.
trait Visit {
    fn header(&mut self, _header: &Header) -> Result<(), JournalError> {
        Ok(())
    }

    /// Ein Zyklus mit vollständigen Eingaben; `delta` ist die Änderung, wie sie im Journal
    /// steht (falls so geschrieben).
    fn cycle(&mut self, _cycle: &Cycle, _delta: Option<&InputDelta>) -> Result<(), JournalError> {
        Ok(())
    }

    fn safety(&mut self, _record: &SafetyRecord) -> Result<(), JournalError> {
        Ok(())
    }

    fn snapshot(&mut self, _snapshot: Box<Snapshot>) -> Result<(), JournalError> {
        Ok(())
    }

    fn resync(&mut self, _resync: Box<Resync>) -> Result<(), JournalError> {
        Ok(())
    }
}

/// Nur prüfen.
struct Check;

impl Visit for Check {}

/// Liest ein Journal und prüft dabei die Hashkette; `visit` erhält die Einträge.
#[allow(clippy::too_many_lines)]
fn read<R: BufRead>(
    reader: R,
    trusted: Option<&VerifyingKey>,
    visit: &mut impl Visit,
) -> Result<Summary, JournalError> {
    let mut safety = Vec::new();
    let mut prev: Hash32 = [0; 32];
    let mut header = None;
    let mut cycles = 0u64;
    let mut checkpoints = 0u64;
    let mut snapshots = 0u64;
    let mut resyncs = 0u64;
    let mut last_ms = 0u64;
    let mut truncated = false;
    let mut seals = 0u64;
    let mut seal_key = None;
    let mut unsealed = 0u64;
    // Eingaben des vorigen Zyklus, Grundlage für die nächste Änderung.
    let mut last_inputs: Vec<Input> = Vec::new();
    // Ein Abzug darf nur unmittelbar auf einen Zyklus mit vollständigen Eingaben folgen,
    // außerhalb des sicheren Zustands der Sicherheitsschicht.
    let mut snapshot_after: Option<u64> = None;
    let mut tripped = false;
    let mut lines = reader.lines().enumerate().peekable();
    while let Some((index, line)) = lines.next() {
        let number = index.saturating_add(1);
        let text = line.map_err(|e| JournalError::Io(e.to_string()))?;
        if text.trim().is_empty() {
            continue;
        }
        let parsed: RawLine<'_> = match serde_json::from_str(&text) {
            Ok(l) => l,
            // Eine unvollständige letzte Zeile ohne Zeilenende entsteht, wenn der Dienst
            // beim Schreiben abbricht. Sie trägt keinen gültigen Hash und wird ignoriert.
            Err(e) if e.is_eof() && lines.peek().is_none() => {
                truncated = true;
                break;
            }
            Err(e) => {
                return Err(JournalError::Syntax {
                    line: number,
                    message: e.to_string(),
                });
            }
        };
        let hash = chain(&prev, parsed.entry.get().as_bytes());
        if from_hex32(parsed.prev) != Some(prev) || from_hex32(parsed.hash) != Some(hash) {
            return Err(JournalError::Chain { line: number });
        }
        let entry: Entry =
            serde_json::from_str(parsed.entry.get()).map_err(|e| JournalError::Syntax {
                line: number,
                message: e.to_string(),
            })?;
        let before = prev;
        prev = hash;
        let follows = snapshot_after.take();
        match (entry, &header) {
            (Entry::Seal(s), Some(_)) => {
                seal::check(seal::SealKind::Journal, &s, &before, trusted, &mut seal_key).map_err(
                    |message| JournalError::Seal {
                        line: number,
                        message,
                    },
                )?;
                seals = seals.saturating_add(1);
                unsealed = 0;
            }
            (Entry::Header(h), None) => {
                if h.format != FORMAT || !(2..=VERSION).contains(&h.version) {
                    return Err(JournalError::Format(format!(
                        "unbekanntes Format {} v{}",
                        h.format, h.version
                    )));
                }
                visit.header(&h)?;
                header = Some(h);
            }
            (Entry::Header(_), Some(_)) => {
                return Err(JournalError::Format(format!(
                    "Zeile {number}: zweiter Kopf"
                )));
            }
            (
                Entry::Cycle(_)
                | Entry::Safety(_)
                | Entry::Seal(_)
                | Entry::Snapshot(_)
                | Entry::Resync(_),
                None,
            ) => {
                return Err(JournalError::Format("Kopf fehlt".into()));
            }
            (Entry::Safety(r), Some(_)) => {
                unsealed = unsealed.saturating_add(1);
                match r.kind {
                    SafetyKind::Tripped => tripped = true,
                    SafetyKind::Reset => tripped = false,
                    SafetyKind::Excluded | SafetyKind::Reintegrated | SafetyKind::Node => {}
                }
                visit.safety(&r)?;
                safety.push(r);
            }
            (Entry::Snapshot(snap), Some(_)) => {
                unsealed = unsealed.saturating_add(1);
                if follows != Some(snap.cycle) || tripped {
                    return Err(JournalError::Format(format!(
                        "Zeile {number}: Zustandsabzug ohne vollständigen Zyklus davor"
                    )));
                }
                snapshots = snapshots.saturating_add(1);
                visit.snapshot(snap)?;
            }
            (Entry::Resync(resync), Some(_)) => {
                unsealed = unsealed.saturating_add(1);
                // Die Übernahme hebt den sicheren Zustand der Schicht auf: Der übernommene
                // Zustand ist maßgeblich.
                tripped = false;
                resyncs = resyncs.saturating_add(1);
                last_ms = resync.now_ms;
                visit.resync(resync)?;
            }
            (Entry::Cycle(mut c), Some(_)) => {
                unsealed = unsealed.saturating_add(1);
                let delta = c.delta.take();
                if let Some(delta) = &delta {
                    delta
                        .apply(&mut last_inputs)
                        .map_err(|message| JournalError::Syntax {
                            line: number,
                            message,
                        })?;
                    c.inputs = std::mem::take(&mut last_inputs);
                } else {
                    snapshot_after = Some(c.cycle);
                }
                visit.cycle(&c, delta.as_ref())?;
                last_inputs = c.inputs;
                cycles = cycles.saturating_add(1);
                checkpoints = checkpoints.saturating_add(u64::from(c.state.is_some()));
                last_ms = c.now_ms;
            }
        }
    }
    let header = header.ok_or_else(|| JournalError::Format("leeres Journal".into()))?;
    Ok(Summary {
        header,
        cycles,
        checkpoints,
        snapshots,
        resyncs,
        last_ms,
        head: to_hex(&prev),
        truncated,
        safety,
        seals,
        seal_key,
        unsealed,
    })
}

/// Liest alle Einträge außer den Siegeln, Zyklen mit vollständigen Eingaben, und prüft
/// dabei die Hashkette (für Werkzeuge, die Journale umschreiben, siehe
/// [`JournalWriter::append`]).
///
/// # Errors
/// [`JournalError`] bei Lese-, Format- oder Kettenfehlern.
pub fn entries<R: BufRead>(reader: R) -> Result<Vec<Entry>, JournalError> {
    struct Collect(Vec<Entry>);

    impl Visit for Collect {
        fn header(&mut self, header: &Header) -> Result<(), JournalError> {
            self.0.push(Entry::Header(header.clone()));
            Ok(())
        }

        fn cycle(&mut self, cycle: &Cycle, _: Option<&InputDelta>) -> Result<(), JournalError> {
            self.0.push(Entry::Cycle(cycle.clone()));
            Ok(())
        }

        fn safety(&mut self, record: &SafetyRecord) -> Result<(), JournalError> {
            self.0.push(Entry::Safety(record.clone()));
            Ok(())
        }

        fn snapshot(&mut self, snapshot: Box<Snapshot>) -> Result<(), JournalError> {
            self.0.push(Entry::Snapshot(snapshot));
            Ok(())
        }

        fn resync(&mut self, resync: Box<Resync>) -> Result<(), JournalError> {
            self.0.push(Entry::Resync(resync));
            Ok(())
        }
    }

    let mut out = Collect(Vec::new());
    read(reader, None, &mut out)?;
    Ok(out.0)
}

/// Prüft die Hashkette eines Journals (REQ-JRN-001).
///
/// # Errors
/// [`JournalError`] bei Lese-, Format- oder Kettenfehlern.
pub fn verify<R: BufRead>(reader: R) -> Result<Summary, JournalError> {
    read(reader, None, &mut Check)
}

/// Prüft Hashkette und Siegel eines Journals; jedes Siegel muss mit `key` erstellt sein
/// (REQ-JRN-004). Ob überhaupt gesiegelt ist und wie viele Einträge nach dem letzten Siegel
/// folgen, steht in der [`Summary`].
///
/// # Errors
/// [`JournalError`] bei Lese-, Format-, Ketten- oder Siegelfehlern.
pub fn verify_sealed<R: BufRead>(reader: R, key: &VerifyingKey) -> Result<Summary, JournalError> {
    read(reader, Some(key), &mut Check)
}

/// Eintrag eines Abschnitts beim parallelen Replay.
enum Item {
    /// Zyklus; Eingaben vollständig oder als Änderung.
    Cycle(Cycle),
    Safety(SafetyKind),
    /// Zustandsabzug am Ende des Abschnitts.
    End(Box<Snapshot>),
}

/// Spielt einen Abschnitt ab: ab `start` (oder dem Anfangszustand); der erste Zyklus bringt
/// vollständige Eingaben mit. Endet der Abschnitt mit einem Abzug, muss der abgespielte
/// Zustand ihm gleichen.
fn replay_segment(
    plan: &PlanView<'_>,
    rules: GenericRules,
    start: Option<OwnedState>,
    items: &std::sync::mpsc::Receiver<Item>,
) -> Result<OwnedState, JournalError> {
    let mut inputs: Vec<Input> = Vec::new();
    let kernel = Interlocking::new(*plan, rules);
    let mut state = start.unwrap_or_else(|| OwnedState::new(plan));
    let mut out = VecOutputs::default();
    // Abgeschaltete Sicherheitsschicht: Die Ausgaben stammen nicht vom Kern. Abschnitte
    // beginnen nie im abgeschalteten Zustand (Abzüge nur außerhalb).
    let mut tripped = false;
    for item in items {
        match item {
            Item::Safety(SafetyKind::Tripped) => tripped = true,
            Item::Safety(SafetyKind::Reset) => {
                tripped = false;
                state.view().reset();
            }
            Item::Safety(SafetyKind::Excluded | SafetyKind::Reintegrated | SafetyKind::Node) => {}
            Item::Cycle(c) => {
                match &c.delta {
                    Some(d) => d.apply(&mut inputs).map_err(JournalError::Format)?,
                    None => inputs = c.inputs,
                }
                if tripped {
                    continue;
                }
                out.clear();
                // Der sichere Zustand ist ein reguläres, aufgezeichnetes Ergebnis.
                let _ = kernel.step(
                    &mut state.view(),
                    Tick::from_millis(c.now_ms),
                    &inputs,
                    &mut out,
                );
                let diverged = |what: &str| JournalError::Divergence {
                    cycle: c.cycle,
                    what: what.into(),
                };
                if state.meta().cycle() != c.cycle {
                    return Err(diverged("cycle"));
                }
                if outputs_digest(&out.commands, &out.events) != c.outputs {
                    return Err(diverged("outputs"));
                }
                if c.state.as_ref().is_some_and(|s| *s != state_digest(&state)) {
                    return Err(diverged("state"));
                }
            }
            Item::End(snapshot) => {
                if snapshot.state != state {
                    return Err(JournalError::Divergence {
                        cycle: snapshot.cycle,
                        what: "snapshot".into(),
                    });
                }
            }
        }
    }
    Ok(state)
}

/// Verteilt die Abschnitte eines Journals auf Arbeitsfäden.
struct Parallel<'scope, 'env, 'p> {
    scope: &'scope std::thread::Scope<'scope, 'env>,
    plan: &'scope PlanView<'p>,
    digest: String,
    rules: Option<GenericRules>,
    workers: usize,
    /// Laufende Abschnitte und die Meldungen fertiger Abschnitte.
    running: usize,
    done: (std::sync::mpsc::Sender<()>, std::sync::mpsc::Receiver<()>),
    /// Zuführung zum laufenden Abschnitt.
    current: Option<std::sync::mpsc::SyncSender<Item>>,
    /// Der laufende Abschnitt hat noch keinen Zyklus erhalten: Der erste geht mit
    /// vollständigen Eingaben hinaus, spätere als Änderung.
    fresh: bool,
    /// Ergebnisse je Abschnitt in Reihenfolge.
    results: Vec<std::thread::ScopedJoinHandle<'scope, Result<OwnedState, JournalError>>>,
}

impl<'scope, 'p: 'scope> Parallel<'scope, '_, 'p> {
    /// Beginnt einen Abschnitt ab `start`. Laufen schon so viele Abschnitte, wie es
    /// Arbeitsfäden gibt, wird zuerst gewartet, bis einer fertig ist; alle laufenden
    /// Abschnitte sind dann vollständig zugeführt, das Warten endet also.
    fn begin(&mut self, start: Option<OwnedState>) -> Result<(), JournalError> {
        let rules = self
            .rules
            .ok_or_else(|| JournalError::Format("Kopf fehlt".into()))?;
        while self.running >= self.workers {
            self.done
                .1
                .recv()
                .map_err(|_| JournalError::Format("Replay abgebrochen".into()))?;
            self.running = self.running.saturating_sub(1);
        }
        let (tx, rx) = std::sync::mpsc::sync_channel(4096);
        let plan = self.plan;
        let done = self.done.0.clone();
        self.running = self.running.saturating_add(1);
        self.fresh = true;
        self.results.push(self.scope.spawn(move || {
            let result = replay_segment(plan, rules, start, &rx);
            let _ = done.send(());
            result
        }));
        self.current = Some(tx);
        Ok(())
    }

    fn send(&self, item: Item) {
        // Ein Abschnitt, der schon abgebrochen hat, nimmt nichts mehr an; sein Fehler kommt
        // über das Ergebnis.
        if let Some(tx) = &self.current {
            let _ = tx.send(item);
        }
    }
}

impl Visit for Parallel<'_, '_, '_> {
    fn header(&mut self, h: &Header) -> Result<(), JournalError> {
        if h.plan_digest != self.digest {
            return Err(JournalError::Plan(format!(
                "Journal gehört zu Plan {}, geladen ist {}",
                h.plan_digest, self.digest
            )));
        }
        self.rules = Some(h.rules.into());
        self.begin(None)
    }

    fn cycle(&mut self, c: &Cycle, delta: Option<&InputDelta>) -> Result<(), JournalError> {
        // Der Leser hat die Eingaben vollständig zusammengesetzt; als Änderung genügt, was
        // der Abschnitt schon kennt.
        let full = self.fresh || delta.is_none();
        self.fresh = false;
        self.send(Item::Cycle(Cycle {
            cycle: c.cycle,
            now_ms: c.now_ms,
            inputs: if full { c.inputs.clone() } else { Vec::new() },
            delta: if full { None } else { delta.cloned() },
            outputs: c.outputs.clone(),
            state: c.state.clone(),
        }));
        Ok(())
    }

    fn safety(&mut self, record: &SafetyRecord) -> Result<(), JournalError> {
        self.send(Item::Safety(record.kind));
        Ok(())
    }

    fn snapshot(&mut self, snapshot: Box<Snapshot>) -> Result<(), JournalError> {
        if !snapshot.state.fits(self.plan) {
            return Err(JournalError::Format(format!(
                "Zustandsabzug nach Zyklus {} passt nicht zum Plan",
                snapshot.cycle
            )));
        }
        let start = snapshot.state.clone();
        self.send(Item::End(snapshot));
        self.current = None;
        self.begin(Some(start))
    }

    /// Eine Übernahme beendet den Abschnitt ohne Vergleich; der nächste beginnt mit dem
    /// übernommenen Zustand.
    fn resync(&mut self, resync: Box<Resync>) -> Result<(), JournalError> {
        if !resync.state.fits(self.plan) {
            return Err(JournalError::Format(format!(
                "Zustandsübernahme bei Zyklus {} passt nicht zum Plan",
                resync.cycle
            )));
        }
        if resync.state.meta().cycle() != resync.cycle {
            return Err(JournalError::Format(format!(
                "Zustandsübernahme bei Zyklus {}: Zustand nennt Zyklus {}",
                resync.cycle,
                resync.state.meta().cycle()
            )));
        }
        self.current = None;
        self.begin(Some(resync.state))
    }
}

/// Spielt ein Journal gegen `plan` erneut ab und vergleicht Ausgaben und Prüfpunkte
/// (REQ-JRN-002). Liefert den rekonstruierten Endzustand.
///
/// Die Abschnitte zwischen Zustandsabzügen laufen parallel (so viele Arbeitsfäden wie
/// Prozessorkerne); jeder Abschnitt muss genau im nächsten Abzug enden. Gemeldet wird wie
/// beim Abspielen am Stück die früheste Abweichung.
///
/// # Errors
/// [`JournalError`] bei Kettenfehlern, unpassendem Plan oder Abweichungen.
pub fn replay<R: BufRead>(
    reader: R,
    plan: &PlanView<'_>,
) -> Result<(Summary, OwnedState), JournalError> {
    let workers = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    std::thread::scope(|scope| {
        let mut parallel = Parallel {
            scope,
            plan,
            digest: to_hex(&plan.digest()),
            rules: None,
            workers,
            running: 0,
            done: std::sync::mpsc::channel(),
            current: None,
            fresh: true,
            results: Vec::new(),
        };
        let read_result = read(reader, None, &mut parallel);
        parallel.current = None;
        // Die früheste Abweichung zählt; ein Lesefehler liegt hinter allen zugeführten
        // Zyklen.
        let mut last = None;
        for handle in parallel.results {
            let state = handle
                .join()
                .map_err(|_| JournalError::Format("Replay abgebrochen".into()))??;
            last = Some(state);
        }
        let summary = read_result?;
        let state = last.ok_or_else(|| JournalError::Format("leeres Journal".into()))?;
        Ok((summary, state))
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use railit_kernel::{Occupancy, PointReport, Request, RequestId};
    use railit_types::{PointId, RouteId, TvdId};

    fn tvd(i: u32, occupied: bool) -> Input {
        Input::Tvd {
            tvd: TvdId::new(i),
            occupancy: if occupied {
                Occupancy::Occupied
            } else {
                Occupancy::Clear
            },
        }
    }

    /// Jede Folge von Eingabelisten übersteht Kodieren und Anwenden unverändert, ob als
    /// Änderung oder vollständig.
    #[test]
    fn deltas_reconstruct_every_input_list() {
        let mut rng = 0x2545_F491_4F6C_DD1Du64;
        let mut next = move |n: u64| {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng % n
        };
        let mut prev: Vec<Input> = Vec::new();
        let mut resolved: Vec<Input> = Vec::new();
        let mut deltas = 0;
        for _ in 0..2000 {
            let len = 40 + next(3);
            let mut list: Vec<Input> = (0..len)
                .map(|i| tvd(u32::try_from(i).unwrap(), next(20) == 0))
                .collect();
            if next(3) == 0 {
                list.push(Input::Request {
                    id: RequestId(next(1000)),
                    request: Request::SetRoute(RouteId::new(1)),
                });
            }
            if next(5) == 0 {
                list.insert(
                    0,
                    Input::Point {
                        point: PointId::new(0),
                        report: PointReport::Left,
                    },
                );
            }
            match InputDelta::between(&prev, &list) {
                Some(d) => {
                    deltas += 1;
                    d.apply(&mut resolved).unwrap();
                }
                None => resolved.clone_from(&list),
            }
            assert_eq!(resolved, list);
            prev = list;
        }
        assert!(deltas > 1000);
    }

    #[test]
    fn malformed_deltas_are_rejected() {
        let base = vec![tvd(0, false), tvd(1, false)];
        let bad = [
            // Neue Eingabe mit Lücke.
            InputDelta {
                len: 4,
                set: vec![(3, tvd(3, false))],
            },
            // Nicht aufsteigend.
            InputDelta {
                len: 2,
                set: vec![(1, tvd(1, true)), (0, tvd(0, true))],
            },
            // Position außerhalb.
            InputDelta {
                len: 1,
                set: vec![(1, tvd(1, true))],
            },
        ];
        for d in bad {
            assert!(d.apply(&mut base.clone()).is_err(), "{d:?}");
        }
    }
}
