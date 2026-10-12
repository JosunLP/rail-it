//! Werkzeuge für die Entwicklung von rail-it.
//!
//! `cargo xtask trace [--check]` prüft die Rückverfolgbarkeit Anforderung → Code → Test
//! (Konzept, Abschnitt 16.1) und schreibt den Bericht `docs/safety/traceability.md`:
//!
//! - Jede Anforderung in `docs/requirements/` hat mindestens einen Nachweis: einen Test bzw.
//!   Kani-Nachweis, der ihre ID nennt, eine im Nachweis genannte Funktion, die es gibt, oder
//!   ein genanntes Szenario.
//! - Jede im Nachweis genannte Funktion (`snake_case` in Backticks) gibt es im Code.
//! - Jede ID, die Code oder Tests nennen, ist dokumentiert.
//! - Jede genannte Gefährdung steht in der Gefährdungsliste.
//!
//! Mit `--check` schlägt der Befehl außerdem fehl, wenn der Bericht nicht aktuell ist (CI).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const REPORT: &str = "docs/safety/traceability.md";

/// Begriffe in Backticks, die wie Funktionsnamen aussehen, aber keine sind.
const NOT_FUNCTIONS: &[&str] = &["no_std"];

/// Nachweis durch Analyse statt Test (Architektur, Typsystem).
const ANALYSIS: &[&str] = &["Architektur", "Typsystem"];

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("trace") {
        trace(args.iter().any(|a| a == "--check"))
    } else {
        eprintln!("Aufruf: cargo xtask trace [--check]");
        ExitCode::from(2)
    }
}

/// Wurzel des Repositorys (über dem Verzeichnis dieses Crates).
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

/// Eine dokumentierte Anforderung.
struct Requirement {
    id: String,
    file: String,
    hazards: Vec<String>,
    evidence: String,
}

/// Fundstelle einer ID im Code.
#[derive(Default)]
struct Mentions {
    code: BTreeSet<String>,
    tests: BTreeSet<String>,
}

fn files(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.filter_map(Result::ok).map(|e| e.path()).collect();
    entries.sort();
    for path in entries {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if path.is_dir() {
            if !matches!(name, "target" | ".git" | "corpus" | "artifacts") {
                files(&path, ext, out);
            }
        } else if path.extension().is_some_and(|e| e == ext) {
            out.push(path);
        }
    }
}

/// Alle Vorkommen von `REQ-XXX-NNN` in einem Text.
fn ids(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while let Some(pos) = text.get(i..).and_then(|t| t.find("REQ-")) {
        let start = i + pos;
        let mut end = start + 4;
        while end < bytes.len() && bytes[end].is_ascii_uppercase() {
            end += 1;
        }
        if end < bytes.len() && bytes[end] == b'-' && end > start + 4 {
            let mut digits = end + 1;
            while digits < bytes.len() && bytes[digits].is_ascii_digit() {
                digits += 1;
            }
            if digits > end + 1 {
                out.push(text[start..digits].to_string());
                i = digits;
                continue;
            }
        }
        i = start + 4;
    }
    out
}

/// Bereichsangaben wie `REQ-COM-001 … REQ-COM-005` bzw. `REQ-PLN-001/002` ausmultiplizieren.
fn expand(text: &str) -> Vec<String> {
    let mut out = ids(text);
    // Kurzformen `REQ-IXL-012/013`.
    for id in ids(text) {
        let Some(at) = text.find(&id) else { continue };
        let mut rest = &text[at + id.len()..];
        let prefix = &id[..=id.rfind('-').unwrap_or(0)];
        while let Some(tail) = rest.strip_prefix('/') {
            let n: String = tail.chars().take_while(char::is_ascii_digit).collect();
            if n.is_empty() {
                break;
            }
            out.push(format!("{prefix}{n}"));
            rest = &tail[n.len()..];
        }
    }
    out
}

/// Bezeichner in Backticks, die wie Funktionsnamen aussehen (`snake_case` mit `_`).
fn function_refs(text: &str) -> Vec<String> {
    text.split('`')
        .skip(1)
        .step_by(2)
        .filter(|t| {
            !NOT_FUNCTIONS.contains(t)
                && t.contains('_')
                && t.chars().next().is_some_and(|c| c.is_ascii_lowercase())
                && t.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        })
        .map(str::to_string)
        .collect()
}

/// Szenarionummern wie „Szenario 08“ oder „Szenarien 18 und 19“.
fn scenario_refs(text: &str) -> Vec<u32> {
    let mut out = Vec::new();
    for (i, _) in text.match_indices("Szenari") {
        let tail: String = text[i..].chars().take(40).collect();
        let mut words = tail.split(|c: char| c.is_whitespace() || c == ',' || c == ')');
        words.next();
        for w in words {
            match w.parse::<u32>() {
                Ok(n) => out.push(n),
                Err(_) if matches!(w, "und" | "bis" | "" | "–") => {}
                Err(_) => break,
            }
        }
    }
    out
}

fn requirements(root: &Path) -> Vec<Requirement> {
    let mut docs = Vec::new();
    files(&root.join("docs/requirements"), "md", &mut docs);
    let mut out = Vec::new();
    for path in docs {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let file = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        for line in text.lines() {
            let cells: Vec<&str> = line.split('|').map(str::trim).collect();
            // | ID | Anforderung | Gefährdung | Nachweis |
            if cells.len() < 6 || !cells[1].starts_with("REQ-") {
                continue;
            }
            out.push(Requirement {
                id: cells[1].to_string(),
                file: file.clone(),
                hazards: cells[3]
                    .split([',', ' '])
                    .filter(|h| h.starts_with("H-"))
                    .map(str::to_string)
                    .collect(),
                evidence: cells[4..cells.len() - 1].join("|"),
            });
        }
    }
    out
}

#[allow(clippy::too_many_lines)]
fn trace(check: bool) -> ExitCode {
    let root = root();
    let reqs = requirements(&root);
    let documented: BTreeSet<&str> = reqs.iter().map(|r| r.id.as_str()).collect();

    let mut sources = Vec::new();
    for dir in ["crates", "apps", "fuzz/fuzz_targets"] {
        files(&root.join(dir), "rs", &mut sources);
    }
    let mut mentions: BTreeMap<String, Mentions> = BTreeMap::new();
    let mut functions: BTreeSet<String> = BTreeSet::new();
    for path in &sources {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let rel = path
            .strip_prefix(&root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        let is_test = rel.contains("/tests/") || rel.ends_with("proofs.rs");
        for id in expand(&text) {
            let m = mentions.entry(id).or_default();
            if is_test {
                m.tests.insert(rel.clone());
            } else {
                m.code.insert(rel.clone());
            }
        }
        for part in text.split("fn ").skip(1) {
            let name: String = part
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                functions.insert(name);
            }
        }
    }
    // Funktionsnamen, die nur als Invariante oder Eigenschaft benannt sind, stehen auch in
    // Kommentaren und Zeichenketten der Kernquellen.
    let mut scenario_files = Vec::new();
    files(&root.join("tests/scenarios"), "toml", &mut scenario_files);
    let scenarios: BTreeSet<u32> = scenario_files
        .iter()
        .filter_map(|p| p.file_name()?.to_str()?.get(..2)?.parse().ok())
        .collect();
    let hazards: BTreeSet<String> = std::fs::read_to_string(root.join("docs/safety/hazard-log.md"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let cell = l.split('|').nth(1)?.trim();
            cell.starts_with("H-").then(|| cell.to_string())
        })
        .collect();

    let mut errors = Vec::new();
    for (id, m) in &mentions {
        if !documented.contains(id.as_str()) {
            let at: Vec<_> = m.code.iter().chain(&m.tests).cloned().collect();
            errors.push(format!(
                "{id} wird in {} genannt, ist aber nicht dokumentiert",
                at.join(", ")
            ));
        }
    }
    let mut report = String::from(
        "# Rückverfolgbarkeit Anforderung → Code → Nachweis\n\n\
         > Erzeugt mit `cargo xtask trace`; die CI prüft mit `cargo xtask trace --check`, dass\n\
         > dieser Bericht aktuell ist. Nicht von Hand bearbeiten.\n\n\
         Je Anforderung: Gefährdungen, Code, der die ID nennt, und Nachweise (Tests und\n\
         Kani-Nachweise, die die ID nennen, im Nachweis genannte Funktionen, Szenarien).\n",
    );
    let mut current_file = String::new();
    let mut evidenced = 0usize;
    for r in &reqs {
        if r.file != current_file {
            current_file.clone_from(&r.file);
            let _ = write!(
                report,
                "\n## [{0}](../requirements/{0})\n\n| ID | Gefährdung | Code | Nachweis |\n|----|------------|------|----------|\n",
                r.file
            );
        }
        let m = mentions.get(&r.id);
        let funcs = function_refs(&r.evidence);
        let mut found = Vec::new();
        for f in &funcs {
            if functions.contains(f) {
                found.push(format!("`{f}`"));
            } else {
                errors.push(format!("{}: Nachweis `{f}` gibt es nicht", r.id));
            }
        }
        let mut scen = Vec::new();
        for n in scenario_refs(&r.evidence) {
            if scenarios.contains(&n) {
                scen.push(format!("Szenario {n:02}"));
            } else {
                errors.push(format!("{}: Szenario {n:02} gibt es nicht", r.id));
            }
        }
        for h in &r.hazards {
            if !hazards.contains(h) {
                errors.push(format!(
                    "{}: Gefährdung {h} fehlt in der Gefährdungsliste",
                    r.id
                ));
            }
        }
        let tests: Vec<String> = m
            .map(|m| m.tests.iter().map(|t| format!("`{t}`")).collect())
            .unwrap_or_default();
        let analysis: Vec<String> = ANALYSIS
            .iter()
            .filter(|a| r.evidence.contains(*a))
            .map(|a| format!("Analyse ({a})"))
            .collect();
        if tests.is_empty() && found.is_empty() && scen.is_empty() && analysis.is_empty() {
            errors.push(format!("{}: kein Nachweis gefunden", r.id));
        } else {
            evidenced += 1;
        }
        let code: Vec<String> = m
            .map(|m| m.code.iter().map(|c| format!("`{c}`")).collect())
            .unwrap_or_default();
        let mut proof: Vec<String> = found;
        proof.extend(tests);
        proof.extend(scen);
        proof.extend(analysis);
        let _ = writeln!(
            report,
            "| {} | {} | {} | {} |",
            r.id,
            r.hazards.join(", "),
            if code.is_empty() {
                "–".into()
            } else {
                code.join("<br>")
            },
            proof.join("<br>")
        );
    }
    let _ = write!(
        report,
        "\n---\n\n{} Anforderungen, davon {evidenced} mit Nachweis.\n",
        reqs.len()
    );

    for e in &errors {
        eprintln!("Fehler: {e}");
    }
    let path = root.join(REPORT);
    let current = std::fs::read_to_string(&path).unwrap_or_default();
    if check {
        if current != report {
            eprintln!("Fehler: {REPORT} ist nicht aktuell – `cargo xtask trace` ausführen");
            return ExitCode::FAILURE;
        }
    } else if current != report
        && let Err(e) = std::fs::write(&path, &report)
    {
        eprintln!("Fehler: {REPORT}: {e}");
        return ExitCode::FAILURE;
    }
    println!(
        "{} Anforderungen, {evidenced} mit Nachweis, {} Fehler",
        reqs.len(),
        errors.len()
    );
    if errors.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
