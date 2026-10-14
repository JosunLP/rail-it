//! Releases vorbereiten (`cargo xtask release`, `cargo xtask notes`).
//!
//! `cargo xtask release 0.11.0` hebt die Version im Workspace, im Bedienplatz und in der
//! Dokumentation an, aktualisiert `Cargo.lock` und schreibt in `CHANGELOG.md` einen Abschnitt
//! mit allen Commits seit dem letzten Tag. Commit und Tag legt der Release-Workflow an
//! (`.github/workflows/release.yml`); `cargo xtask notes 0.11.0` liefert ihm den Text des
//! Releases.

use std::fmt::Write as _;
use std::path::Path;
use std::process::{Command, ExitCode};

const CHANGELOG: &str = "CHANGELOG.md";
const HEAD: &str = "# Änderungen\n\nAlle Releases von rail-it. Jeder Abschnitt nennt die Commits seit dem vorigen Release.\n";

/// Version nach Semantic Versioning (`MAJOR.MINOR.PATCH` mit optionalem `-pre`).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Version {
    major: u64,
    minor: u64,
    patch: u64,
    pre: Option<String>,
}

impl Version {
    fn parse(text: &str) -> Option<Self> {
        let text = text.strip_prefix('v').unwrap_or(text);
        let (core, pre) = match text.split_once('-') {
            Some((c, p))
                if !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '.') =>
            {
                (c, Some(p.to_string()))
            }
            Some(_) => return None,
            None => (text, None),
        };
        let mut parts = core.split('.').map(|p| {
            (!p.is_empty()
                && p.chars().all(|c| c.is_ascii_digit())
                && (p == "0" || !p.starts_with('0')))
            .then(|| p.parse().ok())
            .flatten()
        });
        let v = Self {
            major: parts.next()??,
            minor: parts.next()??,
            patch: parts.next()??,
            pre,
        };
        parts.next().is_none().then_some(v)
    }

    /// Reihenfolge: eine Vorabversion liegt vor der Version ohne Zusatz.
    fn key(&self) -> (u64, u64, u64, bool, String) {
        (
            self.major,
            self.minor,
            self.patch,
            self.pre.is_none(),
            self.pre.clone().unwrap_or_default(),
        )
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if let Some(p) = &self.pre {
            write!(f, "-{p}")?;
        }
        Ok(())
    }
}

fn run(root: &Path, program: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(program)
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|e| format!("{program}: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{program} {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn read(root: &Path, file: &str) -> Result<String, String> {
    std::fs::read_to_string(root.join(file)).map_err(|e| format!("{file}: {e}"))
}

fn write(root: &Path, file: &str, text: &str) -> Result<(), String> {
    std::fs::write(root.join(file), text).map_err(|e| format!("{file}: {e}"))
}

/// Version im Abschnitt `[workspace.package]`.
fn workspace_version(manifest: &str) -> Option<(usize, usize, Version)> {
    let section = manifest.find("[workspace.package]")?;
    let rel = manifest.get(section..)?.find("\nversion = \"")?;
    let start = section + rel + "\nversion = \"".len();
    let len = manifest.get(start..)?.find('"')?;
    Some((
        start,
        start + len,
        Version::parse(manifest.get(start..start + len)?)?,
    ))
}

/// Setzt `"version": "…"` in einer `package.json`.
fn set_json_version(text: &str, version: &Version) -> Option<String> {
    let key = "\"version\": \"";
    let start = text.find(key)? + key.len();
    let len = text.get(start..)?.find('"')?;
    Some(format!(
        "{}{version}{}",
        text.get(..start)?,
        text.get(start + len..)?
    ))
}

/// Letzter Release-Tag (`v…`), falls es einen gibt.
fn last_tag(root: &Path) -> Option<String> {
    run(
        root,
        "git",
        &["describe", "--tags", "--abbrev=0", "--match", "v*"],
    )
    .ok()
    .map(|t| t.trim().to_string())
    .filter(|t| !t.is_empty())
}

/// Commit einer Phase (`P10-3: …`, `Phase 4: …`).
fn is_phase(subject: &str) -> bool {
    let head = subject.split(':').next().unwrap_or("");
    subject.contains(':')
        && (head.starts_with("Phase")
            || head.starts_with('P') && head[1..].starts_with(|c: char| c.is_ascii_digit()))
}

/// Abschnitt des Changelogs für `version` aus den Commits seit dem letzten Tag.
fn section(root: &Path, version: &Version, date: &str) -> Result<String, String> {
    let range = last_tag(root).map_or_else(|| "HEAD".to_string(), |t| format!("{t}..HEAD"));
    let log = run(root, "git", &["log", "--no-merges", "--format=%s", &range])?;
    let (mut features, mut fixes, mut docs, mut other) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for subject in log.lines().map(str::trim).filter(|s| !s.is_empty()) {
        let lower = subject.to_lowercase();
        if subject.starts_with("Release v") || subject.starts_with("Merge ") {
            continue;
        }
        if lower.starts_with("fix") || lower.contains("fehler") || lower.contains("behoben") {
            fixes.push(subject);
        } else if lower.starts_with("feat") || is_phase(subject) {
            features.push(subject);
        } else if lower.starts_with("docs") {
            docs.push(subject);
        } else {
            other.push(subject);
        }
    }
    let mut out = format!("## [{version}] – {date}\n");
    let groups = [
        ("Neu", features),
        ("Behoben", fixes),
        ("Dokumentation", docs),
        ("Weiteres", other),
    ];
    for (title, items) in groups {
        if items.is_empty() {
            continue;
        }
        let _ = write!(out, "\n### {title}\n\n");
        for item in items.iter().rev() {
            let _ = writeln!(out, "- {item}");
        }
    }
    Ok(out)
}

/// `cargo xtask release VERSION [--dry-run]`.
pub(crate) fn release(root: &Path, args: &[String]) -> ExitCode {
    match prepare(root, args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("release: {e}");
            ExitCode::FAILURE
        }
    }
}

fn prepare(root: &Path, args: &[String]) -> Result<(), String> {
    let dry = args.iter().any(|a| a == "--dry-run");
    let requested = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .ok_or("Aufruf: cargo xtask release VERSION [--dry-run]")?;
    let version = Version::parse(requested)
        .ok_or_else(|| format!("„{requested}“ ist keine Version (z. B. 0.11.0)"))?;
    let manifest = read(root, "Cargo.toml")?;
    let (start, end, current) =
        workspace_version(&manifest).ok_or("Cargo.toml ohne Workspace-Version")?;
    if last_tag(root).is_some() && version.key() <= current.key() {
        return Err(format!("{version} liegt nicht nach {current}"));
    }
    if run(root, "git", &["tag", "--list", &format!("v{version}")])?.trim() == format!("v{version}")
    {
        return Err(format!("Tag v{version} gibt es schon"));
    }
    let date = run(root, "date", &["-u", "+%Y-%m-%d"])?.trim().to_string();
    let notes = section(root, &version, &date)?;
    if dry {
        println!("{current} → {version}\n\n{notes}");
        return Ok(());
    }
    write(
        root,
        "Cargo.toml",
        &format!("{}{version}{}", &manifest[..start], &manifest[end..]),
    )?;
    for file in ["apps/railitd/ui/package.json", "docs/package.json"] {
        if let Ok(text) = read(root, file) {
            let updated =
                set_json_version(&text, &version).ok_or_else(|| format!("{file}: ohne Version"))?;
            write(root, file, &updated)?;
        }
    }
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    run(root, &cargo, &["update", "--workspace", "--offline"])?;
    let old = read(root, CHANGELOG).unwrap_or_default();
    let rest = old.strip_prefix(HEAD).unwrap_or(&old).trim_start();
    write(
        root,
        CHANGELOG,
        &format!("{HEAD}\n{notes}\n{rest}")
            .trim_end()
            .to_string()
            .replace("\n\n\n", "\n\n"),
    )?;
    let mut changelog = read(root, CHANGELOG)?;
    changelog.push('\n');
    write(root, CHANGELOG, &changelog)?;
    println!("{current} → {version}");
    Ok(())
}

/// `cargo xtask notes VERSION`: Abschnitt des Changelogs für das Release.
pub(crate) fn notes(root: &Path, args: &[String]) -> ExitCode {
    let Some(version) = args.first().and_then(|a| Version::parse(a)) else {
        eprintln!("Aufruf: cargo xtask notes VERSION");
        return ExitCode::from(2);
    };
    let Ok(text) = read(root, CHANGELOG) else {
        eprintln!("notes: {CHANGELOG} fehlt");
        return ExitCode::FAILURE;
    };
    let head = format!("## [{version}]");
    let Some(start) = text.find(&head) else {
        eprintln!("notes: kein Abschnitt für {version}");
        return ExitCode::FAILURE;
    };
    let body = &text[start..];
    let body = body
        .get(head.len()..)
        .and_then(|b| b.find("\n## [").map(|e| &body[..head.len() + e]))
        .unwrap_or(body);
    // Ohne die Überschrift; der Titel des Releases nennt die Version.
    let body = body.split_once('\n').map_or("", |(_, b)| b);
    println!("{}", body.trim());
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_parse_and_order() {
        let v = |s| Version::parse(s).unwrap();
        assert_eq!(v("0.11.0").to_string(), "0.11.0");
        assert_eq!(v("v1.2.3-rc.1").to_string(), "1.2.3-rc.1");
        assert!(v("1.0.0-rc.1").key() < v("1.0.0").key());
        assert!(v("0.10.0").key() < v("0.11.0").key());
        for bad in ["1.2", "1.2.3.4", "01.2.3", "1.2.x", "1.2.3-", "a"] {
            assert!(Version::parse(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn workspace_and_package_versions_are_replaced() {
        let m = "[workspace]\nmembers = []\n\n[workspace.package]\nversion = \"0.1.0\"\nedition = \"2024\"\n";
        let (s, e, v) = workspace_version(m).unwrap();
        assert_eq!(v.to_string(), "0.1.0");
        assert_eq!(&m[s..e], "0.1.0");
        let json = "{\n  \"name\": \"x\",\n  \"version\": \"0.1.0\",\n  \"private\": true\n}\n";
        let out = set_json_version(json, &Version::parse("0.11.0").unwrap()).unwrap();
        assert!(out.contains("\"version\": \"0.11.0\""));
        assert!(out.contains("\"private\": true"));
    }
}
