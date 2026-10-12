//! Laufzeit von rail-it.
//!
//! - [`Runtime`]: koppelt Sicherheitskern und Feldebene ([`FieldAdapter`]) im festen Zyklus:
//!   Feld fortschreiben → Meldungen sammeln → Kern → Stellbefehle ans Feld.
//! - [`Snapshot`] und [`NamedEvent`]: Zustand und Ereignisse mit fachlichen Namen, für API,
//!   CLI und Protokoll.
//! - [`load_plan`]: Plan-Image laden und prüfen (optional mit Signatur).
//! - [`scenario`]: deklarative Betriebsszenarien ausführen.
//! - [`keys`]: Schlüssel für signierte Plan-Images.
//!
//! Jeder Stellbereich läuft über die Sicherheitsschicht `railit-safety`: ein, zwei oder drei
//! Kanäle mit Voter und optionalem Watchdog ([`SafetyConfig`]). Dies ist die Server-Variante
//! (`std`); die Embedded-Variante ist `railit-embedded` (Konzept, Abschnitt 13).

mod directory;
mod field;
pub mod keys;
mod names;
mod runtime;
pub mod scenario;

pub use directory::Directory;
pub use field::{DynField, FieldAdapter, SimulatedField};
pub use names::{
    AreaSnapshot, AuxKind, AuxSnapshot, BorderSnapshot, LxSnapshot, NamedAux, NamedEvent,
    PointSnapshot, RouteSnapshot, SignalSnapshot, Snapshot, TvdSnapshot,
};
pub use names::{ChannelSnapshot, SafetySnapshot};
pub use railit_adapters;
pub use railit_etcs;
pub use railit_ops;
pub use railit_safety;
pub use runtime::{
    Addressed, AreaEvent, AreaSafetyEvent, CycleOutput, OPS_REQUEST_BASE, Runtime, SafetyConfig,
};

use railit_plan::{PlanError, PlanView, VerifyingKey};
use railit_topology::Topology;

/// Lädt ein Plan-Image für die gesamte Laufzeit des Prozesses.
///
/// Mit `key` wird eine gültige Signatur verlangt (REQ-PLN-004). Das Image wird bewusst im
/// Speicher belassen (`'static`), weil der Kern es während des gesamten Betriebs liest.
///
/// # Errors
/// [`PlanError`], wenn das Image ungültig ist oder die Signatur fehlt bzw. falsch ist.
pub fn load_plan(
    bytes: Vec<u8>,
    key: Option<&VerifyingKey>,
) -> Result<PlanView<'static>, PlanError> {
    // Erst prüfen, dann dauerhaft ablegen: Ungültige Images belegen keinen Speicher.
    match key {
        Some(k) => PlanView::parse_signed(&bytes, k)?,
        None => PlanView::parse(&bytes)?,
    };
    let image: &'static [u8] = Box::leak(bytes.into_boxed_slice());
    PlanView::parse(image)
}

/// Projektiert ein Netz (je Stellbereich, falls es welche hat) und baut eine Runtime mit
/// dem Simulator für Szenarien, Tests und Ausbildung. Die Pläne bleiben unsigniert.
///
/// # Errors
/// Beschreibung, wenn die Projektierung fehlschlägt.
pub fn simulation(
    net: Topology,
    rules: railit_rules::GenericRules,
    sim: railit_sim::SimConfig,
    cycle: railit_types::Duration,
) -> Result<Runtime<railit_sim::Simulator>, String> {
    simulation_with(net, rules, sim, cycle, Ok)
}

/// Wie [`simulation`], das Feld um den Simulator legt `field` (z. B. EULYNX über RaSTA).
///
/// # Errors
/// Beschreibung, wenn die Projektierung oder der Aufbau des Felds fehlschlägt.
pub fn simulation_with<F: FieldAdapter>(
    net: Topology,
    rules: railit_rules::GenericRules,
    sim: railit_sim::SimConfig,
    cycle: railit_types::Duration,
    field: impl FnOnce(railit_sim::Simulator) -> Result<F, String>,
) -> Result<Runtime<F>, String> {
    if net.areas().len() == 0 {
        let compiled = railit_planning::compile(&net, None).map_err(|e| e.to_string())?;
        let plan = load_plan(compiled.image, None).map_err(|e| e.to_string())?;
        let field = field(railit_sim::Simulator::new(net, sim))?;
        return Ok(Runtime::new(plan, rules, field, cycle));
    }
    let compiled =
        railit_planning::compile_areas(&net, None, &railit_planning::PlanningConfig::default())
            .map_err(|e| e.to_string())?;
    let mut areas = Vec::with_capacity(compiled.len());
    for a in compiled {
        let plan = load_plan(a.compiled.image, None).map_err(|e| e.to_string())?;
        areas.push((plan, a.cut));
    }
    let field = field(railit_sim::Simulator::new(net.clone(), sim))?;
    Runtime::for_network(&net, areas, rules, field, cycle)
}

/// Prüft, ob Plan und Netz zusammengehören (gleiche Elemente in gleicher Reihenfolge).
///
/// Der Simulator arbeitet auf dem Netz, der Kern auf dem Plan; beide verwenden dieselben
/// IDs. Passen sie nicht zusammen, darf der Betrieb nicht starten.
///
/// # Errors
/// Beschreibung der ersten Abweichung.
pub fn check_correspondence(plan: &PlanView<'_>, net: &Topology) -> Result<(), String> {
    if plan.network_name() != net.name() {
        return Err(format!(
            "Plan „{}“ gehört nicht zum Netz „{}“",
            plan.network_name(),
            net.name()
        ));
    }
    let points = net
        .points()
        .map(|(id, p)| (plan.point_name(id), p.name.as_str()));
    let signals = net
        .signals()
        .map(|(id, s)| (plan.signal_name(id), s.name.as_str()));
    let tvds = net
        .tvds()
        .map(|(id, t)| (plan.tvd_name(id), t.name.as_str()));
    if plan.point_count() != net.points().len()
        || plan.signal_count() != net.signals().len()
        || plan.tvd_count() != net.tvds().len()
    {
        return Err("Anzahl der Elemente in Plan und Netz verschieden".into());
    }
    for (in_plan, in_net) in points.chain(signals).chain(tvds) {
        if in_plan != Some(in_net) {
            return Err(format!(
                "Element „{in_net}“ fehlt im Plan oder steht an anderer Stelle"
            ));
        }
    }
    Ok(())
}
