# Anforderungen an die RBC-Funktion für ETCS Level 2 (`REQ-ETCS`)

Umgesetzt in `crates/railit-etcs` (RBC) auf Grundlage von `railit_movement::etcs`
(REQ-MOV-004). Nachweise in den Szenarien 16 und 17 (`tests/scenarios/`) und in
`apps/railitd/tests/api.rs`.

| ID | Anforderung | Gefährdung | Nachweis |
|----|-------------|------------|----------|
| REQ-ETCS-001 | Ein Zug mit ETCS-Fahrzeuggerät meldet sich mit seiner ersten Positionsmeldung beim RBC an (Start of Mission, Betriebsart *Stand By*) und wird abgemeldet, sobald keine Meldung mehr kommt (End of Mission). Bezugspunkt ist das zuletzt passierte bzw. das nächste Signal oder die ETCS-Halt-Tafel. Das RBC zeigt angemeldete Züge mit Betriebsart und Fahrerlaubnis an (`GET /v1/etcs`, gRPC `Etcs`, `railit etcs`). | H-009 | `etcs_train_over_api`, Szenario 16 |
| REQ-ETCS-002 | Das RBC entscheidet nichts selbst: Jede Fahrerlaubnis folgt nach REQ-MOV-004 aus den Fahrstraßen, denen der Kern Fahrt gegeben hat (Betriebsart *Full Supervision*). An einer Bereichsgrenze reicht sie über den Annäherungsabschnitt des Nachbarn nur bis zu dessen Einfahrsignal und setzt sich nur entlang dessen Fahrstraßen mit Fahrt fort. Löst sich eine Fahrstraße hinter dem Zug regulär auf, rückt der Bezugspunkt auf ihr Zielsignal. | H-001, H-009, H-012 | `etcs_train_over_api`, Szenario 16 (drei Stellbereiche, Mischbetrieb mit konventionellem Zug) |
| REQ-ETCS-003 | Nimmt der Kern Fahrt zurück, verkürzt das RBC die Fahrerlaubnis im selben Zyklus. Wird die Fahrstraße, in der ein Zug fährt, per Hilfshandlung aufgelöst oder fällt ihr Signal wegen einer Störung auf Halt, hat der fahrende Zug keine Fahrerlaubnis mehr: Das RBC löst eine Notbremsung aus (Betriebsart *Trip*, Ereignis `emergency_stop`). | H-001, H-009 | Szenario 17 |
