//! `railit self`: Version, Prüfung auf neue Releases, Aktualisierung und Deinstallation einer
//! Installation über den Installer (`install.sh` bzw. `install.ps1`).
//!
//! Der Installer legt die Programme nach `<Präfix>/bin` und schreibt ein Manifest nach
//! `<Präfix>/share/rail-it/manifest`; daneben liegt der Installer selbst. Aktualisieren und
//! Entfernen übergeben an ihn – so gilt für jeden Weg dieselbe Prüfung der Prüfsummen und
//! dasselbe Manifest.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use serde_json::{Value, json};

use crate::online::{Failure, TlsFiles, tls_config};
use crate::{EXIT_INVALID, EXIT_USAGE, Output, print_json};

/// Version dieses Programms.
pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Repository der Releases.
const REPO: &str = "JosunLP/rail-it";
/// Einzeiler zur Installation.
const INSTALL_HINT: &str = "curl -fsSL https://josunlp.github.io/rail-it/install.sh | sh";

/// Eine Installation über den Installer.
#[derive(Debug)]
struct Installation {
    prefix: PathBuf,
    data: PathBuf,
    values: BTreeMap<String, String>,
    files: Vec<String>,
}

impl Installation {
    /// Installation, zu der dieses Programm gehört (oder unter `RAILIT_PREFIX`).
    fn find() -> Option<Self> {
        let prefix = std::env::var_os("RAILIT_PREFIX")
            .map(PathBuf::from)
            .or_else(|| {
                let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
                Some(exe.parent()?.parent()?.to_path_buf())
            })?;
        Self::at(&prefix)
    }

    fn at(prefix: &Path) -> Option<Self> {
        let data = prefix.join("share").join("rail-it");
        let text = std::fs::read_to_string(data.join("manifest")).ok()?;
        let mut values = BTreeMap::new();
        let mut files = Vec::new();
        for line in text.lines() {
            let line = line.trim_start_matches('\u{feff}');
            match line.split_once('=') {
                Some(("file", f)) => files.push(f.to_string()),
                Some((k, v)) => {
                    values.insert(k.to_string(), v.to_string());
                }
                None => {}
            }
        }
        Some(Self {
            prefix: prefix.to_path_buf(),
            data,
            values,
            files,
        })
    }

    fn get(&self, key: &str) -> &str {
        self.values.get(key).map_or("", String::as_str)
    }

    fn json(&self) -> Value {
        json!({
            "prefix": self.prefix.display().to_string(),
            "version": self.get("version"),
            "target": self.get("target"),
            "files": self.files,
        })
    }
}

/// Neueste Version laut GitHub (ohne Vorabversionen).
fn latest() -> Result<String, Failure> {
    let repo = std::env::var("RAILIT_REPO").unwrap_or_else(|_| REPO.to_string());
    let url = std::env::var("RAILIT_RELEASES_API")
        .unwrap_or_else(|_| format!("https://api.github.com/repos/{repo}/releases/latest"));
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .tls_config(tls_config(&TlsFiles::default())?)
        .timeout_global(Some(std::time::Duration::from_secs(15)))
        .build()
        .into();
    let mut resp = agent
        .get(&url)
        .header("User-Agent", concat!("railit/", env!("CARGO_PKG_VERSION")))
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| Failure(EXIT_USAGE, format!("{url}: {e}")))?;
    let status = resp.status().as_u16();
    let body: Value = resp.body_mut().read_json().unwrap_or(Value::Null);
    if status != 200 {
        return Err(Failure(
            EXIT_USAGE,
            format!(
                "{url}: HTTP {status} {}",
                body["message"].as_str().unwrap_or("")
            ),
        ));
    }
    body["tag_name"]
        .as_str()
        .map(|t| t.trim_start_matches('v').to_string())
        .ok_or_else(|| Failure(EXIT_USAGE, format!("{url}: Antwort ohne tag_name")))
}

/// Vergleicht zwei Versionen `X.Y.Z[-pre]`; Vorabversionen liegen vor der Version selbst.
fn newer(candidate: &str, current: &str) -> bool {
    fn key(v: &str) -> Option<(u64, u64, u64, bool, String)> {
        let (core, pre) = v.split_once('-').map_or((v, None), |(c, p)| (c, Some(p)));
        let mut n = core.split('.').map(|p| p.parse::<u64>().ok());
        Some((
            n.next()??,
            n.next()??,
            n.next()??,
            pre.is_none(),
            pre.unwrap_or_default().to_string(),
        ))
    }
    matches!((key(candidate), key(current)), (Some(a), Some(b)) if a > b)
}

/// `railit self info`.
pub(crate) fn info(output: Output) -> ExitCode {
    let install = Installation::find();
    match output {
        Output::Json => print_json(&json!({
            "version": VERSION,
            "installation": install.as_ref().map(Installation::json),
        })),
        Output::Text => {
            println!("railit {VERSION}");
            match &install {
                Some(i) => println!(
                    "Installiert über den Installer: {} ({}, {})",
                    i.prefix.display(),
                    i.get("version"),
                    i.get("target")
                ),
                None => println!("Nicht über den Installer installiert ({INSTALL_HINT})."),
            }
        }
    }
    ExitCode::SUCCESS
}

/// `railit self check`: Exit-Code 0, wenn aktuell, 1, wenn ein neueres Release vorliegt.
pub(crate) fn check(output: Output) -> ExitCode {
    let latest = match latest() {
        Ok(v) => v,
        Err(f) => return f.exit(),
    };
    let update = newer(&latest, VERSION);
    match output {
        Output::Json => print_json(&json!({
            "version": VERSION,
            "latest": latest,
            "update_available": update,
        })),
        Output::Text if update => {
            println!(
                "rail-it {latest} ist verfügbar (installiert: {VERSION}); `railit self update`"
            );
        }
        Output::Text => println!("rail-it {VERSION} ist aktuell."),
    }
    if update {
        ExitCode::from(EXIT_INVALID)
    } else {
        ExitCode::SUCCESS
    }
}

/// Ruft den Installer der Installation mit `action` auf.
fn installer(action: &str, version: Option<&str>) -> ExitCode {
    let Some(install) = Installation::find() else {
        eprintln!(
            "Fehler: railit wurde nicht über den Installer installiert; installieren mit\n  {INSTALL_HINT}"
        );
        return ExitCode::from(EXIT_USAGE);
    };
    let prefix = install.prefix.display().to_string();
    let mut cmd = if cfg!(windows) {
        let script = install.data.join("install.ps1");
        let mut c = Command::new("powershell");
        c.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(script)
            .arg(action)
            .args(["-Prefix", &prefix]);
        if let Some(v) = version {
            c.args(["-Version", v]);
        }
        c
    } else {
        let script = install.data.join("install.sh");
        let mut c = Command::new("sh");
        c.arg(script).arg(action).args(["--prefix", &prefix]);
        if let Some(v) = version {
            c.args(["--version", v]);
        }
        c
    };
    if cfg!(windows) && action == "uninstall" {
        // Ein laufendes Programm lässt sich unter Windows nicht löschen: Der Installer läuft
        // weiter, nachdem railit sich beendet hat.
        cmd.env("RAILIT_WAIT_PID", std::process::id().to_string());
        return match cmd.spawn() {
            Ok(_) => {
                println!("rail-it wird entfernt …");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("Fehler: Installer: {e}");
                ExitCode::from(EXIT_USAGE)
            }
        };
    }
    match cmd.status() {
        Ok(s) if s.success() => ExitCode::SUCCESS,
        Ok(s) => ExitCode::from(u8::try_from(s.code().unwrap_or(1)).unwrap_or(1)),
        Err(e) => {
            eprintln!("Fehler: Installer: {e}");
            ExitCode::from(EXIT_USAGE)
        }
    }
}

/// `railit self update [--version X.Y.Z]`.
pub(crate) fn update(version: Option<&str>) -> ExitCode {
    installer("update", version)
}

/// `railit self uninstall`.
pub(crate) fn uninstall() -> ExitCode {
    installer("uninstall", None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare() {
        assert!(newer("0.11.0", "0.10.3"));
        assert!(newer("1.0.0", "1.0.0-rc.2"));
        assert!(!newer("1.0.0-rc.2", "1.0.0"));
        assert!(!newer("0.11.0", "0.11.0"));
        assert!(!newer("x", "0.1.0"));
    }

    #[test]
    fn manifest_is_read() {
        let dir = std::env::temp_dir().join(format!("railit-self-{}", std::process::id()));
        let data = dir.join("share").join("rail-it");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(
            data.join("manifest"),
            "\u{feff}version=0.11.0\ntarget=x86_64-unknown-linux-musl\nfile=/a/railit\nfile=/a/railitd\n",
        )
        .unwrap();
        let i = Installation::at(&dir).unwrap();
        assert_eq!(i.get("version"), "0.11.0");
        assert_eq!(i.files, ["/a/railit", "/a/railitd"]);
        assert!(Installation::at(&dir.join("x")).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
