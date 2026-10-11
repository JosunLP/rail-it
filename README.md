# rail-it

![License: NCSL v1.0](https://img.shields.io/badge/license-NCSL--v1.0-blueviolet.svg)
![Commercial use: Permission required](https://img.shields.io/badge/commercial%20use-permission%20required-orange.svg)

Modulare, in Rust geschriebene Engine für Netz- und Stellwerkmanagement von Bahnsystemen.
Sie umfasst Netzmodell, Fahrwegsicherung (deutsches ESTW), Fahrerlaubnis über Lichtsignale
oder ETCS sowie die Betriebsführung. Sie läuft als Dienst mit API und CLI, auf Linux-Servern
und – für den Sicherheitskern – auf Embedded-Hardware.

- **Konzept:** [docs/KONZEPT.md](docs/KONZEPT.md)
- **Anforderungen:** [docs/requirements/](docs/requirements/)
- **Gefährdungsliste:** [docs/safety/hazard-log.md](docs/safety/hazard-log.md)

## Stand

Phase 0 (Fundament) ist umgesetzt:

| Crate | Inhalt |
|-------|--------|
| `railit-types` | Typsichere IDs, Längen, Geschwindigkeiten, Zeit, Bitsets (`no_std`, ohne Allokator) |
| `railit-topology` | Netzmodell mit Ports, Validierung, Navigation durch Weichen (`no_std` + `alloc`) |
| `railit-import` | Import des rail-it-TOML-Formats mit vollständiger Fehlerliste |
| `railit` (CLI) | `railit net validate` und `railit net info` |

## Schnellstart

```sh
cargo run -p railit -- net validate examples/beispielstadt.toml
cargo run -p railit -- net info examples/beispielstadt.toml
cargo run -p railit -- net info --output json examples/beispielstadt.toml
```

Exit-Codes: `0` gültig, `1` Netz ungültig, `2` Bedien- oder Lesefehler.

## Entwicklung

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo build --target thumbv7em-none-eabihf -p railit-types -p railit-topology
cargo deny check
```

Die Toolchain ist in `rust-toolchain.toml` festgelegt. Die sicherheitsrelevanten Crates
verbieten `unsafe`, `unwrap`, `panic`, direkte Indexierung, ungeprüfte Arithmetik und
Gleitkommazahlen.

## Lizenz

rail-it steht unter der [Non-Commercial Source License (NCSL) v1.0](LICENSE). Nicht
kommerzielle Nutzung ist erlaubt. Für jede kommerzielle Nutzung ist eine schriftliche Lizenz
nötig: https://josunlp.de
