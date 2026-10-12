# Anforderungen an das Netzmodell (`REQ-TOP`)

Umgesetzt in `crates/railit-topology`, nachgewiesen in
`crates/railit-topology/tests/validation.rs`. Jede Prüfung meldet ihre ID über
`TopologyError::requirement()`. Stellbereiche, Betriebsstellen und Teilnetze sind in
`crates/railit-import/tests/areas.rs` nachgewiesen, der Import von railML 3 in
`crates/railit-import/tests/railml.rs`.

| ID | Anforderung | Gefährdung | Nachweis |
|----|-------------|------------|----------|
| REQ-TOP-001 | Jeder fachliche Name ist über alle Elementarten hinweg eindeutig. | H-004 | `duplicate_names_are_rejected_across_kinds` |
| REQ-TOP-002 | Jeder Port ist genau einmal verbunden. Netzenden werden ausdrücklich als Prellbock oder Netzgrenze modelliert; ein offener Port ist ein Fehler. | H-004 | `open_and_double_connected_ports_are_rejected` |
| REQ-TOP-003 | Verbindungen bestehen nur zwischen existierenden Ports der jeweiligen Elementart. Ein Port kann nicht mit sich selbst verbunden werden. IDs aus fremden Netzen werden abgewiesen. | H-004 | `invalid_ports_and_self_connections_are_rejected` |
| REQ-TOP-004 | Gleisabschnitte haben eine Länge größer null. Die Anzahl Elemente je Art ist auf `u32::MAX` begrenzt. | H-004 | `zero_length_tracks_are_rejected` |
| REQ-TOP-005 | Punktobjekte (Signale) liegen innerhalb ihres Gleisabschnitts. | H-004 | `signals_outside_their_track_are_rejected` |
| REQ-TOP-006 | Jedes Gleis und jede Weiche gehört zu genau einem Gleisfreimeldeabschnitt; kein Abschnitt ist leer. | H-001, H-005 | `tvd_coverage_must_be_complete_and_unique` |
| REQ-TOP-007 | Eine Weiche ist nur zwischen Spitze und linkem Strang (Lage links) oder Spitze und rechtem Strang (Lage rechts) befahrbar, nie zwischen den beiden Strängen. Jeder Weg durch eine Weiche nennt die erforderliche Lage. | H-002 | `points_can_only_be_traversed_tip_to_branch`, Property-Test `exits_stay_on_node` |
| REQ-TOP-008 | Ein Netz ist nur nach vollständiger, erfolgreicher Validierung verfügbar und danach unveränderlich. | H-004 | Typsystem: `Topology` hat keinen öffentlichen Konstruktor und keine verändernden Methoden. |
| REQ-TOP-009 | Die Validierung meldet alle gefundenen Fehler, nicht nur den ersten. | – (Qualität) | `all_errors_are_collected_with_context` (railit-import) |
| REQ-TOP-010 | Ein Netz aus mehreren nicht verbundenen Teilen erzeugt eine Warnung. | H-004 | `open_and_double_connected_ports_are_rejected` |
| REQ-TOP-011 | Sind Stellbereiche festgelegt, gehört jeder Gleisfreimeldeabschnitt genau einem Stellbereich an; kein Stellbereich ist leer. | H-004, H-012 | `nordbahn_has_three_areas_and_two_links`, `every_tvd_needs_exactly_one_area` |
| REQ-TOP-012 | Die Haltsignale eines Bahnsteigs sind Signale, an denen Fahrstraßen enden (Haupt- oder Mehrabschnittssignale). | H-004 | `platforms_need_route_signals` |
| REQ-TOP-013 | Jeder Stellbereich lässt sich als eigenes, vollständig validiertes Teilnetz ausschneiden. Verbindungen zu Nachbarbereichen werden zu Grenzen (`>Nachbar`); welcher Bereich beim Anlauf die Erlaubnis übernimmt, ist eindeutig festgelegt. Die Abbildung der IDs auf das Gesamtnetz bleibt erhalten. | H-004, H-012 | `area_cut_replaces_neighbours_by_borders`, `border_terminals_can_be_written_by_hand` |
| REQ-TOP-014 | Ein Netz aus railML 3 durchläuft dieselbe Validierung wie ein Netz aus rail-it-TOML. Fehlende oder widersprüchliche Angaben (Länge, Wirkrichtung, Verbindungen) sind Fehler mit Angabe des railML-Elements und werden nie ergänzt oder geraten. Elemente, die rail-it nicht übernimmt, werden als Warnung gemeldet. Export und erneuter Import ergeben dasselbe Netz und bitgleiche Plan-Images. | H-004 | `handmade_railml_is_imported`, `errors_are_collected_with_location`, `beispielstadt_survives_the_roundtrip`, `nordbahn_with_areas_and_stations_survives_the_roundtrip`, `railml_roundtrip_yields_identical_plans`, `railml_export_validate_compile` |
