# Anforderungen an das Netzmodell (`REQ-TOP`)

Umgesetzt in `crates/railit-topology`, nachgewiesen in
`crates/railit-topology/tests/validation.rs`. Jede Prüfung meldet ihre ID über
`TopologyError::requirement()`.

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
