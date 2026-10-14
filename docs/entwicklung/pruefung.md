# Tests und Nachweise

## Schnelle Prüfungen

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo xtask trace            # Rückverfolgbarkeit prüfen, docs/safety/traceability.md erzeugen
cargo deny check             # Lizenzen und Advisories
```

## Eigenschaftstests

Property-Tests (proptest) prüfen die Invarianten des Kerns gegen zufällige Folgen von
Anforderungen, Meldungen, Störungen und Übernahmen, RaSTA unter Verlust, Verdopplung und
Vertauschung, und die Übernahme mit fahrenden Zügen:

```sh
PROPTEST_CASES=10000 cargo test --release -p railit-kernel --all-features \
      --test invariants --test borders
PROPTEST_CASES=10000 cargo test --release -p railit-rasta --test rasta
TAKEOVER_CASES=300 cargo test --release -p railit-runtime --test takeover
```

## Szenarien

```sh
cargo run -p railit -- scenario run tests/scenarios/*.toml
cargo run -p railit -- scenario run --channels 3 tests/scenarios/*.toml
cargo run -p railit -- scenario run --node tests/scenarios/*.toml
cargo run -p railit -- scenario run --journal tests/scenarios/*.toml
```

## Bedienplatz

```sh
cd apps/railitd/ui
bun run check        # TypeScript
bun run test         # Unit-Tests: Prüfcode, Anzeigemodell
bun run e2e          # Playwright gegen railitd (Beispielstadt)
```

## Ohne Standardbibliothek

```sh
cargo build --target armv7r-none-eabihf \
      -p railit-types -p railit-topology -p railit-plan -p railit-rules -p railit-kernel \
      -p railit-movement -p railit-rasta -p railit-safety -p railit-embedded
```

## Formale Nachweise und Fuzzing

```sh
cargo kani -p railit-safety && cargo kani -p railit-types && cargo kani -p railit-embedded
cargo kani -p railit-rasta -Z stubbing
sh fuzz/seed.sh && cargo +nightly fuzz run plan_image -- -max_total_time=60
```

Kani weist Voter, Watchdog, Ausgabepuffer, Bitmengen, Zeit, die Telegramme des sicheren
Rechners und RaSTA für alle Eingaben nach. Fuzz-Ziele gibt es für alle Eingangsschnittstellen:
Plan-Image, Netzformate (TOML, railML), Fahrplan, Journal, EULYNX SCI, RaSTA, Telegramme und
Spiegel des sicheren Rechners sowie der Feldrechner.

## Leistung

```sh
cargo run --release -p railit -- bench --check
```

Ziele und Messwerte: [Metriken und Leistung](../leitfaden/metriken).

## CI

Die CI (`.github/workflows/ci.yml`) läuft bei jedem Push und täglich:

| Job | Inhalt |
|-----|--------|
| Format, Lints, Tests, Doku | Bedienplatz bauen, fmt, Clippy, Tests, rustdoc, Rückverfolgbarkeit |
| Eigenschaftstests | Kern und RaSTA mit je 5000 Fällen |
| Bedienplatz | Typprüfung, Unit-Tests, Bauen, Playwright Ende zu Ende |
| Installer | `install.sh` und `install.ps1` auf Linux, macOS und Windows |
| Leistung | `railit bench --check` |
| no_std | Sicherheitsrelevante Crates für Cortex-M und Cortex-R |
| Kani | Formale Nachweise |
| Fuzzing | Alle Fuzz-Ziele mit Zeitbudget |
| Lizenzen & Advisories | `cargo deny` |

Dazu baut `.github/workflows/docs.yml` bei jedem Push und Pull Request diese Dokumentation
(tote Verweise brechen den Build ab) und veröffentlicht sie von `main` aus auf GitHub Pages.
