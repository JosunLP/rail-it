# Anforderungen an das Betriebsjournal (`REQ-JRN`)

Umgesetzt in `crates/railit-journal`, `crates/railit-runtime` und `apps/railitd`. Nachweise in
`crates/railit-runtime/tests/journal.rs`, `apps/railitd/tests/api.rs` und
`apps/railit/tests/cli.rs`.

| ID | Anforderung | Gefährdung | Nachweis |
|----|-------------|------------|----------|
| REQ-JRN-001 | Das Journal hält je Zyklus Zeitpunkt und alle Eingaben des Kerns fest, verkettet über SHA-256. Jede Veränderung, Entfernung oder Einfügung einer Zeile wird bei der Prüfung erkannt. | H-011 | `tampering_breaks_the_chain`, `journal_verify_and_replay` |
| REQ-JRN-002 | Ein Journal lässt sich gegen den aufgezeichneten Plan deterministisch abspielen. Ausgaben (Hash je Zyklus) und Zustand (Hash an Prüfpunkten) müssen übereinstimmen; die erste Abweichung wird gemeldet. Ein fremder Plan wird abgewiesen. | H-011, H-007 | `journal_replays_to_the_same_state`, `forged_journal_is_detected_by_replay`, `replay_needs_the_recorded_plan`, `aux_operation_and_journal` |
| REQ-JRN-003 | Ein Journal wird nie überschrieben. Ein Schreibfehler beendet nur das Journal, nicht den Betrieb, und wird gemeldet. Eine beim Abbruch unvollständige letzte Zeile wird erkannt und ignoriert. | H-011 | `railitd --journal` (öffnet nur neue Dateien), `Runtime::journal_error`, `aux_operation_and_journal` |
