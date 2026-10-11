# Anforderungen an den Sicherheitskern (`REQ-IXL`)

Umgesetzt in `crates/railit-kernel`. Nachweise in `crates/railit-kernel/tests/kernel.rs`,
im Property-Test `crates/railit-kernel/tests/invariants.rs` und in den Szenarien unter
`tests/scenarios/`.

| ID | Anforderung | Gefährdung | Nachweis |
|----|-------------|------------|----------|
| REQ-IXL-001 | Der Kern ist eine deterministische Funktion von Zustand, Zeit und Eingaben: keine I/O, keine Uhr, keine Allokation, keine Panikpfade, kein `unsafe`. | H-007 | `kernel_is_deterministic`; Lints (`deny`) in `lib.rs`; `no_std`-Build ohne Allokator in der CI |
| REQ-IXL-002 | Feindliche Fahrstraßen sind nie gleichzeitig eingestellt. | H-001 | `conflicting_routes_are_rejected`, Invariante `no_conflicting_routes`, Property-Test, Szenario 03 |
| REQ-IXL-003 | Eine Fahrstraße wird nur zugelassen, wenn alle ihre Abschnitte frei gemeldet und unverschlossen und alle ihre Weichen unverschlossen sind. | H-001, H-005 | `occupied_or_disturbed_sections_block_routes`, Szenario 07 |
| REQ-IXL-004 | Eine Weiche wird nur gestellt, wenn sie unverschlossen ist (oder gerade von einer Fahrstraße in Einstellung reserviert wurde) und ihr Abschnitt frei ist. | H-003 | `single_point_operation`, Prüfung in `command_point`, Property-Test |
| REQ-IXL-005 | Ein Signal zeigt nur Fahrt, solange alle Weichen der Fahrstraße überwacht in Lage, alle Abschnitte frei und das Signal intakt ist. Fällt eine Bedingung weg, fällt es sofort auf Halt und zeigt erst nach erneuter Bedienung wieder Fahrt. | H-001, H-002 | `signal_drops_when_point_loses_supervision`, `signal_drops_when_section_ahead_is_occupied`, Invariante `proceed_only_when_safe`, Szenario 04 |
| REQ-IXL-006 | Wird eine Fahrstraße nach der Fahrtstellung zurückgenommen, bleibt sie für den Zeitverschluss festgelegt. Fährt währenddessen ein Zug ein, wird alarmiert und die Fahrstraße bleibt verschlossen. | H-001, H-009 | `cancelling_respects_time_lock`, `train_entering_cancelled_route_keeps_locks`, Szenario 05 |
| REQ-IXL-007 | Fahrstraßen werden zugbewirkt in Fahrtrichtung aufgelöst: ein Abschnitt erst, wenn der Zug ihn befahren und geräumt hat und im folgenden Abschnitt ist. | H-001, H-003 | `route_lifecycle_with_partial_release`, Szenarien 01 und 02 |
| REQ-IXL-008 | Nach jedem Zyklus werden die Invarianten geprüft. Eine Verletzung führt in den sicheren Zustand: alle Signale Halt, keine Stellbefehle, alle Anforderungen abgelehnt. Beim Start erhalten alle Signale Halt. | H-007 | `invariant_violation_enters_safe_state`, `first_cycle_commands_stop_everywhere`, Property-Test |
| REQ-IXL-009 | Fehlende oder veraltete Meldungen gelten als gestört; gestört wird wie besetzt behandelt. | H-005 | `nothing_is_clear_without_reports`, `stale_reports_drop_the_signal` |
| REQ-IXL-010 | Erreichen die Weichen einer Fahrstraße ihre Lage nicht innerhalb der Umstellzeit, wird die Einstellung abgebrochen und alarmiert. | H-002 | `setting_times_out_when_points_do_not_move`, Szenario 06 |
