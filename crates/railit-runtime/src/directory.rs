//! Namensauflösung über alle Stellbereiche.

use railit_kernel::AuxOperation;
use railit_plan::PlanView;
use railit_types::{BorderId, LxId, PointId, RouteId, SignalId, TvdId};

use crate::names::{AuxKind, NamedAux};

/// Pläne aller Stellbereiche mit ihren Namen; löst fachliche Namen in (Bereich, ID) auf.
///
/// Element- und Fahrstraßennamen sind im ganzen Netz eindeutig. Grenzen heißen in jedem
/// Bereich nach ihrem Nachbarn (`">Strecke"`); netzweit werden sie mit dem Bereich davor
/// adressiert (`"Adorf>Strecke"`).
#[derive(Clone, Debug)]
pub struct Directory {
    areas: Vec<(String, PlanView<'static>)>,
}

impl Directory {
    /// Verzeichnis über die Bereiche `areas` (Name, Plan).
    pub fn new(areas: Vec<(String, PlanView<'static>)>) -> Self {
        Self { areas }
    }

    /// Anzahl der Bereiche.
    pub fn len(&self) -> usize {
        self.areas.len()
    }

    /// Gibt es keinen Bereich?
    pub fn is_empty(&self) -> bool {
        self.areas.is_empty()
    }

    /// Name und Plan eines Bereichs.
    pub fn area(&self, area: usize) -> Option<(&str, &PlanView<'static>)> {
        self.areas.get(area).map(|(n, p)| (n.as_str(), p))
    }

    /// Alle Bereiche.
    pub fn areas(&self) -> impl Iterator<Item = (&str, &PlanView<'static>)> {
        self.areas.iter().map(|(n, p)| (n.as_str(), p))
    }

    /// Index des Bereichs mit dem Namen `name`.
    pub fn find_area(&self, name: &str) -> Option<usize> {
        self.areas.iter().position(|(n, _)| n == name)
    }

    fn find_in<T>(&self, f: impl Fn(&PlanView<'static>) -> Option<T>) -> Option<(usize, T)> {
        self.areas
            .iter()
            .enumerate()
            .find_map(|(i, (_, p))| f(p).map(|x| (i, x)))
    }

    /// Fahrstraße mit dem Namen `name`.
    pub fn find_route(&self, name: &str) -> Option<(usize, RouteId)> {
        self.find_in(|p| p.find_route(name))
    }

    /// Weiche mit dem Namen `name`.
    pub fn find_point(&self, name: &str) -> Option<(usize, PointId)> {
        self.find_in(|p| p.find_point(name))
    }

    /// Signal mit dem Namen `name`.
    pub fn find_signal(&self, name: &str) -> Option<(usize, SignalId)> {
        self.find_in(|p| p.find_signal(name))
    }

    /// Gleisfreimeldeabschnitt mit dem Namen `name`.
    pub fn find_tvd(&self, name: &str) -> Option<(usize, TvdId)> {
        self.find_in(|p| p.find_tvd(name))
    }

    /// Bahnübergang mit dem Namen `name`.
    pub fn find_lx(&self, name: &str) -> Option<(usize, LxId)> {
        self.find_in(|p| p.find_lx(name))
    }

    /// Grenze mit der Adresse `Bereich>Nachbar` (z. B. `"Adorf>Strecke"`). Bei nur einem
    /// Bereich genügt der Grenzname (`">Strecke"`).
    pub fn find_border(&self, address: &str) -> Option<(usize, BorderId)> {
        let prefixed = self.areas.iter().enumerate().find_map(|(i, (name, plan))| {
            let rest = address.strip_prefix(name.as_str())?;
            plan.find_border(rest).map(|b| (i, b))
        });
        prefixed.or_else(|| {
            (self.areas.len() == 1)
                .then(|| self.find_in(|p| p.find_border(address)))
                .flatten()
        })
    }

    /// Adresse einer Grenze, siehe [`Self::find_border`].
    pub fn border_address(&self, area: usize, border: BorderId) -> Option<String> {
        let (name, plan) = self.areas.get(area)?;
        Some(format!("{name}{}", plan.border_name(border)?))
    }

    /// Löst eine benannte Hilfshandlung auf: Bereich und Hilfshandlung des Kerns.
    ///
    /// # Errors
    /// Beschreibung, wenn das Element in keinem Bereich existiert.
    pub fn resolve_aux(&self, aux: &NamedAux) -> Result<(usize, AuxOperation), String> {
        if aux.kind == AuxKind::TakePermission {
            return self
                .find_border(&aux.element)
                .map(|(a, b)| (a, AuxOperation::TakePermission(b)))
                .ok_or_else(|| format!("unbekannte Grenze „{}“", aux.element));
        }
        let mut last = String::from("kein Stellbereich");
        for (i, (_, plan)) in self.areas.iter().enumerate() {
            match aux.resolve(plan) {
                Ok(op) => return Ok((i, op)),
                Err(e) => last = e,
            }
        }
        Err(last)
    }
}
