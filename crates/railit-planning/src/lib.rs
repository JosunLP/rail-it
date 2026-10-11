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
//! Fahrstraßenregeln in Phase 1:
//!
//! - Fahrstraßen beginnen an Haupt-, Mehrabschnittssignalen und ETCS-Halt-Tafeln.
//! - Sie enden am nächsten solchen Signal in Fahrtrichtung oder an einem Netzende.
//! - Alle Wege über Weichen werden erzeugt; jede Fahrstraße nennt ihre Weichenlagen.
//! - Zwei Fahrstraßen sind feindlich, wenn sie dasselbe Startsignal, einen
//!   Gleisfreimeldeabschnitt oder eine Weiche gemeinsam haben (REQ-PLN-012).
//!
//! Durchrutschwege und Flankenschutz folgen in Phase 2.

mod error;
mod generate;
mod verify;

use railit_plan::{Plan, PlanView, SigningKey};
use railit_topology::Topology;

pub use error::{PlanningError, PlanningReport};
pub use generate::generate;
pub use verify::verify;

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
    let plan = generate(topology)?;
    verify(topology, &plan)?;

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
