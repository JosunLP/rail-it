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
//! - `hash = SHA-256(prev ‖ JSON(entry))`, `prev` ist der Hash der Vorgängerzeile (beim Kopf
//!   32 Nullbytes). Wird eine Zeile verändert, entfernt oder eingefügt, bricht die Kette
//!   (REQ-JRN-001).
//!
//! [`verify`] prüft die Kette, [`replay`] spielt das Journal gegen einen Plan erneut ab und
//! vergleicht Ausgaben und Prüfpunkte (REQ-JRN-002). Der Zustands-Hash beruht auf der
//! `Hash`-Implementierung des Kernzustands; Prüfpunkte sind daher nur zwischen Programmen
//! gleicher Version und Plattform vergleichbar. Die Ausgaben-Hashes sind plattformunabhängig.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::io::{self, BufRead, Write};

use railit_kernel::{Command, Event, Input, Interlocking, OwnedState, VecOutputs};
use railit_plan::PlanView;
use railit_rules::GenericRules;
use railit_types::{Duration, Tick};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Kennung des Formats im Kopf.
pub const FORMAT: &str = "railit-journal";
/// Version des Formats.
pub const VERSION: u32 = 1;

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
        }
    }
}

/// Eintrag eines Zyklus.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cycle {
    /// Zyklusnummer des Kerns nach dem Zyklus.
    pub cycle: u64,
    /// Zeitpunkt in Millisekunden.
    pub now_ms: u64,
    /// Eingaben des Kerns.
    pub inputs: Vec<Input>,
    /// SHA-256 der Ausgaben (Hex), siehe [`outputs_digest`].
    pub outputs: String,
    /// Hash des Kernzustands nach dem Zyklus (nur an Prüfpunkten), siehe [`state_digest`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
}

/// Inhalt einer Journalzeile.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Entry {
    /// Kopf.
    Header(Header),
    /// Zyklus.
    Cycle(Cycle),
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Line {
    entry: Entry,
    prev: String,
    hash: String,
}

fn chain(prev: &Hash32, entry: &Entry) -> io::Result<Hash32> {
    let json = serde_json::to_vec(entry).map_err(io::Error::other)?;
    let mut h = Sha256::new();
    h.update(prev);
    h.update(&json);
    Ok(h.finalize().into())
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

/// Schreibt ein Journal.
#[derive(Debug)]
pub struct JournalWriter<W: Write> {
    out: W,
    prev: Hash32,
    checkpoint_every: u64,
}

impl<W: Write> JournalWriter<W> {
    /// Beginnt ein Journal mit `header`.
    ///
    /// # Errors
    /// Schreibfehler.
    pub fn new(out: W, header: Header) -> io::Result<Self> {
        let checkpoint_every = header.checkpoint_every;
        let mut w = Self {
            out,
            prev: [0; 32],
            checkpoint_every,
        };
        w.write(&Entry::Header(header))?;
        Ok(w)
    }

    /// Schreibt einen Eintrag unverändert in die Kette (für Werkzeuge, die Journale
    /// umschreiben, und für Tests). Die Kette ist danach gültig; ob der Inhalt zum Plan
    /// passt, zeigt erst [`replay`].
    ///
    /// # Errors
    /// Schreibfehler.
    pub fn append(&mut self, entry: &Entry) -> io::Result<()> {
        self.write(entry)
    }

    fn write(&mut self, entry: &Entry) -> io::Result<()> {
        let hash = chain(&self.prev, entry)?;
        let line = Line {
            entry: entry.clone(),
            prev: to_hex(&self.prev),
            hash: to_hex(&hash),
        };
        serde_json::to_writer(&mut self.out, &line).map_err(io::Error::other)?;
        self.out.write_all(b"\n")?;
        self.prev = hash;
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
        let checkpoint = self.checkpoint_every > 0 && cycle.is_multiple_of(self.checkpoint_every);
        self.write(&Entry::Cycle(Cycle {
            cycle,
            now_ms: now.as_millis(),
            inputs: inputs.to_vec(),
            outputs: outputs_digest(&out.commands, &out.events),
            state: checkpoint.then(|| state_digest(state)),
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
    /// Zeitpunkt des letzten Zyklus in Millisekunden.
    pub last_ms: u64,
    /// Hash der letzten vollständigen Zeile (Hex).
    pub head: String,
    /// Die letzte Zeile war unvollständig (Abbruch beim Schreiben) und wurde ignoriert.
    pub truncated: bool,
}

/// Liest ein Journal und prüft dabei die Hashkette. `on_cycle` erhält jeden Zyklus.
fn read<R: BufRead>(
    reader: R,
    mut on_header: impl FnMut(&Header) -> Result<(), JournalError>,
    mut on_cycle: impl FnMut(&Cycle) -> Result<(), JournalError>,
) -> Result<Summary, JournalError> {
    let mut prev: Hash32 = [0; 32];
    let mut header = None;
    let mut cycles = 0u64;
    let mut checkpoints = 0u64;
    let mut last_ms = 0u64;
    let mut truncated = false;
    let mut lines = reader.lines().enumerate().peekable();
    while let Some((index, line)) = lines.next() {
        let number = index.saturating_add(1);
        let text = line.map_err(|e| JournalError::Io(e.to_string()))?;
        if text.trim().is_empty() {
            continue;
        }
        let parsed: Line = match serde_json::from_str(&text) {
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
        let hash = chain(&prev, &parsed.entry).map_err(|e| JournalError::Io(e.to_string()))?;
        if from_hex32(&parsed.prev) != Some(prev) || from_hex32(&parsed.hash) != Some(hash) {
            return Err(JournalError::Chain { line: number });
        }
        prev = hash;
        match (&parsed.entry, &header) {
            (Entry::Header(h), None) => {
                if h.format != FORMAT || h.version != VERSION {
                    return Err(JournalError::Format(format!(
                        "unbekanntes Format {} v{}",
                        h.format, h.version
                    )));
                }
                on_header(h)?;
                header = Some(h.clone());
            }
            (Entry::Header(_), Some(_)) => {
                return Err(JournalError::Format(format!(
                    "Zeile {number}: zweiter Kopf"
                )));
            }
            (Entry::Cycle(_), None) => {
                return Err(JournalError::Format("Kopf fehlt".into()));
            }
            (Entry::Cycle(c), Some(_)) => {
                on_cycle(c)?;
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
        last_ms,
        head: to_hex(&prev),
        truncated,
    })
}

/// Prüft die Hashkette eines Journals (REQ-JRN-001).
///
/// # Errors
/// [`JournalError`] bei Lese-, Format- oder Kettenfehlern.
pub fn verify<R: BufRead>(reader: R) -> Result<Summary, JournalError> {
    read(reader, |_| Ok(()), |_| Ok(()))
}

/// Spielt ein Journal gegen `plan` erneut ab und vergleicht Ausgaben und Prüfpunkte
/// (REQ-JRN-002). Liefert den rekonstruierten Endzustand.
///
/// # Errors
/// [`JournalError`] bei Kettenfehlern, unpassendem Plan oder Abweichungen.
pub fn replay<R: BufRead>(
    reader: R,
    plan: &PlanView<'_>,
) -> Result<(Summary, OwnedState), JournalError> {
    let mut state = OwnedState::new(plan);
    let mut out = VecOutputs::default();
    let kernel: std::cell::OnceCell<Interlocking<'_, GenericRules>> = std::cell::OnceCell::new();
    let digest = to_hex(&plan.digest());
    let summary = read(
        reader,
        |h| {
            if h.plan_digest != digest {
                return Err(JournalError::Plan(format!(
                    "Journal gehört zu Plan {}, geladen ist {digest}",
                    h.plan_digest
                )));
            }
            kernel
                .set(Interlocking::new(*plan, h.rules.into()))
                .map_err(|_| JournalError::Format("zweiter Kopf".into()))
        },
        |c| {
            let Some(k) = kernel.get() else {
                return Err(JournalError::Format("Kopf fehlt".into()));
            };
            out.clear();
            // Der sichere Zustand ist ein reguläres, aufgezeichnetes Ergebnis.
            let _ = k.step(
                &mut state.view(),
                Tick::from_millis(c.now_ms),
                &c.inputs,
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
            Ok(())
        },
    )?;
    Ok((summary, state))
}
