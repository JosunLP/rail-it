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

Phase 0 (Fundament), Phase 1 (minimaler Kern), Phase 2 (ESTW vollständig: Durchrutschwege,
Flankenschutz, Bahnübergänge, Hilfshandlungen mit Zwei-Personen-Regel, Ersatzsignal,
Erkennung unerlaubter Besetzung, Signalbegriffe Ks und H/V, Journal und Replay) und Phase 3
(Netz und Betrieb: mehrere Stellbereiche mit je eigenem Kern, Streckenblock mit
Erlaubniswechsel, Zugnummernmeldung, Fahrplan, Zuglenkung, Disposition, gRPC-Vertrag,
Benutzer mit Rollen, TLS/mTLS, Auditprotokoll) und Phase 4 (Europa: ETCS Level 2 mit
RBC-Funktion und Mischbetrieb, Feldanbindung über EULYNX SCI und RaSTA, Import und Export von
railML 3) sind umgesetzt.

| Crate | Inhalt | |
|-------|--------|---|
| `railit-types` | Typsichere IDs, Längen, Zeit, Weichenlagen, Bitsets | `no_std`, ohne Allokator |
| `railit-topology` | Netzmodell mit Ports, Validierung, Navigation durch Weichen | `no_std` + `alloc` |
| `railit-plan` | Plan-Image: Binärformat, Zero-Copy-Sicht, SHA-256, Ed25519 | `no_std`, ohne Allokator |
| `railit-planning` | Fahrstraßen, Durchrutschwege, Flankenschutz, Feindlichkeiten; unabhängige Gegenprüfung | |
| `railit-rules` | Regelwerk-Schnittstelle, Lehrregelwerk | `no_std` |
| `railit-kernel` | Sicherheitskern: Fahrwegsicherung, Hilfshandlungen, Invarianten, sicherer Zustand | `no_std`, ohne Allokator |
| `railit-movement` | Signalbegriffe Ks und H/V und ETCS-Fahrerlaubnis aus dem Kernzustand | `no_std`, ohne Allokator |
| `railit-etcs` | RBC-Funktion für ETCS Level 2: Anmeldung, Fahrerlaubnis über Bereichsgrenzen, Notbremsung | |
| `railit-rasta` | RaSTA-Sicherheits- und Wiederholschicht nach EN 50159 | `no_std`, ohne Allokator |
| `railit-adapters` | Feldanbindung über EULYNX SCI-P/-LS/-TDS/-LX und RaSTA zu simulierten Objektsteuerungen | |
| `railit-journal` | Betriebsjournal mit Hashkette, Prüfpunkten und Replay; Auditprotokoll | |
| `railit-sim` | Simulator: Weichen, Signale, Gleisfreimeldung, Bahnübergänge, Züge (auch mit ETCS), Störungen | |
| `railit-ops` | Zugnummernmeldung, Fahrplan, Zuglenkung, Selbstblock, Disposition | |
| `railit-runtime` | Zyklus Kerne ↔ Feld für mehrere Stellbereiche, Zustandsbild, Szenarien, Schlüssel | |
| `railit-import` | rail-it-TOML, railML 3 (Import und Export) | |
| `railitd` | Dienst mit REST/JSON-API, gRPC (`proto/railit/v1`), Ereignisstrom, Rollen, TLS/mTLS, Audit | |
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

### Netz mit mehreren Bahnhöfen: Nordbahn

Drei Stellbereiche (Bahnhof Adorf, eingleisige Strecke mit Selbstblock, Bahnhof Bstadt), je
mit eigenem Kern; ein Fahrplan lässt zwei Züge mit Zuglenkung verkehren.

```sh
railit plan compile examples/nordbahn.toml --out nordbahn/ --key railit.key

railitd --network examples/nordbahn.toml --plan nordbahn/ --trusted-key railit.pub \
        --timetable examples/nordbahn-fahrplan.toml --ars --speed 10 &

railit areas            # Stellbereiche, Grenzen, Erlaubnisse
railit ops list         # Züge mit Zugnummer, nächstem Halt, Verspätung
railit ops hold RE1     # Disposition: zurückhalten, Gleiswechsel, Wartezeit
railit border request 'Strecke>Adorf'   # Anführungszeichen: > ist ein Shell-Zeichen
railit scenario run tests/scenarios/1[3-5]_*.toml
```

### ETCS, EULYNX und railML

```sh
# ETCS Level 2: Zug mit Fahrzeuggerät fährt nach der Fahrerlaubnis des RBC
railit scenario run tests/scenarios/16_nordbahn_etcs.toml tests/scenarios/17_etcs_notbremsung.toml
railit etcs             # angemeldete Züge, Betriebsart, Fahrerlaubnis (online)

# Feldanbindung über EULYNX SCI und RaSTA statt direkt am Simulator
railitd --network examples/beispielstadt.toml --plan beispielstadt.rplan \
        --trusted-key railit.pub --field eulynx &

# Netz als railML 3 ausgeben und wieder einlesen
railit net export examples/nordbahn.toml --out nordbahn.railml
railit net validate nordbahn.railml
railit plan compile nordbahn.railml --out nordbahn/ --key railit.key
```

Für den Betrieb mit Benutzern: `railitd … --users users.toml --audit audit.log
--tls-cert server.pem --tls-key server.key [--tls-client-ca ca.pem]`; die CLI meldet sich mit
`--token` bzw. `--ca`, `--cert`, `--key` an (siehe [API](docs/api.md)).

Exit-Codes der CLI: `0` Erfolg, `1` ungültig oder abgelehnt, `2` Bedien-, Lese- oder
Verbindungsfehler. Mit `--output json` liefern alle Befehle JSON.

## Entwicklung

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo build --target thumbv7em-none-eabihf \
      -p railit-types -p railit-topology -p railit-plan -p railit-rules -p railit-kernel \
      -p railit-movement -p railit-rasta
cargo deny check
PROPTEST_CASES=10000 cargo test --release -p railit-kernel --all-features \
      --test invariants --test borders
PROPTEST_CASES=10000 cargo test --release -p railit-rasta --test rasta
```

Die Toolchain ist in `rust-toolchain.toml` festgelegt. Die sicherheitsrelevanten Crates
verbieten `unsafe`, `unwrap`, `panic`, direkte Indexierung, ungeprüfte Arithmetik und
Gleitkommazahlen.

## Lizenz

rail-it steht unter der [Non-Commercial Source License (NCSL) v1.0](LICENSE). Nicht
kommerzielle Nutzung ist erlaubt. Für jede kommerzielle Nutzung ist eine schriftliche Lizenz
nötig: support@josunlp.de
