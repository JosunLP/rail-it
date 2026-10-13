# Sicherheitsnachweis (Safety Case)

> Stand: Phase 7 · Gliederung nach EN 50129, Abschnitt 5 · lebendes Dokument
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
| Einsatzvarianten | 1. alles auf dem Server (`railitd` mit 1, 2 oder 3 Kanälen je Stellbereich); 2. verteilt: Kern und Sicherheitsschicht auf einem sicheren Rechner (`railit-embedded`; auf dem Server als `railit-node`, Ziel Cortex-R), angebunden über RaSTA mit `railitd --node` |
| Schnittstellen | Plan-Image (signiert), Feld über EULYNX SCI-P/-LS/-TDS/-LX auf RaSTA, Nachbarstellwerke über Grenzmeldungen, ETCS-Fahrzeuge über die RBC-Funktion, Bedienung über REST/gRPC und den Bedienplatz (`/ui/`) |
| Gefährdungen | [Gefährdungsliste](hazard-log.md), H-001 bis H-016 |
| Anforderungen | [Anforderungen](../requirements/README.md), 101 Anforderungen mit stabilen IDs |

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
| Prüfung gegen ein vom Kern unabhängiges Modell unter beliebigen Bedienungen und Meldungen | Property-Tests `invariants`, `borders` (5 000 Fälle in der CI) |
| Robustheit gegen jeden strukturell gültigen Plan | Fuzz-Ziel `plan_image` (Kern, Signalbilder, Fahrerlaubnis auf veränderten Plänen), REQ-SAF-007 |
| Signalisierung folgt nur dem Kern | REQ-MOV-001 … 004, REQ-ETCS-002/003 |
| Zwischen Stellbereichen: Erlaubniswechsel und Grenzsicherung | REQ-IXL-017 … 021, Property-Tests mit zwei und drei echten Kernen |

### G2 – Der Plan entspricht dem Netz und ist unverfälscht

| Strategie | Belege |
|-----------|--------|
| Vollständige Netzvalidierung, auch für railML-Import | REQ-TOP-001 … 014 |
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

Der Bedienplatz selbst bleibt SIL 0. Das Argument stützt sich nicht auf seine Richtigkeit:
Eine falsche Anzeige kann den Bediener täuschen, aber der Kern führt eine Hilfshandlung nur
aus, wenn die Bestätigung genau die angeforderte wiederholt, und jede Regelbedienung prüft er
ohnehin selbst. Die Anzeigeüberwachung und der Prüfcode verringern die Wahrscheinlichkeit
einer Fehlbedienung, sie sind keine Sicherheitsfunktion.
| Rücksetzen der Sicherheitsschicht nur durch die Instandhaltung, protokolliert | REQ-SAF-004 |

### G6 – Jeder Hergang ist nachvollziehbar

| Strategie | Belege |
|-----------|--------|
| Journal aller Eingaben mit Hashkette, deterministisches Abspielen, auch über Abschaltung und Rücksetzen der Sicherheitsschicht | REQ-JRN-001 … 003, `journal_covers_trip_and_reset_of_the_safety_layer` |

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
7. Journal und Auditprotokoll außerhalb des Stellwerksrechners sichern.
8. Hilfshandlungen am Bedienplatz bestätigen; wer bestätigt, liest die Hilfshandlung aus der
   eigenen Anzeige und gibt das Element selbst ein, ohne es vom anfordernden Bediener zu
   übernehmen. Zeigt der Bedienplatz „Anzeige gestört“, wird nicht bedient.

## Teil 7 – Offene Punkte bis zu einer Zulassung

| Punkt | Bezug |
|-------|-------|
| Unabhängige Rollen (Verifikation, Validierung, ISA) besetzen | EN 50126/50129 |
| Systematische Gefährdungsanalyse mit Bewertung (FMEA, FTA, quantitative Ziele) | Gefährdungsliste |
| Qualifizierte Toolchain (z. B. Ferrocene) für die SIL-4-Crates | Konzept 16.2 |
| Hardware des sicheren Rechners festlegen; Kanäle auf getrennter Hardware bzw. im Lockstep, Speicherprüfungen | H-015, Konzept 19 |
| Journal auf dem Knoten des sicheren Rechners (heute: mit `--node` kein Journal); Knoten auf der Zielhardware | REQ-EMB, REQ-JRN |
| Kani-Nachweise für den gesamten Kernzyklus (heute Eigenschaftstests und Fuzzing) | H-007 |
| Sicherheitscode von RaSTA nach Zulassungsanforderung prüfen | H-014 |
| Regelwerke `de-estw` und `etcs` mit den Regeln eines Betreibers | Teil 5 |
| Import aus PlanPro, SCI-ILS zwischen Stellwerksrechnern | Konzept 18 |
| Prüfung gegen reale EULYNX-Objektsteuerungen und ETCS-Fahrzeuggeräte | H-010, H-014 |
| Ergonomische Prüfung des Bedienplatzes mit Bedienern; ggf. zweikanalige sichere Anzeige | H-016 |

## Teil 8 – Schlussfolgerung

Für den Geltungsbereich – Simulation, Ausbildung und Prüfstände – sind die identifizierten
Gefährdungen H-001 bis H-016 durch Anforderungen beherrscht, deren Umsetzung durch Tests,
Szenarien, Eigenschaftstests, formale Nachweise und Fuzzing belegt und lückenlos
rückverfolgbar ist. Für den realen Bahnbetrieb reicht das nicht: Die Punkte aus Teil 7,
insbesondere unabhängige Begutachtung, qualifizierte Toolchain und Hardware, sind
Voraussetzung jeder Zulassung.
