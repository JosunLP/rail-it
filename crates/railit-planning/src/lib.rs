//! Projektierung von rail-it.
//!
//! Aus einem validierten Netz ([`railit_topology::Topology`]) entsteht in drei Schritten ein
//! Plan-Image für den Sicherheitskern:
//!
//! 1. [`generate`]: Signalstandorte prüfen, alle Fahrstraßen erzeugen, Feindlichkeiten
//!    bestimmen.
//! 2. [`verify`]: **unabhängige Gegenprüfung** mit einem zweiten, anders aufgebauten
//!    Algorithmus (REQ-PLN-013). Der Generator gilt als nicht sicherheitsrelevant; erst die
//!    Übereinstimmung beider Verfahren macht den Plan vertrauenswürdig.
//! 3. [`compile`]: Plan-Image schreiben (optional signiert), zurücklesen und mit dem Plan
//!    vergleichen.
//!
//! Regeln:
//!
//! - Fahrstraßen beginnen an Haupt-, Mehrabschnittssignalen und ETCS-Halt-Tafeln und enden
//!   am nächsten solchen Signal in Fahrtrichtung oder an einem Netzende (REQ-PLN-011).
//! - Hinter dem Zielsignal liegt ein **Durchrutschweg** bis zur ersten Gleisfreimeldegrenze
//!   nach [`PlanningConfig::overlap_length`] (REQ-PLN-015).
//! - Jede Weiche in Fahrweg und Durchrutschweg erhält **Flankenschutz** durch ein Schutz-
//!   signal oder eine Schutzweiche (REQ-PLN-016).
//! - **Bahnübergänge** im Fahrweg und im Durchrutschweg gehören zur Fahrstraße (REQ-PLN-018).
//! - Die **Geschwindigkeit** ist die kleinste der abzweigend befahrenen Weichen.
//! - **Vorsignale** kündigen das nächste Hauptsignal an (REQ-PLN-017).
//! - **Feindlichkeiten** nach REQ-PLN-012 (siehe `generate`).
//! - **Grenzen** zu Nachbarbereichen sind Fahrstraßenziele; je Grenze wird der
//!   Annäherungsabschnitt bis zum Einfahrsignal samt Durchrutschweg bestimmt (REQ-PLN-019).
//!
//! Netze mit Stellbereichen werden mit [`compile_areas`] je Bereich projektiert.

mod error;
mod generate;
mod verify;

use railit_plan::{Plan, PlanView, SigningKey};
use railit_topology::{AreaCut, Topology};
use railit_types::Length;

pub use error::{PlanningError, PlanningReport};
pub use generate::{generate, generate_with};
pub use verify::{verify, verify_with};

/// Parameter der Projektierung.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PlanningConfig {
    /// Mindestlänge des Durchrutschwegs hinter dem Zielsignal.
    pub overlap_length: Length,
}

impl Default for PlanningConfig {
    /// Durchrutschweg 100 m.
    fn default() -> Self {
        Self {
            overlap_length: Length::from_mm(100_000),
        }
    }
}

/// Ergebnis von [`compile`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Compiled {
    /// Der gegengeprüfte Plan.
    pub plan: Plan,
    /// Das Plan-Image.
    pub image: Vec<u8>,
}

/// Erzeugt, prüft und kompiliert den Plan für `topology`.
///
/// Mit `key` wird das Image signiert (REQ-PLN-004).
///
/// # Errors
/// [`PlanningReport`] mit allen Fehlern aus Generierung, Gegenprüfung oder Kompilierung.
pub fn compile(topology: &Topology, key: Option<&SigningKey>) -> Result<Compiled, PlanningReport> {
    compile_with(topology, key, &PlanningConfig::default())
}

/// Wie [`compile`], mit eigener Konfiguration.
///
/// # Errors
/// Wie [`compile`].
pub fn compile_with(
    topology: &Topology,
    key: Option<&SigningKey>,
    cfg: &PlanningConfig,
) -> Result<Compiled, PlanningReport> {
    let plan = generate_with(topology, cfg)?;
    verify_with(topology, &plan, cfg)?;

    let image = match key {
        Some(k) => plan.to_signed_image(k),
        None => plan.to_image(),
    }
    .map_err(|e| PlanningReport::single(PlanningError::Image(e)))?;

    // Rundreise: Das geschriebene Image muss exakt den geprüften Plan enthalten.
    let view =
        PlanView::parse(&image).map_err(|e| PlanningReport::single(PlanningError::Image(e)))?;
    if Plan::from_view(&view) != plan {
        return Err(PlanningReport::single(PlanningError::Verification(
            "Plan-Image weicht nach dem Zurücklesen vom Plan ab".into(),
        )));
    }
    Ok(Compiled { plan, image })
}

/// Ergebnis von [`compile_areas`] für einen Stellbereich.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledArea {
    /// Name des Bereichs.
    pub name: String,
    /// Teilnetz mit Abbildung auf das Gesamtnetz.
    pub cut: AreaCut,
    /// Plan und Image des Bereichs.
    pub compiled: Compiled,
}

/// Projektiert jeden Stellbereich eines Netzes für sich (REQ-PLN-019).
///
/// # Errors
/// [`PlanningReport`] mit allen Fehlern aller Bereiche; jeder Fehler nennt seinen Bereich.
pub fn compile_areas(
    topology: &Topology,
    key: Option<&SigningKey>,
    cfg: &PlanningConfig,
) -> Result<Vec<CompiledArea>, PlanningReport> {
    if topology.areas().len() == 0 {
        return Err(PlanningReport::single(PlanningError::Inconsistent(
            "das Netz hat keine Stellbereiche".into(),
        )));
    }
    let mut out = Vec::new();
    let mut errors = Vec::new();
    for (id, area) in topology.areas() {
        let wrap = |error: PlanningError| PlanningError::InArea {
            area: area.name.clone(),
            error: Box::new(error),
        };
        let cut = match topology.area_cut(id) {
            Ok(cut) => cut,
            Err(report) => {
                errors.extend(
                    report
                        .errors
                        .iter()
                        .map(|e| wrap(PlanningError::Inconsistent(e.to_string()))),
                );
                continue;
            }
        };
        match compile_with(&cut.topology, key, cfg) {
            Ok(compiled) => out.push(CompiledArea {
                name: area.name.clone(),
                cut,
                compiled,
            }),
            Err(report) => errors.extend(report.errors.into_iter().map(wrap)),
        }
    }
    if errors.is_empty() {
        Ok(out)
    } else {
        Err(PlanningReport { errors })
    }
}
