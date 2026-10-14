# Sicherheitsnachweis (Safety Case)

> Stand: Phase 10 · Gliederung nach EN 50129, Abschnitt 5 · lebendes Dokument
>
> **Geltungsbereich:** rail-it als **Kandidat für den Laborbetrieb** – Simulation, Ausbildung,
> Prüfstände. rail-it ist **nicht zugelassen** für den realen Bahnbetrieb. Dieser Nachweis
> bündelt die Argumente und Belege, die bisher vorliegen, und benennt offen, was für eine
> Zulassung fehlt (Teil 7). Er ersetzt keine Begutachtung durch einen unabhängigen
> Sachverständigen (ISA).

## Teil 1 – Definition des Systems

| | |
|---|---|
| Gegenstand | Engine für Netz- und Stellwerkmanagement: Fahrwegsicherung (Kern), Signalisierung (Ks, H/V, ETCS Level 2), Betriebsführung, Dienst mit API, CLI und Bedienplatz |
| Beschreibung | [Konzept](../KONZEPT.md), Abschnitte 1 bis 15 |
| Sicherheitsrelevante Teile (SIL 4 angestrebt) | `railit-types`, `railit-plan`, `railit-rules`, `railit-kernel`, `railit-movement`, `railit-safety`, `railit-rasta`, `railit-embedded` – alle `no_std`, ohne `unsafe`, ohne Panikpfade |
| Nicht sicherheitsrelevant (SIL 0) | Import, Projektierungswerkzeug (abgesichert durch unabhängige Gegenprüfung und Signatur), Simulator, Betriebsführung, Dienst, CLI, Bedienplatz und Spurplan-Geometrie |
| Einsatzvarianten | 1. alles auf dem Server (`railitd` mit 1, 2 oder 3 Kanälen je Stellbereich); 2. verteilt: Kern und Sicherheitsschicht auf einem sicheren Rechner (`railit-embedded`; auf dem Server als `railit-node`, Ziel Cortex-R), angebunden über RaSTA mit `railitd --node`; 3. Haupt- und Bereitschaftsrechner (`railitd --standby … --takeover-after-ms`) an einem Feldrechner mit Umschalter (`railit-field`), Feld über RaSTA/UDP |
| Sicherheitsrelevant außerhalb des Kerns | Umschalter des Feldrechners (`railit_adapters::net`): Er entscheidet, welches Stellwerk stellt. Heute Teil des simulierten Feldrechners; für einen realen Einsatz in sicherer Feldtechnik umzusetzen (Teil 7) |
| Schnittstellen | Plan-Image (signiert), Feld über EULYNX SCI-P/-LS/-TDS/-LX auf RaSTA, Nachbarstellwerke über Grenzmeldungen, ETCS-Fahrzeuge über die RBC-Funktion, Bedienung über REST/gRPC und den Bedienplatz (`/ui/`) |
| Gefährdungen | [Gefährdungsliste](hazard-log.md), H-001 bis H-017 |
| Anforderungen | [Anforderungen](../requirements/README.md), 120 Anforderungen mit stabilen IDs |

## Teil 2 – Qualitätsmanagementbericht

| Maßnahme | Umsetzung | Beleg |
|----------|-----------|-------|
| Versionsverwaltung | Git; jede Änderung als Commit mit Beschreibung | Repository |
| Reproduzierbarkeit | Gepinnte Toolchain (`rust-toolchain.toml`), `Cargo.lock`, `--locked` in der CI | CI |
| Codierregeln | Lints auf `deny` in allen SIL-4-Crates: kein `unwrap`/`expect`/`panic`, keine direkte Indexierung, keine ungeprüfte Arithmetik, keine Gleitkommazahlen, keine `as`-Umwandlungen; `forbid(unsafe_code)` im gesamten Workspace; Clippy pedantisch | `lib.rs` der Crates, `Cargo.toml` |
| Lieferkette | `cargo-deny` (Lizenzen, Advisories, Quellen) | CI-Job „Lizenzen & Advisories“ |
| Prüfung jeder Änderung | Format, Clippy, alle Tests, Dokumentation ohne Warnung, `no_std`-Builds für Cortex-M und Cortex-R, Eigenschaftstests mit 5 000 Fällen, Kani, Rückverfolgbarkeit; Fuzzing wöchentlich | `.github/workflows/ci.yml` |
| Offen | QM-System (z. B. ISO 9001), Review-Pflicht mit unabhängigem Prüfer, Konfigurationsmanagement für Pläne im Betrieb | Teil 7 |

## Teil 3 – Sicherheitsmanagementbericht

| Tätigkeit (EN 50126) | Umsetzung | Beleg |
|----------------------|-----------|-------|
| Gefährdungsidentifikation | Gefährdungsliste, mit jeder Phase fortgeschrieben | [hazard-log.md](hazard-log.md) |
| Sicherheitsanforderungen | Je Gefährdung Anforderungen mit ID, Gefährdungsbezug und Nachweis | [requirements/](../requirements/README.md) |
| Rückverfolgbarkeit | Anforderung → Code → Nachweis, automatisch erzeugt und in der CI geprüft (`cargo xtask trace --check`): jede ID im Code ist dokumentiert, jede Anforderung hat einen Nachweis, jeder genannte Nachweis existiert | [traceability.md](traceability.md) |
| Verifikation | Modul- und Integrationstests, Eigenschaftstests (Invarianten, Streckenblock, RaSTA, Voter), unabhängige Gegenprüfung der Projektierung, formale Nachweise (Kani), Fuzzing | CI-Jobs, Teil 4 |
| Validierung | Deklarative Betriebsszenarien (21) gegen den Simulator, alle zusätzlich mit 2-von-3-Kanälen; Ende-zu-Ende-Tests von Dienst und CLI | `tests/scenarios/`, `apps/*/tests/` |
| Betrieb und Nachvollziehbarkeit | Betriebsjournal mit Hashkette und Replay, Auditprotokoll aller Bedienungen | REQ-JRN, REQ-SEC |
| Rollen | Implementierung, Verifikation, Validierung und Begutachtung müssen vor einer Zulassung unabhängig besetzt werden | Teil 7 |

## Teil 4 – Technischer Sicherheitsbericht

Die Argumentation folgt einer Zielstruktur: Das Hauptziel wird in Teilziele zerlegt, jedes
Teilziel durch eine Strategie und Belege gestützt.

**G0 – rail-it ist für den Laborbetrieb hinreichend sicher.**
Kontext: nur Simulation, Ausbildung, Prüfstände (Teil 1); Anwendungsbedingungen (Teil 6).

### G1 – Die Fahrwegsicherung lässt keine unsichere Fahrerlaubnis zu

| Strategie | Belege |
|-----------|--------|
| Sicherheitsprinzipien im Kern: nur positiv bestätigte Zustände, Fahrt nur bei erfüllten Bedingungen, Verschlüsse, Durchrutschwege, Flankenschutz, Bahnübergänge, Zeitverschluss | REQ-IXL-002 … 016 |
| Invarianten nach jedem Zyklus mit unabhängiger Neuberechnung; Verletzung ⇒ sicherer Zustand | REQ-IXL-008, `invariants.rs` |
| Invariantenprüfung in linearer Zeit, gleichwertig zur direkten Fassung (Vergleich auf zufälligen und verfälschten Zuständen), Zustand danach bitgleich; Leistungsziele in der CI gemessen | REQ-IXL-023/024, REQ-PLN-020, `linear_check_agrees_with_reference` |
| Prüfung gegen ein vom Kern unabhängiges Modell unter beliebigen Bedienungen und Meldungen | Property-Tests `invariants`, `borders` (5 000 Fälle in der CI) |
| Robustheit gegen jeden strukturell gültigen Plan | Fuzz-Ziel `plan_image` (Kern, Signalbilder, Fahrerlaubnis auf veränderten Plänen), REQ-SAF-007 |
| Signalisierung folgt nur dem Kern | REQ-MOV-001 … 004, REQ-ETCS-002/003 |
| Zwischen Stellbereichen: Erlaubniswechsel und Grenzsicherung | REQ-IXL-017 … 021, Property-Tests mit zwei und drei echten Kernen |

### G2 – Der Plan entspricht dem Netz und ist unverfälscht

| Strategie | Belege |
|-----------|--------|
| Vollständige Netzvalidierung, auch für railML- und PlanPro-Import; Kreuzungen und Kreuzungsweichen | REQ-TOP-001 … 017 |
| Gekoppelte Weichen: Partnerweiche in jeder Fahrstraße verlangt, beim Laden geprüft, nur gemeinsam gestellt | REQ-PLN-021, REQ-IXL-026, Szenario 25 |
| Generator und unabhängige Gegenprüfung müssen übereinstimmen | REQ-PLN-013 |
| Plan-Image mit SHA-256 und Ed25519; nur geprüfte Pläne werden verwendet | REQ-PLN-001 … 004 |
| Abgleich Plan ↔ Netz beim Start | REQ-SVC-001 |

### G3 – Zufällige Hardwarefehler und Softwarefehler führen in den sicheren Zustand

| Strategie | Belege |
|-----------|--------|
| Redundante Kanäle mit Voter (2-von-2, 2-von-3), Vergleich von Ausgaben und Zustand | REQ-SAF-001 … 003, Property-Test gegen Referenzkern mit Fehlerinjektion |
| Voter formal nachgewiesen: gibt nur bestätigte Ausgaben weiter, schaltet genau ohne Mehrheit ab | Kani `voter_forwards_only_confirmed_outputs` (alle Eingaben) |
| Sicherer Zustand unabhängig von den Kanälen, allein aus dem Plan | REQ-SAF-004 |
| Watchdog für Rechenzeit und Zyklusabstand | REQ-SAF-005, Kani `watchdog_detects_every_overrun` (fand einen Randfehler bei unbegrenzter Frist, behoben) |
| Keine Panikpfade, keine Überläufe | Lints, Kani-Nachweise (REQ-SAF-006), Fuzzing (REQ-SAF-007) |
| Embedded-Runtime ohne Allokator für sichere Rechner | REQ-EMB-001 … 004, Build für `armv7r-none-eabihf` |
| Sicherer Rechner im Dienst: Knoten maßgeblich, Spiegel im Dienst per Digest geprüft und per Zustandsabgleich nachgeführt; Ausfall des Knotens ⇒ alle Signale Halt | REQ-EMB-005, REQ-EMB-006, alle Szenarien über Knoten, `node_process_over_udp` (eigener Prozess über UDP, Ausfall und Neustart) |

### G4 – Die Übertragung ist gegen die Bedrohungen nach EN 50159 gesichert

| Strategie | Belege |
|-----------|--------|
| RaSTA: Sicherheitscode, Kennungen, Datenindex, Zeitstempel, Lebenszeichen | REQ-COM-001 … 005, Property-Tests unter Verlust, Verdopplung, Vertauschung in beide Richtungen |
| Verlust führt zur Wiederholung statt zum Abbau; Wiederholung idempotent | REQ-COM-007 (20 % Verlust ohne Abbau; vorher brachen Strecken schon bei 5 % Verlust ab) |
| Fehlende Meldungen wirken wie gestört | REQ-COM-006, REQ-IXL-009, REQ-EMB-002 |
| Empfang beliebiger Bytes ohne Panik | Kani `closed_connection_never_delivers` und MD4-Nachweise; Fuzz-Ziele `rasta` (aufgebaute Verbindung), `sci`, `node_wire`, `mirror_state` |

### G5 – Bedienhandlungen können keine unsichere Lage herbeiführen

| Strategie | Belege |
|-----------|--------|
| Bedienung stellt nur Anforderungen; Stellbefehle erzeugt nur der Kern | REQ-SVC-002 |
| Anmeldung, Rollen, Zuständigkeit je Stellbereich, Auditprotokoll | REQ-SEC-001 … 005 |
| Hilfshandlungen nur mit zweitem Bediener, Frist und Zählwerk | REQ-IXL-014, REQ-SVC-006 |
| Bestätigung wiederholt die Hilfshandlung; Abweichung ⇒ nichts ausgeführt, Hilfshandlung verfällt (SIL-4-Kern) | REQ-IXL-022, `confirmation_must_restate_the_operation` |
| Prüfcode der Anzeige: Der bestätigende Bedienplatz erkennt eine verfälschte Anzeige, der Dienst einen abweichenden Code | REQ-UI-003, `check_code_vectors`, E2E-Tests des Bedienplatzes |
| Veraltete Anzeige wird erkannt und sperrt die Bedienung | REQ-UI-002 |
| Spurplan aus der Topologie berechnet, nicht von Hand gezeichnet | REQ-UI-001 |
| Rücksetzen der Sicherheitsschicht nur durch die Instandhaltung, protokolliert | REQ-SAF-004 |

Der Bedienplatz selbst bleibt SIL 0. Das Argument stützt sich nicht auf seine Richtigkeit:
Eine falsche Anzeige kann den Bediener täuschen, aber der Kern führt eine Hilfshandlung nur
aus, wenn die Bestätigung genau die angeforderte wiederholt, und jede Regelbedienung prüft er
ohnehin selbst. Die Anzeigeüberwachung und der Prüfcode verringern die Wahrscheinlichkeit
einer Fehlbedienung, sie sind keine Sicherheitsfunktion.

### G6 – Jeder Hergang ist nachvollziehbar

| Strategie | Belege |
|-----------|--------|
| Journal aller Eingaben mit Hashkette, deterministisches Abspielen, auch über Abschaltung und Rücksetzen der Sicherheitsschicht | REQ-JRN-001 … 003, `journal_covers_trip_and_reset_of_the_safety_layer` |
| Siegel des Stellwerks über Journal und Auditprotokoll: Eine neu geschriebene Kette ist erkennbar | REQ-JRN-004, `rewritten_journal_cannot_carry_the_station_seal` |
| Eingaben als Änderung, Zustandsabzüge und paralleles Replay mit Gleichheitsprüfung an jeder Abschnittsgrenze; ein gefälschter Abzug fällt auf | REQ-JRN-005/006, `forged_snapshot_is_detected_by_replay` |
| Journal auch im Knotenbetrieb: geprüfte Zyklen des Spiegels und Zustandsübernahmen; alle Szenarien in allen Betriebsarten mitgeschrieben und abgespielt | REQ-JRN-007, `node_journal_replays_across_resyncs` |
| Mitlesen: Ein zweiter Rechner prüft und rechnet das Journal laufend nach und zeigt den Zustand ohne Bedienung an; Rückschau am Bedienplatz ohne Bedienung und ohne verfälschte Signalfarben | REQ-JRN-009, REQ-SVC-010/011, REQ-UI-005, `standby_follows_the_primary` |

### G7 – Der Ausfall des Stellwerksrechners wird ohne unsichere Lage überbrückt

| Strategie | Belege |
|-----------|--------|
| Ausgaben (Stellbefehle, Grenzmeldungen, ETCS-Fahrerlaubnisse) erst nach Quittung des Bereitschaftsrechners; Quittung mit Zyklus und Hash; Zusage, die Sperre eine bestimmte Zeit zu halten | REQ-SVC-012, `outputs_wait_for_all_areas`, `lapse_releases_everything_and_rearms_after_catching_up` |
| Nur ein Stellwerk stellt: Umschalter im Feldrechner, Wechsel erst nach Abbau aller Verbindungen und Umschaltzeit, nur mit echtem Verbindungsaufbau; ein zurückkehrender Hauptrechner bleibt ausgesperrt | REQ-SVC-013, `second_interlocking_is_rejected_while_the_leader_lives`, `standby_leads_after_the_leader_is_lost` |
| Übernahme mit dem mitgelesenen Zustand nur innerhalb der Zusage, sonst Neustart; Zustand restriktiv angeglichen (Verschlüsse bleiben, Fahrt fällt auf Halt), Invarianten geprüft | REQ-SVC-014, REQ-IXL-025, `kernel_never_becomes_unsafe` (mit Übernahme als Aktion) |
| Ausfall an zufälliger Stelle mit Zügen, verspätetem Mitlesen und Bedienung danach: keine Entgleisung, kein Aufschneiden, kein offener Bahnübergang, kein Haltfall-Überfahren | `takeover_in_beispielstadt_is_safe`, `takeover_in_nordbahn_is_safe` (je 300 Fälle lokal) |
| Gegenprobe: Ohne Sperre öffnet ein Bereitschaftsrechner mit Rückstand einen Bahnübergang vor einem Zug | `stale_takeover_without_gate_opens_the_crossing_before_a_train` |
| Drei Prozesse (Feldrechner, Haupt- und Bereitschaftsrechner), Ausfall des Hauptrechners: warme Übernahme mit Zusage, Neustart ohne | `standby_takes_over_with_the_followed_state`, `standby_restarts_without_a_promise` |

Das Argument stützt sich auf eine Zeitbedingung: Der Bereitschaftsrechner übernimmt den
mitgelesenen Zustand nur, wenn er die Führung am Feldrechner erhalten hat, bevor die Zusage
des Hauptrechners abläuft. Bis dahin hat der Hauptrechner nichts freigegeben, was der
Bereitschaftsrechner nicht kennt, und danach erreicht er das Feld nicht mehr. Die Zusage wird
mit der Uhr des Bereitschaftsrechners ab dem Absenden der Quittung gerechnet und um zehn
Prozent und 200 ms gekürzt. Sie ist nur sinnvoll, wenn die Frist (`--standby-gate-ms`) die
Erkennung (`--takeover-after-ms`), `t_max` von RaSTA und die Umschaltzeit deutlich übersteigt
(Anwendungsbedingung 9).

## Teil 5 – Verwandte Sicherheitsnachweise

rail-it ist als **generisches Produkt** angelegt. Für einen Einsatz sind zusätzlich nötig:

- ein **generischer Anwendungsnachweis** je Regelwerk (heute nur das Lehrregelwerk
  `GenericRules`; für `de-estw` und `etcs` sind die Regeln des Betreibers einzuarbeiten),
- ein **spezifischer Anwendungsnachweis** je Anlage (Netz, Plan, Feldelemente,
  Hardware),
- die Nachweise der eingesetzten Komponenten Dritter: Objektsteuerungen (EULYNX),
  Fahrzeuggeräte (ETCS), Rechnerhardware, Toolchain.

## Teil 6 – Sicherheitsbezogene Anwendungsbedingungen

1. Nur Simulation, Ausbildung und Prüfstände; keine Steuerung realer Fahrzeuge oder
   Feldelemente im Betrieb.
2. Nur signierte Pläne (`--trusted-key`); `--allow-unsigned` ausschließlich für Tests.
3. Jeder Plan wird mit `railit plan compile` erzeugt; Generator und Gegenprüfung müssen
   übereinstimmen. Pläne werden vor der Verwendung fachlich freigegeben.
4. Im Mehrbenutzerbetrieb Benutzerdatei mit Rollen, TLS bzw. mTLS und Auditprotokoll;
   Bedienernummern sind eindeutig je Person.
5. Für Prüfstände mit realen Objektsteuerungen: Betrieb mit `--channels 2` oder `3` und
   Watchdog (`--watchdog-ms`); RaSTA-Parameter (`t_max`, Anfangswerte des Sicherheitscodes)
   je Anlage festlegen.
6. Rücksetzen der Sicherheitsschicht nur nach Klärung der Ursache; danach gelten alle
   Elemente als unbekannt, Fahrstraßen sind neu einzustellen.
7. Journal und Auditprotokoll mit `--seal-key` siegeln und außerhalb des Stellwerksrechners
   sichern; den öffentlichen Schlüssel zur Prüfung getrennt verwahren.
8. Hilfshandlungen am Bedienplatz bestätigen; wer bestätigt, liest die Hilfshandlung aus der
   eigenen Anzeige und gibt das Element selbst ein, ohne es vom anfordernden Bediener zu
   übernehmen. Zeigt der Bedienplatz „Anzeige gestört“, wird nicht bedient.
9. Bereitschaftsrechner nur mit Feldrechner und Umschalter (`--field remote`); genau ein
   Bereitschaftsrechner mit eigenem Zugang der Rolle `bereitschaft`. Die Frist der
   Ausgabesperre (`--standby-gate-ms`) muss die Summe aus Erkennungszeit
   (`--takeover-after-ms`), `t_max` von RaSTA und Umschaltzeit des Feldrechners mit Reserve
   übersteigen (Vorgabe im Test: 8 s gegenüber 0,5 s + 1,8 s + 0,5 s). Nach einer Übernahme
   sind Fahrstraßen mit Fahrt neu einzustellen; nach einem Neustart gelten alle Elemente als
   unbekannt.

## Teil 7 – Offene Punkte bis zu einer Zulassung

| Punkt | Bezug |
|-------|-------|
| Unabhängige Rollen (Verifikation, Validierung, ISA) besetzen | EN 50126/50129 |
| Systematische Gefährdungsanalyse mit Bewertung (FMEA, FTA, quantitative Ziele) | Gefährdungsliste |
| Qualifizierte Toolchain (z. B. Ferrocene) für die SIL-4-Crates | Konzept 16.2 |
| Hardware des sicheren Rechners festlegen; Kanäle auf getrennter Hardware bzw. im Lockstep, Speicherprüfungen | H-015, Konzept 19 |
| Knoten auf der Zielhardware | REQ-EMB |
| Zeitbedingungen der Übernahme auf Zielhardware nachweisen (Gangunterschied der Uhren, Laufzeiten); Umschalter in realer Feldtechnik; Übernahme der Betriebsführung | REQ-SVC-012 … 014, H-017 |
| Kani-Nachweise für den gesamten Kernzyklus (heute Eigenschaftstests und Fuzzing) | H-007 |
| Sicherheitscode von RaSTA nach Zulassungsanforderung prüfen | H-014 |
| Regelwerke `de-estw` und `etcs` mit den Regeln eines Betreibers | Teil 5 |
| PlanPro-Import gegen Planungen realer Anlagen prüfen (Signalstandort und Freimeldegrenze, Kreuzungen, Gleissperren); SCI-ILS zwischen Stellwerken auf getrennter Hardware prüfen (heute getrennte Prozesse über UDP, REQ-COM-010); Betriebsführung, ETCS und Bereitschaftsrechner bei verteilten Stellbereichen | Konzept 18, H-012 |
| Prüfung gegen reale EULYNX-Objektsteuerungen und ETCS-Fahrzeuggeräte | H-010, H-014 |
| Ergonomische Prüfung des Bedienplatzes mit Bedienern; ggf. zweikanalige sichere Anzeige | H-016 |

## Teil 8 – Schlussfolgerung

Für den Geltungsbereich – Simulation, Ausbildung und Prüfstände – sind die identifizierten
Gefährdungen H-001 bis H-016 durch Anforderungen beherrscht, deren Umsetzung durch Tests,
Szenarien, Eigenschaftstests, formale Nachweise und Fuzzing belegt und lückenlos
rückverfolgbar ist. Für den realen Bahnbetrieb reicht das nicht: Die Punkte aus Teil 7,
insbesondere unabhängige Begutachtung, qualifizierte Toolchain und Hardware, sind
Voraussetzung jeder Zulassung.
