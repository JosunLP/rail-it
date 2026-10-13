# Anforderungen an Dienst und Bedienung (`REQ-SVC`)

Umgesetzt in `crates/railit-runtime`, `apps/railitd` und `apps/railit`.

Anmeldung, Rollen, TLS und Auditprotokoll: siehe [sicherheit.md](sicherheit.md).

| ID | Anforderung | Gefährdung | Nachweis |
|----|-------------|------------|----------|
| REQ-SVC-001 | `railitd` startet nur, wenn Plan und Netz zusammengehören (gleiche Elemente in gleicher Reihenfolge). | H-004 | `check_correspondence`; Startprüfung in `railitd` |
| REQ-SVC-002 | Bedienung und Betriebsführung stellen nur Anforderungen an den Kern. Stellbefehle an Feldelemente erzeugt ausschließlich der Kern. | H-003, H-006 | Architektur: API und Engine besitzen keinen Weg zu `Command` außer über `Interlocking::step` |
| REQ-SVC-003 | Mit Benutzerdatei oder Zugangstoken verlangt die API eine Anmeldung für alle Pfade außer `/v1/health` (siehe REQ-SEC-001). | H-006 | `operate_station_over_api`, `roles_areas_and_audit_with_tokens` |
| REQ-SVC-004 | Jede Anforderung erhält eine eindeutige Antwort des Kerns (angenommen oder abgelehnt mit Grund). | H-006 | `operate_station_over_api`, Szenarien |
| REQ-SVC-005 | Geheime Schlüssel werden nur für den Eigentümer lesbar angelegt und nie überschrieben. | H-008 | `keygen_compile_verify_info` |
| REQ-SVC-006 | Hilfshandlungen über die API verlangen eine Bedienernummer: mit Benutzerverwaltung die des angemeldeten Benutzers, sonst den Kopf `X-Railit-Operator`. Die Antwort enthält das Ereignis der Hilfshandlung (angefordert, ausgeführt, verfallen). | H-006 | `aux_operation_and_journal`, `mutual_tls_identifies_users_by_certificate` |
| REQ-SVC-007 | Ein Netz mit Stellbereichen läuft mit einem Kern je Bereich; jeder Bereich hat seinen signierten Plan, der zu seinem Teilnetz passen muss, und sein eigenes Journal. Grenzmeldungen werden im folgenden Zyklus zugestellt; Namen werden über alle Bereiche aufgelöst, Grenzen als `Bereich>Nachbar` adressiert. | H-004, H-012 | `network_with_areas_over_api`, `every_area_journal_replays`, Szenarien 13–15 |
| REQ-SVC-008 | Die gRPC-Schnittstelle `railit.v1` liegt auf dem Port der REST-API, hinter derselben Anmeldung, und nutzt dieselbe Dienstschicht: gleiche Rechte, gleiches Auditprotokoll, gleiche Antworten. Ablehnungen des Kerns sind Antworten, keine Fehler. | H-006 | `interlocking_over_grpc`, gRPC über mTLS in `mutual_tls_identifies_users_by_certificate` |
| REQ-SVC-009 | Der Dienst liefert Metriken für den Betrieb im Prometheus-Textformat (`GET /v1/metrics`, Anmeldung wie `/v1/state`): Zyklen und Rechenzeit je Zyklus (Histogramm, Überschreitungen der Zykluszeit), Anforderungen nach Ergebnis, Meldungen, Hilfshandlungen, sicherer Zustand, Kanäle, Fahrstraßen, Journal und Anbindung der Knoten. Metriken haben keine Rückwirkung auf den Betrieb. | H-015, H-016 | `operate_station_over_api`, `safe_computer_nodes_over_api`, `histogram_is_cumulative_and_consistent` |
| REQ-SVC-010 | Rückschau: Mit `--journal` beantwortet der Dienst Fragen an seine eigenen Journale (`GET /v1/journal`, `/v1/journal/state?at_ms=`, `/v1/journal/events?from_ms=&to_ms=`, je Stellbereich mit `area`). Zustand und Ereignisse werden ab dem letzten Zustandsabzug rekonstruiert und gegen das Journal geprüft (REQ-JRN-008); die Rückschau ist reine Anzeige ohne Verbindung zum laufenden Betrieb. | H-011, H-016 | `aux_operation_and_journal` |
