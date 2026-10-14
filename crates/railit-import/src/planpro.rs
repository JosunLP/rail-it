//! PlanPro (DB `InfraGO`, Versionen 1.9 und 1.10): Import und Export der Topologie und der
//! Außenanlagen eines Stellwerks.
//!
//! PlanPro beschreibt die Anlage als Knoten-Kanten-Modell (`TOP_Knoten`, `TOP_Kante`) mit
//! Punktobjekten auf den Kanten (`Punkt_Objekt_TOP_Kante`: Kante, Abstand vom Knoten A,
//! Wirkrichtung). Alle Werte stehen in Kindelementen `<Wert>`. Übernommen wird:
//!
//! | PlanPro                                         | rail-it                                   |
//! |-------------------------------------------------|-------------------------------------------|
//! | `TOP_Kante` (`TOP_Laenge`, `Kantenname`)         | Gleisabschnitt(e)                         |
//! | `TOP_Knoten` mit Anschlüssen `Spitze`, `Links`, `Rechts` | Weiche                           |
//! | `W_Kr_Gsp_Element` / `W_Kr_Gsp_Komponente`       | Name der Weiche; `Zungenpaar`: `Stammgleis` (gerader Strang), `Geschwindigkeit_L`/`_R` |
//! | `TOP_Knoten` mit einer Kante, Anschluss `Ende`   | Prellbock                                 |
//! | `TOP_Knoten` mit einer Kante, anderer Anschluss  | Netzgrenze (offenes Ende)                 |
//! | `TOP_Knoten` mit zwei Kanten                     | Verbindung zweier Gleisenden              |
//! | `Signal` (`Signal_Real`, `Signal_Art`, `Signalsystem`) | Haupt-, Mehrabschnitts-, Vor- oder Sperrsignal; `Block_Signal` als Selbstblocksignal |
//! | `BUE_Anlage`                                     | Bahnübergang                              |
//! | `FMA_Komponente` (`ID_FMAgrenze`)                | Grenze der Gleisfreimeldung               |
//! | `FMA_Anlage` → `Gleis_Abschnitt` (`Bezeichnung`)  | Name des Gleisfreimeldeabschnitts         |
//!
//! Gelesen werden die Container von `LST_Zustand` und die Zielzustände
//! (`LST_Planung/…/LST_Zustand_Ziel`) aller Ausgaben, nie der Startzustand. Elemente werden
//! über ihren lokalen Namen erkannt, der Namensraum (1.9 oder 1.10) ist gleichgültig. Wie beim
//! railML-Import wird nichts ergänzt oder geraten: Fehlt eine Angabe, die das Netzmodell braucht
//! (Bezeichnung, Wirkrichtung, Signalsystem), ist das ein Fehler mit Fundstelle. Was rail-it
//! nicht abbilden kann (Kreuzungen, fiktive Signale, Gleissperren, Geometrie, Planungsdaten),
//! wird mit Warnung übergangen bzw. als Fehler gemeldet, wenn es die Sicherheit beträfe.
//!
//! Intern wird PlanPro in die Mikrotopologie übersetzt, die auch der railML-Import liest; der
//! Netzaufbau und seine Prüfungen sind daher für beide Formate dieselben. Fundstellen aus dieser
//! Stufe nennen die PlanPro-Kennungen (GUID).
//!
//! Der **Export** schreibt PlanPro 1.10 als `LST_Zustand` mit derselben Teilmenge (ohne
//! Geometrie und Verwaltungsdaten); Kennungen sind aus den Namen abgeleitet und damit stabil.
//! Netze mit Stellbereichen lassen sich nicht ausgeben, Betriebsstellen entfallen mit Warnung.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;

use railit_topology::{Topology, Validated};
use roxmltree::{Document, Node};

use crate::{Diagnostic, ImportError, metres, railml};

/// Namensraum von PlanPro 1.10, in dem [`export`] schreibt.
pub const NAMESPACE: &str = "http://www.plan-pro.org/modell/PlanPro/1.10.0";

/// Ende einer Kante: Index und Ende B (`true`) bzw. A.
type End = (usize, bool);

/// Kante (`TOP_Kante`).
#[derive(Clone, Debug, PartialEq)]
struct Edge {
    id: String,
    name: String,
    length_mm: u32,
}

/// Weiche an einem Knoten.
#[derive(Clone, Debug, Default, PartialEq)]
struct Switch {
    name: Option<String>,
    /// Abzweigender Strang: `Some(true)` links, `Some(false)` rechts.
    diverging_left: Option<bool>,
    speed_kmh: Option<u16>,
}

/// Art eines Knotens.
#[derive(Clone, Debug, PartialEq)]
enum NodeKind {
    /// Spitze, links, rechts.
    Switch([End; 3], Switch),
    Plain(End, End),
    BufferStop(End),
    Boundary(End),
}

#[derive(Clone, Debug, PartialEq)]
struct TopNode {
    id: String,
    name: Option<String>,
    kind: NodeKind,
}

/// Lage eines Punktobjekts: Kante, Abstand vom Knoten A, Wirkrichtung (`true`: in Richtung der
/// Kante).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct Spot {
    edge: usize,
    at_mm: u32,
    forward: Option<bool>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum SigKind {
    Main,
    Combined,
    Distant,
    Shunting,
}

#[derive(Clone, Debug, PartialEq)]
struct Sig {
    id: String,
    name: String,
    spot: Spot,
    kind: SigKind,
    hv: bool,
    auto: bool,
}

#[derive(Clone, Debug, PartialEq)]
struct Named {
    id: String,
    name: String,
    spot: Spot,
}

#[derive(Clone, Debug, PartialEq)]
struct Tvd {
    id: String,
    name: String,
    detectors: Vec<String>,
}

/// Mikrotopologie zwischen PlanPro und railML.
#[derive(Clone, Debug, Default, PartialEq)]
struct Micro {
    name: String,
    edges: Vec<Edge>,
    nodes: Vec<TopNode>,
    signals: Vec<Sig>,
    lxs: Vec<Named>,
    detectors: Vec<Named>,
    tvds: Vec<Tvd>,
}

fn local<'a>(n: &Node<'a, '_>) -> &'a str {
    n.tag_name().name()
}

fn child<'a, 'input>(n: Node<'a, 'input>, name: &str) -> Option<Node<'a, 'input>> {
    n.children().find(|c| c.is_element() && local(c) == name)
}

fn children<'a, 'input: 'a>(
    n: Node<'a, 'input>,
    name: &'a str,
) -> impl Iterator<Item = Node<'a, 'input>> + 'a {
    n.children()
        .filter(move |c| c.is_element() && local(c) == name)
}

/// Wert unter dem Pfad `path` (jeweils erstes Kindelement), gelesen aus `<Wert>`.
fn wert<'a>(n: Node<'a, '_>, path: &[&str]) -> Option<&'a str> {
    let mut at = n;
    for p in path {
        at = child(at, p)?;
    }
    child(at, "Wert")
        .and_then(|w| w.text())
        .map(str::trim)
        .filter(|t| !t.is_empty())
}

fn id_of<'a>(n: Node<'a, '_>) -> Option<&'a str> {
    wert(n, &["Identitaet"])
}

fn short(id: &str) -> &str {
    let start = id.len().saturating_sub(5);
    id.get(start..).unwrap_or(id)
}

fn diag(context: String, message: impl Into<String>) -> Diagnostic {
    Diagnostic {
        context,
        message: message.into(),
        requirement: None,
    }
}

fn mm_of(value: &str) -> Option<u32> {
    value
        .trim()
        .parse::<f64>()
        .ok()
        .and_then(metres)
        .map(railit_topology::Length::as_mm)
}

/// Bezeichnung eines Objekts: `Bezeichnung_Aussenanlage`, sonst `Bezeichnung_Tabelle`.
fn designation(n: Node<'_, '_>) -> Option<String> {
    wert(n, &["Bezeichnung", "Bezeichnung_Aussenanlage"])
        .or_else(|| wert(n, &["Bezeichnung", "Bezeichnung_Tabelle"]))
        .map(str::to_string)
}

struct Reader {
    errors: Vec<Diagnostic>,
    warnings: Vec<String>,
}

/// Container mit den Fachdaten: `LST_Zustand` und die Zielzustände aller Ausgaben.
fn containers<'a, 'input>(root: Node<'a, 'input>) -> Vec<Node<'a, 'input>> {
    let mut out = Vec::new();
    if let Some(c) = child(root, "LST_Zustand").and_then(|z| child(z, "Container")) {
        out.push(c);
    }
    if let Some(fd) = child(root, "LST_Planung").and_then(|p| child(p, "Fachdaten")) {
        for a in children(fd, "Ausgabe_Fachdaten") {
            if let Some(c) = child(a, "LST_Zustand_Ziel").and_then(|z| child(z, "Container")) {
                out.push(c);
            }
        }
    }
    out
}

/// Objekte einer Art aus allen Containern.
fn objects<'a, 'input: 'a>(
    containers: &'a [Node<'a, 'input>],
    name: &'a str,
) -> impl Iterator<Item = Node<'a, 'input>> + 'a {
    containers.iter().flat_map(move |c| children(*c, name))
}

impl Reader {
    fn context(n: Node<'_, '_>) -> String {
        match (designation(n), id_of(n)) {
            (Some(name), _) => format!("{} „{name}“", local(&n)),
            (None, Some(id)) => format!("{} „{id}“", local(&n)),
            (None, None) => local(&n).to_string(),
        }
    }

    /// Lage aus dem ersten `Punkt_Objekt_TOP_Kante`.
    fn spot(
        &mut self,
        n: Node<'_, '_>,
        edges: &HashMap<&str, usize>,
        micro: &Micro,
    ) -> Option<Spot> {
        let ctx = Self::context(n);
        let Some(p) = child(n, "Punkt_Objekt_TOP_Kante") else {
            self.errors.push(diag(
                ctx,
                "ohne Punkt_Objekt_TOP_Kante (Lage auf einer TOP_Kante)",
            ));
            return None;
        };
        if children(n, "Punkt_Objekt_TOP_Kante").count() > 1 {
            self.warnings.push(format!(
                "{ctx}: mehrere Punkt_Objekt_TOP_Kante, übernommen wird die erste"
            ));
        }
        let Some(edge) = wert(p, &["ID_TOP_Kante"]).and_then(|id| edges.get(id).copied()) else {
            self.errors
                .push(diag(ctx, "ID_TOP_Kante verweist auf keine TOP_Kante"));
            return None;
        };
        let length = micro.edges.get(edge).map_or(0, |e| e.length_mm);
        let Some(at_mm) = wert(p, &["Abstand"])
            .and_then(mm_of)
            .filter(|at| *at <= length)
        else {
            self.errors.push(diag(
                ctx,
                "Abstand fehlt oder liegt nicht auf der TOP_Kante (Meter ab Knoten A)",
            ));
            return None;
        };
        let forward = match wert(p, &["Wirkrichtung"]) {
            Some("in") => Some(true),
            Some("gegen") => Some(false),
            _ => None,
        };
        Some(Spot {
            edge,
            at_mm,
            forward,
        })
    }
}

/// Liest PlanPro und validiert das Netz vollständig.
///
/// # Errors
/// [`ImportError::Syntax`] bei ungültigem XML oder fehlendem `PlanPro_Schnittstelle`,
/// [`ImportError::Invalid`] mit allen Fehlern, wenn das beschriebene Netz ungültig ist.
pub fn parse(source: &str) -> Result<Validated, ImportError> {
    parse_with_warnings(source).map(|(v, _)| v)
}

/// Wie [`parse`], gibt zusätzlich die Warnungen des Imports zurück.
///
/// # Errors
/// Wie [`parse`].
pub fn parse_with_warnings(source: &str) -> Result<(Validated, Vec<String>), ImportError> {
    let doc = Document::parse(source).map_err(|e| ImportError::Syntax(e.to_string()))?;
    let root = doc.root_element();
    if local(&root) != "PlanPro_Schnittstelle" {
        return Err(ImportError::Syntax(
            "Wurzelelement ist nicht <PlanPro_Schnittstelle>".into(),
        ));
    }
    let mut r = Reader {
        errors: Vec::new(),
        warnings: Vec::new(),
    };
    let micro = read(&mut r, root);
    if !r.errors.is_empty() {
        return Err(ImportError::Invalid {
            errors: r.errors,
            warnings: r.warnings,
        });
    }
    match railml::parse_with_warnings(&micro.to_railml()) {
        Ok((validated, mut w)) => {
            r.warnings.append(&mut w);
            Ok((validated, r.warnings))
        }
        Err(ImportError::Invalid {
            errors,
            mut warnings,
        }) => {
            r.warnings.append(&mut warnings);
            Err(ImportError::Invalid {
                errors,
                warnings: r.warnings,
            })
        }
        Err(e) => Err(e),
    }
}

// Eine Abbildung je Objektart, der Reihe nach; aufgeteilt wäre sie schwerer zu lesen.
#[allow(clippy::too_many_lines)]
fn read(r: &mut Reader, root: Node<'_, '_>) -> Micro {
    let containers = containers(root);
    let mut micro = Micro {
        name: wert(
            root,
            &["PlanPro_Schnittstelle_Allg", "Bezeichnung_Planung_Projekt"],
        )
        .unwrap_or("PlanPro")
        .to_string(),
        ..Micro::default()
    };
    if containers.is_empty() {
        r.errors.push(diag(
            String::new(),
            "keine Fachdaten (LST_Zustand/Container oder LST_Zustand_Ziel/Container)",
        ));
        return micro;
    }

    // Kanten und ihre Anschlüsse.
    let mut edges: HashMap<&str, usize> = HashMap::new();
    let mut incident: BTreeMap<String, Vec<(End, String)>> = BTreeMap::new();
    let mut node_order: Vec<String> = Vec::new();
    let mut names: HashMap<String, usize> = HashMap::new();
    for n in objects(&containers, "TOP_Knoten") {
        if let Some(id) = id_of(n) {
            node_order.push(id.to_string());
        }
    }
    for n in objects(&containers, "TOP_Kante") {
        let ctx = Reader::context(n);
        let Some(id) = id_of(n) else {
            r.errors.push(diag(ctx, "ohne Identitaet"));
            continue;
        };
        let Some(length_mm) = wert(n, &["TOP_Kante_Allg", "TOP_Laenge"]).and_then(mm_of) else {
            r.errors.push(diag(ctx, "ohne gültige TOP_Laenge"));
            continue;
        };
        let index = micro.edges.len();
        let name = wert(n, &["TOP_Kante_Allg", "Kantenname"])
            .filter(|k| !names.contains_key(*k))
            .map_or_else(|| format!("K{}", index.saturating_add(1)), str::to_string);
        names.insert(name.clone(), index);
        edges.insert(id, index);
        micro.edges.push(Edge {
            id: id.to_string(),
            name,
            length_mm,
        });
        for (end, node, anschluss) in [
            (false, "ID_TOP_Knoten_A", "TOP_Anschluss_A"),
            (true, "ID_TOP_Knoten_B", "TOP_Anschluss_B"),
        ] {
            let Some(node) = wert(n, &[node]) else {
                r.errors.push(diag(ctx.clone(), format!("ohne {node}")));
                continue;
            };
            let kind = wert(n, &["TOP_Kante_Allg", anschluss]).unwrap_or("sonstige");
            incident
                .entry(node.to_string())
                .or_default()
                .push(((index, end), kind.to_string()));
            if !node_order.iter().any(|x| x == node) {
                node_order.push(node.to_string());
            }
        }
    }
    let node_names: HashMap<String, String> = objects(&containers, "TOP_Knoten")
        .filter_map(|n| Some((id_of(n)?.to_string(), wert(n, &["Knotenname"])?.to_string())))
        .collect();

    let switches = read_switches(r, &containers, &edges, &micro);
    for id in node_order {
        let Some(ends) = incident.get(&id) else {
            continue;
        };
        let ctx = format!("TOP_Knoten „{id}“");
        let find = |k: &str| ends.iter().find(|(_, a)| a == k).map(|(e, _)| *e);
        let kind = match ends.as_slice() {
            [(e, a)] if a == "Ende" => NodeKind::BufferStop(*e),
            [(e, a)] if matches!(a.as_str(), "Spitze" | "Links" | "Rechts") => {
                r.errors
                    .push(diag(ctx, format!("Anschluss {a} ohne Weiche")));
                continue;
            }
            [(e, _)] => NodeKind::Boundary(*e),
            [_, _, _] => {
                let (Some(t), Some(l), Some(rr)) = (find("Spitze"), find("Links"), find("Rechts"))
                else {
                    r.errors.push(diag(
                        ctx,
                        "drei Kanten, aber nicht je einmal Spitze, Links und Rechts",
                    ));
                    continue;
                };
                let sw = switches.get(&id).cloned().unwrap_or_default();
                NodeKind::Switch([t, l, rr], sw)
            }
            [(a, ka), (b, kb)] => {
                if [ka, kb]
                    .iter()
                    .any(|k| matches!(k.as_str(), "Spitze" | "Links" | "Rechts"))
                {
                    r.errors.push(diag(
                        ctx,
                        "Weichenanschluss an einem Knoten mit zwei Kanten",
                    ));
                    continue;
                }
                NodeKind::Plain(*a, *b)
            }
            more => {
                r.errors.push(diag(
                    ctx,
                    format!(
                        "{} Kanten an einem Knoten (Kreuzungen werden nicht unterstützt)",
                        more.len()
                    ),
                ));
                continue;
            }
        };
        micro.nodes.push(TopNode {
            name: node_names.get(&id).cloned(),
            id,
            kind,
        });
    }
    // Weichen in der Reihenfolge ihrer W_Kr_Gsp_Element, unbenannte danach.
    let order: Vec<String> = objects(&containers, "W_Kr_Gsp_Element")
        .filter_map(|n| designation(n))
        .collect();
    let rank = |n: &TopNode| match &n.kind {
        NodeKind::Switch(_, s) => s
            .name
            .as_ref()
            .and_then(|name| order.iter().position(|o| o == name))
            .unwrap_or(usize::MAX),
        _ => usize::MAX,
    };
    micro
        .nodes
        .sort_by_key(|n| (!matches!(n.kind, NodeKind::Switch(..)), rank(n)));

    read_signals(r, &containers, &edges, &mut micro);
    for n in objects(&containers, "BUE_Anlage").collect::<Vec<_>>() {
        let ctx = Reader::context(n);
        let Some(spot) = r.spot(n, &edges, &micro) else {
            continue;
        };
        let Some(name) = designation(n) else {
            r.errors.push(diag(ctx, "ohne Bezeichnung"));
            continue;
        };
        micro.lxs.push(Named {
            id: id_of(n).unwrap_or_default().to_string(),
            name,
            spot,
        });
    }
    read_detection(r, &containers, &edges, &mut micro);
    micro
}

/// Weichen je `TOP_Knoten` aus `W_Kr_Gsp_Komponente` und `W_Kr_Gsp_Element`.
fn read_switches(
    r: &mut Reader,
    containers: &[Node<'_, '_>],
    edges: &HashMap<&str, usize>,
    micro: &Micro,
) -> HashMap<String, Switch> {
    let elements: HashMap<&str, Node<'_, '_>> = objects(containers, "W_Kr_Gsp_Element")
        .filter_map(|n| Some((id_of(n)?, n)))
        .collect();
    let mut out = HashMap::new();
    // Knoten der Kanten (A, B) für die Zuordnung der Komponenten.
    let ends: HashMap<&str, (String, String)> = objects(containers, "TOP_Kante")
        .filter_map(|n| {
            Some((
                id_of(n)?,
                (
                    wert(n, &["ID_TOP_Knoten_A"])?.to_string(),
                    wert(n, &["ID_TOP_Knoten_B"])?.to_string(),
                ),
            ))
        })
        .collect();
    for n in objects(containers, "W_Kr_Gsp_Komponente") {
        let ctx = Reader::context(n);
        let element = wert(n, &["ID_W_Kr_Gsp_Element"]).and_then(|id| elements.get(id));
        if child(n, "Kreuzung").is_some() {
            r.errors.push(diag(
                ctx,
                "Kreuzungen und Kreuzungsweichen werden nicht unterstützt",
            ));
            continue;
        }
        if element.is_some_and(|e| child(*e, "Gleissperre_Element").is_some()) {
            r.warnings
                .push(format!("{ctx}: Gleissperre nicht übernommen"));
            continue;
        }
        let Some(zp) = child(n, "Zungenpaar") else {
            continue;
        };
        // Der Knoten der Weiche: Abstand 0 (Knoten A) oder Kantenlänge (Knoten B).
        let node = children(n, "Punkt_Objekt_TOP_Kante").find_map(|p| {
            let edge = wert(p, &["ID_TOP_Kante"])?;
            let length = micro.edges.get(*edges.get(edge)?)?.length_mm;
            let at = wert(p, &["Abstand"]).and_then(mm_of)?;
            let (a, b) = ends.get(edge)?;
            if at == 0 {
                Some(a.clone())
            } else if at == length {
                Some(b.clone())
            } else {
                None
            }
        });
        let Some(node) = node else {
            r.warnings.push(format!(
                "{ctx}: liegt auf keinem TOP_Knoten (Abstand 0 oder Kantenlänge), nicht zugeordnet"
            ));
            continue;
        };
        let diverging_left = match wert(zp, &["Stammgleis"]) {
            Some("links") => Some(false),
            Some("rechts") => Some(true),
            _ => None,
        };
        let speed = |side: &str| {
            wert(zp, &[side])
                .and_then(|v| v.parse::<u16>().ok())
                .filter(|v| *v > 0)
        };
        let speed_kmh = match diverging_left {
            Some(true) => speed("Geschwindigkeit_L"),
            Some(false) => speed("Geschwindigkeit_R"),
            None => None,
        };
        out.insert(
            node,
            Switch {
                name: element.and_then(|e| designation(*e)),
                diverging_left,
                speed_kmh,
            },
        );
    }
    out
}

fn read_signals(
    r: &mut Reader,
    containers: &[Node<'_, '_>],
    edges: &HashMap<&str, usize>,
    micro: &mut Micro,
) {
    for n in objects(containers, "Signal").collect::<Vec<_>>() {
        let ctx = Reader::context(n);
        let Some(real) = child(n, "Signal_Real") else {
            r.warnings
                .push(format!("{ctx}: fiktives Signal, nicht übernommen"));
            continue;
        };
        let Some(schirm) = child(real, "Signal_Real_Aktiv_Schirm") else {
            r.warnings.push(format!(
                "{ctx}: ohne Signal_Real_Aktiv_Schirm (kein Lichtsignal), nicht übernommen"
            ));
            continue;
        };
        let kind = match wert(schirm, &["Signal_Art"]) {
            Some("Hauptsignal" | "Hauptsperrsignal" | "Zugdeckungssignal") => SigKind::Main,
            Some("Mehrabschnittssignal" | "Mehrabschnittssperrsignal") => SigKind::Combined,
            Some("Vorsignal" | "Vorsignalwiederholer") => SigKind::Distant,
            Some("Sperrsignal") => SigKind::Shunting,
            Some(other) => {
                r.warnings
                    .push(format!("{ctx}: Signal_Art „{other}“ nicht übernommen"));
                continue;
            }
            None => {
                r.errors.push(diag(ctx, "ohne Signal_Art"));
                continue;
            }
        };
        let hv = match wert(schirm, &["Signalsystem"]) {
            Some("Ks") => false,
            Some("HV") => true,
            Some(other) => {
                r.errors.push(diag(
                    ctx,
                    format!("Signalsystem „{other}“ wird nicht unterstützt (Ks, HV)"),
                ));
                continue;
            }
            None => {
                r.errors.push(diag(ctx, "ohne Signalsystem"));
                continue;
            }
        };
        let Some(name) = designation(n) else {
            r.errors.push(diag(ctx, "ohne Bezeichnung"));
            continue;
        };
        let Some(spot) = r.spot(n, edges, micro) else {
            continue;
        };
        if spot.forward.is_none() {
            r.errors.push(diag(
                ctx,
                "Wirkrichtung fehlt oder ist „beide“ (in oder gegen)",
            ));
            continue;
        }
        micro.signals.push(Sig {
            id: id_of(n).unwrap_or_default().to_string(),
            name,
            spot,
            kind,
            hv,
            auto: wert(real, &["Signal_Funktion"]) == Some("Block_Signal"),
        });
    }
}

fn read_detection(
    r: &mut Reader,
    containers: &[Node<'_, '_>],
    edges: &HashMap<&str, usize>,
    micro: &mut Micro,
) {
    let mut bounds: HashMap<String, Vec<String>> = HashMap::new();
    for n in objects(containers, "FMA_Komponente").collect::<Vec<_>>() {
        let Some(spot) = r.spot(n, edges, micro) else {
            continue;
        };
        let id = id_of(n).unwrap_or_default().to_string();
        for g in
            children(n, "ID_FMAgrenze").filter_map(|g| g.children().find(|c| local(c) == "Wert"))
        {
            if let Some(t) = g.text() {
                bounds
                    .entry(t.trim().to_string())
                    .or_default()
                    .push(id.clone());
            }
        }
        micro.detectors.push(Named {
            name: designation(n).unwrap_or_else(|| id.clone()),
            id,
            spot,
        });
    }
    let sections: HashMap<&str, Node<'_, '_>> = objects(containers, "Gleis_Abschnitt")
        .filter_map(|n| Some((id_of(n)?, n)))
        .collect();
    for n in objects(containers, "FMA_Anlage") {
        let Some(id) = id_of(n) else {
            continue;
        };
        let name = wert(n, &["ID_Gleis_Abschnitt"])
            .and_then(|g| sections.get(g))
            .and_then(|g| designation(*g))
            .or_else(|| wert(n, &["Bezeichnung", "Bezeichnung_Kennbuchstabe"]).map(str::to_string))
            .unwrap_or_else(|| format!("T-{}", short(id)));
        micro.tvds.push(Tvd {
            id: id.to_string(),
            name,
            detectors: bounds.remove(id).unwrap_or_default(),
        });
    }
    if micro.detectors.is_empty() {
        r.warnings.push(
            "keine FMA_Komponente: jedes Gleis und jede Weiche bildet einen eigenen \
             Gleisfreimeldeabschnitt"
                .into(),
        );
    }
}

fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

fn metre(mm: u32) -> String {
    format!("{}.{:03}", mm / 1000, mm % 1000)
}

impl Micro {
    fn edge_length(&self, e: usize) -> u32 {
        self.edges.get(e).map_or(0, |e| e.length_mm)
    }

    fn spot_railml(&self, s: Spot) -> String {
        let dir = match s.forward {
            Some(true) => " applicationDirection=\"normal\"",
            Some(false) => " applicationDirection=\"reverse\"",
            None => "",
        };
        format!(
            "<spotLocation netElementRef=\"{}\"{dir} pos=\"{}\"/>",
            esc(self.edges.get(s.edge).map_or("", |e| e.id.as_str())),
            metre(s.at_mm)
        )
    }

    fn end_spot(&self, (e, at_b): End) -> Spot {
        Spot {
            edge: e,
            at_mm: if at_b { self.edge_length(e) } else { 0 },
            forward: None,
        }
    }

    /// Mikrotopologie als railML 3 mit den rail-it-Erweiterungen.
    // Ein Abschnitt je Objektart in Dokumentreihenfolge.
    #[allow(clippy::too_many_lines)]
    fn to_railml(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "<railML xmlns=\"{}\" xmlns:railit=\"{}\" version=\"3.3\">",
            railml::NAMESPACE,
            railml::RAILIT_NAMESPACE
        );
        let _ = writeln!(
            out,
            "<infrastructure id=\"is\"><name name=\"{}\"/><topology><netElements>",
            esc(&self.name)
        );
        for e in &self.edges {
            let _ = writeln!(
                out,
                "<netElement id=\"{}\" length=\"{}\"><name name=\"{}\"/></netElement>",
                esc(&e.id),
                metre(e.length_mm),
                esc(&e.name)
            );
        }
        let _ = writeln!(out, "</netElements><netRelations>");
        let mut rel = 0usize;
        let mut relation = |out: &mut String, (a, ea): End, (b, eb): End| {
            rel = rel.saturating_add(1);
            let id = format!("pp-nr{rel}");
            let _ = writeln!(
                out,
                "<netRelation id=\"{id}\" positionOnA=\"{}\" positionOnB=\"{}\" navigability=\"Both\"><elementA ref=\"{}\"/><elementB ref=\"{}\"/></netRelation>",
                u8::from(ea),
                u8::from(eb),
                esc(self.edges.get(a).map_or("", |e| e.id.as_str())),
                esc(self.edges.get(b).map_or("", |e| e.id.as_str())),
            );
            id
        };
        let mut switches = String::new();
        let mut terminals = String::new();
        for (k, n) in self.nodes.iter().enumerate() {
            match &n.kind {
                NodeKind::Plain(a, b) => {
                    relation(&mut out, *a, *b);
                }
                NodeKind::Switch([t, l, r], sw) => {
                    let left = relation(&mut out, *t, *l);
                    let right = relation(&mut out, *t, *r);
                    let name = sw
                        .name
                        .clone()
                        .or_else(|| n.name.clone())
                        .unwrap_or_else(|| format!("W-{}", short(&n.id)));
                    let course = match sw.diverging_left {
                        Some(true) => " continueCourse=\"right\" branchCourse=\"left\"",
                        Some(false) => " continueCourse=\"left\" branchCourse=\"right\"",
                        None => "",
                    };
                    let speed = sw
                        .speed_kmh
                        .map(|s| format!(" branchingSpeed=\"{s}\""))
                        .unwrap_or_default();
                    let (ls, rs) = match sw.diverging_left {
                        Some(true) => (speed.as_str(), ""),
                        Some(false) => ("", speed.as_str()),
                        None => ("", ""),
                    };
                    let _ = writeln!(
                        switches,
                        "<switchIS id=\"pp-sw{k}\" type=\"ordinarySwitch\"{course}><name name=\"{}\"/>{}<leftBranch netRelationRef=\"{left}\"{ls}/><rightBranch netRelationRef=\"{right}\"{rs}/></switchIS>",
                        esc(&name),
                        self.spot_railml(self.end_spot(*t)),
                    );
                }
                NodeKind::BufferStop(e) | NodeKind::Boundary(e) => {
                    let (tag, extra) = if matches!(n.kind, NodeKind::BufferStop(_)) {
                        ("bufferStop", "")
                    } else {
                        ("border", " isOpenEnd=\"true\"")
                    };
                    let name = n
                        .name
                        .clone()
                        .unwrap_or_else(|| format!("E-{}", short(&n.id)));
                    let _ = writeln!(
                        terminals,
                        "<{tag} id=\"pp-t{k}\"{extra}><name name=\"{}\"/>{}</{tag}>",
                        esc(&name),
                        self.spot_railml(self.end_spot(*e))
                    );
                }
            }
        }
        let _ = writeln!(out, "</netRelations></topology><functionalInfrastructure>");
        // Prellböcke und Grenzen gemeinsam in Dokumentreihenfolge.
        let _ = writeln!(out, "<bufferStops>{terminals}</bufferStops>");
        let _ = writeln!(out, "<levelCrossingsIS>");
        for lx in &self.lxs {
            let _ = writeln!(
                out,
                "<levelCrossingIS id=\"{}\"><name name=\"{}\"/>{}</levelCrossingIS>",
                esc(&lx.id),
                esc(&lx.name),
                self.spot_railml(lx.spot)
            );
        }
        let _ = writeln!(out, "</levelCrossingsIS><signalsIS>");
        for s in &self.signals {
            let kind = match s.kind {
                SigKind::Main => "main",
                SigKind::Combined => "combined",
                SigKind::Distant => "distant",
                SigKind::Shunting => "shunting",
            };
            let _ = writeln!(
                out,
                "<signalIS id=\"{}\" isSwitchable=\"true\" railit:kind=\"{kind}\"{}{}><name name=\"{}\"/>{}</signalIS>",
                esc(&s.id),
                if s.hv { " railit:system=\"hv\"" } else { "" },
                if s.auto { " railit:auto=\"true\"" } else { "" },
                esc(&s.name),
                self.spot_railml(s.spot)
            );
        }
        let _ = writeln!(
            out,
            "</signalsIS><switchesIS>{switches}</switchesIS><trainDetectionElements>"
        );
        for d in &self.detectors {
            let _ = writeln!(
                out,
                "<trainDetectionElement id=\"{}\">{}</trainDetectionElement>",
                esc(&d.id),
                self.spot_railml(d.spot)
            );
        }
        let _ = writeln!(
            out,
            "</trainDetectionElements></functionalInfrastructure></infrastructure>"
        );
        let _ = writeln!(
            out,
            "<interlocking><assetsForInterlockings><assetsForInterlocking id=\"afi\"><tvdSections>"
        );
        for t in &self.tvds {
            let _ = write!(
                out,
                "<tvdSection id=\"{}\"><name name=\"{}\"/>",
                esc(&t.id),
                esc(&t.name)
            );
            for d in &t.detectors {
                let _ = write!(out, "<hasDemarcatingTraindetector ref=\"{}\"/>", esc(d));
            }
            let _ = writeln!(out, "</tvdSection>");
        }
        let _ = writeln!(
            out,
            "</tvdSections></assetsForInterlocking></assetsForInterlockings></interlocking></railML>"
        );
        out
    }
}

// ── Export ──────────────────────────────────────────────────────────────────────────────

/// Stabile Kennung (GUID-Form) aus Art und Name.
fn guid(kind: &str, name: &str) -> String {
    let hash = |seed: u64| {
        let mut h = seed;
        for b in kind.bytes().chain([0]).chain(name.bytes()) {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01B3);
        }
        h
    };
    let (a, b) = (hash(0xCBF2_9CE4_8422_2325), hash(0x8422_2325_CBF2_9CE4));
    format!(
        "{:08X}-{:04X}-4{:03X}-8{:03X}-{:012X}",
        a >> 32,
        (a >> 16) & 0xFFFF,
        a & 0x0FFF,
        (b >> 48) & 0x0FFF,
        b & 0xFFFF_FFFF_FFFF
    )
}

fn rattr<'a>(n: Node<'a, '_>, name: &str) -> Option<&'a str> {
    n.attributes()
        .find(|a| a.name() == name && a.namespace().is_none())
        .map(|a| a.value())
}

fn rext<'a>(n: Node<'a, '_>, name: &str) -> Option<&'a str> {
    n.attribute((railml::RAILIT_NAMESPACE, name))
}

fn rname(n: Node<'_, '_>) -> Option<String> {
    child(n, "name")
        .and_then(|c| rattr(c, "name"))
        .map(str::to_string)
}

fn rdesc<'a, 'input: 'a>(
    doc: &'a Document<'input>,
    name: &'a str,
) -> impl Iterator<Item = Node<'a, 'input>> + 'a {
    doc.descendants()
        .filter(move |c| c.is_element() && local(c) == name)
}

impl Micro {
    /// Mikrotopologie aus dem railML-Export von rail-it.
    // Ein Abschnitt je Objektart in Dokumentreihenfolge.
    #[allow(clippy::too_many_lines)]
    fn from_railml(source: &str) -> Result<Self, String> {
        let doc = Document::parse(source).map_err(|e| e.to_string())?;
        let mut m = Self {
            name: rdesc(&doc, "infrastructure")
                .next()
                .and_then(rname)
                .unwrap_or_default(),
            ..Self::default()
        };
        let mut by_id: HashMap<&str, usize> = HashMap::new();
        for n in rdesc(&doc, "netElement") {
            let id = rattr(n, "id").ok_or("netElement ohne id")?;
            let name = rname(n).ok_or("netElement ohne Namen")?;
            by_id.insert(id, m.edges.len());
            m.edges.push(Edge {
                id: guid("TOP_Kante", &name),
                length_mm: rattr(n, "length").and_then(mm_of).ok_or("Länge fehlt")?,
                name,
            });
        }
        let spot = |n: Node<'_, '_>| -> Result<Spot, String> {
            let loc = child(n, "spotLocation").ok_or("spotLocation fehlt")?;
            Ok(Spot {
                edge: rattr(loc, "netElementRef")
                    .and_then(|r| by_id.get(r).copied())
                    .ok_or("netElementRef unbekannt")?,
                at_mm: rattr(loc, "pos").and_then(mm_of).ok_or("pos fehlt")?,
                forward: match rattr(loc, "applicationDirection") {
                    Some("normal") => Some(true),
                    Some("reverse") => Some(false),
                    _ => None,
                },
            })
        };
        let mut relations: BTreeMap<&str, (End, End)> = BTreeMap::new();
        for n in rdesc(&doc, "netRelation") {
            let e = |side: &str, pos: &str| -> Option<End> {
                let el = child(n, side).and_then(|c| rattr(c, "ref"))?;
                Some((*by_id.get(el)?, rattr(n, pos)? == "1"))
            };
            let id = rattr(n, "id").ok_or("netRelation ohne id")?;
            relations.insert(
                id,
                (
                    e("elementA", "positionOnA").ok_or("netRelation unvollständig")?,
                    e("elementB", "positionOnB").ok_or("netRelation unvollständig")?,
                ),
            );
        }
        // Endstellen (Prellböcke, Grenzen) zuerst, in der Reihenfolge des Netzes.
        for n in doc
            .descendants()
            .filter(|n| n.is_element() && matches!(local(n), "bufferStop" | "border"))
        {
            if rext(n, "kind") == Some("border") {
                return Err(
                    "Grenzen zu Nachbarbereichen (Stellbereiche) sind in PlanPro nicht abbildbar"
                        .into(),
                );
            }
            let s = spot(n)?;
            let end = (s.edge, s.at_mm > 0);
            let name = rname(n).ok_or("Endstelle ohne Namen")?;
            m.nodes.push(TopNode {
                id: guid("TOP_Knoten", &name),
                name: Some(name),
                kind: if local(&n) == "bufferStop" {
                    NodeKind::BufferStop(end)
                } else {
                    NodeKind::Boundary(end)
                },
            });
        }
        for n in rdesc(&doc, "switchIS") {
            let name = rname(n).ok_or("Weiche ohne Namen")?;
            let mut rel = |side: &str| {
                child(n, side)
                    .and_then(|c| rattr(c, "netRelationRef"))
                    .and_then(|r| relations.remove(r))
            };
            let (Some((l0, l1)), Some((r0, r1))) = (rel("leftBranch"), rel("rightBranch")) else {
                return Err(format!("Weiche „{name}“: Zweige fehlen"));
            };
            let (tip, left, right) = if l0 == r0 {
                (l0, l1, r1)
            } else if l0 == r1 {
                (l0, l1, r0)
            } else if l1 == r0 {
                (l1, l0, r1)
            } else {
                (l1, l0, r0)
            };
            let diverging_left = match rattr(n, "branchCourse") {
                Some("left") => Some(true),
                Some("right") => Some(false),
                _ => None,
            };
            let side = if diverging_left == Some(true) {
                "leftBranch"
            } else {
                "rightBranch"
            };
            let speed_kmh = child(n, side)
                .and_then(|c| rattr(c, "branchingSpeed"))
                .and_then(|v| v.parse().ok());
            m.nodes.push(TopNode {
                id: guid("TOP_Knoten", &name),
                name: None,
                kind: NodeKind::Switch(
                    [tip, left, right],
                    Switch {
                        name: Some(name),
                        diverging_left,
                        speed_kmh,
                    },
                ),
            });
        }
        for (id, (a, b)) in relations {
            m.nodes.push(TopNode {
                id: guid("TOP_Knoten", id),
                name: None,
                kind: NodeKind::Plain(a, b),
            });
        }
        // Signale; Vorsignale am Mast eines Hauptsignals machen es zum Mehrabschnittssignal.
        let mut parents = Vec::new();
        for n in rdesc(&doc, "signalIS") {
            if let Some(p) = rattr(n, "belongsToParent") {
                parents.push(p.to_string());
            }
        }
        for n in rdesc(&doc, "signalIS") {
            if rattr(n, "belongsToParent").is_some() {
                continue;
            }
            let name = rname(n).ok_or("Signal ohne Namen")?;
            let movement = child(n, "isTrainMovementSignal").and_then(|c| rattr(c, "type"));
            let kind = match (child(n, "isEtcsSignal"), movement) {
                (Some(_), _) => {
                    return Err(format!(
                        "Signal „{name}“: ETCS-Halt-Tafeln sind in PlanPro nicht abbildbar"
                    ));
                }
                (None, Some("distant")) => SigKind::Distant,
                (None, Some("shunting")) => SigKind::Shunting,
                (None, _) if rattr(n, "id").is_some_and(|id| parents.iter().any(|p| p == id)) => {
                    SigKind::Combined
                }
                (None, _) => SigKind::Main,
            };
            m.signals.push(Sig {
                id: guid("Signal", &name),
                spot: spot(n)?,
                kind,
                hv: rext(n, "system") == Some("hv"),
                auto: rext(n, "auto") == Some("true"),
                name,
            });
        }
        for n in rdesc(&doc, "levelCrossingIS") {
            let name = rname(n).ok_or("Bahnübergang ohne Namen")?;
            m.lxs.push(Named {
                id: guid("BUE_Anlage", &name),
                spot: spot(n)?,
                name,
            });
        }
        let mut det_ids: HashMap<&str, String> = HashMap::new();
        for n in rdesc(&doc, "trainDetectionElement") {
            let id = rattr(n, "id").ok_or("Gleisfreimeldeelement ohne id")?;
            let g = guid("FMA_Komponente", id);
            det_ids.insert(id, g.clone());
            m.detectors.push(Named {
                id: g,
                name: id.to_string(),
                spot: spot(n)?,
            });
        }
        for n in rdesc(&doc, "tvdSection") {
            let name = rname(n).ok_or("Abschnitt ohne Namen")?;
            m.tvds.push(Tvd {
                id: guid("FMA_Anlage", &name),
                detectors: children(n, "hasDemarcatingTraindetector")
                    .filter_map(|c| rattr(c, "ref"))
                    .filter_map(|r| det_ids.get(r).cloned())
                    .collect(),
                name,
            });
        }
        Ok(m)
    }

    /// PlanPro 1.10 (`LST_Zustand`).
    // Ein Abschnitt je Objektart in Dokumentreihenfolge; `o` ist die Ausgabe, `w` schreibt
    // einen Wert.
    #[allow(clippy::too_many_lines, clippy::many_single_char_names)]
    fn to_planpro(&self) -> String {
        let mut o = String::new();
        let w = |o: &mut String, ind: usize, tag: &str, value: &str| {
            let _ = writeln!(o, "{:ind$}<{tag}><Wert>{}</Wert></{tag}>", "", esc(value));
        };
        let open = |o: &mut String, ind: usize, tag: &str, id: &str| {
            let _ = writeln!(
                o,
                "{:ind$}<{tag}>\n{:ind2$}<Identitaet><Wert>{id}</Wert></Identitaet>",
                "",
                "",
                ind2 = ind + 2
            );
        };
        let close = |o: &mut String, ind: usize, tag: &str| {
            let _ = writeln!(o, "{:ind$}</{tag}>", "");
        };
        let _ = writeln!(o, "<?xml version=\"1.0\" encoding=\"UTF-8\"?>");
        let _ = writeln!(
            o,
            "<nsPlanPro:PlanPro_Schnittstelle xmlns:nsPlanPro=\"{NAMESPACE}\">"
        );
        w(
            &mut o,
            2,
            "Identitaet",
            &guid("PlanPro_Schnittstelle", &self.name),
        );
        let _ = writeln!(o, "  <PlanPro_Schnittstelle_Allg>");
        w(&mut o, 4, "Bezeichnung_Planung_Projekt", &self.name);
        w(&mut o, 4, "Werkzeug_Name", "rail-it");
        w(&mut o, 4, "Werkzeug_Version", env!("CARGO_PKG_VERSION"));
        let _ = writeln!(o, "  </PlanPro_Schnittstelle_Allg>");
        let _ = writeln!(o, "  <LST_Zustand>");
        w(&mut o, 4, "Identitaet", &guid("LST_Zustand", &self.name));
        let _ = writeln!(o, "    <Container>");
        let ind = 6;
        let edge_id = |e: usize| self.edges.get(e).map_or("", |e| e.id.as_str());
        // Knoten je Kantenende.
        let mut at: HashMap<End, (&str, &'static str)> = HashMap::new();
        for n in &self.nodes {
            match &n.kind {
                NodeKind::Switch([t, l, r], _) => {
                    at.insert(*t, (&n.id, "Spitze"));
                    at.insert(*l, (&n.id, "Links"));
                    at.insert(*r, (&n.id, "Rechts"));
                }
                NodeKind::Plain(a, b) => {
                    at.insert(*a, (&n.id, "Verbindung"));
                    at.insert(*b, (&n.id, "Verbindung"));
                }
                NodeKind::BufferStop(e) => {
                    at.insert(*e, (&n.id, "Ende"));
                }
                NodeKind::Boundary(e) => {
                    at.insert(*e, (&n.id, "Ende_Bestdig"));
                }
            }
        }
        for n in &self.nodes {
            open(&mut o, ind, "TOP_Knoten", &n.id);
            w(&mut o, ind + 2, "ID_GEO_Knoten", &guid("GEO_Knoten", &n.id));
            if let Some(name) = &n.name {
                w(&mut o, ind + 2, "Knotenname", name);
            }
            close(&mut o, ind, "TOP_Knoten");
        }
        for (i, e) in self.edges.iter().enumerate() {
            open(&mut o, ind, "TOP_Kante", &e.id);
            let (na, aa) = at.get(&(i, false)).copied().unwrap_or(("", "sonstige"));
            let (nb, ab) = at.get(&(i, true)).copied().unwrap_or(("", "sonstige"));
            w(&mut o, ind + 2, "ID_TOP_Knoten_A", na);
            w(&mut o, ind + 2, "ID_TOP_Knoten_B", nb);
            let _ = writeln!(o, "{:i$}<TOP_Kante_Allg>", "", i = ind + 2);
            w(&mut o, ind + 4, "Kantenname", &e.name);
            w(&mut o, ind + 4, "TOP_Anschluss_A", aa);
            w(&mut o, ind + 4, "TOP_Anschluss_B", ab);
            w(&mut o, ind + 4, "TOP_Laenge", &metre(e.length_mm));
            let _ = writeln!(o, "{:i$}</TOP_Kante_Allg>", "", i = ind + 2);
            close(&mut o, ind, "TOP_Kante");
        }
        let punkt = |o: &mut String, s: Spot| {
            let _ = writeln!(o, "{:i$}<Punkt_Objekt_TOP_Kante>", "", i = ind + 2);
            w(o, ind + 4, "Abstand", &metre(s.at_mm));
            w(o, ind + 4, "ID_TOP_Kante", edge_id(s.edge));
            if let Some(f) = s.forward {
                w(o, ind + 4, "Wirkrichtung", if f { "in" } else { "gegen" });
            }
            let _ = writeln!(o, "{:i$}</Punkt_Objekt_TOP_Kante>", "", i = ind + 2);
        };
        let bez = |o: &mut String, name: &str| {
            let _ = writeln!(o, "{:i$}<Bezeichnung>", "", i = ind + 2);
            w(o, ind + 4, "Bezeichnung_Aussenanlage", name);
            let _ = writeln!(o, "{:i$}</Bezeichnung>", "", i = ind + 2);
        };
        for n in &self.nodes {
            let NodeKind::Switch([t, _, _], sw) = &n.kind else {
                continue;
            };
            let name = sw.name.clone().unwrap_or_default();
            let element = guid("W_Kr_Gsp_Element", &name);
            open(&mut o, ind, "W_Kr_Gsp_Element", &element);
            bez(&mut o, &name);
            let _ = writeln!(o, "{:i$}<Weiche_Element/>", "", i = ind + 2);
            close(&mut o, ind, "W_Kr_Gsp_Element");
            open(
                &mut o,
                ind,
                "W_Kr_Gsp_Komponente",
                &guid("W_Kr_Gsp_Komponente", &name),
            );
            punkt(&mut o, self.end_spot(*t));
            w(&mut o, ind + 2, "ID_W_Kr_Gsp_Element", &element);
            let _ = writeln!(o, "{:i$}<Zungenpaar>", "", i = ind + 2);
            if let Some(left) = sw.diverging_left {
                if let Some(s) = sw.speed_kmh {
                    let side = if left {
                        "Geschwindigkeit_L"
                    } else {
                        "Geschwindigkeit_R"
                    };
                    w(&mut o, ind + 4, side, &s.to_string());
                }
                w(
                    &mut o,
                    ind + 4,
                    "Stammgleis",
                    if left { "rechts" } else { "links" },
                );
            }
            let _ = writeln!(o, "{:i$}</Zungenpaar>", "", i = ind + 2);
            close(&mut o, ind, "W_Kr_Gsp_Komponente");
        }
        for s in &self.signals {
            open(&mut o, ind, "Signal", &s.id);
            punkt(&mut o, s.spot);
            bez(&mut o, &s.name);
            let _ = writeln!(o, "{:i$}<Signal_Real>", "", i = ind + 2);
            if s.auto {
                w(&mut o, ind + 4, "Signal_Funktion", "Block_Signal");
            }
            let _ = writeln!(o, "{:i$}<Signal_Real_Aktiv_Schirm>", "", i = ind + 4);
            let art = match s.kind {
                SigKind::Main => "Hauptsignal",
                SigKind::Combined => "Mehrabschnittssignal",
                SigKind::Distant => "Vorsignal",
                SigKind::Shunting => "Sperrsignal",
            };
            w(&mut o, ind + 6, "Signal_Art", art);
            w(
                &mut o,
                ind + 6,
                "Signalsystem",
                if s.hv { "HV" } else { "Ks" },
            );
            let _ = writeln!(o, "{:i$}</Signal_Real_Aktiv_Schirm>", "", i = ind + 4);
            let _ = writeln!(o, "{:i$}</Signal_Real>", "", i = ind + 2);
            close(&mut o, ind, "Signal");
        }
        for lx in &self.lxs {
            open(&mut o, ind, "BUE_Anlage", &lx.id);
            punkt(&mut o, lx.spot);
            bez(&mut o, &lx.name);
            close(&mut o, ind, "BUE_Anlage");
        }
        for d in &self.detectors {
            open(&mut o, ind, "FMA_Komponente", &d.id);
            punkt(&mut o, d.spot);
            for t in self.tvds.iter().filter(|t| t.detectors.contains(&d.id)) {
                w(&mut o, ind + 2, "ID_FMAgrenze", &t.id);
            }
            close(&mut o, ind, "FMA_Komponente");
        }
        for t in &self.tvds {
            let section = guid("Gleis_Abschnitt", &t.name);
            open(&mut o, ind, "FMA_Anlage", &t.id);
            w(&mut o, ind + 2, "ID_Gleis_Abschnitt", &section);
            close(&mut o, ind, "FMA_Anlage");
            open(&mut o, ind, "Gleis_Abschnitt", &section);
            bez(&mut o, &t.name);
            close(&mut o, ind, "Gleis_Abschnitt");
        }
        let _ = writeln!(o, "    </Container>");
        let _ = writeln!(o, "  </LST_Zustand>");
        let _ = writeln!(o, "</nsPlanPro:PlanPro_Schnittstelle>");
        o
    }
}

/// Schreibt ein Netz als PlanPro 1.10. Liefert das Dokument und Warnungen (entfallene
/// Betriebsstellen).
///
/// # Errors
/// Beschreibung, wenn das Netz Stellbereiche, Grenzen zu Nachbarbereichen oder
/// ETCS-Halt-Tafeln hat, die PlanPro hier nicht abbildet.
pub fn export(net: &Topology) -> Result<(String, Vec<String>), String> {
    if net.areas().len() > 0 {
        return Err("Netze mit Stellbereichen lassen sich nicht nach PlanPro ausgeben".into());
    }
    let mut warnings = Vec::new();
    if net.stations().len() > 0 {
        warnings.push(format!(
            "{} Betriebsstellen sind in PlanPro nicht abgebildet und entfallen",
            net.stations().len()
        ));
    }
    let micro = Micro::from_railml(&railml::export(net)?)?;
    Ok((micro.to_planpro(), warnings))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guids_are_stable_and_distinct() {
        let a = guid("Signal", "A");
        assert_eq!(a, guid("Signal", "A"));
        assert_ne!(a, guid("Signal", "B"));
        assert_ne!(a, guid("TOP_Kante", "A"));
        assert_eq!(a.len(), 36);
        assert_eq!(a.as_bytes()[14], b'4');
    }

    #[test]
    fn metres_have_three_decimals() {
        assert_eq!(metre(1_000_000), "1000.000");
        assert_eq!(metre(5), "0.005");
    }
}
