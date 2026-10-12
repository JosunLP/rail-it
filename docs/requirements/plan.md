# Anforderungen an Projektierung und Plan-Image (`REQ-PLN`)

Umgesetzt in `crates/railit-plan` und `crates/railit-planning`. Das Plan-Image hat das
Format v2 (Durchrutschwege, Flankenschutz, Bahnübergänge, Signalarten und -systeme). Nachweise in
`crates/railit-plan/tests/image.rs` und `crates/railit-planning/tests/planning.rs`.

## Plan-Image

| ID | Anforderung | Gefährdung | Nachweis |
|----|-------------|------------|----------|
| REQ-PLN-001 | Der Kern arbeitet nur mit einem Plan, der vollständig geprüft wurde (`PlanView::parse`). Ein teilweise gültiger Plan wird nie verwendet. | H-004 | Typsystem: `PlanView` entsteht nur über `parse`; `railit_runtime::load_plan` |
| REQ-PLN-002 | Inkonsistente Pläne werden abgewiesen: ungültige Verweise, leere Fahrstraßen, doppelte Weichen oder Abschnitte, Weichen außerhalb der Abschnitte ihrer Fahrstraße, unsymmetrische oder reflexive Feindlichkeiten, ungültige Namen. | H-004 | `inconsistent_plans_are_rejected` |
| REQ-PLN-003 | Jede Veränderung des Images wird erkannt (SHA-256 über den gesamten Inhalt). | H-004, H-008 | `every_single_bit_flip_is_detected` (jedes Byte, mehrere Bits) |
| REQ-PLN-004 | Ein Plan kann mit Ed25519 signiert werden. `railitd` startet nur mit einem Plan, dessen Signatur zum hinterlegten Schlüssel passt; unsignierte Pläne nur mit ausdrücklicher Freigabe (`--allow-unsigned`). | H-008 | `signatures_are_checked`, `unsigned_or_foreign_plans_are_refused` (railitd), `keygen_compile_verify_info` (CLI) |

## Projektierung

| ID | Anforderung | Gefährdung | Nachweis |
|----|-------------|------------|----------|
| REQ-PLN-010 | Hauptsignale (auch Mehrabschnittssignale, ETCS-Halt-Tafeln) stehen am Gleisende in Wirkrichtung, und dahinter beginnt ein anderer Gleisfreimeldeabschnitt. | H-001, H-005 | `signals_must_stand_at_tvd_boundaries` |
| REQ-PLN-011 | Fahrstraßen werden vollständig erzeugt: jeder Weg von einem Startsignal bis zum nächsten Hauptsignal gleicher Richtung oder einem Netzende, mit allen Weichenlagen und den Abschnitten in Fahrreihenfolge. | H-001, H-002 | `example_routes_are_generated`, Property-Test `ladder_stations_plan_and_verify` |
| REQ-PLN-012 | Zwei Fahrstraßen sind feindlich, wenn sie (1) dasselbe Startsignal haben, (2) einen Abschnitt oder (3) eine Weiche des Fahrwegs teilen, (4) der Durchrutschweg der einen im Fahrweg der anderen liegt – außer die andere beginnt am Zielsignal der einen (anschließende Fahrstraße), (5) ihre Durchrutschwege einen Abschnitt teilen, (6) sie für eine Weiche (Fahrweg, Durchrutschweg oder Flankenschutz) verschiedene Lagen verlangen oder (7) die eine an einem Schutzsignal der anderen beginnt. | H-001 | `example_routes_are_generated`, `example_phase2_data`, `ladder_stations_plan_and_verify` |
| REQ-PLN-013 | Ein zweites, unabhängig aufgebautes Verfahren prüft jeden Plan gegen das Netz (Fahrstraßen, Feindlichkeiten, Elemente). Nur übereinstimmende Pläne werden geschrieben; das geschriebene Image wird zurückgelesen und verglichen. | H-004 | `verification_detects_tampering`, `compile` |
| REQ-PLN-014 | Wege, die ohne Zielsignal in sich zurückführen, und Fahrstraßen, die einen Abschnitt mehrfach befahren, sind Fehler. | H-004 | `loops_without_signal_are_rejected` |
| REQ-PLN-015 | Jede Fahrstraße mit Zielsignal erhält einen Durchrutschweg: ab dem Zielsignal bis zur ersten Grenze eines Gleisfreimeldeabschnitts hinter der Durchrutschweglänge (Vorgabe 100 m). Spitz befahrene Weichen im Durchrutschweg liegen im geraden Strang (ohne Angabe links). Endet die Fahrstraße an Netzgrenze oder Prellbock, gibt es keinen Durchrutschweg. | H-001, H-009 | `example_phase2_data` |
| REQ-PLN-016 | Für jede Weiche in Fahrweg und Durchrutschweg ist der nicht befahrene Strang geschützt: durch ein Signal, das auf die Weiche zu gerichtet ist (Haupt- oder Rangiersignal), oder durch eine Schutzweiche in abweisender Lage; spitz erreichte Weichen werden auf beiden Strängen bis zur Tiefe 16 weiterverfolgt, ein Prellbock schützt. Lässt sich ein Strang nicht schützen (Netzgrenze), ist das ein Projektierungsfehler. Jede Maßnahme ist dem Abschnitt zugeordnet, den sie schützt. | H-001 | `flanks_must_be_protectable`, `example_phase2_data` |
| REQ-PLN-017 | Jedes Vorsignal kündigt genau ein Hauptsignal an: das nächste in Wirkrichtung, ohne spitz befahrene Weiche dazwischen. Sonst ist das ein Projektierungsfehler. | H-009 | `distant_signals_need_a_unique_main_signal` |
| REQ-PLN-018 | Bahnübergänge im Fahrweg und im Durchrutschweg (innerhalb der Durchrutschweglänge) gehören zur Fahrstraße. Die Geschwindigkeit einer Fahrstraße ist die kleinste Geschwindigkeit der abzweigend befahrenen Weichen (ohne solche Weichen: Streckengeschwindigkeit). | H-010, H-002 | `example_phase2_data`, `level_crossing_within_long_overlap` |
