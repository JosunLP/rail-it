# Mitarbeiten

## Voraussetzungen

| Werkzeug | Version | Wofür |
|----------|---------|-------|
| Rust | 1.97 (fest in `rust-toolchain.toml`) | Alle Crates und Programme |
| Bun | ≥ 1.4 | Bedienplatz (`apps/railitd/ui`) und Dokumentation (`docs/`) |
| Nightly, cargo-fuzz | | Fuzzing (optional) |
| Kani | | Formale Nachweise (optional) |

## Bauen

```sh
(cd apps/railitd/ui && bun install && bun run build)   # Bedienplatz nach ui/dist
cargo build --release
```

`railitd` bettet den gebauten Bedienplatz beim Übersetzen ein (`apps/railitd/build.rs`). Der
Build liegt nicht im Repository: Fehlt `ui/dist`, bettet `railitd` einen Platzhalter ein; mit
`RAILIT_REQUIRE_UI=1` (CI, Releases) scheitert das Bauen stattdessen.

## Aufbau des Repositorys

| Pfad | Inhalt |
|------|--------|
| `crates/` | Bibliotheken: Typen, Topologie, Plan, Projektierung, Kern, Sicherheitsschicht, RaSTA, Adapter, Journal, Simulator, Betriebsführung, Runtime, Import, Spurplan |
| `apps/` | Programme `railit`, `railitd` (mit `ui/`), `railit-field`, `railit-node` |
| `proto/` | gRPC-Vertrag `railit.v1` |
| `examples/` | Beispielnetze und Fahrplan |
| `tests/scenarios/` | Szenarien |
| `fuzz/` | Fuzz-Ziele für cargo-fuzz |
| `xtask/` | `cargo xtask trace`, `release`, `notes` |
| `scripts/` | Installer, Packen, Tests der Installer |
| `docs/` | Diese Dokumentation (VitePress), Anforderungen, Sicherheitsnachweis |

Überblick über die Crates: [Architektur](../leitfaden/architektur).

## Regeln für den Code

- Die sicherheitsrelevanten Crates (`railit-types`, `-topology`, `-plan`, `-rules`, `-kernel`,
  `-movement`, `-rasta`, `-safety`, `-embedded`) sind `no_std` und verbieten `unsafe`,
  `unwrap`, `panic`, direkte Indexierung, ungeprüfte Arithmetik und Gleitkommazahlen. Clippy
  setzt das durch.
- Jede Anforderung (`REQ-…`) ist mit Code und Nachweis verknüpft; `cargo xtask trace` prüft
  das ([Rückverfolgbarkeit](../safety/traceability)).
- Neue Gefährdungen gehören in die [Gefährdungsliste](../safety/hazard-log), ihr Argument in
  den [Safety Case](../safety/safety-case).
- Dokumentation, Bezeichner in der Domäne und Meldungen sind deutsch.

## Vor jedem Commit

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo xtask trace
(cd apps/railitd/ui && bun run check && bun run test)
```

Mehr dazu unter [Tests und Nachweise](./pruefung).

## Lizenz

rail-it steht unter der Non-Commercial Source License (NCSL) v1.0. Beiträge werden unter
derselben Lizenz aufgenommen.
