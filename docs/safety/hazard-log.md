# Gefährdungsliste (Hazard Log)

> Stand: Phase 0 · lebendes Dokument nach EN 50126. Die Liste wird mit jeder Phase
> erweitert. Bewertungen (Häufigkeit, Schwere, Risiko) folgen mit der systematischen
> Gefährdungsanalyse; hier sind zunächst die Gefährdungen und die geplanten bzw.
> umgesetzten Gegenmaßnahmen erfasst.

| ID | Gefährdung | Mögliche Ursache in rail-it | Gegenmaßnahmen | Status |
|----|------------|-----------------------------|----------------|--------|
| H-001 | Zusammenstoß zweier Züge | Feindliche Fahrstraßen gleichzeitig eingestellt; Belegung nicht erkannt; Fahrerlaubnis trotz Besetzung | Feindlichkeitsprüfung über Bitsets (Phase 1), Invarianten nach jedem Zyklus (Phase 1), lückenlose Gleisfreimeldung im Modell (REQ-TOP-006), „gestört“ gilt als „besetzt“ | offen; REQ-TOP-006 umgesetzt |
| H-002 | Entgleisung an einer Weiche | Fahrerlaubnis ohne überwachte Endlage; Weg über eine Weiche, der physisch nicht existiert | Strukturell nur zulässige Wege durch Weichen (REQ-TOP-007); Endlagenüberwachung als Voraussetzung der Fahrerlaubnis (Phase 1) | offen; REQ-TOP-007 umgesetzt |
| H-003 | Umstellen einer Weiche unter einem Zug | Stellbefehl trotz Verschluss oder Belegung | Verschluss festgelegter Fahrstraßen; Stellbefehle nur aus dem Kern (Phase 1) | offen |
| H-004 | Falsches Netzmodell (Projektierungsfehler) | Fehler in Netzbeschreibung oder Import; Modell weicht von der Realität ab | Vollständige Validierung (REQ-TOP-001…010); unabhängige Gegenprüfung der Projektierung (Phase 1); signiertes Plan-Image (Phase 1) | teilweise umgesetzt |
| H-005 | Zug wird nicht erkannt | Lücke in der Gleisfreimeldung; Kommunikationsverlust zu Achszählern | Lückenlose TVD-Abdeckung (REQ-TOP-006); Heartbeat-Überwachung, Ausfall ⇒ „gestört“ (Phase 1) | offen; REQ-TOP-006 umgesetzt |
| H-006 | Unberechtigte oder versehentliche Bedienhandlung | Fehlbedienung, Angriff über API | RBAC, mTLS, Audit (Phase 1/3); Hilfshandlungen nur mit zweiter Bestätigung und Zählwerk (Phase 2) | offen |
| H-007 | Fehlfunktion durch Softwarefehler im Kern | Panik, Überlauf, undefiniertes Verhalten | `forbid(unsafe_code)`; keine `unwrap`/`panic`/Indexierung/ungeprüfte Arithmetik (Clippy `deny`); deterministischer Kern; Redundanz mit Voter (Phase 5) | Lints ab Phase 0 aktiv |
