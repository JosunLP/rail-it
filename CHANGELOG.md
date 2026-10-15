# Änderungen

Alle Releases von rail-it. Jeder Abschnitt nennt die Commits seit dem vorigen Release.

## [1.0.0] – 2026-10-15

### Neu

- feat(journal): Betriebsjournal mit Hashkette, Prüfpunkten und Replay
- feat(railitd, cli): Hilfshandlungen und Journal in Dienst und Kommandozeile
- feat(topology, plan, planning): Stellbereiche, Bereichsgrenzen, Plan-Image v3
- feat(kernel): Erlaubniswechsel und Grenzsicherung zwischen Stellbereichen
- feat(runtime, railitd): Netz-Runtime mit mehreren Stellbereichen
- feat(ops): Zugnummernmeldung, Fahrplan, Zuglenkung und Disposition
- feat(railitd, journal): Benutzer, Rollen, Auditprotokoll, TLS und mTLS
- feat(railitd): gRPC-Vertrag railit.v1 auf dem Port der REST-API
- feat(cli): Stellbereiche, Betriebsführung, Audit, Token und TLS-Client
- feat(plan, movement): Plan v4 mit Längen und ETCS-Fahrerlaubnis
- feat(etcs): RBC für ETCS Level 2 und Züge mit Führerraumsignalisierung
- feat(rasta, adapters): EULYNX SCI über RaSTA mit simulierten Objektsteuerungen
- feat(import): railML-3-Import und -Export für Infrastrukturnetze
- feat(safety): redundante Kanäle mit Voter (2-von-2, 2-von-3), Watchdog, sicherer Zustand
- feat(runtime, railitd): Stellbereiche mit redundanten Kanälen, Voter und Watchdog
- feat(embedded): Runtime für den sicheren Rechner (Cortex-R) mit Binärprotokoll über RaSTA
- feat(proofs, fuzz): formale Nachweise mit Kani und Fuzzing aller Eingangsschnittstellen
- feat(layout): automatischer Spurplan aus dem Netzmodell, /v1/layout, railit net layout
- feat(kernel, railitd): sichere Anzeige- und Bestätigungskette für Hilfshandlungen
- feat(ui): Web-Bedienplatz mit Anzeigeüberwachung und sicherer Bestätigungskette; Phase 6 abgeschlossen
- feat(node): sicherer Rechner im Dienst – Knoten mit geprüftem Spiegel; RaSTA-Wiederholung robust
- feat(journal): Siegel des Stellwerks über Journal und Auditprotokoll
- feat(ui): Bedienplatz für Netz und Betrieb – Zuglenkung, Disposition, Stelltisch-Eingabe
- P7-4: Leistungsmessung mit Budget in der CI, lineare Invarianten, paralleles Replay
- P8-1: Journal im Knotenbetrieb
- P8-2: Diagnose aus dem Journal – Zustand zu einem Zeitpunkt, Ereignisse eines Zeitfensters
- P8-3: Metriken für den Betrieb (GET /v1/metrics, Prometheus-Textformat)
- P9-1: Mitlesen eines wachsenden Journals
- P9-2: Rückschau im Dienst (REST)
- P9-3: Rückschau im Bedienplatz
- P9-4: Bereitschaftsrechner über den Journalstrom; Phase 9 abgeschlossen
- P10-1: Kern – Übernahme eines mitgelesenen Zustands
- P10-2: Ausgabe erst nach Quittung und Übernahme in der Runtime
- P10-3: Feldrechner über das Netz mit Umschalter
- P10-4: Übernahme des Betriebs durch den Bereitschaftsrechner
- P10-5: Doku, Sicherheitsnachweis und Konzept v0.12 – Phase 10 abgeschlossen
- P11-1: Nachbarstellwerke über EULYNX SCI-ILS und RaSTA
- P11-2: Stellbereiche in getrennten Prozessen über SCI-ILS auf UDP
- P12-1: PlanPro-Import und -Export (1.9, 1.10)
- P12-2: Kreuzungen ohne bewegliche Teile
- P13-1: Kreuzungsweichen und gekoppelte Weichen
- P13-2: Fahrplan und Zuglenkung über Prozessgrenzen
- P13-3: PZB im Simulator
- P13-4: Gekoppelte Weichen am Feld als ein Antrieb
- P13-5: Bereitschaftsrechner je Teilnetz
- P14-1: Kreuzungsweichen in PlanPro
- P14-2: Hauptsignale kurz vor der Freimeldegrenze
- P14-3: PZB 90 im Simulator
- P15-1: Plan-Freigabe im Vier-Augen-Prinzip
- P15-2: Sperrlisten für mTLS
- P15-3: Betriebsführung bei der Übernahme mitnehmen
- P15-4: RaSTA-Redundanzschicht
- P15-5: Zyklische Speicherprüfung
- P15-6: ETCS über Prozessgrenzen
- P15-7: Zeitstempel für Siegel und unabhängige Zeugen
- P15-8: FMEA und Sicherheitsnachweis für Version 1.0

### Behoben

- fix(kernel): keine Gegenfahrt in überdeckte Annäherungsabschnitte
- fix: Shellcheck SC2015 im Installer-Test
- fix: Installer-Test verfälscht die Prüfsumme in jedem Fall
- fix(deps): rustls 0.23.46 wegen RUSTSEC-2026-0349

### Dokumentation

- docs: Phase 2 – Anforderungen, Gefährdungsliste, API, Konzept v0.4, README
- docs: Phase 3 abgeschlossen – Anforderungen, Gefährdungen, API, Konzept v0.5
- docs: Phase 4 abgeschlossen – ETCS, EULYNX/RaSTA, railML; Konzept v0.6
- docs, ci: Phase 5 abgeschlossen – Sicherheitsnachweis, Rückverfolgbarkeit, Kani und Fuzzing in der CI; Konzept v0.7
- docs: Phase 7 abgeschlossen, Konzept v0.9, Phase 8 (Verfügbarkeit und Diagnose) angelegt
- docs: Phase 8 abgeschlossen, Konzept v0.10

### Weiteres

- fuzz: Journal mitlesen und abspielen gegen beliebige und neu verkettete Eingaben
- Kern: Annäherungsabschnitte beim Anlauf vor der ersten Anforderung verschließen
- build: Bedienplatz in der Pipeline bauen statt das Bundle einzuchecken
- Releases, Installer, Updater und Uninstaller
- R-3: VitePress-Dokumentation auf GitHub Pages
