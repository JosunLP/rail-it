# Anforderungen an die Betriebsführung (`REQ-OPS`)

Umgesetzt in `crates/railit-ops` und eingebunden über `railit-runtime` (`Runtime::enable_ops`).
Nachweise in `crates/railit-ops/tests/ops.rs`, in den Szenarien 14 und 15 und in
`apps/railitd/tests/api.rs`.

Die Betriebsführung ist **nicht sicherheitsrelevant**: Sie stellt dieselben Anforderungen wie
ein Bediener (Fahrstraße einstellen, Erlaubnis anfordern); jede Anforderung prüft der Kern
(REQ-SVC-002). Eine Fehlentscheidung kann den Betrieb verzögern, aber keinen Zug gefährden.

| ID | Anforderung | Gefährdung | Nachweis |
|----|-------------|------------|----------|
| REQ-OPS-001 | Zugnummernmeldung: Eine Zugnummer steht an einem Hauptsignal. Fährt ein Zug in eine Fahrstraße ein, wandert die Nummer an deren Zielsignal; an einer Bereichsgrenze an das Einfahrsignal des Nachbarn, an einer Netzgrenze verlässt sie das Netz. Ankunft und Abfahrt an Halten werden mit Verspätung gemeldet. | H-013 | `train_numbers_follow_the_routes`, Szenario 14 |
| REQ-OPS-002 | Ein Fahrplan wird vor dem Betrieb gegen das Netz geprüft: Zugnummern eindeutig, Betriebsstellen, Bahnsteiggleise und Einsetzsignale vorhanden, Zeiten aufsteigend. | H-013 | `timetable_is_validated_against_the_network`, `times_are_parsed_and_formatted` |
| REQ-OPS-003 | Zuglenkung: Für einen Zug mit Fahrplan wird die erste Fahrstraße des kürzesten Weges zum nächsten Halt angefordert, auch über Bereichsgrenzen; für die Grenzen auf diesem Weg werden die Erlaubnisse angefordert. An einem Halt fährt der Zug frühestens zur Abfahrtszeit und nach der Mindesthaltezeit ab. Nicht ausgeführte Anforderungen werden nach einer Wartezeit wiederholt. | H-013 | `ars_departs_on_time_and_requests_permissions`, `ars_waits_for_permission_and_follows_platform_changes`, Szenario 14 |
| REQ-OPS-004 | Selbstblock: Steht eine Zugnummer an einem Selbstblocksignal, wird die Fahrstraße ab diesem Signal angefordert – auch ohne Fahrplan und Zuglenkung. | H-013 | `auto_signals_clear_for_any_described_train` |
| REQ-OPS-005 | Disposition: Ein Zug kann zurückgehalten und freigegeben werden, sein nächster Halt an einer Betriebsstelle an ein anderes Gleis gelegt und eine zusätzliche Wartezeit verfügt werden – auch bevor er eingesetzt ist. | H-013 | `ars_waits_for_permission_and_follows_platform_changes`, Szenario 15, `timetable_and_dispatching_over_api` |
