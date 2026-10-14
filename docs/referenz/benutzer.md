# Benutzerdatei

`railitd --users users.toml --audit audit.log` meldet Benutzer einzeln an, mit Token oder
Client-Zertifikat, und prüft ihre Rollen. Einführung:
[Benutzer, Rollen, TLS](../leitfaden/zugang).

```toml
[[user]]
name = "fdl-adorf"
roles = ["fahrdienstleiter"]
operator = 101
areas = ["Adorf"]
token_sha256 = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"

[[user]]
name = "fdl-bstadt"
roles = ["fahrdienstleiter"]
operator = 102
cert_cn = "fdl-bstadt"

[[user]]
name = "bereitschaft"
roles = ["bereitschaft"]
token_sha256 = "…"
```

## `[[user]]`

| Schlüssel | Typ | Bedeutung |
|-----------|-----|-----------|
| `name` | Text | Benutzername, eindeutig; steht im Auditprotokoll |
| `roles` | Liste | Mindestens eine Rolle |
| `operator` | Zahl | Bedienernummer für Hilfshandlungen, eindeutig (Zwei-Personen-Regel) |
| `areas` | Liste | Nur diese Stellbereiche bedienen (optional; ohne Angabe alle) |
| `token_sha256` | 64 Hex-Zeichen | SHA-256 des Tokens |
| `cert_cn` | Text | CN des Client-Zertifikats (mTLS), eindeutig |

Jeder Benutzer braucht `token_sha256`, `cert_cn` oder beides.

## Rollen

| Rolle | Rechte |
|-------|--------|
| `beobachter` | lesen |
| `fahrdienstleiter` | lesen, Stellwerk bedienen, Hilfshandlungen |
| `disponent` | lesen, Betriebsführung |
| `instandhaltung` | lesen, Simulation, Auditprotokoll, Sicherheitsschicht |
| `projektierer` | lesen, Auditprotokoll |
| `bereitschaft` | lesen, Journal quittieren (Bereitschaftsrechner) |
| `administration` | alles |

## Token erzeugen

```sh
railit auth token fdl-adorf
railit auth token fdl-adorf --output json   # {"user", "token", "token_sha256"}
```

Der Befehl erzeugt ein zufälliges Token und gibt dessen Hash für die Datei aus. Nur der Hash
steht in der Benutzerdatei; das Token selbst erhält der Benutzer (`--token` bzw.
`RAILIT_TOKEN`).

## Prüfungen beim Start

`railitd` startet nicht, wenn die Datei einen dieser Fehler enthält:

- Benutzername leer oder doppelt, Benutzer ohne Rolle,
- Bedienernummer doppelt,
- unbekannter Stellbereich in `areas`,
- Benutzer ohne Token und ohne Zertifikat,
- `token_sha256` ungültig oder doppelt, `cert_cn` doppelt.
