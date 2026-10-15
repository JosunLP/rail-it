# Rückverfolgbarkeit Anforderung → Code → Nachweis

> Erzeugt mit `cargo xtask trace`; die CI prüft mit `cargo xtask trace --check`, dass
> dieser Bericht aktuell ist. Nicht von Hand bearbeiten.

Je Anforderung: Gefährdungen, Code, der die ID nennt, und Nachweise (Tests und
Kani-Nachweise, die die ID nennen, im Nachweis genannte Funktionen, Szenarien).

## [bedienplatz.md](../requirements/bedienplatz.md)

| ID | Gefährdung | Code | Nachweis |
|----|------------|------|----------|
| REQ-UI-001 | H-016 | `apps/railitd/ui/src/plan.ts`<br>`crates/railit-layout/src/lib.rs` | `beispielstadt_is_a_station_with_two_tracks`<br>`nordbahn_has_three_areas_from_west_to_east`<br>`railml_branch_and_svg`<br>`operator_desk_is_static_and_locked_down`<br>`apps/railitd/tests/api.rs`<br>`apps/railitd/tests/security.rs`<br>`apps/railitd/ui/e2e/bedienplatz.test.ts`<br>`apps/railitd/ui/e2e/nordbahn.test.ts`<br>`crates/railit-layout/tests/layout.rs` |
| REQ-UI-002 | H-016 | `apps/railitd/ui/src/main.ts`<br>`apps/railitd/ui/src/model.ts` | `operator_desk_is_static_and_locked_down`<br>`apps/railitd/tests/security.rs`<br>`apps/railitd/ui/e2e/bedienplatz.test.ts`<br>`apps/railitd/ui/test/model.test.ts` |
| REQ-UI-003 | H-006, H-016 | `apps/railitd/src/service.rs`<br>`apps/railitd/ui/src/check.ts`<br>`apps/railitd/ui/src/main.ts`<br>`crates/railit-runtime/src/names.rs` | `check_code_vectors`<br>`aux_operation_and_journal`<br>`apps/railitd/tests/api.rs`<br>`apps/railitd/ui/e2e/bedienplatz.test.ts`<br>`apps/railitd/ui/test/check.test.ts` |
| REQ-UI-004 | H-006, H-013, H-016 | – | `apps/railitd/ui/e2e/nordbahn.test.ts`<br>`apps/railitd/ui/test/model.test.ts` |
| REQ-UI-005 | H-016, H-011 | `apps/railitd/ui/src/api.ts`<br>`apps/railitd/ui/src/main.ts`<br>`apps/railitd/ui/src/model.ts`<br>`apps/railitd/ui/src/panels.ts` | `apps/railitd/ui/e2e/bedienplatz.test.ts`<br>`apps/railitd/ui/test/review.test.ts` |

## [betrieb.md](../requirements/betrieb.md)

| ID | Gefährdung | Code | Nachweis |
|----|------------|------|----------|
| REQ-OPS-001 | H-013 | `crates/railit-ops/src/ops.rs` | `train_numbers_follow_the_routes`<br>`apps/railitd/tests/api.rs`<br>`crates/railit-ops/tests/ops.rs`<br>Szenario 14 |
| REQ-OPS-002 | H-013 | – | `timetable_is_validated_against_the_network`<br>`times_are_parsed_and_formatted`<br>`crates/railit-ops/tests/ops.rs` |
| REQ-OPS-003 | H-013 | `crates/railit-ops/src/ops.rs` | `ars_departs_on_time_and_requests_permissions`<br>`ars_waits_for_permission_and_follows_platform_changes`<br>`crates/railit-ops/tests/ops.rs`<br>Szenario 14 |
| REQ-OPS-004 | H-013 | `crates/railit-ops/src/ops.rs` | `auto_signals_clear_for_any_described_train`<br>`crates/railit-ops/tests/ops.rs` |
| REQ-OPS-005 | H-013 | `crates/railit-ops/src/ops.rs` | `ars_waits_for_permission_and_follows_platform_changes`<br>`timetable_and_dispatching_over_api`<br>Szenario 15 |
| REQ-OPS-006 | H-013 | `crates/railit-adapters/src/ils.rs`<br>`crates/railit-ops/src/net.rs`<br>`crates/railit-ops/src/ops.rs`<br>`crates/railit-runtime/src/runtime.rs` | `trains_are_handed_over_eastwards`<br>`trains_are_handed_over_westwards`<br>`handover_telegram_roundtrip`<br>`handover_waits_for_the_link`<br>`only_local_trains_are_spawned_and_described`<br>`takeover_does_not_spawn_running_trains_again`<br>`sci_ils`<br>`crates/railit-ops/tests/ops.rs`<br>`crates/railit-runtime/tests/ils.rs` |
| REQ-OPS-007 | H-013, H-017 | `apps/railitd/src/api.rs`<br>`apps/railitd/src/standby.rs`<br>`apps/railitd/src/takeover.rs`<br>`crates/railit-ops/src/ops.rs`<br>`crates/railit-runtime/src/names.rs` | `ops_state_moves_to_another_computer`<br>`standby_takes_over_with_the_followed_state`<br>`apps/railitd/tests/field.rs`<br>`crates/railit-ops/tests/ops.rs` |

## [dienst.md](../requirements/dienst.md)

| ID | Gefährdung | Code | Nachweis |
|----|------------|------|----------|
| REQ-SVC-001 | H-004 | – | `check_correspondence` |
| REQ-SVC-002 | H-003, H-006 | – | Analyse (Architektur) |
| REQ-SVC-003 | H-006 | – | `operate_station_over_api`<br>`roles_areas_and_audit_with_tokens` |
| REQ-SVC-004 | H-006 | – | `operate_station_over_api` |
| REQ-SVC-005 | H-008 | – | `keygen_compile_verify_info` |
| REQ-SVC-006 | H-006 | – | `aux_operation_and_journal`<br>`mutual_tls_identifies_users_by_certificate` |
| REQ-SVC-007 | H-004, H-012 | – | `network_with_areas_over_api`<br>`every_area_journal_replays` |
| REQ-SVC-008 | H-006 | – | `interlocking_over_grpc`<br>`mutual_tls_identifies_users_by_certificate`<br>`apps/railitd/tests/grpc.rs` |
| REQ-SVC-009 | H-015, H-016 | `apps/railitd/src/api.rs`<br>`apps/railitd/src/metrics.rs` | `operate_station_over_api`<br>`safe_computer_nodes_over_api`<br>`histogram_is_cumulative_and_consistent`<br>`apps/railitd/tests/api.rs` |
| REQ-SVC-010 | H-011, H-016 | `apps/railitd/src/api.rs`<br>`apps/railitd/src/review.rs` | `aux_operation_and_journal`<br>`apps/railitd/tests/api.rs` |
| REQ-SVC-011 | H-011, H-015, H-016 | `apps/railitd/src/api.rs`<br>`apps/railitd/src/standby.rs`<br>`apps/railitd/ui/src/model.ts`<br>`apps/railitd/ui/src/panels.ts`<br>`crates/railit-runtime/src/replication.rs` | `standby_follows_the_primary`<br>`area_names_are_encoded`<br>`apps/railitd/tests/api.rs`<br>`apps/railitd/ui/e2e/bedienplatz.test.ts`<br>`apps/railitd/ui/test/role.test.ts` |
| REQ-SVC-012 | H-001, H-003, H-010, H-015, H-017 | `apps/railitd/src/auth.rs`<br>`apps/railitd/src/engine.rs`<br>`apps/railitd/src/metrics.rs`<br>`apps/railitd/src/service.rs`<br>`apps/railitd/src/standby.rs`<br>`apps/railitd/src/takeover.rs`<br>`crates/railit-runtime/src/replication.rs`<br>`crates/railit-runtime/src/runtime.rs` | `outputs_wait_for_all_areas`<br>`lapse_releases_everything_and_rearms_after_catching_up`<br>`area_without_journal_lapses`<br>`takeover_in_beispielstadt_is_safe`<br>`takeover_in_nordbahn_is_safe`<br>`stale_takeover_without_gate_opens_the_crossing_before_a_train`<br>`gate_covers_only_a_current_standby`<br>`apps/railitd/tests/field.rs`<br>`crates/railit-runtime/tests/takeover.rs` |
| REQ-SVC-013 | H-001, H-003, H-015, H-017 | `apps/railit-field/src/main.rs`<br>`apps/railitd/src/takeover.rs`<br>`crates/railit-adapters/src/net.rs`<br>`crates/railit-fuzz/src/lib.rs` | `second_interlocking_is_rejected_while_the_leader_lives`<br>`standby_leads_after_the_leader_is_lost`<br>`switch_per_area`<br>`apps/railitd/tests/field.rs`<br>`crates/railit-adapters/tests/net.rs` |
| REQ-SVC-014 | H-001, H-003, H-015, H-016, H-017 | `apps/railitd/src/takeover.rs`<br>`apps/railitd/ui/src/model.ts` | `standby_takes_over_with_the_followed_state`<br>`standby_restarts_without_a_promise`<br>`apps/railitd/tests/field.rs`<br>`apps/railitd/ui/test/role.test.ts` |

## [embedded.md](../requirements/embedded.md)

| ID | Gefährdung | Code | Nachweis |
|----|------------|------|----------|
| REQ-EMB-001 | H-007, H-015 | – | `node_matches_reference_kernel`<br>`memory_capacity_is_checked`<br>`crates/railit-embedded/tests/node.rs` |
| REQ-EMB-002 | H-005, H-014 | `crates/railit-runtime/src/remote.rs` | `link_loss_is_fail_safe`<br>`crates/railit-embedded/tests/node.rs` |
| REQ-EMB-003 | H-014 | – | `safety_layer_over_the_wire`<br>`malformed_payload_drops_the_link`<br>`roundtrip_across_payloads`<br>`garbage_is_malformed`<br>`packer_respects_the_payload_limit`<br>`node_wire`<br>`crates/railit-embedded/tests/node.rs` |
| REQ-EMB-004 | H-007, H-015 | – | `watchdog_trips_on_overrun`<br>`crates/railit-embedded/tests/node.rs` |
| REQ-EMB-005 | H-007, H-015, H-016 | `crates/railit-runtime/src/remote.rs` | `all_scenarios_pass_on_safe_computer_nodes`<br>`node_process_over_udp`<br>`safe_computer_nodes_over_api`<br>`apps/railit-node/tests/udp.rs`<br>`apps/railitd/tests/api.rs`<br>`crates/railit-runtime/tests/scenarios.rs` |
| REQ-EMB-006 | H-015, H-016 | `crates/railit-embedded/src/node.rs`<br>`crates/railit-kernel/src/kernel.rs`<br>`crates/railit-runtime/src/remote.rs` | `state_sync_rebuilds_the_mirror`<br>`state_transfer_reproduces_the_state`<br>`mirror_state`<br>`crates/railit-embedded/tests/node.rs`<br>Szenario 18 |

## [etcs.md](../requirements/etcs.md)

| ID | Gefährdung | Code | Nachweis |
|----|------------|------|----------|
| REQ-ETCS-001 | H-009 | – | `etcs_train_over_api`<br>`apps/railitd/tests/api.rs`<br>Szenario 16 |
| REQ-ETCS-002 | H-001, H-009, H-012 | `crates/railit-etcs/src/lib.rs` | `etcs_train_over_api`<br>`apps/railitd/tests/api.rs`<br>Szenario 16 |
| REQ-ETCS-003 | H-001, H-009 | `crates/railit-etcs/src/lib.rs` | Szenario 17 |

## [fahrerlaubnis.md](../requirements/fahrerlaubnis.md)

| ID | Gefährdung | Code | Nachweis |
|----|------------|------|----------|
| REQ-MOV-001 | H-002, H-009 | – | `ks_diverging_entry_shows_zs3_and_is_announced`<br>`hv_diverging_is_hp2_and_vr2`<br>`hv_combined_and_distant` |
| REQ-MOV-002 | H-009 | – | `pictures_are_consistent`<br>`signal_drop_turns_announcement_back`<br>`crates/railit-movement/tests/aspects.rs` |
| REQ-MOV-003 | H-009 | – | `display_of_dark_and_substitute`<br>Analyse (Architektur) |
| REQ-MOV-004 | H-001, H-002, H-009 | `crates/railit-movement/src/etcs.rs` | `etcs_authority_follows_the_routes_with_proceed`<br>`etcs_authority_carries_speed_and_survives_passing`<br>`crates/railit-movement/tests/aspects.rs` |
| REQ-MOV-007 | H-009 | `crates/railit-sim/src/lib.rs` | `pzb_2000_hz_brakes_at_a_stop_signal`<br>`pzb_500_hz_stops_before_the_announced_signal`<br>`pzb_90_supervision`<br>`crates/railit-sim/tests/sim.rs`<br>Szenario 26 |

## [journal.md](../requirements/journal.md)

| ID | Gefährdung | Code | Nachweis |
|----|------------|------|----------|
| REQ-JRN-001 | H-011 | `crates/railit-journal/src/lib.rs`<br>`crates/railit-runtime/src/runtime.rs` | `tampering_breaks_the_chain`<br>`journal_verify_and_replay`<br>`apps/railit/tests/cli.rs`<br>`apps/railitd/tests/api.rs`<br>`crates/railit-runtime/tests/journal.rs` |
| REQ-JRN-002 | H-011, H-007 | `crates/railit-journal/src/lib.rs`<br>`crates/railit-runtime/src/scenario.rs` | `journal_replays_to_the_same_state`<br>`forged_journal_is_detected_by_replay`<br>`replay_needs_the_recorded_plan`<br>`aux_operation_and_journal`<br>`apps/railit/tests/cli.rs`<br>`apps/railitd/tests/api.rs`<br>`crates/railit-runtime/tests/journal.rs`<br>`crates/railit-runtime/tests/scenarios.rs` |
| REQ-JRN-003 | H-011 | `crates/railit-runtime/src/runtime.rs` | `aux_operation_and_journal` |
| REQ-JRN-004 | H-011 | `crates/railit-journal/src/audit.rs`<br>`crates/railit-journal/src/lib.rs`<br>`crates/railit-journal/src/seal.rs`<br>`crates/railit-runtime/src/runtime.rs` | `sealed_journal_verifies_with_the_station_key`<br>`rewritten_journal_cannot_carry_the_station_seal`<br>`tampered_signature_is_detected`<br>`sealed_audit_log`<br>`foreign_seals_block_reopening`<br>`sealed_journal_and_audit_log`<br>`apps/railitd/tests/api.rs`<br>`crates/railit-journal/tests/seal.rs` |
| REQ-JRN-005 | H-011 | `crates/railit-journal/src/lib.rs` | `deltas_reconstruct_every_input_list`<br>`malformed_deltas_are_rejected`<br>`forged_journal_is_detected_by_replay` |
| REQ-JRN-006 | H-011 | `apps/railit/src/bench.rs`<br>`crates/railit-journal/src/lib.rs`<br>`crates/railit-runtime/src/scenario.rs` | `snapshots_split_the_replay`<br>`forged_snapshot_is_detected_by_replay`<br>`journal_covers_trip_and_reset_of_the_safety_layer`<br>`crates/railit-runtime/tests/scenarios.rs` |
| REQ-JRN-007 | H-011 | `crates/railit-journal/src/lib.rs`<br>`crates/railit-runtime/src/remote.rs`<br>`crates/railit-runtime/src/scenario.rs` | `node_journal_replays_across_resyncs`<br>`safe_computer_nodes_over_api`<br>`all_scenarios_pass_on_safe_computer_nodes`<br>`apps/railitd/tests/api.rs`<br>`crates/railit-runtime/tests/journal.rs`<br>`crates/railit-runtime/tests/scenarios.rs` |
| REQ-JRN-008 | H-011 | `apps/railitd/src/review.rs`<br>`crates/railit-journal/src/lib.rs` | `window_replays_from_the_nearest_snapshot`<br>`journal_follow_state_and_events`<br>`apps/railit/tests/cli.rs`<br>`crates/railit-runtime/tests/journal.rs` |
| REQ-JRN-009 | H-011, H-015 | `crates/railit-journal/src/lib.rs` | `follower_tracks_the_running_interlocking`<br>`follower_reads_node_journal`<br>`journal_follow_state_and_events`<br>`mutated_journals`<br>`rechained_snapshot_mutations`<br>`apps/railit/tests/cli.rs`<br>`crates/railit-fuzz/tests/smoke.rs`<br>`crates/railit-runtime/tests/journal.rs` |

## [kern.md](../requirements/kern.md)

| ID | Gefährdung | Code | Nachweis |
|----|------------|------|----------|
| REQ-IXL-001 | H-007 | `crates/railit-kernel/src/kernel.rs` | `kernel_is_deterministic`<br>`crates/railit-kernel/tests/kernel.rs` |
| REQ-IXL-002 | H-001 | `crates/railit-kernel/src/kernel.rs`<br>`crates/railit-kernel/src/lib.rs` | `conflicting_routes_are_rejected`<br>`no_conflicting_routes`<br>`crates/railit-kernel/tests/kernel.rs`<br>Szenario 03 |
| REQ-IXL-003 | H-001, H-005 | `crates/railit-kernel/src/kernel.rs`<br>`crates/railit-kernel/src/lib.rs` | `occupied_or_disturbed_sections_block_routes`<br>`continuation_may_use_the_overlap`<br>`flank_signals_are_held_at_stop`<br>`crates/railit-kernel/tests/kernel.rs`<br>Szenario 07<br>Szenario 08 |
| REQ-IXL-004 | H-003 | `crates/railit-kernel/src/kernel.rs` | `single_point_operation`<br>`aux_force_point`<br>`command_point`<br>`crates/railit-kernel/tests/kernel.rs` |
| REQ-IXL-005 | H-001, H-002, H-010 | `crates/railit-kernel/src/kernel.rs`<br>`crates/railit-kernel/src/lib.rs` | `signal_drops_when_point_loses_supervision`<br>`signal_drops_when_section_ahead_is_occupied`<br>`occupied_overlap_blocks_proceed`<br>`proceed_only_when_safe`<br>`crates/railit-kernel/tests/kernel.rs`<br>Szenario 04 |
| REQ-IXL-006 | H-001, H-009 | `crates/railit-kernel/src/kernel.rs` | `cancelling_respects_time_lock`<br>`train_entering_cancelled_route_keeps_locks`<br>`crates/railit-kernel/tests/kernel.rs`<br>`crates/railit-kernel/tests/phase2.rs`<br>Szenario 05 |
| REQ-IXL-007 | H-001, H-003 | `crates/railit-kernel/src/kernel.rs` | `route_lifecycle_with_partial_release`<br>`overlap_release_waits_for_time_and_vacancy`<br>`flank_protection_is_released_with_its_section`<br>Szenario 01<br>Szenario 02 |
| REQ-IXL-008 | H-007 | `crates/railit-kernel/src/invariants.rs`<br>`crates/railit-kernel/src/kernel.rs`<br>`crates/railit-kernel/src/lib.rs` | `invariant_violation_enters_safe_state`<br>`first_cycle_commands_stop_everywhere`<br>`crates/railit-kernel/tests/invariants.rs`<br>`crates/railit-kernel/tests/kernel.rs` |
| REQ-IXL-009 | H-005 | `crates/railit-adapters/src/lib.rs`<br>`crates/railit-kernel/src/lib.rs` | `nothing_is_clear_without_reports`<br>`stale_reports_drop_the_signal`<br>`crates/railit-kernel/tests/kernel.rs` |
| REQ-IXL-010 | H-002 | `crates/railit-kernel/src/kernel.rs` | `setting_times_out_when_points_do_not_move`<br>Szenario 06 |
| REQ-IXL-011 | H-009 | `crates/railit-kernel/src/kernel.rs`<br>`crates/railit-kernel/src/lib.rs` | `unauthorized_occupancy_is_alarmed`<br>`signal_passed_at_danger`<br>`crates/railit-kernel/tests/borders.rs`<br>`crates/railit-kernel/tests/phase2.rs`<br>Szenario 11 |
| REQ-IXL-012 | H-001, H-009 | `crates/railit-kernel/src/kernel.rs`<br>`crates/railit-kernel/src/lib.rs` | `overlap_is_locked_and_points_are_set`<br>`continuation_may_use_the_overlap`<br>`crates/railit-kernel/tests/phase2.rs`<br>Szenario 08 |
| REQ-IXL-013 | H-001 | `crates/railit-kernel/src/kernel.rs`<br>`crates/railit-kernel/src/lib.rs` | `flank_signals_are_held_at_stop`<br>`crates/railit-kernel/tests/phase2.rs`<br>Szenario 08 |
| REQ-IXL-014 | H-006 | `crates/railit-kernel/src/kernel.rs`<br>`crates/railit-kernel/src/lib.rs` | `aux_needs_a_second_operator_in_time`<br>`aux_release_of_occupied_route`<br>`aux_reset_tvd`<br>`aux_force_point`<br>`apps/railitd/tests/api.rs`<br>`crates/railit-kernel/tests/borders.rs`<br>`crates/railit-kernel/tests/coupling.rs`<br>`crates/railit-kernel/tests/phase2.rs`<br>Szenario 10<br>Szenario 12 |
| REQ-IXL-015 | H-001, H-009 | `crates/railit-kernel/src/kernel.rs`<br>`crates/railit-kernel/src/lib.rs` | `substitute_signal_lets_train_pass`<br>`substitute_signal_times_out_and_drops`<br>`cancel_with_substitute_uses_time_lock`<br>`substitute_only_when_justified`<br>`crates/railit-kernel/tests/phase2.rs`<br>Szenario 10 |
| REQ-IXL-016 | H-010 | `crates/railit-kernel/src/kernel.rs`<br>`crates/railit-kernel/src/lib.rs` | `level_crossing_must_be_closed_before_proceed`<br>`level_crossing_opens_after_train_has_left`<br>`shared_locks_match_routes`<br>`crates/railit-kernel/tests/phase2.rs`<br>Szenario 09 |
| REQ-IXL-017 | H-012 | `crates/railit-adapters/src/ils.rs`<br>`crates/railit-kernel/src/invariants.rs`<br>`crates/railit-kernel/src/invariants/reference.rs`<br>`crates/railit-kernel/src/kernel/border.rs` | `never_both_sides_hold_permission`<br>`designated_side_takes_permission_after_settle_time`<br>`only_newer_offers_are_taken`<br>`take_back_permission_by_aux_operation`<br>`permission_conflict_is_alarmed`<br>`apps/railitd/tests/api.rs`<br>`crates/railit-kernel/tests/borders.rs`<br>`crates/railit-runtime/tests/ils.rs` |
| REQ-IXL-018 | H-012 | `crates/railit-kernel/src/kernel/border.rs` | `permission_is_given_only_when_safe`<br>`crates/railit-kernel/tests/borders.rs` |
| REQ-IXL-019 | H-001, H-012 | `crates/railit-kernel/src/kernel/border.rs` | `incoming_trains_are_authorized_only_without_permission`<br>`borders_are_consistent`<br>`crates/railit-kernel/tests/borders.rs` |
| REQ-IXL-020 | H-001, H-012 | `crates/railit-kernel/src/invariants.rs`<br>`crates/railit-kernel/src/invariants/reference.rs`<br>`crates/railit-kernel/src/kernel/border.rs` | `exit_needs_permission_and_clear_neighbour`<br>`lost_neighbour_drops_exit_signal`<br>`proceed_only_when_safe`<br>`crates/railit-kernel/tests/borders.rs` |
| REQ-IXL-021 | H-012 | `crates/railit-kernel/src/kernel/border.rs` | `never_two_stations_feed_the_single_track`<br>`single_track_direction_changes_in_order`<br>`crates/railit-kernel/tests/borders.rs`<br>`crates/railit-runtime/tests/ils.rs`<br>Szenario 13 |
| REQ-IXL-022 | H-006, H-016 | `apps/railitd/src/service.rs`<br>`crates/railit-kernel/src/io.rs` | `confirmation_must_restate_the_operation`<br>`apps/railitd/tests/api.rs`<br>`apps/railitd/ui/e2e/bedienplatz.test.ts`<br>`crates/railit-kernel/tests/phase2.rs`<br>Szenario 12 |
| REQ-IXL-023 | H-007 | `crates/railit-kernel/src/invariants.rs` | `linear_check_agrees_with_reference`<br>`kernel_never_becomes_unsafe` |
| REQ-IXL-024 | H-015 | `apps/railit/src/bench.rs` | `bench_measures_a_small_corridor` |
| REQ-IXL-025 | H-001, H-003, H-015, H-017 | `apps/railitd/src/takeover.rs`<br>`crates/railit-kernel/src/kernel.rs`<br>`crates/railit-runtime/src/runtime.rs` | `proceed_falls_to_halt_and_locks_remain`<br>`locked_route_and_train_in_route_continue`<br>`pending_aux_expires_and_substitute_goes_out`<br>`safe_state_is_kept`<br>`kernel_never_becomes_unsafe`<br>`crates/railit-kernel/tests/common/mod.rs`<br>`crates/railit-kernel/tests/invariants.rs`<br>`crates/railit-kernel/tests/takeover.rs`<br>`crates/railit-runtime/tests/takeover.rs` |
| REQ-IXL-026 | H-002, H-003 | `crates/railit-kernel/src/kernel.rs` | `single_move_takes_the_partner_along`<br>`route_locks_all_four_points`<br>`forced_move_takes_the_partner_along`<br>`coupled_points_share_one_drive`<br>`crates/railit-adapters/tests/drives.rs`<br>`crates/railit-kernel/tests/coupling.rs`<br>Szenario 25<br>Szenario 27 |

## [kommunikation.md](../requirements/kommunikation.md)

| ID | Gefährdung | Code | Nachweis |
|----|------------|------|----------|
| REQ-COM-001 | H-005, H-014 | – | `corruption_is_detected`<br>`crates/railit-rasta/tests/rasta.rs`<br>Szenario 18 |
| REQ-COM-002 | H-014 | – | `foreign_sender_is_rejected`<br>`crates/railit-rasta/tests/rasta.rs` |
| REQ-COM-003 | H-014 | `crates/railit-rasta/src/lib.rs` | `replayed_messages_are_discarded`<br>`lost_messages_are_retransmitted_in_order`<br>`lost_last_message_is_noticed_by_heartbeat`<br>`delivery_is_an_in_order_prefix`<br>`both_directions_deliver_in_order_prefixes`<br>`crates/railit-rasta/tests/rasta.rs` |
| REQ-COM-004 | H-014 | – | `delayed_messages_break_the_connection`<br>`crates/railit-rasta/tests/rasta.rs` |
| REQ-COM-005 | H-005, H-014 | – | `heartbeats_keep_the_connection_alive`<br>`silence_times_out`<br>`crates/railit-rasta/tests/rasta.rs` |
| REQ-COM-006 | H-005, H-014 | – | `field_over_eulynx_and_rasta`<br>`apps/railitd/tests/api.rs`<br>Szenario 18<br>Szenario 19 |
| REQ-COM-007 | H-005, H-014 | `crates/railit-rasta/src/lib.rs` | `twenty_percent_loss_does_not_break_the_connection`<br>`both_directions_deliver_in_order_prefixes`<br>`crates/railit-rasta/tests/rasta.rs`<br>Szenario 18 |
| REQ-COM-008 | H-014, H-015 | `apps/railit-field/src/main.rs`<br>`apps/railitd/src/field.rs`<br>`crates/railit-adapters/src/ils.rs`<br>`crates/railit-adapters/src/net.rs`<br>`crates/railit-fuzz/src/lib.rs` | `leader_connects_and_controls_the_field`<br>`remote_field_over_rasta`<br>`field_network_must_match`<br>`apps/railitd/tests/field.rs`<br>`crates/railit-adapters/tests/net.rs` |
| REQ-COM-009 | H-012, H-014 | – | `telegram_roundtrip_and_rejects`<br>`interrupted_link_delivers_nothing_and_recovers`<br>`corruption_is_never_delivered`<br>`permission_stays_exclusive_over_disturbed_ils`<br>`direction_change_waits_for_the_link`<br>`all_scenarios_pass_over_sci_ils`<br>`borders_over_sci_ils`<br>`sci_ils`<br>`apps/railitd/tests/api.rs`<br>`crates/railit-runtime/tests/ils.rs`<br>`crates/railit-runtime/tests/scenarios.rs`<br>Szenario 22 |
| REQ-COM-010 | H-012, H-014 | `crates/railit-adapters/src/ils.rs`<br>`crates/railit-adapters/src/net.rs` | `split_processes_exchange_over_the_network`<br>`border_between_processes`<br>`areas_in_separate_processes`<br>`switch_per_area`<br>`standby_takes_over_a_part_of_the_network`<br>`ils_net`<br>`apps/railitd/tests/api.rs`<br>`apps/railitd/tests/field.rs`<br>`crates/railit-adapters/tests/net.rs`<br>`crates/railit-runtime/tests/ils.rs` |
| REQ-COM-011 | H-014 | `apps/railit-field/src/main.rs`<br>`crates/railit-adapters/src/net.rs`<br>`crates/railit-adapters/src/redundant.rs`<br>`crates/railit-fuzz/src/lib.rs`<br>`crates/railit-rasta/src/redundancy.rs` | `duplicates_from_the_second_channel_are_filtered`<br>`a_lost_frame_comes_over_the_other_channel`<br>`a_gap_is_skipped_after_t_seq`<br>`diagnosis_counts_missing_frames_per_channel`<br>`redundant_channels_survive_a_channel_failure`<br>`rasta_redundancy`<br>`apps/railitd/tests/field.rs` |

## [plan.md](../requirements/plan.md)

| ID | Gefährdung | Code | Nachweis |
|----|------------|------|----------|
| REQ-PLN-001 | H-004 | `crates/railit-plan/src/lib.rs`<br>`crates/railit-plan/src/view.rs` | Analyse (Typsystem) |
| REQ-PLN-002 | H-004 | `crates/railit-plan/src/error.rs`<br>`crates/railit-plan/src/view.rs`<br>`crates/railit-planning/src/error.rs` | `inconsistent_plans_are_rejected`<br>`crates/railit-plan/tests/image.rs` |
| REQ-PLN-003 | H-004, H-008 | – | `every_single_bit_flip_is_detected`<br>`crates/railit-plan/tests/image.rs` |
| REQ-PLN-004 | H-008 | `crates/railit-plan/src/approval.rs`<br>`crates/railit-plan/src/lib.rs`<br>`crates/railit-plan/src/model.rs`<br>`crates/railit-plan/src/view.rs`<br>`crates/railit-planning/src/lib.rs`<br>`crates/railit-runtime/src/keys.rs`<br>`crates/railit-runtime/src/lib.rs` | `signatures_are_checked`<br>`unsigned_or_foreign_plans_are_refused`<br>`keygen_compile_verify_info`<br>`apps/railitd/tests/api.rs`<br>`crates/railit-plan/tests/image.rs` |
| REQ-PLN-010 | H-001, H-005 | `crates/railit-planning/src/error.rs`<br>`crates/railit-planning/src/generate.rs`<br>`crates/railit-planning/src/lib.rs`<br>`crates/railit-planning/src/verify.rs` | `signals_must_stand_at_tvd_boundaries`<br>`signals_may_stand_shortly_before_the_tvd_boundary`<br>`crates/railit-planning/tests/planning.rs` |
| REQ-PLN-011 | H-001, H-002 | `crates/railit-planning/src/error.rs`<br>`crates/railit-planning/src/lib.rs` | `example_routes_are_generated`<br>`ladder_stations_plan_and_verify` |
| REQ-PLN-012 | H-001 | `crates/railit-plan/src/view.rs`<br>`crates/railit-planning/src/generate.rs`<br>`crates/railit-planning/src/lib.rs` | `example_routes_are_generated`<br>`example_phase2_data`<br>`ladder_stations_plan_and_verify`<br>`crates/railit-planning/tests/planning.rs` |
| REQ-PLN-013 | H-004 | `crates/railit-planning/src/error.rs`<br>`crates/railit-planning/src/lib.rs`<br>`crates/railit-planning/src/verify.rs` | `verification_detects_tampering`<br>`crates/railit-planning/tests/planning.rs` |
| REQ-PLN-014 | H-004 | `crates/railit-planning/src/error.rs` | `loops_without_signal_are_rejected`<br>`crates/railit-planning/tests/planning.rs` |
| REQ-PLN-015 | H-001, H-009 | `crates/railit-planning/src/generate.rs`<br>`crates/railit-planning/src/lib.rs` | `example_phase2_data`<br>`crates/railit-planning/tests/planning.rs` |
| REQ-PLN-016 | H-001 | `crates/railit-planning/src/error.rs`<br>`crates/railit-planning/src/generate.rs`<br>`crates/railit-planning/src/lib.rs` | `flanks_must_be_protectable`<br>`example_phase2_data`<br>`crates/railit-planning/tests/planning.rs` |
| REQ-PLN-017 | H-009 | `crates/railit-planning/src/error.rs`<br>`crates/railit-planning/src/generate.rs`<br>`crates/railit-planning/src/lib.rs` | `distant_signals_need_a_unique_main_signal`<br>`crates/railit-planning/tests/planning.rs` |
| REQ-PLN-018 | H-010, H-002 | `crates/railit-planning/src/lib.rs` | `example_phase2_data`<br>`level_crossing_within_long_overlap`<br>`crates/railit-planning/tests/planning.rs` |
| REQ-PLN-019 | H-001, H-012 | `crates/railit-planning/src/error.rs`<br>`crates/railit-planning/src/generate.rs`<br>`crates/railit-planning/src/lib.rs` | `nordbahn_compiles_per_area`<br>`borders_need_an_entry_signal`<br>`inconsistent_border_data_is_rejected`<br>`crates/railit-planning/tests/planning.rs` |
| REQ-PLN-020 | H-007 | `crates/railit-plan/src/view.rs` | `inconsistent_phase2_data_is_rejected` |
| REQ-PLN-021 | H-001, H-002, H-004 | `crates/railit-plan/src/format.rs`<br>`crates/railit-plan/src/model.rs`<br>`crates/railit-plan/src/view.rs`<br>`crates/railit-planning/src/error.rs`<br>`crates/railit-planning/src/generate.rs` | `routes_over_a_double_slip_require_all_four_points`<br>`single_slip_has_one_curve`<br>`crossover_points_move_together`<br>`tampered_couplings_are_rejected`<br>`coupled_points_must_not_lie_in_an_approach`<br>`crates/railit-kernel/tests/coupling.rs`<br>`crates/railit-planning/tests/slip.rs`<br>Szenario 25 |
| REQ-PLN-022 | H-008, H-004 | `apps/railit-node/src/main.rs`<br>`apps/railit/src/plan.rs`<br>`apps/railitd/src/main.rs`<br>`crates/railit-plan/src/approval.rs`<br>`crates/railit-runtime/src/keys.rs` | `plans_need_approvals_from_distinct_trusted_reviewers`<br>`plans_need_four_eyes_approval`<br>`keygen_compile_verify_info`<br>`apps/railit/tests/cli.rs` |

## [sicherheit.md](../requirements/sicherheit.md)

| ID | Gefährdung | Code | Nachweis |
|----|------------|------|----------|
| REQ-SEC-001 | H-006 | `apps/railitd/src/auth.rs` | `users_file_is_validated`<br>`roles_areas_and_audit_with_tokens`<br>`token_and_audit_verification`<br>`apps/railit/tests/cli.rs`<br>`apps/railitd/tests/security.rs` |
| REQ-SEC-002 | H-006 | `apps/railitd/src/tls.rs` | `mutual_tls_identifies_users_by_certificate`<br>`apps/railitd/tests/security.rs` |
| REQ-SEC-003 | H-006 | `apps/railitd/src/auth.rs` | `roles_grant_their_permissions`<br>`roles_areas_and_audit_with_tokens`<br>`interlocking_over_grpc`<br>`apps/railitd/tests/security.rs` |
| REQ-SEC-004 | H-006, H-011 | `apps/railit/src/admin.rs`<br>`apps/railitd/src/service.rs`<br>`crates/railit-journal/src/audit.rs` | `chain_detects_tampering`<br>`reopened_log_continues_the_chain`<br>`roles_areas_and_audit_with_tokens`<br>`token_and_audit_verification`<br>`apps/railit/tests/cli.rs`<br>`apps/railitd/tests/security.rs` |
| REQ-SEC-005 | H-006 | – | `open_mode_only_on_loopback` |
| REQ-SEC-006 | H-006 | `apps/railitd/src/tls.rs` | `revoked_client_certificates_are_refused`<br>`apps/railitd/tests/security.rs` |

## [sicherheitsschicht.md](../requirements/sicherheitsschicht.md)

| ID | Gefährdung | Code | Nachweis |
|----|------------|------|----------|
| REQ-SAF-001 | H-001, H-007, H-015 | `crates/railit-safety/src/channels.rs`<br>`crates/railit-safety/src/lib.rs` | `healthy_channels_are_transparent`<br>`two_out_of_two_trips_on_stray_command`<br>`two_out_of_two_detects_state_divergence`<br>`voted_outputs_are_reference_outputs_or_safe`<br>`all_scenarios_pass_with_three_channels`<br>`voter_forwards_only_confirmed_outputs`<br>`crates/railit-runtime/tests/scenarios.rs`<br>`crates/railit-safety/tests/voter.rs`<br>Szenario 21 |
| REQ-SAF-002 | H-007, H-015 | `crates/railit-safety/src/channels.rs`<br>`crates/railit-safety/src/lib.rs` | `two_out_of_three_masks_one_channel`<br>`voted_outputs_are_reference_outputs_or_safe`<br>`voter_forwards_only_confirmed_outputs`<br>`redundant_channels_over_api`<br>`apps/railitd/tests/api.rs`<br>`crates/railit-safety/tests/voter.rs`<br>Szenario 20 |
| REQ-SAF-003 | H-007, H-015 | `crates/railit-kernel/src/state.rs`<br>`crates/railit-runtime/src/runtime.rs`<br>`crates/railit-safety/src/channels.rs`<br>`crates/railit-safety/src/lib.rs` | `two_out_of_three_masks_one_channel`<br>`journal_covers_trip_and_reset_of_the_safety_layer`<br>`redundant_channels_over_api`<br>`apps/railitd/tests/api.rs`<br>`crates/railit-safety/tests/voter.rs`<br>Szenario 20 |
| REQ-SAF-004 | H-001, H-006, H-009, H-015 | `crates/railit-runtime/src/runtime.rs`<br>`crates/railit-safety/src/channels.rs`<br>`crates/railit-safety/src/lib.rs` | `two_out_of_two_trips_on_stray_command`<br>`external_trip_holds_all_signals`<br>`output_overflow_trips`<br>`journal_covers_trip_and_reset_of_the_safety_layer`<br>`redundant_channels_over_api`<br>`roles_grant_their_permissions`<br>`apps/railitd/tests/api.rs`<br>`crates/railit-runtime/tests/journal.rs`<br>Szenario 20<br>Szenario 21 |
| REQ-SAF-005 | H-007, H-015 | `crates/railit-safety/src/lib.rs`<br>`crates/railit-safety/src/watchdog.rs` | `deadline_and_gap`<br>`backward_clock_trips_even_without_deadline`<br>`external_trip_holds_all_signals`<br>`watchdog_detects_every_overrun`<br>`watchdog_trips_on_overrun`<br>`crates/railit-safety/tests/voter.rs` |
| REQ-SAF-006 | H-007, H-014 | – | `voter_forwards_only_confirmed_outputs`<br>`watchdog_detects_every_overrun`<br>`fixed_outputs_never_lose_silently`<br>`bitset_insert_remove_are_exact`<br>`bitset_union_is_exact`<br>`time_never_overflows`<br>`md4_never_panics_on_shortest_pdu`<br>`md4_never_panics_on_two_blocks`<br>`closed_connection_never_delivers`<br>`packer_respects_the_payload_limit`<br>`crates/railit-embedded/src/proofs.rs`<br>`crates/railit-rasta/src/proofs.rs`<br>`crates/railit-safety/src/proofs.rs`<br>`crates/railit-types/src/proofs.rs` |
| REQ-SAF-007 | H-004, H-008, H-014 | – | `seeds_pass`<br>`plan_mutation_sweep`<br>`truncated_sources`<br>`random_plan_mutations`<br>`random_bytes`<br>`crates/railit-fuzz/tests/smoke.rs` |
| REQ-SAF-008 | H-015 | `crates/railit-embedded/src/node.rs`<br>`crates/railit-runtime/src/remote.rs`<br>`crates/railit-runtime/src/runtime.rs`<br>`crates/railit-safety/src/channels.rs`<br>`crates/railit-safety/src/lib.rs`<br>`crates/railit-safety/src/memory.rs` | `a_flipped_plan_bit_is_found_within_one_pass`<br>`intact_memory_passes_and_counts_passes`<br>`memory_check_finds_a_corrupted_plan`<br>`crates/railit-embedded/tests/node.rs` |

## [topologie.md](../requirements/topologie.md)

| ID | Gefährdung | Code | Nachweis |
|----|------------|------|----------|
| REQ-TOP-001 | H-004 | `crates/railit-topology/src/error.rs` | `duplicate_names_are_rejected_across_kinds`<br>`crates/railit-import/tests/toml_format.rs`<br>`crates/railit-topology/tests/validation.rs` |
| REQ-TOP-002 | H-004 | `crates/railit-topology/src/builder.rs`<br>`crates/railit-topology/src/error.rs` | `open_and_double_connected_ports_are_rejected`<br>`apps/railit/tests/cli.rs`<br>`crates/railit-import/tests/toml_format.rs`<br>`crates/railit-topology/tests/validation.rs` |
| REQ-TOP-003 | H-004 | `crates/railit-topology/src/error.rs` | `invalid_ports_and_self_connections_are_rejected`<br>`crates/railit-topology/tests/validation.rs` |
| REQ-TOP-004 | H-004 | `crates/railit-topology/src/error.rs` | `zero_length_tracks_are_rejected`<br>`crates/railit-topology/tests/validation.rs` |
| REQ-TOP-005 | H-004 | `crates/railit-topology/src/error.rs` | `signals_outside_their_track_are_rejected`<br>`crates/railit-import/tests/toml_format.rs`<br>`crates/railit-topology/tests/validation.rs` |
| REQ-TOP-006 | H-001, H-005 | `crates/railit-topology/src/builder.rs`<br>`crates/railit-topology/src/error.rs` | `tvd_coverage_must_be_complete_and_unique`<br>`crates/railit-import/tests/toml_format.rs`<br>`crates/railit-topology/tests/validation.rs` |
| REQ-TOP-007 | H-002 | `crates/railit-topology/src/lib.rs`<br>`crates/railit-topology/src/topology.rs` | `points_can_only_be_traversed_tip_to_branch`<br>`exits_stay_on_node`<br>`crates/railit-topology/tests/validation.rs` |
| REQ-TOP-008 | H-004 | `crates/railit-topology/src/lib.rs`<br>`crates/railit-topology/src/topology.rs` | Analyse (Typsystem) |
| REQ-TOP-009 |  | `crates/railit-topology/src/builder.rs`<br>`crates/railit-topology/src/error.rs` | `all_errors_are_collected_with_context` |
| REQ-TOP-010 | H-004 | `crates/railit-topology/src/builder.rs`<br>`crates/railit-topology/src/error.rs` | `open_and_double_connected_ports_are_rejected` |
| REQ-TOP-011 | H-004, H-012 | `crates/railit-topology/src/builder.rs`<br>`crates/railit-topology/src/error.rs` | `nordbahn_has_three_areas_and_two_links`<br>`every_tvd_needs_exactly_one_area`<br>`crates/railit-import/tests/areas.rs` |
| REQ-TOP-012 | H-004 | `crates/railit-topology/src/builder.rs`<br>`crates/railit-topology/src/error.rs` | `platforms_need_route_signals` |
| REQ-TOP-013 | H-004, H-012 | `crates/railit-topology/src/area.rs` | `area_cut_replaces_neighbours_by_borders`<br>`border_terminals_can_be_written_by_hand`<br>`crates/railit-import/tests/areas.rs` |
| REQ-TOP-014 | H-004 | – | `handmade_railml_is_imported`<br>`errors_are_collected_with_location`<br>`beispielstadt_survives_the_roundtrip`<br>`nordbahn_with_areas_and_stations_survives_the_roundtrip`<br>`railml_roundtrip_yields_identical_plans`<br>`railml_export_validate_compile`<br>`apps/railit/tests/cli.rs`<br>`crates/railit-import/tests/railml.rs`<br>`crates/railit-planning/tests/planning.rs` |
| REQ-TOP-015 | H-004 | – | `handmade_planpro_is_imported`<br>`errors_are_collected_with_location`<br>`beispielstadt_survives_the_roundtrip`<br>`generated_corridor_survives_the_roundtrip`<br>`planpro_roundtrip_yields_identical_plans`<br>`planpro_network`<br>`crates/railit-import/tests/planpro.rs`<br>`crates/railit-planning/tests/planning.rs`<br>Szenario 23 |
| REQ-TOP-016 | H-001, H-004 | `crates/railit-import/src/toml_format.rs`<br>`crates/railit-topology/src/builder.rs`<br>`crates/railit-topology/src/error.rs`<br>`crates/railit-topology/src/model.rs` | `crossing_legs_share_one_tvd`<br>`crossing_survives_the_roundtrip`<br>`crates/railit-import/tests/planpro.rs`<br>`crates/railit-import/tests/railml.rs`<br>`crates/railit-import/tests/toml_format.rs`<br>Szenario 24 |
| REQ-TOP-017 | H-001, H-002, H-004 | `crates/railit-adapters/src/lib.rs`<br>`crates/railit-import/src/planpro.rs`<br>`crates/railit-import/src/railml.rs`<br>`crates/railit-sim/src/lib.rs`<br>`crates/railit-topology/src/area.rs`<br>`crates/railit-topology/src/builder.rs`<br>`crates/railit-topology/src/error.rs`<br>`crates/railit-topology/src/model.rs`<br>`crates/railit-topology/src/topology.rs` | `double_slip_is_built_from_coupled_points`<br>`invalid_couplings_are_rejected`<br>`slip_names_are_checked_first`<br>`slips_and_couplings_survive_the_roundtrip`<br>`slips_survive_the_planpro_roundtrip`<br>`broken_slip_is_rejected`<br>`crates/railit-adapters/tests/drives.rs`<br>`crates/railit-import/tests/planpro.rs`<br>`crates/railit-import/tests/railml.rs`<br>`crates/railit-planning/tests/slip.rs`<br>Szenario 25 |

---

134 Anforderungen, davon 134 mit Nachweis.
