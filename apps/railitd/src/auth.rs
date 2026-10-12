//! Benutzer, Rollen und Rechte (REQ-SEC-001 … REQ-SEC-003).
//!
//! Benutzerdatei (TOML):
//!
//! ```toml
//! [[user]]
//! name = "fdl-adorf"
//! roles = ["fahrdienstleiter"]
//! operator = 101                 # Bedienernummer für Hilfshandlungen (eindeutig)
//! areas = ["Adorf"]              # optional: nur diese Stellbereiche bedienen
//! token_sha256 = "<64 Hex>"      # Anmeldung mit Token (`railit auth token`)
//! cert_cn = "fdl-adorf"          # oder mit Client-Zertifikat (mTLS)
//! ```
//!
//! | Rolle | Rechte |
//! |-------|--------|
//! | `beobachter` | lesen |
//! | `fahrdienstleiter` | lesen, Stellwerk bedienen, Hilfshandlungen |
//! | `disponent` | lesen, Betriebsführung (Zugnummern, Zuglenkung, Disposition) |
//! | `instandhaltung` | lesen, Simulation (Züge, Störungen), Auditprotokoll |
//! | `projektierer` | lesen, Auditprotokoll |
//! | `administration` | alles |

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Rolle eines Benutzers.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Role {
    Beobachter,
    Fahrdienstleiter,
    Disponent,
    Instandhaltung,
    Projektierer,
    Administration,
}

/// Recht für eine Gruppe von Handlungen.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Permission {
    /// Zustände, Pläne, Ereignisse lesen.
    Read,
    /// Fahrstraßen, Weichen, Erlaubnisse.
    Operate,
    /// Hilfshandlungen anfordern und bestätigen.
    Aux,
    /// Betriebsführung: Zugnummern, Zuglenkung, Disposition.
    Dispatch,
    /// Simulation: Züge einsetzen und entfernen, Störungen.
    Simulate,
    /// Auditprotokoll lesen.
    Audit,
    /// Technik: Kanäle der Sicherheitsschicht prüfen, wieder aufnehmen, zurücksetzen.
    Maintain,
}

impl Role {
    const fn grants(self, p: Permission) -> bool {
        use Permission::{Audit, Aux, Dispatch, Maintain, Operate, Read, Simulate};
        match self {
            Self::Administration => true,
            Self::Beobachter => matches!(p, Read),
            Self::Fahrdienstleiter => matches!(p, Read | Operate | Aux),
            Self::Disponent => matches!(p, Read | Dispatch),
            Self::Instandhaltung => matches!(p, Read | Simulate | Audit | Maintain),
            Self::Projektierer => matches!(p, Read | Audit),
        }
    }
}

/// Angemeldeter Benutzer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct Principal {
    pub(crate) name: String,
    pub(crate) roles: Vec<Role>,
    /// Bedienernummer für die Zwei-Personen-Regel; ohne Benutzerdatei aus dem Kopf
    /// `X-Railit-Operator`.
    pub(crate) operator: Option<u32>,
    /// Stellbereiche, die der Benutzer bedienen darf (`None`: alle).
    pub(crate) areas: Option<Vec<String>>,
}

impl Principal {
    /// Ohne Benutzerverwaltung (offener Betrieb oder ein gemeinsames Token): alle Rechte.
    pub(crate) fn unrestricted(name: &str) -> Self {
        Self {
            name: name.into(),
            roles: vec![Role::Administration],
            operator: None,
            areas: None,
        }
    }

    pub(crate) fn may(&self, p: Permission) -> bool {
        self.roles.iter().any(|r| r.grants(p))
    }

    /// Darf der Benutzer im Stellbereich handeln?
    pub(crate) fn covers(&self, area: Option<&str>) -> bool {
        match (&self.areas, area) {
            (None, _) | (Some(_), None) => true,
            (Some(list), Some(a)) => list.iter().any(|x| x == a),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UsersFile {
    #[serde(default, rename = "user")]
    users: Vec<UserEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UserEntry {
    name: String,
    roles: Vec<Role>,
    #[serde(default)]
    operator: Option<u32>,
    #[serde(default)]
    areas: Option<Vec<String>>,
    #[serde(default)]
    token_sha256: Option<String>,
    #[serde(default)]
    cert_cn: Option<String>,
}

/// SHA-256 eines Tokens.
pub(crate) fn token_hash(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

fn parse_hash(text: &str) -> Option<[u8; 32]> {
    let text = text.trim();
    if text.len() != 64 || !text.is_ascii() {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(text.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

/// Benutzerverwaltung.
#[derive(Debug, Default)]
pub(crate) struct Users {
    by_token: HashMap<[u8; 32], Principal>,
    by_cn: HashMap<String, Principal>,
}

impl Users {
    /// Liest und prüft die Benutzerdatei: Namen, Bedienernummern, Token und Zertifikatsnamen
    /// sind eindeutig, jeder Benutzer hat eine Rolle und eine Anmeldeart, Stellbereiche sind
    /// bekannt.
    pub(crate) fn load(path: &Path, known_areas: &[String]) -> Result<Self, String> {
        let source =
            std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::parse(&source, known_areas).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub(crate) fn parse(source: &str, known_areas: &[String]) -> Result<Self, String> {
        let file: UsersFile = toml::from_str(source).map_err(|e| e.to_string())?;
        let mut users = Self::default();
        let mut names = BTreeSet::new();
        let mut operators = BTreeSet::new();
        for u in file.users {
            let n = &u.name;
            if n.is_empty() || !names.insert(n.clone()) {
                return Err(format!("Benutzername „{n}“ leer oder doppelt"));
            }
            if u.roles.is_empty() {
                return Err(format!("Benutzer „{n}“ hat keine Rolle"));
            }
            if let Some(op) = u.operator
                && !operators.insert(op)
            {
                return Err(format!("Bedienernummer {op} doppelt (Zwei-Personen-Regel)"));
            }
            for a in u.areas.iter().flatten() {
                if !known_areas.contains(a) {
                    return Err(format!("Benutzer „{n}“: Stellbereich „{a}“ unbekannt"));
                }
            }
            if u.token_sha256.is_none() && u.cert_cn.is_none() {
                return Err(format!("Benutzer „{n}“ hat weder Token noch Zertifikat"));
            }
            let principal = Principal {
                name: n.clone(),
                roles: u.roles,
                operator: u.operator,
                areas: u.areas,
            };
            if let Some(hash) = &u.token_sha256 {
                let hash = parse_hash(hash)
                    .ok_or_else(|| format!("Benutzer „{n}“: token_sha256 ungültig"))?;
                if users.by_token.insert(hash, principal.clone()).is_some() {
                    return Err(format!("Benutzer „{n}“: Token doppelt"));
                }
            }
            if let Some(cn) = u.cert_cn
                && users.by_cn.insert(cn.clone(), principal).is_some()
            {
                return Err(format!("Zertifikatsname „{cn}“ doppelt"));
            }
        }
        Ok(users)
    }

    pub(crate) fn by_token(&self, token: &str) -> Option<&Principal> {
        self.by_token.get(&token_hash(token))
    }

    pub(crate) fn by_cn(&self, cn: &str) -> Option<&Principal> {
        self.by_cn.get(cn)
    }
}

/// Anmeldeverfahren des Dienstes.
#[derive(Debug)]
pub(crate) enum AuthMode {
    /// Keine Anmeldung (nur Entwicklung und Simulation).
    Open,
    /// Ein gemeinsames Token mit allen Rechten.
    Token(String),
    /// Benutzerdatei mit Rollen.
    Users(Users),
}

impl AuthMode {
    /// Meldet einen Zugriff an: Client-Zertifikat (mTLS) vor Token.
    pub(crate) fn authenticate(
        &self,
        bearer: Option<&str>,
        cert_cn: Option<&str>,
    ) -> Option<Principal> {
        match self {
            Self::Open => Some(Principal::unrestricted("anonym")),
            Self::Token(token) => {
                let ok = bearer.is_some_and(|b| token_hash(b) == token_hash(token));
                ok.then(|| Principal::unrestricted("token"))
            }
            Self::Users(users) => match cert_cn {
                Some(cn) => users.by_cn(cn).cloned(),
                None => bearer.and_then(|b| users.by_token(b)).cloned(),
            },
        }
    }

    /// Bedienernummern kommen aus der Benutzerdatei, nicht aus dem Kopf.
    pub(crate) const fn operators_from_users(&self) -> bool {
        matches!(self, Self::Users(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(token: &str) -> String {
        railit_journal::to_hex(&token_hash(token))
    }

    fn areas() -> Vec<String> {
        vec!["Adorf".into(), "Bstadt".into()]
    }

    #[test]
    fn roles_grant_their_permissions() {
        let p = |roles: Vec<Role>| Principal {
            name: "x".into(),
            roles,
            operator: None,
            areas: Some(vec!["Adorf".into()]),
        };
        let fdl = p(vec![Role::Fahrdienstleiter]);
        assert!(fdl.may(Permission::Operate) && fdl.may(Permission::Aux));
        assert!(!fdl.may(Permission::Dispatch) && !fdl.may(Permission::Simulate));
        assert!(fdl.covers(Some("Adorf")) && !fdl.covers(Some("Bstadt")));
        let obs = p(vec![Role::Beobachter]);
        assert!(obs.may(Permission::Read) && !obs.may(Permission::Operate));
        let both = p(vec![Role::Disponent, Role::Instandhaltung]);
        assert!(both.may(Permission::Dispatch) && both.may(Permission::Audit));
        assert!(Principal::unrestricted("a").may(Permission::Simulate));
        // Sicherheitsschicht zurücksetzen: Technik, nicht Betrieb.
        assert!(both.may(Permission::Maintain));
        assert!(!fdl.may(Permission::Maintain) && !obs.may(Permission::Maintain));
    }

    #[test]
    fn users_file_is_validated() {
        let ok = format!(
            r#"
            [[user]]
            name = "fdl"
            roles = ["fahrdienstleiter"]
            operator = 1
            areas = ["Adorf"]
            token_sha256 = "{}"
            [[user]]
            name = "dispo"
            roles = ["disponent"]
            cert_cn = "dispo.example"
            "#,
            hex("geheim")
        );
        let users = Users::parse(&ok, &areas()).unwrap();
        assert_eq!(users.by_token("geheim").unwrap().name, "fdl");
        assert!(users.by_token("falsch").is_none());
        assert_eq!(users.by_cn("dispo.example").unwrap().name, "dispo");

        let mode = AuthMode::Users(users);
        assert_eq!(
            mode.authenticate(Some("geheim"), None).unwrap().operator,
            Some(1)
        );
        // Ein Zertifikat hat Vorrang; ein unbekanntes wird nicht durch ein Token ersetzt.
        assert!(mode.authenticate(Some("geheim"), Some("fremd")).is_none());

        for bad in [
            "[[user]]\nname = \"a\"\nroles = []\ncert_cn = \"a\"",
            "[[user]]\nname = \"a\"\nroles = [\"beobachter\"]",
            "[[user]]\nname = \"a\"\nroles = [\"chef\"]\ncert_cn = \"a\"",
            "[[user]]\nname = \"a\"\nroles = [\"beobachter\"]\ntoken_sha256 = \"zz\"",
            "[[user]]\nname = \"a\"\nroles = [\"beobachter\"]\nareas = [\"Cdorf\"]\ncert_cn = \"a\"",
            "[[user]]\nname = \"a\"\nroles = [\"beobachter\"]\noperator = 1\ncert_cn = \"a\"\n[[user]]\nname = \"b\"\nroles = [\"beobachter\"]\noperator = 1\ncert_cn = \"b\"",
            "[[user]]\nname = \"a\"\nroles = [\"beobachter\"]\ncert_cn = \"a\"\n[[user]]\nname = \"a\"\nroles = [\"beobachter\"]\ncert_cn = \"b\"",
        ] {
            assert!(Users::parse(bad, &areas()).is_err(), "{bad}");
        }
    }

    #[test]
    fn shared_token_and_open_mode() {
        let mode = AuthMode::Token("t".into());
        assert!(mode.authenticate(Some("t"), None).is_some());
        assert!(mode.authenticate(Some("u"), None).is_none());
        assert!(mode.authenticate(None, None).is_none());
        assert!(AuthMode::Open.authenticate(None, None).is_some());
        assert!(!AuthMode::Open.operators_from_users());
    }
}
