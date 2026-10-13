# Anforderungen an den Bedienplatz (`REQ-UI`)

Umgesetzt in `crates/railit-layout` (Spurplan-Geometrie), `apps/railitd/src/ui.rs`
(Auslieferung) und `apps/railitd/ui` (TypeScript mit bQuery, gebaut mit Bun). Der Bedienplatz
ist **nicht sicherheitsrelevant** (SIL 0): Er stellt nur Anforderungen, die der Kern prüft
(REQ-SVC-002). Seine Aufgabe für die Sicherheit ist, dass der Bediener eine falsche oder
veraltete Anzeige erkennt und dass eine Fehlanzeige nicht zur Ausführung einer anderen als
der angeforderten Hilfshandlung führen kann – das sichert der Kern (REQ-IXL-022).

Nachweise in `crates/railit-layout/tests/layout.rs`, `apps/railitd/tests/api.rs`,
`apps/railitd/tests/security.rs`, den Unit-Tests `apps/railitd/ui/test/` (`bun test`) und
den Ende-zu-Ende-Tests `apps/railitd/ui/e2e/` (Playwright gegen einen echten `railitd`).

| ID | Anforderung | Gefährdung | Nachweis |
|----|-------------|------------|----------|
| REQ-UI-001 | Der Spurplan wird aus der Topologie berechnet (`/v1/layout`), nicht von Hand gezeichnet: Jedes Gleis, jede Weiche, jedes Signal, jeder Bahnübergang und jede Bereichsgrenze des Netzes erscheint genau einmal, Gleise liegen von West nach Ost, Signale stehen an ihrem Gleis in Wirkrichtung. Der Bedienplatz zeichnet ihn ohne eigenes Wissen über das Netz und stellt jeden Zustand des Zustandsbilds dar (Freimeldung, Fahrstraßen, Durchrutschwege, Weichenlage, Signalbegriffe, Ersatzsignal, Zugnummern, Erlaubnisse). Die Seite ist ohne Anmeldung erreichbar, enthält aber keine Daten und läuft unter einer strengen Content-Security-Policy. | H-016 | `beispielstadt_is_a_station_with_two_tracks`, `nordbahn_has_three_areas_from_west_to_east`, `railml_branch_and_svg`, `operator_desk_is_static_and_locked_down`, E2E „Spurplan wird aus /v1/layout gezeichnet“ |
| REQ-UI-002 | Anzeigeüberwachung: Kommt 2 s lang kein Zustandsbild mit fortgeschrittener Stellwerkszeit, gilt die Anzeige als gestört. Der Spurplan wird abgedeckt und als gestört gekennzeichnet, alle Bedienungen und Bestätigungen sind gesperrt, bis wieder ein aktuelles Zustandsbild vorliegt. Der sichere Zustand des Kerns wird gesondert angezeigt. | H-016 | Unit-Tests `model.test.ts` (Anzeigeüberwachung), E2E „Anzeigeüberwachung“ (Dienst beendet), `operator_desk_is_static_and_locked_down` |
| REQ-UI-003 | Sichere Anzeige- und Bestätigungskette für Hilfshandlungen: Der Dienst liefert zu jeder offenen Hilfshandlung einen Prüfcode (SHA-256 über Stellbereich, Nummer, Art, Element, Lage). Der bestätigende Bedienplatz berechnet ihn aus dem, was er anzeigt; weicht er ab, wird „Anzeige gestört“ gemeldet und die Bestätigung gesperrt. Der zweite Bediener gibt das Element selbst ein; die Bestätigung trägt die so wiederholte Hilfshandlung und den Prüfcode der Anzeige. Der Dienst weist einen abweichenden Prüfcode ab (`check_mismatch`), der Kern eine abweichende Hilfshandlung (REQ-IXL-022). | H-006, H-016 | `check_code_vectors`, `aux_operation_and_journal`, Unit-Tests `check.test.ts` (gleiche Prüfvektoren wie der Dienst), E2E „Verfälschte Anzeige“ und „Bestätigung mit abweichendem Element“ |
| REQ-UI-004 | Netz und Betrieb am Bedienplatz: Stellbereiche, Grenzen mit Erlaubnisrichtung, Zugnummern am Signal; Zuglenkung ein- und ausschalten, Züge zurückhalten und freigeben; Stelltisch-Eingabe über die Tastatur (`Start Ziel`, `FRT Fahrstraße`, `Weiche L/R`). Alle Handlungen laufen über die API mit Anmeldung und Audit und sind bei gestörter Anzeige gesperrt. | H-006, H-013, H-016 | `trainText`, `parseCommand` (Unit-Tests `model.test.ts`), E2E `nordbahn.test.ts` |
