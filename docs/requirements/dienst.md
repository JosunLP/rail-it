# Anforderungen an Dienst und Bedienung (`REQ-SVC`)

Umgesetzt in `crates/railit-runtime`, `apps/railitd` und `apps/railit`.

> Die Bedienerkennung identifiziert Bediener für die Zwei-Personen-Regel, authentifiziert sie
> aber noch nicht. Eigene Zugänge je Bediener und Rollen folgen in Phase 3.

| ID | Anforderung | Gefährdung | Nachweis |
|----|-------------|------------|----------|
| REQ-SVC-001 | `railitd` startet nur, wenn Plan und Netz zusammengehören (gleiche Elemente in gleicher Reihenfolge). | H-004 | `check_correspondence`; Startprüfung in `railitd` |
| REQ-SVC-002 | Bedienung und Betriebsführung stellen nur Anforderungen an den Kern. Stellbefehle an Feldelemente erzeugt ausschließlich der Kern. | H-003, H-006 | Architektur: API und Engine besitzen keinen Weg zu `Command` außer über `Interlocking::step` |
| REQ-SVC-003 | Ist ein Zugangstoken gesetzt, verlangt die API es für alle Pfade außer `/v1/health`. | H-006 | `operate_station_over_api` |
| REQ-SVC-004 | Jede Anforderung erhält eine eindeutige Antwort des Kerns (angenommen oder abgelehnt mit Grund). | H-006 | `operate_station_over_api`, Szenarien |
| REQ-SVC-005 | Geheime Schlüssel werden nur für den Eigentümer lesbar angelegt und nie überschrieben. | H-008 | `keygen_compile_verify_info` |
| REQ-SVC-006 | Hilfshandlungen über die API verlangen eine Bedienerkennung (`X-Railit-Operator`); die Antwort enthält das Ereignis der Hilfshandlung (angefordert, ausgeführt, verfallen). | H-006 | `aux_operation_and_journal` |
