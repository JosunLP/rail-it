//! railML 3: Import und Export der Infrastruktur.
//!
//! Übernommen wird die Mikrotopologie mit den Elementen, die das Netzmodell kennt:
//!
//! | railML 3                                   | rail-it                                   |
//! |--------------------------------------------|-------------------------------------------|
//! | `netElement` (Mikroebene, mit `length`)    | Gleisabschnitt(e)                         |
//! | `netRelation` (befahrbar)                  | Verbindung zweier Gleisenden              |
//! | `switchIS` (`ordinarySwitch`)              | Weiche; `branchCourse` = abzweigender Strang, `branchingSpeed` |
//! | `bufferStop`                               | Prellbock                                 |
//! | `border` (`isOpenEnd`)                     | Netzgrenze                                |
//! | `trainDetectionElement`                    | Grenze der Gleisfreimeldung; teilt das Gleis |
//! | `tvdSection` (`hasDemarcatingTraindetector`) | Name des Gleisfreimeldeabschnitts       |
//! | `signalIS` (`isTrainMovementSignal`)       | Haupt-, Vor- oder Rangiersignal           |
//! | `signalIS` (`isEtcsSignal`)                | ETCS-Halt-Tafel                           |
//! | `signalIS` mit `belongsToParent`           | Vorsignal am Mast: Mehrabschnittssignal   |
//! | `levelCrossingIS`                          | Bahnübergang                              |
//! | `crossing` (`simpleCrossing`, zwei `spotLocation`) | Kreuzung ohne bewegliche Teile    |
//!
//! Gleisfreimeldeabschnitte ergeben sich aus der Topologie: Zusammenhängende Gleise und
//! Weichen zwischen Gleisfreimeldeeinrichtungen bilden einen Abschnitt. Liegt eine Einrichtung
//! innerhalb eines `netElement`, wird es dort in zwei Gleisabschnitte geteilt (`Name/1`,
//! `Name/2`, …). Den Namen übernimmt der Abschnitt vom `tvdSection` mit denselben
//! begrenzenden Einrichtungen, sonst heißt er `T-<erstes Gleis>`.
//!
//! Was railML nicht ausdrückt, steht in Attributen und Elementen des Namensraums
//! [`RAILIT_NAMESPACE`]: Signalsystem (`railit:system="hv"`), Selbstblocksignale
//! (`railit:auto="true"`), Grenzen zu Nachbarbereichen (`railit:kind="border"`,
//! `railit:initial`), Stellbereiche, Betriebsstellen, Kreuzungsweichen (`railit:slip`, aus
//! Weichen, Gleisen und Kreuzung nach den Namen von `TopologyBuilder::add_slip`) und
//! gekoppelte Weichen (`railit:coupling`) in `railit:operation`.
//!
//! Elemente werden über ihren lokalen Namen erkannt; der railML-Namensraum (3.2 oder 3.3) ist
//! daher gleichgültig.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;

use roxmltree::{Document, Node};

use railit_topology::{
    DEFAULT_DIVERGING_SPEED, Direction, ElementRef, Length, NodeRef, Point, PointId, PointPosition,
    Port, PortRef, SignalKind, SignalSystem, SlipKind, Speed, TerminalKind, Topology,
    TopologyBuilder, TrackId, TvdId, TvdMember, Validated,
};

use crate::{Diagnostic, ImportError, metres, model_error};

/// Namensraum von railML 3.3, in dem [`export`] schreibt.
pub const NAMESPACE: &str = "https://www.railml.org/schemas/3.3";
/// Namensraum der rail-it-Erweiterungen.
pub const RAILIT_NAMESPACE: &str = "https://josunlp.de/rail-it/railml/1";

/// Gleisende eines `netElement`: Anfang (`false`) oder Ende (`true`).
type End = (usize, bool);

struct NetElement {
    id: String,
    name: String,
    length_mm: u32,
    /// Teilungspunkte (mm ab Anfang), aufsteigend, ohne 0 und Länge.
    cuts: Vec<u32>,
    /// Erster Gleisabschnitt im Builder.
    first_track: usize,
}

impl NetElement {
    const fn pieces(&self) -> usize {
        self.cuts.len().saturating_add(1)
    }
}

/// Punktobjekt auf einem `netElement`.
#[derive(Copy, Clone)]
struct Spot {
    element: usize,
    at_mm: u32,
    direction: Option<Direction>,
}

struct Reader<'a, 'input> {
    doc: &'a Document<'input>,
    elements: Vec<NetElement>,
    by_id: HashMap<&'a str, usize>,
    errors: Vec<Diagnostic>,
    warnings: Vec<String>,
    /// Namen der Gleisabschnitte im Builder.
    track_names: Vec<String>,
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

fn descendants<'a, 'input: 'a>(
    doc: &'a Document<'input>,
    name: &'a str,
) -> impl Iterator<Item = Node<'a, 'input>> + 'a {
    doc.descendants()
        .filter(move |c| c.is_element() && local(c) == name)
}

/// Attribut im railML-Namensraum (ohne Präfix).
fn attr<'a>(n: Node<'a, '_>, name: &str) -> Option<&'a str> {
    n.attributes()
        .find(|a| a.name() == name && a.namespace().is_none())
        .map(|a| a.value())
}

/// Attribut im rail-it-Namensraum.
fn ext<'a>(n: Node<'a, '_>, name: &str) -> Option<&'a str> {
    n.attribute((RAILIT_NAMESPACE, name))
}

/// Fachlicher Name: `name/@name`, sonst `designator/@entry`, sonst `id`.
fn display_name(n: Node<'_, '_>) -> String {
    child(n, "name")
        .and_then(|c| attr(c, "name"))
        .or_else(|| child(n, "designator").and_then(|c| attr(c, "entry")))
        .or_else(|| attr(n, "id"))
        .unwrap_or("?")
        .to_string()
}

fn context(n: Node<'_, '_>) -> String {
    match attr(n, "id") {
        Some(id) => format!("{} „{id}“", local(&n)),
        None => local(&n).to_string(),
    }
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
        .map(Length::as_mm)
}

impl Reader<'_, '_> {
    fn element(&self, id: &str) -> Option<usize> {
        self.by_id.get(id).copied()
    }

    fn length(&self, e: usize) -> u32 {
        self.elements.get(e).map_or(0, |e| e.length_mm)
    }

    fn read_elements(&mut self) {
        for n in descendants(self.doc, "netElement") {
            // Meso- und Makroebene fassen Elemente der Mikroebene nur zusammen.
            if child(n, "elementCollectionUnordered").is_some()
                || child(n, "elementCollectionOrdered").is_some()
            {
                continue;
            }
            let Some(id) = attr(n, "id") else {
                self.errors.push(diag(context(n), "ohne id"));
                continue;
            };
            let Some(length_mm) = attr(n, "length").and_then(mm_of) else {
                self.errors.push(diag(
                    context(n),
                    "ohne gültige Länge (Attribut length in Metern)",
                ));
                continue;
            };
            self.by_id.insert(id, self.elements.len());
            self.elements.push(NetElement {
                id: id.to_string(),
                name: display_name(n),
                length_mm,
                cuts: Vec::new(),
                first_track: 0,
            });
        }
    }

    /// Ort eines Punktobjekts aus seiner `spotLocation`.
    fn spot(&mut self, n: Node<'_, '_>) -> Option<Spot> {
        let Some(loc) = child(n, "spotLocation") else {
            self.errors.push(diag(context(n), "ohne spotLocation"));
            return None;
        };
        self.spot_at(n, loc)
    }

    /// Ort aus der `spotLocation` `loc` des Objekts `n`.
    fn spot_at(&mut self, n: Node<'_, '_>, loc: Node<'_, '_>) -> Option<Spot> {
        let Some(element) = attr(loc, "netElementRef").and_then(|r| self.element(r)) else {
            self.errors.push(diag(
                context(n),
                "spotLocation verweist auf kein netElement der Mikroebene",
            ));
            return None;
        };
        let length = self.length(element);
        let at_mm = if let Some(pos) = attr(loc, "pos") {
            mm_of(pos)
        } else {
            attr(loc, "intrinsicCoord")
                .and_then(|c| c.trim().parse::<f64>().ok())
                .filter(|c| (0.0..=1.0).contains(c))
                .and_then(|c| metres(c * f64::from(length) / 1000.0))
                .map(Length::as_mm)
        };
        let Some(at_mm) = at_mm.filter(|at| *at <= length) else {
            self.errors.push(diag(
                context(n),
                "spotLocation ohne gültige Lage (pos in Metern oder intrinsicCoord 0…1)",
            ));
            return None;
        };
        let direction = match attr(loc, "applicationDirection") {
            Some("normal") => Some(Direction::AToB),
            Some("reverse") => Some(Direction::BToA),
            _ => None,
        };
        Some(Spot {
            element,
            at_mm,
            direction,
        })
    }

    /// Gleisabschnitt (Index im Builder) und Lage darin für einen Punkt auf einem Element.
    /// Liegt der Punkt genau auf einer Teilung, entscheidet die Richtung: in Fahrtrichtung
    /// steht das Objekt am Ende des vorderen Abschnitts.
    fn piece_at(&self, spot: Spot) -> Option<(usize, u32)> {
        let e = self.elements.get(spot.element)?;
        let mut start = 0u32;
        for (k, cut) in e.cuts.iter().enumerate() {
            let before = spot.at_mm < *cut
                || (spot.at_mm == *cut && spot.direction != Some(Direction::BToA));
            if before {
                return Some((
                    e.first_track.checked_add(k)?,
                    spot.at_mm.checked_sub(start)?,
                ));
            }
            start = *cut;
        }
        Some((
            e.first_track.checked_add(e.cuts.len())?,
            spot.at_mm.checked_sub(start)?,
        ))
    }
}

/// Liest railML 3 und validiert das Netz vollständig.
///
/// # Errors
/// [`ImportError::Syntax`] bei ungültigem XML oder fehlendem `railML`-Wurzelelement,
/// [`ImportError::Invalid`] mit allen Fehlern, wenn das beschriebene Netz ungültig ist.
pub fn parse(source: &str) -> Result<Validated, ImportError> {
    parse_with_warnings(source).map(|(v, _)| v)
}

/// Wie [`parse`], gibt zusätzlich die Warnungen des Imports zurück (übergangene Elemente,
/// angenommene Gleisfreimeldung); die Warnungen des Netzmodells stehen in [`Validated`].
///
/// # Errors
/// Wie [`parse`].
pub fn parse_with_warnings(source: &str) -> Result<(Validated, Vec<String>), ImportError> {
    let doc = Document::parse(source).map_err(|e| ImportError::Syntax(e.to_string()))?;
    if local(&doc.root_element()) != "railML" {
        return Err(ImportError::Syntax(
            "Wurzelelement ist nicht <railML>".into(),
        ));
    }
    let name = descendants(&doc, "infrastructure")
        .next()
        .map_or_else(|| "railML".to_string(), display_name);
    let mut r = Reader {
        doc: &doc,
        elements: Vec::new(),
        by_id: HashMap::new(),
        errors: Vec::new(),
        warnings: Vec::new(),
        track_names: Vec::new(),
    };
    r.read_elements();
    let detectors = read_detectors(&mut r);
    let mut b = TopologyBuilder::new(name);
    add_tracks(&mut r, &mut b);
    let mut graph = Graph { plain: Vec::new() };
    let points = connect(&mut r, &mut b, &mut graph);
    add_terminals(&mut r, &mut b);
    add_signals(&mut r, &mut b);
    add_level_crossings(&mut r, &mut b);
    add_crossings(&mut r, &mut b);
    add_tvds(&mut r, &mut b, &graph, &points, &detectors);
    add_couplings(&mut r, &mut b);
    add_operation(&mut r, &mut b);

    let Reader {
        mut errors,
        mut warnings,
        ..
    } = r;
    match b.build() {
        Ok(validated) if errors.is_empty() => Ok((validated, warnings)),
        Ok(validated) => {
            warnings.extend(validated.warnings.iter().map(ToString::to_string));
            Err(ImportError::Invalid { errors, warnings })
        }
        Err(report) => {
            errors.extend(report.errors.iter().map(|e| model_error(String::new(), e)));
            warnings.extend(report.warnings.iter().map(ToString::to_string));
            Err(ImportError::Invalid { errors, warnings })
        }
    }
}

/// Gleisfreimeldeeinrichtung: `id` und Ort.
struct Detector {
    id: String,
    spot: Spot,
}

fn read_detectors(r: &mut Reader<'_, '_>) -> Vec<Detector> {
    let mut out = Vec::new();
    for n in descendants(r.doc, "trainDetectionElement") {
        if let Some(spot) = r.spot(n) {
            out.push(Detector {
                id: attr(n, "id").unwrap_or_default().to_string(),
                spot,
            });
        }
    }
    for d in &out {
        if let Some(e) = r.elements.get_mut(d.spot.element)
            && d.spot.at_mm > 0
            && d.spot.at_mm < e.length_mm
            && !e.cuts.contains(&d.spot.at_mm)
        {
            e.cuts.push(d.spot.at_mm);
        }
    }
    for e in &mut r.elements {
        e.cuts.sort_unstable();
    }
    if out.is_empty() {
        r.warnings.push(
            "keine Gleisfreimeldeeinrichtungen (trainDetectionElement): jedes Gleis und jede \
             Weiche bildet einen eigenen Abschnitt"
                .into(),
        );
    }
    out
}

fn add_tracks(r: &mut Reader<'_, '_>, b: &mut TopologyBuilder) {
    let mut next = 0usize;
    for e in &mut r.elements {
        let r_names = &mut r.track_names;
        e.first_track = next;
        let mut start = 0u32;
        let ends = e
            .cuts
            .iter()
            .copied()
            .chain([e.length_mm])
            .collect::<Vec<_>>();
        let pieces = ends.len();
        for (k, end) in ends.into_iter().enumerate() {
            let name = if pieces == 1 {
                e.name.clone()
            } else {
                format!("{}/{}", e.name, k.saturating_add(1))
            };
            let length = Length::from_mm(end.saturating_sub(start));
            r_names.push(name.clone());
            if let Err(err) = b.add_track(name, length) {
                r.errors
                    .push(model_error(format!("netElement „{}“", e.id), &err));
            }
            start = end;
            next = next.saturating_add(1);
        }
        for k in 1..pieces {
            let a = track(e.first_track.saturating_add(k).saturating_sub(1));
            let c = track(e.first_track.saturating_add(k));
            if let (Some(a), Some(c)) = (a, c) {
                let _ = b.connect(
                    PortRef::new(NodeRef::Track(a), Port::B),
                    PortRef::new(NodeRef::Track(c), Port::A),
                );
            }
        }
    }
}

fn track(index: usize) -> Option<TrackId> {
    TrackId::from_index(index)
}

impl Reader<'_, '_> {
    /// Port des Gleisabschnitts am Ende `end` eines Elements.
    fn port_of(&self, (e, at_end): End) -> Option<PortRef> {
        let el = self.elements.get(e)?;
        let (index, port) = if at_end {
            (
                el.first_track.checked_add(el.pieces())?.checked_sub(1)?,
                Port::B,
            )
        } else {
            (el.first_track, Port::A)
        };
        Some(PortRef::new(NodeRef::Track(track(index)?), port))
    }

    fn end_name(&self, (e, at_end): End) -> String {
        let name = self.elements.get(e).map_or("?", |e| e.name.as_str());
        format!("{name}.{}", if at_end { "b" } else { "a" })
    }
}

/// Verbindungen zwischen Gleisfreimeldeelementen für die Abschnittsbildung.
struct Graph {
    /// Gleisende ↔ Gleisende (ohne Weiche).
    plain: Vec<(End, End)>,
}

/// Weiche: Index im Builder und angeschlossene Gleisenden (Spitze, links, rechts).
struct SwitchEnds {
    point: usize,
    name: String,
    ends: [End; 3],
}

fn relation_ends(r: &Reader<'_, '_>, n: Node<'_, '_>) -> Option<(End, End)> {
    let a = child(n, "elementA")
        .and_then(|c| attr(c, "ref"))
        .and_then(|id| r.element(id))?;
    let b = child(n, "elementB")
        .and_then(|c| attr(c, "ref"))
        .and_then(|id| r.element(id))?;
    let pa = attr(n, "positionOnA")?;
    let pb = attr(n, "positionOnB")?;
    Some(((a, pa.trim() == "1"), (b, pb.trim() == "1")))
}

fn connect(r: &mut Reader<'_, '_>, b: &mut TopologyBuilder, graph: &mut Graph) -> Vec<SwitchEnds> {
    let mut relations: BTreeMap<&str, (End, End)> = BTreeMap::new();
    for n in descendants(r.doc, "netRelation") {
        if attr(n, "navigability") == Some("None") {
            continue;
        }
        let Some(id) = attr(n, "id") else {
            continue;
        };
        match relation_ends(r, n) {
            Some(ends) => {
                relations.insert(id, ends);
            }
            None => r.errors.push(diag(
                context(n),
                "elementA/elementB oder positionOnA/positionOnB fehlen oder verweisen auf kein \
                 netElement der Mikroebene",
            )),
        }
    }
    let mut used = BTreeSet::new();
    let mut points = Vec::new();
    for n in descendants(r.doc, "switchIS") {
        if let Some(s) = add_switch(r, b, n, &relations, &mut used) {
            points.push(s);
        }
    }
    for n in descendants(r.doc, "crossing") {
        if attr(n, "type") != Some("simpleCrossing") || children(n, "spotLocation").count() != 2 {
            r.errors.push(diag(
                context(n),
                "nur Kreuzungen ohne bewegliche Teile (type=\"simpleCrossing\" mit zwei \
                 spotLocation) werden unterstützt; Kreuzungsweichen nicht",
            ));
        }
    }
    for (id, (x, y)) in &relations {
        if used.contains(id) {
            continue;
        }
        let (Some(px), Some(py)) = (r.port_of(*x), r.port_of(*y)) else {
            continue;
        };
        if let Err(e) = b.connect(px, py) {
            r.errors
                .push(model_error(format!("netRelation „{id}“"), &e));
        }
        graph.plain.push((*x, *y));
    }
    points
}

fn add_switch<'d>(
    r: &mut Reader<'_, '_>,
    b: &mut TopologyBuilder,
    n: Node<'_, '_>,
    relations: &BTreeMap<&'d str, (End, End)>,
    used: &mut BTreeSet<&'d str>,
) -> Option<SwitchEnds> {
    let ctx = context(n);
    if let Some(kind) = attr(n, "type")
        && kind != "ordinarySwitch"
    {
        r.errors.push(diag(
            ctx,
            format!("Weichenart „{kind}“ wird nicht unterstützt"),
        ));
        return None;
    }
    let branch = |side: &str| {
        child(n, side)
            .and_then(|c| attr(c, "netRelationRef"))
            .and_then(|id| relations.get_key_value(id))
            .map(|(id, ends)| (*id, *ends))
    };
    let (Some((left_id, left)), Some((right_id, right))) =
        (branch("leftBranch"), branch("rightBranch"))
    else {
        r.errors.push(diag(
            ctx,
            "leftBranch und rightBranch müssen auf befahrbare netRelations verweisen",
        ));
        return None;
    };
    let [(l0, l1), (r0, r1)] = [left, right];
    let (tip, left_end, right_end) = if l0 == r0 {
        (l0, l1, r1)
    } else if l0 == r1 {
        (l0, l1, r0)
    } else if l1 == r0 {
        (l1, l0, r1)
    } else if l1 == r1 {
        (l1, l0, r0)
    } else {
        r.errors.push(diag(
            ctx,
            "die Relationen von leftBranch und rightBranch haben kein gemeinsames Gleisende \
             (Weichenspitze)",
        ));
        return None;
    };
    used.insert(left_id);
    used.insert(right_id);
    let name = display_name(n);
    let point = match b.add_point(name.clone()) {
        Ok(p) => p,
        Err(e) => {
            r.errors.push(model_error(ctx, &e));
            return None;
        }
    };
    let diverging = match attr(n, "branchCourse") {
        Some("left") => Some((PointPosition::Left, "leftBranch")),
        Some("right") => Some((PointPosition::Right, "rightBranch")),
        _ => None,
    };
    if let Some((position, side)) = diverging {
        let speed = child(n, side)
            .and_then(|c| attr(c, "branchingSpeed"))
            .and_then(|v| v.trim().parse::<f64>().ok())
            .filter(|v| v.is_finite() && *v > 0.0 && *v < 1000.0)
            .map_or(DEFAULT_DIVERGING_SPEED, |v| {
                // Wertebereich oben geprüft.
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                Speed::from_kmh(v.round() as u16)
            });
        if let Err(e) = b.set_point_diverging(point, position, speed) {
            r.errors.push(model_error(ctx.clone(), &e));
        }
    }
    for (end, port) in [
        (tip, Port::Tip),
        (left_end, Port::Left),
        (right_end, Port::Right),
    ] {
        if let Some(p) = r.port_of(end)
            && let Err(e) = b.connect(PortRef::new(NodeRef::Point(point), port), p)
        {
            r.errors.push(model_error(ctx.clone(), &e));
        }
    }
    Some(SwitchEnds {
        point: point.index()?,
        name,
        ends: [tip, left_end, right_end],
    })
}

fn add_terminals(r: &mut Reader<'_, '_>, b: &mut TopologyBuilder) {
    let mut ends = Vec::new();
    for n in r
        .doc
        .descendants()
        .filter(|n| n.is_element() && matches!(local(n), "bufferStop" | "border"))
    {
        let kind = if local(&n) == "bufferStop" {
            TerminalKind::BufferStop
        } else if ext(n, "kind") == Some("border") {
            TerminalKind::Border {
                initial: ext(n, "initial") == Some("true"),
            }
        } else if attr(n, "isOpenEnd") == Some("true") {
            TerminalKind::Boundary
        } else {
            // Grenzen ohne offenes Ende (z. B. zwischen Betriebsstellen) sind kein Netzende.
            continue;
        };
        let Some(spot) = r.spot(n) else {
            continue;
        };
        let length = r.length(spot.element);
        let end = (spot.element, spot.at_mm.saturating_mul(2) >= length);
        ends.push((n, kind, end));
    }
    for (n, kind, end) in ends {
        let ctx = context(n);
        let id = match b.add_terminal(display_name(n), kind) {
            Ok(id) => id,
            Err(e) => {
                r.errors.push(model_error(ctx, &e));
                continue;
            }
        };
        if let Some(p) = r.port_of(end)
            && let Err(e) = b.connect(PortRef::new(NodeRef::Terminal(id), Port::End), p)
        {
            r.errors.push(model_error(
                format!("{ctx} am Gleisende {}", r.end_name(end)),
                &e,
            ));
        }
    }
}

/// Signalart nach railML; `None`: kein Signal für Zugfahrten (wird übergangen).
fn signal_kind(n: Node<'_, '_>) -> Option<Result<SignalKind, String>> {
    if let Some(kind) = ext(n, "kind") {
        return Some(match kind {
            "main" => Ok(SignalKind::Main),
            "distant" => Ok(SignalKind::Distant),
            "combined" => Ok(SignalKind::Combined),
            "shunting" => Ok(SignalKind::Shunting),
            "etcs_stop_marker" => Ok(SignalKind::EtcsStopMarker),
            other => Err(format!("railit:kind „{other}“ ist unbekannt")),
        });
    }
    if child(n, "isEtcsSignal").is_some() {
        return Some(Ok(SignalKind::EtcsStopMarker));
    }
    let movement = child(n, "isTrainMovementSignal")?;
    Some(match attr(movement, "type") {
        None | Some("main") => Ok(SignalKind::Main),
        Some("distant") => Ok(SignalKind::Distant),
        Some("shunting") => Ok(SignalKind::Shunting),
        Some(other) => Err(format!("Signalart „{other}“ wird nicht unterstützt")),
    })
}

fn parent_of<'a>(n: Node<'a, '_>) -> Option<&'a str> {
    attr(n, "belongsToParent").or_else(|| child(n, "belongsToParent").and_then(|c| attr(c, "ref")))
}

fn add_signals(r: &mut Reader<'_, '_>, b: &mut TopologyBuilder) {
    let nodes = descendants(r.doc, "signalIS").collect::<Vec<_>>();
    // Vorsignale am Mast eines Hauptsignals machen es zum Mehrabschnittssignal.
    let kinds: HashMap<&str, Option<Result<SignalKind, String>>> = nodes
        .iter()
        .filter_map(|n| Some((attr(*n, "id")?, signal_kind(*n))))
        .collect();
    let mut combined = BTreeSet::new();
    let mut merged = BTreeSet::new();
    for n in &nodes {
        if let (Some(Ok(SignalKind::Distant)), Some(parent)) = (signal_kind(*n), parent_of(*n))
            && matches!(kinds.get(parent), Some(Some(Ok(SignalKind::Main))))
        {
            combined.insert(parent);
            if let Some(id) = attr(*n, "id") {
                merged.insert(id);
            }
        }
    }
    for n in nodes {
        let id = attr(n, "id").unwrap_or_default();
        if merged.contains(id) {
            continue;
        }
        let ctx = context(n);
        let kind = match signal_kind(n) {
            None => {
                r.warnings.push(format!(
                    "{ctx}: kein Signal für Zugfahrten (isTrainMovementSignal, isEtcsSignal), \
                     nicht übernommen"
                ));
                continue;
            }
            Some(Err(message)) => {
                r.errors.push(diag(ctx, message));
                continue;
            }
            Some(Ok(SignalKind::Main)) if combined.contains(id) => SignalKind::Combined,
            Some(Ok(kind)) => kind,
        };
        let Some(spot) = r.spot(n) else {
            continue;
        };
        let Some(direction) = spot.direction else {
            r.errors.push(diag(
                ctx,
                "Wirkrichtung fehlt (applicationDirection normal oder reverse)",
            ));
            continue;
        };
        let Some((index, position)) = r.piece_at(spot) else {
            continue;
        };
        let Some(t) = track(index) else {
            continue;
        };
        let id = match b.add_signal(
            display_name(n),
            t,
            Length::from_mm(position),
            direction,
            kind,
        ) {
            Ok(id) => id,
            Err(e) => {
                r.errors.push(model_error(ctx, &e));
                continue;
            }
        };
        let system = match ext(n, "system") {
            None | Some("ks") => SignalSystem::Ks,
            Some("hv") => SignalSystem::Hv,
            Some(other) => {
                r.errors.push(diag(
                    ctx.clone(),
                    format!("railit:system „{other}“ ist unbekannt"),
                ));
                SignalSystem::Ks
            }
        };
        if let Err(e) = b.set_signal_system(id, system) {
            r.errors.push(model_error(ctx.clone(), &e));
        }
        if ext(n, "auto") == Some("true")
            && let Err(e) = b.set_signal_auto(id, true)
        {
            r.errors.push(model_error(ctx, &e));
        }
    }
}

/// Kreuzungen ohne bewegliche Teile: zwei `spotLocation` auf den gekreuzten Elementen.
fn add_crossings(r: &mut Reader<'_, '_>, b: &mut TopologyBuilder) {
    for n in descendants(r.doc, "crossing").collect::<Vec<_>>() {
        let locs = children(n, "spotLocation").collect::<Vec<_>>();
        if attr(n, "type") != Some("simpleCrossing") || locs.len() != 2 {
            continue;
        }
        let mut legs = Vec::new();
        for loc in locs {
            let Some(spot) = r.spot_at(n, loc) else {
                continue;
            };
            if let Some((index, position)) = r.piece_at(spot)
                && let Some(t) = track(index)
            {
                legs.push((t, Length::from_mm(position)));
            }
        }
        if let [x, y] = legs[..]
            && let Err(e) = b.add_crossing(display_name(n), [x, y])
        {
            r.errors.push(model_error(context(n), &e));
        }
    }
}

fn add_level_crossings(r: &mut Reader<'_, '_>, b: &mut TopologyBuilder) {
    for n in descendants(r.doc, "levelCrossingIS").collect::<Vec<_>>() {
        let ctx = context(n);
        let Some(spot) = r.spot(n) else {
            continue;
        };
        let Some((index, position)) = r.piece_at(spot) else {
            continue;
        };
        let Some(t) = track(index) else {
            continue;
        };
        if let Err(e) = b.add_level_crossing(display_name(n), t, Length::from_mm(position)) {
            r.errors.push(model_error(ctx, &e));
        }
    }
}

/// Union-Find über Gleisabschnitte (zuerst) und Weichen (danach).
struct Components {
    parent: Vec<usize>,
}

impl Components {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
        }
    }

    fn find(&mut self, mut x: usize) -> usize {
        while let Some(&p) = self.parent.get(x) {
            if p == x {
                break;
            }
            let grand = self.parent.get(p).copied().unwrap_or(p);
            if let Some(slot) = self.parent.get_mut(x) {
                *slot = grand;
            }
            x = p;
        }
        x
    }

    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if let Some(slot) = self.parent.get_mut(ra) {
            *slot = rb;
        }
    }
}

/// Gleisfreimeldeeinrichtungen je Gleisende.
fn detectors_at_ends<'d>(
    r: &Reader<'_, '_>,
    detectors: &'d [Detector],
) -> BTreeMap<End, Vec<&'d str>> {
    let mut at_end: BTreeMap<End, Vec<&str>> = BTreeMap::new();
    for d in detectors {
        let length = r.length(d.spot.element);
        if d.spot.at_mm == 0 {
            at_end
                .entry((d.spot.element, false))
                .or_default()
                .push(&d.id);
        } else if d.spot.at_mm == length {
            at_end
                .entry((d.spot.element, true))
                .or_default()
                .push(&d.id);
        }
    }
    at_end
}

/// Index des Gleisabschnitts am Ende eines Elements.
fn track_index(r: &Reader<'_, '_>, end: End) -> Option<usize> {
    r.port_of(end).and_then(|p| match p.node {
        NodeRef::Track(t) => t.index(),
        _ => None,
    })
}

/// Zusammenhängende Gleise und Weichen ohne trennende Einrichtung.
fn tvd_components(
    r: &Reader<'_, '_>,
    graph: &Graph,
    points: &[SwitchEnds],
    detectors: &[Detector],
    at_end: &BTreeMap<End, Vec<&str>>,
) -> Components {
    let tracks = r.track_names.len();
    let point_count = points
        .iter()
        .map(|p| p.point.saturating_add(1))
        .max()
        .unwrap_or(0);
    let mut c = Components::new(tracks.saturating_add(point_count));
    // Ohne Einrichtungen bildet jedes Element einen eigenen Abschnitt.
    let separated = |end: &End| detectors.is_empty() || at_end.contains_key(end);
    for (x, y) in &graph.plain {
        if separated(x) || separated(y) {
            continue;
        }
        if let (Some(a), Some(b)) = (track_index(r, *x), track_index(r, *y)) {
            c.union(a, b);
        }
    }
    for s in points {
        for end in s.ends {
            if separated(&end) {
                continue;
            }
            if let Some(t) = track_index(r, end) {
                c.union(t, tracks.saturating_add(s.point));
            }
        }
    }
    c
}

/// Begrenzende Einrichtungen je Abschnitt (Wurzel in `c`).
fn tvd_boundaries<'d>(
    r: &Reader<'_, '_>,
    c: &mut Components,
    graph: &Graph,
    points: &[SwitchEnds],
    detectors: &'d [Detector],
    at_end: &BTreeMap<End, Vec<&'d str>>,
) -> BTreeMap<usize, BTreeSet<&'d str>> {
    let tracks = r.track_names.len();
    let mut boundary: BTreeMap<usize, BTreeSet<&str>> = BTreeMap::new();
    let mut add = |c: &mut Components, node: usize, ids: &[&'d str]| {
        let root = c.find(node);
        boundary
            .entry(root)
            .or_default()
            .extend(ids.iter().copied());
    };
    for (index, e) in r.elements.iter().enumerate() {
        for (k, cut) in e.cuts.iter().enumerate() {
            let ids = detectors
                .iter()
                .filter(|d| d.spot.element == index && d.spot.at_mm == *cut)
                .map(|d| d.id.as_str())
                .collect::<Vec<_>>();
            let before = e.first_track.saturating_add(k);
            add(c, before, &ids);
            add(c, before.saturating_add(1), &ids);
        }
    }
    for (end, ids) in at_end {
        if let Some(t) = track_index(r, *end) {
            add(c, t, ids);
        }
    }
    // Die Einrichtung am Gleisende begrenzt auch den Abschnitt auf der anderen Seite.
    for (x, y) in &graph.plain {
        for (here, there) in [(x, y), (y, x)] {
            if let (Some(ids), Some(t)) = (at_end.get(here), track_index(r, *there)) {
                add(c, t, ids);
            }
        }
    }
    for s in points {
        for end in &s.ends {
            if let Some(ids) = at_end.get(end) {
                add(c, tracks.saturating_add(s.point), ids);
            }
        }
    }
    boundary
}

fn add_tvds(
    r: &mut Reader<'_, '_>,
    b: &mut TopologyBuilder,
    graph: &Graph,
    points: &[SwitchEnds],
    detectors: &[Detector],
) {
    let at_end = detectors_at_ends(r, detectors);
    let mut c = tvd_components(r, graph, points, detectors, &at_end);
    let mut boundary = tvd_boundaries(r, &mut c, graph, points, detectors, &at_end);
    // Benannte Abschnitte in der Reihenfolge des Dokuments.
    let mut named: HashMap<BTreeSet<&str>, (usize, String)> = HashMap::new();
    for (k, n) in descendants(r.doc, "tvdSection").enumerate() {
        let set = children(n, "hasDemarcatingTraindetector")
            .filter_map(|c| attr(c, "ref"))
            .collect::<BTreeSet<_>>();
        if !set.is_empty() {
            named.insert(set, (k, display_name(n)));
        }
    }
    // Ein Abschnitt aus mehreren nicht verbundenen Teilen (Kreuzung): Teile, deren Grenzen alle
    // zu einem benannten Abschnitt gehören und ihn zusammen genau begrenzen, sind ein Abschnitt.
    let sets: Vec<BTreeSet<&str>> = named.keys().cloned().collect();
    for set in &sets {
        let parts: Vec<usize> = boundary
            .iter()
            .filter(|(_, b)| !b.is_empty() && b.is_subset(set) && !named.contains_key(*b))
            .map(|(root, _)| *root)
            .collect();
        let union: BTreeSet<&str> = parts
            .iter()
            .filter_map(|p| boundary.get(p))
            .flatten()
            .copied()
            .collect();
        if parts.len() > 1 && union == *set {
            // Nur Teile, die keinem anderen benannten Abschnitt entsprechen (oben), und nur,
            // wenn sie zusammen genau diesen Abschnitt begrenzen.
            let mut merged = BTreeSet::new();
            for p in &parts {
                merged.extend(boundary.remove(p).unwrap_or_default());
                if let Some(first) = parts.first() {
                    c.union(*first, *p);
                }
            }
            boundary.insert(c.find(parts.first().copied().unwrap_or_default()), merged);
        }
    }

    let tracks = r.track_names.len();
    let mut members: BTreeMap<usize, Vec<TvdMember>> = BTreeMap::new();
    for t in 0..tracks {
        if let Some(id) = track(t) {
            members
                .entry(c.find(t))
                .or_default()
                .push(TvdMember::Track(id));
        }
    }
    for s in points {
        if let Some(id) = PointId::from_index(s.point) {
            members
                .entry(c.find(tracks.saturating_add(s.point)))
                .or_default()
                .push(TvdMember::Point(id));
        }
    }
    let mut groups = members
        .into_iter()
        .map(|(root, m)| {
            let name = boundary.get(&root).and_then(|set| named.get(set)).cloned();
            (name, m)
        })
        .collect::<Vec<_>>();
    groups.sort_by_key(|(name, m)| {
        (
            name.as_ref().map_or(usize::MAX, |n| n.0),
            m.first().copied(),
        )
    });
    for (name, m) in groups {
        let name = name.map_or_else(
            || {
                let first = m.first().and_then(|first| match *first {
                    TvdMember::Track(t) => t.index().and_then(|i| r.track_names.get(i)),
                    TvdMember::Point(p) => p
                        .index()
                        .and_then(|i| points.iter().find(|s| s.point == i))
                        .map(|s| &s.name),
                });
                format!("T-{}", first.map_or("?", String::as_str))
            },
            |(_, name)| name,
        );
        if let Err(e) = b.add_tvd(name.clone(), &m) {
            r.errors
                .push(model_error(format!("Gleisfreimeldeabschnitt „{name}“"), &e));
        }
    }
}

/// Kreuzungsweichen und Kopplungen aus `railit:operation` (REQ-TOP-017).
fn add_couplings(r: &mut Reader<'_, '_>, b: &mut TopologyBuilder) {
    let ours =
        |n: &Node<'_, '_>| n.is_element() && n.tag_name().namespace() == Some(RAILIT_NAMESPACE);
    for slip in r
        .doc
        .descendants()
        .filter(|n| ours(n) && local(n) == "slip")
    {
        let name = attr(slip, "name").unwrap_or_default().to_string();
        let ctx = format!("railit:slip „{name}“");
        let kind = match attr(slip, "kind") {
            Some("single") => SlipKind::Single,
            Some("double") => SlipKind::Double,
            other => {
                r.errors.push(diag(
                    ctx,
                    format!("kind „{}“ ist unbekannt", other.unwrap_or_default()),
                ));
                continue;
            }
        };
        if let Err(e) = b.recognize_slip(&name, kind) {
            r.errors.push(model_error(ctx, &e));
        }
    }
    for c in r
        .doc
        .descendants()
        .filter(|n| ours(n) && local(n) == "coupling")
    {
        let first = attr(c, "first").unwrap_or_default();
        let second = attr(c, "second").unwrap_or_default();
        let ctx = format!("railit:coupling „{first}“ – „{second}“");
        let inverted = match attr(c, "inverted") {
            Some("true") => true,
            Some("false") | None => false,
            Some(other) => {
                r.errors.push(diag(
                    ctx,
                    format!("inverted „{other}“ ist kein Wahrheitswert"),
                ));
                continue;
            }
        };
        match (b.lookup(first), b.lookup(second)) {
            (Some(ElementRef::Point(x)), Some(ElementRef::Point(y))) => {
                if let Err(e) = b.couple_points(x, y, inverted) {
                    r.errors.push(model_error(ctx, &e));
                }
            }
            _ => r
                .errors
                .push(diag(ctx, "beide Angaben müssen Weichen sein")),
        }
    }
}

/// Stellbereiche und Betriebsstellen aus `railit:operation`.
fn add_operation(r: &mut Reader<'_, '_>, b: &mut TopologyBuilder) {
    let ours =
        |n: &Node<'_, '_>| n.is_element() && n.tag_name().namespace() == Some(RAILIT_NAMESPACE);
    for area in r
        .doc
        .descendants()
        .filter(|n| ours(n) && local(n) == "area")
    {
        let name = attr(area, "name").unwrap_or_default().to_string();
        let ctx = format!("railit:area „{name}“");
        let mut tvds = Vec::new();
        for t in area.children().filter(|n| ours(n) && local(n) == "tvd") {
            let tvd = attr(t, "name").unwrap_or_default();
            match b.lookup(tvd) {
                Some(ElementRef::Tvd(id)) => tvds.push(id),
                _ => r.errors.push(diag(
                    ctx.clone(),
                    format!("„{tvd}“ ist kein Gleisfreimeldeabschnitt"),
                )),
            }
        }
        if let Err(e) = b.add_area(name, &tvds) {
            r.errors.push(model_error(ctx, &e));
        }
    }
    for station in r
        .doc
        .descendants()
        .filter(|n| ours(n) && local(n) == "station")
    {
        let name = attr(station, "name").unwrap_or_default().to_string();
        let ctx = format!("railit:station „{name}“");
        let mut platforms = Vec::new();
        for p in station
            .children()
            .filter(|n| ours(n) && local(n) == "platform")
        {
            let mut signals = Vec::new();
            for s in p.children().filter(|n| ours(n) && local(n) == "signal") {
                let signal = attr(s, "name").unwrap_or_default();
                match b.lookup(signal) {
                    Some(ElementRef::Signal(id)) => signals.push(id),
                    _ => r
                        .errors
                        .push(diag(ctx.clone(), format!("„{signal}“ ist kein Signal"))),
                }
            }
            platforms.push((attr(p, "name").unwrap_or_default().to_string(), signals));
        }
        if let Err(e) = b.add_station(name, &platforms) {
            r.errors.push(model_error(ctx, &e));
        }
    }
}

// ---------------------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------------------

/// Text für XML-Attribute und -Inhalte.
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
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

/// Millimeter als Meter mit drei Nachkommastellen.
fn m(mm: u32) -> String {
    format!("{}.{:03}", mm / 1000, mm % 1000)
}

fn ne(t: TrackId) -> String {
    format!("ne{}", t.raw())
}

fn track_end(port: PortRef) -> Option<(TrackId, bool)> {
    match (port.node, port.port) {
        (NodeRef::Track(t), Port::A) => Some((t, false)),
        (NodeRef::Track(t), Port::B) => Some((t, true)),
        _ => None,
    }
}

fn spot_xml(net: &Topology, t: TrackId, at_mm: u32, dir: Option<Direction>) -> String {
    let length = net.track(t).map_or(1, |t| t.length.as_mm().max(1));
    let coord = f64::from(at_mm) / f64::from(length);
    let direction = match dir {
        Some(Direction::AToB) => " applicationDirection=\"normal\"",
        Some(Direction::BToA) => " applicationDirection=\"reverse\"",
        None => "",
    };
    format!(
        "<spotLocation netElementRef=\"{}\"{direction} intrinsicCoord=\"{coord:.6}\" pos=\"{}\"/>",
        ne(t),
        m(at_mm)
    )
}

/// Gleisfreimeldeeinrichtung beim Export: `id`, Gleisende und die Abschnitte beiderseits.
struct DetectorOut {
    id: String,
    end: (TrackId, bool),
    sides: [Option<TvdId>; 2],
}

/// Verbindungen und Weichen beim Export.
#[derive(Default)]
struct Connections {
    relations: String,
    switches: String,
    detectors: Vec<DetectorOut>,
    count: usize,
}

impl Connections {
    fn relation(&mut self, (a, ea): (TrackId, bool), (b, eb): (TrackId, bool)) -> String {
        self.count = self.count.saturating_add(1);
        let id = format!("nr{}", self.count);
        let _ = writeln!(
            self.relations,
            "        <netRelation id=\"{id}\" positionOnA=\"{}\" positionOnB=\"{}\" navigability=\"Both\"><elementA ref=\"{}\"/><elementB ref=\"{}\"/></netRelation>",
            u8::from(ea),
            u8::from(eb),
            ne(a),
            ne(b)
        );
        id
    }

    fn detector(&mut self, end: (TrackId, bool), sides: [Option<TvdId>; 2]) {
        if sides[0] != sides[1] {
            let id = format!("tde{}", self.detectors.len().saturating_add(1));
            self.detectors.push(DetectorOut { id, end, sides });
        }
    }

    fn collect(net: &Topology) -> Result<Self, String> {
        let mut c = Self::default();
        // Gleis an Gleis.
        for (t, _) in net.tracks() {
            for (port, end) in [(Port::A, false), (Port::B, true)] {
                if let Some(there) = net
                    .linked(PortRef::new(NodeRef::Track(t), port))
                    .and_then(track_end)
                    && (t, end) < there
                {
                    c.relation((t, end), there);
                    c.detector(
                        (t, end),
                        [
                            net.tvd_of(TvdMember::Track(t)),
                            net.tvd_of(TvdMember::Track(there.0)),
                        ],
                    );
                }
            }
        }
        for (p, point) in net.points() {
            c.switch(net, p, point)?;
        }
        Ok(c)
    }

    fn switch(&mut self, net: &Topology, p: PointId, point: &Point) -> Result<(), String> {
        let port = |port| {
            net.linked(PortRef::new(NodeRef::Point(p), port))
                .and_then(track_end)
        };
        let (Some(tip), Some(left), Some(right)) =
            (port(Port::Tip), port(Port::Left), port(Port::Right))
        else {
            return Err(format!(
                "Weiche „{}“: railML verlangt an jedem Strang ein Gleis (netElement)",
                point.name
            ));
        };
        let nl = self.relation(tip, left);
        let nr = self.relation(tip, right);
        let speed = format!(" branchingSpeed=\"{}\"", point.diverging_speed.as_kmh());
        let (course, speed_left, speed_right) = match point.diverging {
            Some(PointPosition::Left) => (
                " continueCourse=\"right\" branchCourse=\"left\"",
                speed.as_str(),
                "",
            ),
            Some(PointPosition::Right) => (
                " continueCourse=\"left\" branchCourse=\"right\"",
                "",
                speed.as_str(),
            ),
            None => ("", "", ""),
        };
        let _ = writeln!(
            self.switches,
            "        <switchIS id=\"sw{}\" type=\"ordinarySwitch\"{course}><name name=\"{}\"/>{}<leftBranch netRelationRef=\"{nl}\"{speed_left}/><rightBranch netRelationRef=\"{nr}\"{speed_right}/></switchIS>",
            p.raw(),
            esc(&point.name),
            spot_xml(net, tip.0, end_mm(net, tip), None)
        );
        let tvd = net.tvd_of(TvdMember::Point(p));
        for end in [tip, left, right] {
            self.detector(end, [net.tvd_of(TvdMember::Track(end.0)), tvd]);
        }
        Ok(())
    }
}

/// Lage eines Gleisendes in mm ab Anfang.
fn end_mm(net: &Topology, (t, end): (TrackId, bool)) -> u32 {
    if end {
        net.track(t).map_or(0, |t| t.length.as_mm())
    } else {
        0
    }
}

/// Schreibt das Netz als railML 3.3 (Infrastruktur und Gleisfreimeldeabschnitte).
///
/// # Errors
/// Beschreibung, wenn das Netz Verbindungen enthält, die railML nicht ohne Gleis dazwischen
/// ausdrücken kann (Weiche an Weiche oder an einem Netzende).
pub fn export(net: &Topology) -> Result<String, String> {
    let c = Connections::collect(net)?;
    let mut out = String::new();
    let _ = writeln!(out, "<?xml version=\"1.0\" encoding=\"UTF-8\"?>");
    let _ = writeln!(
        out,
        "<railML xmlns=\"{NAMESPACE}\" xmlns:railit=\"{RAILIT_NAMESPACE}\" version=\"3.3\">"
    );
    let _ = writeln!(
        out,
        "  <infrastructure id=\"is\"><name name=\"{}\"/>",
        esc(net.name())
    );
    let _ = writeln!(out, "    <topology>\n      <netElements>");
    for (t, track) in net.tracks() {
        let _ = writeln!(
            out,
            "        <netElement id=\"{}\" length=\"{}\"><name name=\"{}\"/></netElement>",
            ne(t),
            m(track.length.as_mm()),
            esc(&track.name)
        );
    }
    let _ = writeln!(out, "      </netElements>\n      <netRelations>");
    out.push_str(&c.relations);
    let _ = writeln!(
        out,
        "      </netRelations>\n    </topology>\n    <functionalInfrastructure>"
    );
    write_terminals(&mut out, net)?;
    if net.level_crossings().len() > 0 {
        let _ = writeln!(out, "      <levelCrossingsIS>");
        for (id, lx) in net.level_crossings() {
            let _ = writeln!(
                out,
                "        <levelCrossingIS id=\"lc{}\"><name name=\"{}\"/>{}</levelCrossingIS>",
                id.raw(),
                esc(&lx.name),
                spot_xml(net, lx.track, lx.position.as_mm(), None)
            );
        }
        let _ = writeln!(out, "      </levelCrossingsIS>");
    }
    write_signals(&mut out, net);
    if !c.switches.is_empty() {
        let _ = writeln!(out, "      <switchesIS>\n{}      </switchesIS>", c.switches);
    }
    if net.crossings().len() > 0 {
        let _ = writeln!(out, "      <crossings>");
        for (k, cr) in net.crossings().enumerate() {
            let _ = writeln!(
                out,
                "        <crossing id=\"cr{k}\" type=\"simpleCrossing\"><name name=\"{}\"/>{}{}</crossing>",
                esc(&cr.name),
                spot_xml(net, cr.legs[0].0, cr.legs[0].1.as_mm(), None),
                spot_xml(net, cr.legs[1].0, cr.legs[1].1.as_mm(), None),
            );
        }
        let _ = writeln!(out, "      </crossings>");
    }
    if !c.detectors.is_empty() {
        let _ = writeln!(out, "      <trainDetectionElements>");
        for d in &c.detectors {
            let _ = writeln!(
                out,
                "        <trainDetectionElement id=\"{}\" type=\"axleCounter\">{}</trainDetectionElement>",
                d.id,
                spot_xml(net, d.end.0, end_mm(net, d.end), None)
            );
        }
        let _ = writeln!(out, "      </trainDetectionElements>");
    }
    let _ = writeln!(out, "    </functionalInfrastructure>\n  </infrastructure>");
    write_tvds(&mut out, net, &c.detectors);
    write_operation(&mut out, net);
    let _ = writeln!(out, "</railML>");
    Ok(out)
}

fn write_terminals(out: &mut String, net: &Topology) -> Result<(), String> {
    let mut borders = String::new();
    let mut buffer_stops = String::new();
    for (id, terminal) in net.terminals() {
        let Some(end) = net
            .linked(PortRef::new(NodeRef::Terminal(id), Port::End))
            .and_then(track_end)
        else {
            return Err(format!(
                "Netzende „{}“: railML verlangt ein Gleis (netElement) davor",
                terminal.name
            ));
        };
        let spot = spot_xml(net, end.0, end_mm(net, end), None);
        let name = esc(&terminal.name);
        let raw = id.raw();
        let _ = match terminal.kind {
            TerminalKind::BufferStop => writeln!(
                buffer_stops,
                "        <bufferStop id=\"bus{raw}\"><name name=\"{name}\"/>{spot}</bufferStop>"
            ),
            TerminalKind::Boundary => writeln!(
                borders,
                "        <border id=\"bo{raw}\" isOpenEnd=\"true\"><name name=\"{name}\"/>{spot}</border>"
            ),
            TerminalKind::Border { initial } => writeln!(
                borders,
                "        <border id=\"bo{raw}\" isOpenEnd=\"true\" railit:kind=\"border\" railit:initial=\"{initial}\"><name name=\"{name}\"/>{spot}</border>"
            ),
        };
    }
    for (tag, body) in [("borders", &borders), ("bufferStops", &buffer_stops)] {
        if !body.is_empty() {
            let _ = writeln!(out, "      <{tag}>\n{body}      </{tag}>");
        }
    }
    Ok(())
}

fn write_tvds(out: &mut String, net: &Topology, detectors: &[DetectorOut]) {
    let _ = writeln!(
        out,
        "  <interlocking>\n    <assetsForInterlockings>\n      <assetsForInterlocking id=\"afi\">\n        <tvdSections>"
    );
    for (id, tvd) in net.tvds() {
        let mut refs = String::new();
        for d in detectors.iter().filter(|d| d.sides.contains(&Some(id))) {
            let _ = write!(refs, "<hasDemarcatingTraindetector ref=\"{}\"/>", d.id);
        }
        let _ = writeln!(
            out,
            "          <tvdSection id=\"tvd{}\"><name name=\"{}\"/>{refs}</tvdSection>",
            id.raw(),
            esc(&tvd.name)
        );
    }
    let _ = writeln!(
        out,
        "        </tvdSections>\n      </assetsForInterlocking>\n    </assetsForInterlockings>\n  </interlocking>"
    );
}

fn write_signals(out: &mut String, net: &Topology) {
    if net.signals().len() == 0 {
        return;
    }
    let _ = writeln!(out, "      <signalsIS>");
    for (id, s) in net.signals() {
        let spot = spot_xml(net, s.track, s.position.as_mm(), Some(s.direction));
        let mut extra = String::new();
        if s.system == SignalSystem::Hv {
            extra.push_str(" railit:system=\"hv\"");
        }
        if s.auto {
            extra.push_str(" railit:auto=\"true\"");
        }
        let function = match s.kind {
            SignalKind::Main | SignalKind::Combined => "<isTrainMovementSignal type=\"main\"/>",
            SignalKind::Distant => "<isTrainMovementSignal type=\"distant\"/>",
            SignalKind::Shunting => "<isTrainMovementSignal type=\"shunting\"/>",
            SignalKind::EtcsStopMarker => "<isEtcsSignal/>",
        };
        let _ = writeln!(
            out,
            "        <signalIS id=\"sig{}\" isSwitchable=\"{}\"{extra}><name name=\"{}\"/>{spot}{function}</signalIS>",
            id.raw(),
            s.kind != SignalKind::EtcsStopMarker,
            esc(&s.name)
        );
        if s.kind == SignalKind::Combined {
            // Vorsignal am Mast des Hauptsignals.
            let _ = writeln!(
                out,
                "        <signalIS id=\"sig{}v\" isSwitchable=\"true\" belongsToParent=\"sig{}\"><name name=\"{} (Vorsignal)\"/>{spot}<isTrainMovementSignal type=\"distant\"/></signalIS>",
                id.raw(),
                id.raw(),
                esc(&s.name)
            );
        }
    }
    let _ = writeln!(out, "      </signalsIS>");
}

fn write_operation(out: &mut String, net: &Topology) {
    if net.areas().len() == 0 && net.stations().len() == 0 && net.couplings().len() == 0 {
        return;
    }
    let _ = writeln!(out, "  <railit:operation>");
    // Kreuzungsweichen (REQ-TOP-017) koppeln ihre Weichen selbst; übrige Kopplungen einzeln.
    for x in net.slips() {
        let kind = match x.kind {
            SlipKind::Single => "single",
            SlipKind::Double => "double",
        };
        let _ = writeln!(
            out,
            "    <railit:slip name=\"{}\" kind=\"{kind}\"/>",
            esc(&x.name)
        );
    }
    for c in net.couplings() {
        if net.slips().any(|x| x.points.contains(&c.points[0])) {
            continue;
        }
        let name = |p| net.point(p).map_or("", |x| x.name.as_str());
        let _ = writeln!(
            out,
            "    <railit:coupling first=\"{}\" second=\"{}\" inverted=\"{}\"/>",
            esc(name(c.points[0])),
            esc(name(c.points[1])),
            c.inverted
        );
    }
    for (_, area) in net.areas() {
        let _ = write!(out, "    <railit:area name=\"{}\">", esc(&area.name));
        for t in &area.tvds {
            if let Some(tvd) = net.tvd(*t) {
                let _ = write!(out, "<railit:tvd name=\"{}\"/>", esc(&tvd.name));
            }
        }
        let _ = writeln!(out, "</railit:area>");
    }
    for (_, station) in net.stations() {
        let _ = writeln!(out, "    <railit:station name=\"{}\">", esc(&station.name));
        for p in &station.platforms {
            let _ = write!(out, "      <railit:platform name=\"{}\">", esc(&p.name));
            for s in &p.signals {
                if let Some(signal) = net.signal(*s) {
                    let _ = write!(out, "<railit:signal name=\"{}\"/>", esc(&signal.name));
                }
            }
            let _ = writeln!(out, "</railit:platform>");
        }
        let _ = writeln!(out, "    </railit:station>");
    }
    let _ = writeln!(out, "  </railit:operation>");
}
