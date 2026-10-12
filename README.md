# rail-it

![License: NCSL v1.0](https://img.shields.io/badge/license-NCSL--v1.0-blueviolet.svg)
![Commercial use: Permission required](https://img.shields.io/badge/commercial%20use-permission%20required-orange.svg)

Modulare, in Rust geschriebene Engine für Netz- und Stellwerkmanagement von Bahnsystemen.
Sie umfasst Netzmodell, Fahrwegsicherung (deutsches ESTW), Fahrerlaubnis über Lichtsignale
oder ETCS sowie die Betriebsführung. Sie läuft als Dienst mit API und CLI, auf Linux-Servern
und – für den Sicherheitskern – auf Embedded-Hardware.

- **Konzept:** [docs/KONZEPT.md](docs/KONZEPT.md)
- **API:** [docs/api.md](docs/api.md)
- **Anforderungen:** [docs/requirements/](docs/requirements/)
- **Gefährdungsliste:** [docs/safety/hazard-log.md](docs/safety/hazard-log.md)

> rail-it ist **nicht zugelassen** für den realen Bahnbetrieb. Der Einsatz ist bis zu einer
> Zulassung auf Simulation, Ausbildung, Prüfstände und Labor beschränkt (Konzept,
> Abschnitt 1).

## Stand

Phase 0 (Fundament), Phase 1 (minimaler Kern) und Phase 2 (ESTW vollständig:
Durchrutschwege, Flankenschutz, Bahnübergänge, Hilfshandlungen mit Zwei-Personen-Regel,
Ersatzsignal, Erkennung unerlaubter Besetzung, Signalbegriffe Ks und H/V, Journal und Replay)
sind umgesetzt.

| Crate | Inhalt | |
|-------|--------|---|
| `railit-types` | Typsichere IDs, Längen, Zeit, Weichenlagen, Bitsets | `no_std`, ohne Allokator |
| `railit-topology` | Netzmodell mit Ports, Validierung, Navigation durch Weichen | `no_std` + `alloc` |
| `railit-plan` | Plan-Image: Binärformat, Zero-Copy-Sicht, SHA-256, Ed25519 | `no_std`, ohne Allokator |
| `railit-planning` | Fahrstraßen, Durchrutschwege, Flankenschutz, Feindlichkeiten; unabhängige Gegenprüfung | |
| `railit-rules` | Regelwerk-Schnittstelle, Lehrregelwerk | `no_std` |
| `railit-kernel` | Sicherheitskern: Fahrwegsicherung, Hilfshandlungen, Invarianten, sicherer Zustand | `no_std`, ohne Allokator |
| `railit-movement` | Signalbegriffe Ks und H/V aus dem Kernzustand | `no_std`, ohne Allokator |
| `railit-journal` | Betriebsjournal mit Hashkette, Prüfpunkten und Replay | |
| `railit-sim` | Simulator: Weichen, Signale, Gleisfreimeldung, Bahnübergänge, Züge, Störungen | |
| `railit-runtime` | Zyklus Kern ↔ Feld, Zustandsbild, Szenarien, Schlüssel | |
| `railit-import` | rail-it-TOML | |
| `railitd` | Dienst mit REST/JSON-API und Ereignisstrom | |
| `railit` | Kommandozeile (offline und online) | |

## Schnellstart

```sh
cargo build --release
export PATH="$PWD/target/release:$PATH"

# Netz prüfen, Schlüssel erzeugen, Plan kompilieren und signieren
railit net validate examples/beispielstadt.toml
railit plan keygen railit
railit plan compile examples/beispielstadt.toml --out beispielstadt.rplan --key railit.key
railit plan info beispielstadt.rplan

# Betriebsszenarien ausführen
railit scenario run tests/scenarios/*.toml

# Dienst starten (Simulation mit 10-fachem Zeitraffer, mit Betriebsjournal)
railitd --network examples/beispielstadt.toml --plan beispielstadt.rplan \
        --trusted-key railit.pub --speed 10 --journal betrieb.journal &

# Bedienen
railit sim train add RE1 --at A --length 120 --speed 80
railit route set A-N1
railit watch            # Ereignisse fortlaufend, Strg+C beendet
railit state            # Signalbilder, Verschlüsse, Bahnübergänge, Hilfshandlungen

# Hilfshandlung: Bediener 1 fordert an, Bediener 2 bestätigt
railit aux request release-route A-N1 --operator 1
railit aux confirm 0 --operator 2

# Journal prüfen und abspielen
railit journal verify betrieb.journal
railit journal replay betrieb.journal --plan beispielstadt.rplan
```

Exit-Codes der CLI: `0` Erfolg, `1` ungültig oder abgelehnt, `2` Bedien-, Lese- oder
Verbindungsfehler. Mit `--output json` liefern alle Befehle JSON.

## Entwicklung

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo build --target thumbv7em-none-eabihf \
      -p railit-types -p railit-topology -p railit-plan -p railit-rules -p railit-kernel \
      -p railit-movement
cargo deny check
PROPTEST_CASES=10000 cargo test --release -p railit-kernel --all-features --test invariants
```

Die Toolchain ist in `rust-toolchain.toml` festgelegt. Die sicherheitsrelevanten Crates
verbieten `unsafe`, `unwrap`, `panic`, direkte Indexierung, ungeprüfte Arithmetik und
Gleitkommazahlen.

## Lizenz

rail-it steht unter der [Non-Commercial Source License (NCSL) v1.0](LICENSE). Nicht
kommerzielle Nutzung ist erlaubt. Für jede kommerzielle Nutzung ist eine schriftliche Lizenz
nötig: support@josunlp.de
