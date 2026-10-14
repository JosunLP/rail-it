# Architektur

```text
   Bedienplatz (Browser)      railit (CLI)        gRPC-Clients
            │                      │                    │
            └──────── REST/JSON · SSE · gRPC ───────────┘
                                   │
   ┌──────────────────────────── railitd ────────────────────────────┐
   │  Dienst: Anmeldung, Rollen, Audit, Metriken, Journal-Rückschau   │
   │  Engine: fester Zyklus                                           │
   │    Feld fortschreiben → Meldungen → Kerne → Befehle ans Feld     │
   │  ┌───────────────┐ ┌───────────────┐       Betriebsführung (ops) │
   │  │ Stellbereich  │ │ Stellbereich  │ …     RBC (ETCS)            │
   │  │ Kanäle+Voter  │ │ oder Knoten   │       Journal je Bereich    │
   │  │  Kern (SIL 4) │ │ (railit-node) │                             │
   │  └───────────────┘ └───────────────┘                             │
   └─────────────────────────────┬─────────────────────────────────────┘
                                 │  direkt · EULYNX/RaSTA · RaSTA/UDP
                     Simulator · Objektsteuerungen · railit-field
```

## Grundsätze

1. **Der Kern ist eine reine Funktion.** `(Zustand, Zeit, Eingaben) → (Zustand, Befehle,
   Ereignisse)` – ohne I/O, ohne Uhr, ohne Allokation, ohne Panikpfade. Damit ist er
   deterministisch, auf jedem Rechner gleich und abspielbar.
2. **Der Plan ist Daten.** Fahrstraßen, Durchrutschwege, Flankenschutz und Feindlichkeiten
   werden offline projektiert, gegengeprüft und als signiertes Image geladen. Der Kern rechnet
   nichts davon zur Laufzeit aus.
3. **Restriktiv bei Unsicherheit.** Fehlende oder alte Meldungen gelten als gestört, ein
   verletzter Invariant führt in den sicheren Zustand: alle Signale Halt.
4. **Bedienen heißt anfordern.** Kein Client stellt etwas selbst – jede Bedienung ist eine
   Anforderung, die der Kern prüft.
5. **Alles ist nachvollziehbar.** Jede Eingabe steht im Journal; das Replay kommt zum selben
   Zustand.

## Bausteine

| Crate / Programm | Aufgabe | Sicherheit |
|------------------|---------|------------|
| `railit-types` | IDs, Längen, Zeit, Bitsets | `no_std`, SIL 4 angestrebt |
| `railit-topology` | Netzmodell, Validierung, Navigation | `no_std` + `alloc` |
| `railit-plan` | Plan-Image: Format, Zero-Copy-Sicht, SHA-256, Ed25519 | `no_std`, SIL 4 |
| `railit-planning` | Fahrstraßengenerator mit unabhängiger Gegenprüfung | abgesichert durch Gegenprüfung |
| `railit-rules` | Regelwerk-Schnittstelle, Lehrregelwerk | `no_std`, SIL 4 |
| `railit-kernel` | Sicherheitskern: Fahrwegsicherung, Hilfshandlungen, Invarianten | `no_std`, SIL 4 |
| `railit-movement` | Signalbegriffe Ks und H/V, ETCS-Fahrerlaubnis | `no_std`, SIL 4 |
| `railit-safety` | Kanäle, Voter, Watchdog, sicherer Zustand | `no_std`, SIL 4 |
| `railit-embedded` | Runtime des sicheren Rechners (Cortex-R) | `no_std`, SIL 4 |
| `railit-rasta` | RaSTA nach EN 50159 | `no_std`, SIL 4 |
| `railit-adapters` | EULYNX SCI, Objektsteuerungen, Feldrechner mit Umschalter | Umschalter sicherheitsrelevant |
| `railit-journal` | Journal, Siegel, Replay, Mitleser, Auditprotokoll | |
| `railit-sim` | Simulator: Weichen, Signale, Gleisfreimeldung, Bahnübergänge, Züge | |
| `railit-ops` | Zugnummern, Fahrplan, Zuglenkung, Disposition | SIL 0 (stellt nur Anforderungen) |
| `railit-etcs` | RBC-Funktion für ETCS Level 2 | |
| `railit-runtime` | Zyklus Kerne ↔ Feld, Ausgabesperre, Übernahme, Szenarien | |
| `railit-import` | rail-it-TOML, railML 3 | |
| `railit-layout` | Spurplan aus der Topologie, SVG | |
| `railitd` | Dienst, API, Bedienplatz, Bereitschaftsrechner | SIL 0 |
| `railit` | Kommandozeile | SIL 0 |
| `railit-node` | Knoten des sicheren Rechners als Serverprogramm | |
| `railit-field` | Feldrechner mit Umschalter | |

Die SIL-4-Crates verbieten `unsafe` und schließen per Lint `unwrap`, Indexzugriffe ohne Prüfung
und ungeprüfte Arithmetik aus. Ihre Kernfunktionen sind mit Property-Tests, Fuzzing und – wo
möglich – Kani-Nachweisen belegt.

## Einsatzvarianten

1. **Alles auf einem Server:** `railitd` mit 1, 2 oder 3 Kanälen je Stellbereich.
2. **Verteilt:** Kern und Sicherheitsschicht auf einem sicheren Rechner (`railit-node`, Ziel
   Cortex-R), angebunden über RaSTA; `railitd` führt einen geprüften Spiegel.
3. **Haupt- und Bereitschaftsrechner** an einem Feldrechner mit Umschalter; bei Ausfall
   übernimmt der Bereitschaftsrechner ([Übernahme](./uebernahme)).

Das ausführliche Konzept mit allen Entscheidungen steht im [Konzept](/KONZEPT).
