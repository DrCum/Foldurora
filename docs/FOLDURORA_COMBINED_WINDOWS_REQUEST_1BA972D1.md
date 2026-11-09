# Windows request — combined workspace, upstream and guarded MCP

Prepared for ONE relay through Slick to the existing Foldurora Windows Builder
on Cumputer, thread `01a1af77-a465-71c3-be08-cd9d1b927f83`. A903 completed successfully; the lead could not verify newer direct status.
Check history with Slick FIRST. If da8ee12f has already started, finish/preserve
that run and queue this distinct source afterward; do not cancel it. If its
request is still prepared/undelivered, mark it superseded and deliver this
superset ONCE instead. Never start both requests concurrently, repeat a delivered
request or infer execution from publication. No replacement Builder.

Repository: https://github.com/DrCum/Foldurora.git
Branch: `feat/mcp-advanced-integration-20261109`
Commit: `1ba972d1419b22ee918515b12ecd5e6d42c4ca45`
Tree: `5cda6cd40d64c9355bf381fa4689532f79c52515`
Cargo.lock Git-LF SHA256: `c869d88f1f837f302fde6ac52dfaa5e7ff0a325feac9ddf6f77da5954a7e852a`
Stacked on PR36/35/34/31/30/29; exact frozen source above, not a docs commit.
Source and remote tree are verified/frozen. A documentation commit containing
this request is NOT the build source. Use matching host/worker binaries.

## Scope and prior evidence

This is the combined Advanced Workspace + queued upstream + guarded functional
MCP checkpoint. It contains the complete da8ee12f workspace group, reviewed
PR18/19/26/32 media/audio/confirmation/controls ports and PR33's independent
controller/character tools, permissions, native Ask, source fences and adapters.
MCP starts disabled; host enablement never enables character assistants.

MCP workspace apply/previous are explicitly unsupported native capabilities in
this integration. Their older alternate context/HUD backend is not integrated.
Current native user workspaces/HUD consent/storage schema stay intact; MCP reads
use fresh actual source/profile/grid/privacy and a geometry/chrome allowlist,
without saved private references. Discovery does not establish native support.
No automatic source repair or enablement of that deferred bridge is requested.
See source docs FOLDURORA_MCP_ADVANCED_INTEGRATION_20261109.md and FOLDURORA_MCP.md.

Linux: format, strict MCP Clippy, qualified aggregate/native/prototype compile
checks;160 actual MCP/session cases (70library+56library+30core+4 real SDK stdio),
13localization and1appearance scan pass. Parent source also passed audio106/0/4
and media25/0/0. Genuine native viewer/net cases compile but remain unrun locally.
Full Linux viewer linking is resource blocked. The old terrain/software-GPU case
remains unresolved; no new full Linux pass or native acceptance is claimed.

A903's separate Windows8970/0/58,1184 coverage, prototype checks and ordinary
viewer/sidecar package remain preserved. Its standalone Vulkan startup is not
hosted or login-bound acceptance. This new source is outside that package:
https://github.com/DrCum/Foldurora/blob/03a6024fb0b96827c8eb277168b0e3e84e52ff7f/docs/FOLDURORA_WINDOWS_VALIDATION_A903E16C.md

## Checks and stop gates

Reuse owned camera-6a3009f-windows and compatible persistent DEV-profile caches.
Preserve prior source/worktrees/heads, binaries, profiles, logs and screenshots.
No clean, new cache/profile, dependency/tool upgrade, source fix, ACL/security/
long-path-policy change, credential use, grid login, install, merge or release.
Record launch context accurately; unobserved child facts remain unobserved.
Canonical lock comparisons must account for existing checkout line endings without
changing Git settings or tracked bytes. Return source fixes to the lead.

Run sequentially, capturing exact commands, exits, stdout/stderr and tools. A
failure stops downstream build/package/smoke; preserve it, never waive it or rerun
unchanged to hope it clears. After parser/format, run the small early changed-MCP
package gate before the expensive native group. Report actual platform totals;
Linux-only stdio/capacity cases are not Windows passes. Then run ONE unfiltered
full workspace suite; only that full run contributes coverage audit rows:

```powershell
if ((git rev-parse HEAD).Trim() -ne '1ba972d1419b22ee918515b12ecd5e6d42c4ca45') { throw 'Wrong source commit' }
if ((git rev-parse 'HEAD^{tree}').Trim() -ne '5cda6cd40d64c9355bf381fa4689532f79c52515') { throw 'Wrong source tree' }
git status --short
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\check-scripts.ps1
cargo fmt --all -- --check
cargo clippy --locked -p aurora-mcp --all-targets -- -D warnings
cargo test --locked -p aurora-mcp --no-fail-fast
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked --no-fail-fast
```

All cases in the full run must pass, in addition to the required-name audit.
Keep all summaries, ignored/failure names, panics and zero-filtered evidence.
Native store long-ordinary-path coverage now includes the new WorkspaceImages
kind; it must execute unchanged legacy-kind assertions and new-kind assertions.
The dedicated managed store corruption/immutability/no-create tests are required.
Do not replace security assertions, alter fixtures to hide failures or silently
filter platform-available cases. Source guards remain intact.

Renderer/session code changed in this MCP integration. After the full suite,
run separate nondefault prototype gates with process-scoped AURORA_RENDER_THREAD=1
and AURORA_GPU_VALIDATION=1, preserving inherited values. Use owned synthetic
windows/devices, no grid or deliberate GPU loss. Keep actual adapter/driver
observations separate from startup smoke; never infer unobserved identity.

```powershell
cargo clippy --locked -p aurora-render --features monitor-prototype --all-targets -- -D warnings
cargo test --locked -p aurora-render --features monitor-prototype --lib monitor -- --nocapture
cargo test --locked -p aurora-render --features monitor-prototype --lib render_thread::tests -- --nocapture
cargo test --locked -p aurora-render --features monitor-prototype --lib session_fence::tests -- --nocapture
cargo test --locked -p aurora-session preview_ -- --nocapture
```

Filters overlap; report distinct names, not an added unique total. Do not package
on any failure. These tests do not enable production inactive rendering.


After the full gate succeeds, build/package ordinary viewer+sidecar WITHOUT
monitor-prototype, using the existing packaging and ZIP verification tooling:

```powershell
cargo build --locked -p aurora-viewer -p aurora-mcp --features aurora-viewer/xr-static-loader
.\target\debug\aurora-viewer.exe --version
.\target\debug\foldurora-mcp.exe --version
.\target\debug\foldurora-mcp.exe --help
.\scripts\package.ps1 -Exe .\target\debug\aurora-viewer.exe -McpExe .\target\debug\foldurora-mcp.exe -Out 'C:\Users\Marty\..Projects\Aurora Viewer\Builds\Foldurora-1ba972d1419b\Foldurora-1ba972d1419b-windows-x64'
```

Viewer must identify 1ba972d14, sidecar0.1.0. Preserve any cached-version mismatch;
use only normal Cargo version regeneration, no overrides/clean. Verify built,
staged and extracted viewer/sidecar hashes match, manifests and required MCP/
licensing documents are included. Return executable versions, package/ZIP path,
byte sizes, SHA256 and verification outcomes. No listener/discovery is expected
from ordinary default startup. ## Required full-run coverage —1356 public-source cases

Retain a903's1184 audited requirements and add these172 public source identifiers
from the integrated source. Source-bind the audit to this SHA/tree. Exact full
name or unique final identifier matches only, exactly one PASS per requirement
from THIS full run alone, zero missing/nonpassing/duplicates. All full cases must
also pass. Earlier package/prototype/filtered rows do not substitute. If the old
private coverage input is unavailable, report through Slick; never publish it,
its identifying metadata, raw rows/logs, or replace it with a weaker list.

No baseline directly annotated test definition was removed. The unintegrated
alternate MCP workspace backend's two Context cases and one HUD queue case were
never part of the lead baseline; record them as not integrated, not passed or
waived. New native projection/refusal cases cover the explicit capability limit.
New Linux/Unix-only source guards remain unchanged: SDK stdio (Linux) and three
native upload-handle cases (Unix) are separate Linux requirements, not Windows
passes. The three native viewer cases remain resource-blocked locally. Never
waive unrelated store/path/slot/archive/creator/RLVa tests. Added public names:

- `a_closed_connection_still_delivers_what_was_read`
- `a_flooding_plugin_is_read_a_budget_per_idle_in_order_and_stays_alive`
- `a_huge_backlog_is_read_within_the_budget_of_each_frame_and_nothing_is_lost`
- `a_message_split_across_reads_is_handed_out_once_complete`
- `a_plugin_disconnect_delivers_queued_messages_before_closing_or_returning_an_error`
- `action_ask_requires_native_resolution_then_exactly_one_execution`
- `ask_requires_native_resolution_and_connection_owned_receipts`
- `assistant_stop_retires_queued_publication_and_owner_grants`
- `asynchronous_native_commit_is_rejected_after_authenticated_disconnect`
- `authenticated_capture_client_endpoint_and_reenable_do_not_reuse_authority`
- `body_markers_do_not_classify_unknown_or_bodyful_successes_as_factual`
- `bounded_recipe_execution_budget_does_not_extend_approval_or_other_tools`
- `bounded_results_queues_dedup_and_disconnect`
- `cached_task_source_requires_original_classes_and_current_native_permission`
- `cancel_requires_exact_request_and_same_circuit_incarnation_once`
- `cancel_retains_tombstone_and_committed_native_prompt_can_still_finish`
- `cancelled_or_expired_preparation_never_opens_on_later_detach_or_shutdown`
- `cleanup_preserves_manual_edits_and_keeps_blocked_drafts_out_of_saved_layer`
- `cold_scope_is_loaded_before_freezing_and_later_descendants_require_fresh_approval`
- `compound_classes_require_native_permissions`
- `contents_copy_correlation_requires_fresh_unique_owned_asset_evidence`
- `decoded_clip_edges_do_not_step_from_silence_or_at_a_loop_seam`
- `decoded_clip_edges_stay_smooth_in_ui_world_and_looped_playback`
- `decoded_short_clips_keep_their_middle_and_raw_pcm_keeps_its_edges`
- `decoded_stereo_edges_preserve_relative_channel_level_and_polarity`
- `decoded_vorbis_clip_has_no_start_stop_or_loop_step`
- `default_disabled_and_domain_separation`
- `delegated_ui_capture_never_runs_in_composite_host_context`
- `delete_shortcut_retains_confirmation_worn_and_selected_item_guards`
- `derez_ack_never_confirms_a_wrong_destination_owner_link_or_ambiguous_item`
- `detached_native_preparation_cannot_follow_same_account_relogin_or_phase_change`
- `duplicate_and_oversized_selections_are_rejected`
- `duplicate_invoke_cannot_deliver_finished_private_result_during_native_update`
- `editor_undo_restores_own_channels_and_refuses_intervening_native_change`
- `encoded_proof_hash_is_exact_lowercase_sha256`
- `every_character_tool_has_one_native_adapter_or_closed_runtime_disposition`
- `exact_native_restore_keeps_unrelated_parked_worker_managed`
- `expiry_and_permanent_delete_and_wire_roundtrip`
- `explicit_native_takeover_preserves_owner_pose_and_grants_with_new_provenance`
- `factual_contents_reconciliation_never_revives_expired_action_authority`
- `final_action_gate_rejects_replacement_and_disabled_host`
- `final_commit_requires_exact_restored_source_and_expires_before_delayed_consumption`
- `final_native_prompt_commit_is_once_and_separate_from_restart_continuation`
- `fragments_are_authenticated_per_owner_conflict_checked_and_replay_bounded`
- `frozen_approval_scope_rejects_changed_linkset_recipe_or_authority`
- `generic_operation_status_withholds_source_after_native_rights_or_asset_changes`
- `generic_private_diagnostic_and_dialog_receipts_are_withheld_after_native_owner_or_circuit_drift`
- `inactive_scope_and_pause_prevent_execution`
- `independent_action_policy_and_client_binding_cannot_be_bypassed`
- `inline_preview_is_a_real_bounded_native_jpeg`
- `inspected_gesture_receipt_rejects_creator_rights_asset_or_source_replacement`
- `interrupted_economy_retires_exact_old_trim_before_another_source_uses_the_slot`
- `interrupted_reads_and_a_later_error_preserve_complete_messages_and_the_partial_tail`
- `invalid_deserialized_permissions_disable_old_allows_until_valid_configuration`
- `invitations_and_peer_beacons_never_create_editor_grants`
- `journal_is_bounded_filtered_and_gaps_invalidate_prior_capture_epochs`
- `library_callback_cannot_substitute_another_asset_owner_or_wearable_type`
- `losing_active_authority_never_revives_after_focus_returns`
- `many_messages_in_one_read_come_out_in_order`
- `mcp_cycling_scope_excludes_unapproved_sources_and_replacement_logins`
- `mcp_workspace_native_capabilities_refuse_unintegrated_mutation_even_when_sdk_discovers_it`
- `mcp_workspace_projection_never_serializes_private_hud_inventory_or_managed_image_references`
- `mesh_review_preserves_requested_destination_until_resident_changes_native_preferences`
- `mesh_review_refuses_occupied_importer_and_only_cancels_its_own_empty_window`
- `metadata_preparation_cannot_spend_or_self_grant_pending_mutation_authority`
- `mixed_folder_copy_obeys_no_copy_policy_and_preserves_native_item_rights`
- `monitor_actions_freeze_native_revision_and_reject_rebound_view`
- `monitor_capture_ignores_unused_stopped_profiles_and_freezes_only_opted_in_eligible_sources`
- `native_busy_accepts_intents_but_never_delivers_private_receipts`
- `native_busy_blocks_delivery_without_disabling_the_filtered_diagnostic_journal`
- `native_continuation_requires_one_real_send_and_explicit_authority`
- `native_create_callback_requires_exact_owner_parent_asset_and_subtype`
- `native_display_withdrawal_replaces_same_binding_and_economy_is_not_receipt_only`
- `native_exit_waits_for_actual_restore_and_keeps_cancel_authoritative`
- `native_extension_is_session_bound_and_receipt_never_proves_effect_completion`
- `native_extension_rejects_unknown_action_and_extra_credential_fields`
- `native_inline_images_accept_only_bounded_typed_jpeg_data`
- `native_mutation_during_probe_is_uncertain_and_revision_never_wraps`
- `native_selection_beam_uses_only_type_seven_exact_finite_payload`
- `native_stop_reset_and_save_invalidate_preserve_snapshot`
- `native_update_and_delete_are_observable_without_synthetic_acknowledgements`
- `native_visibility_changes_retire_private_rows_camera_artifacts_and_inline_preview`
- `native_workspace_additional_capture_emits_only_a_current_typed_source_request`
- `native_workspace_display_background_edits_are_typed_and_topology_change_retires_the_target`
- `native_workspace_image_chooser_emits_captured_display_target_without_mutating_private_background`
- `native_workspace_images_corrupt_library_is_not_repaired_or_overwritten_by_an_import`
- `native_workspace_images_kind_keeps_fixed_identity_paths_and_all_legacy_limits`
- `native_workspace_images_missing_load_and_cancelled_publication_never_create_a_store`
- `native_workspace_images_publish_load_exact_pixels_and_refuse_identity_or_immutable_id_replacement`
- `native_workspace_local_import_busy_control_emits_only_one_current_source_request`
- `operation_dedup_is_owned_by_authenticated_connection_and_incarnation`
- `operation_events_have_independent_authenticated_owner_lanes_and_no_foreign_gap`
- `owned_operation_cancel_disarms_restart_during_retired_source_interval`
- `owned_teleport_marker_cannot_cancel_a_new_manual_or_same_destination_request`
- `owner_reservations_require_grant_and_exact_revision_preserve_other_channels`
- `pending_folder_scope_rejects_a_new_descendant_even_if_folder_record_matches`
- `plans_do_not_follow_a_reused_client_number_into_another_endpoint`
- `policy_and_rename_inputs_are_closed`
- `preference_preview_budget_starts_after_approval_and_is_bounded_to_native_ttl`
- `preferences_reset_cancel_escape_outside_and_window_close_leave_settings_alone`
- `preferences_reset_confirmation_keeps_the_requested_page_and_runs_once_with_undo`
- `preferences_reset_dialogs_keep_the_footer_and_actions_visible`
- `preferences_reset_existing_demo_undo_still_responds`
- `preferences_reset_footer_only_opens_the_requested_confirmation`
- `preferences_reset_modified_button_asks_before_changing_any_row`
- `prerequisite_reads_reject_foreign_connection_or_retired_endpoint`
- `presentation_actions_wait_for_native_gui_cover_instead_of_broker_poll_dispatch`
- `presentation_captures_both_owners_and_revalidates_until_actual_handoff`
- `presentation_queue_dispatch_rechecks_revocation_and_cannot_replay`
- `preview_saved_projection_and_backup_preserve_baseline_and_unrelated_edits`
- `private_create_take_evidence_keeps_raw_transaction_and_exact_boolean_approval_separate_from_delivery`
- `private_reads_stay_never_and_never_denies`
- `protected_content_and_object_status_aliases_cannot_refresh_original_expiry_or_cross_clients`
- `public_control_types_and_native_bounds_are_closed`
- `quaternion_joint_scope_is_closed`
- `reader_and_completed_output_keep_admission_across_endpoint_reset`
- `refused_native_restore_fallback_keeps_core_barrier_until_exact_final_ack_or_real_exit`
- `remote_message_validation_is_atomic_and_cannot_replace_session_with_bad_snapshot`
- `replacement_and_stale_observation_do_not_reuse_enablement`
- `retired_camera_descriptors_cannot_refund_live_or_queued_encoder_memory`
- `revision_strings_are_exact_and_activity_comes_from_native_descriptors`
- `revocation_and_session_restart_retire_leases_and_editor_history`
- `revocation_blocks_async_callbacks_and_finished_results`
- `rotation_presets_roundtrip_and_refuse_wider_state`
- `same_caller_uuid_across_clients_never_collides_in_native_take_or_dialog_callbacks`
- `source_bound_test_stop_can_cancel_after_permissions_are_revoked_and_paused`
- `staged_exit_restores_exact_source_and_finalization_never_rehosts_detached_worker`
- `submitted_metadata_and_artifact_bytes_require_the_original_authenticated_owner`
- `system_font_popup_width_stays_fixed_across_ui_scales`
- `the_backspace_key_deletes_on_a_mac_only`
- `thumbnail_uses_native_jpeg2000_with_bounded_dimensions_and_validates_pixel_shape`
- `toolbar_style_rows_respond_at_their_visible_positions_in_both_looks`
- `tracked_rez_receipts_require_exact_client_endpoint_binding_and_policy`
- `transaction_identity_and_expiry_are_checked_independently_of_visible_uuid`
- `trusted_spending_review_cannot_override_never_or_disconnect`
- `trusted_spending_review_remains_pending_even_when_classes_allow`
- `typed_camera_image_requires_original_completed_capture_client_and_current_authority`
- `typed_status_cannot_cross_content_authority_domains`
- `unknown_oversized_and_expired_journal_cursors_require_resync`
- `upload_execution_budget_starts_after_approval_without_reviving_preview`
- `upload_status_dependency_is_atomic_before_execute_and_after_cached_delivery`
- `wire_pose_roundtrip_rejects_duplicates_nonfinite_and_unknown_flags`
- `withholding_is_capture_owned_and_body_free_ack_survives_creator_rights_loss`
- `workspace_additional_expansion_requires_its_original_painted_owner_and_keeps_main_state`
- `workspace_additional_inventory_schema_refuses_hidden_navigation_and_invalid_geometry`
- `workspace_additional_inventory_scope_preserves_absence_and_original_context`
- `workspace_additional_minimized_state_survives_native_next_paint_without_latent_close`
- `workspace_additional_pending_requires_exact_native_scope_and_never_extends_its_deadline`
- `workspace_additional_preview_scope_never_invents_hidden_folder_navigation`
- `workspace_additional_window_uses_one_native_role_and_never_drops_dependents_or_partial_windows`
- `workspace_background_texture_interest_preserves_hover_permanent_and_world_references`
- `workspace_display_background_codec_refuses_excess_collapsed_unknown_and_mismatched_colors`
- `workspace_display_background_geometry_ignores_origin_and_order_requires_canonical_bounds`
- `workspace_display_background_paint_clips_overrides_and_falls_back_on_unknown_topology`
- `workspace_display_background_preview_and_previous_keep_full_original_rules_and_source_fence`
- `workspace_display_background_roundtrip_preserves_common_and_only_matches_original_arrangement`
- `workspace_local_image_decoder_rejects_corrupt_png_even_when_the_structural_envelope_accepts_its_header`
- `workspace_local_image_envelope_refuses_paths_bad_base64_and_oversized_pixel_headers`
- `workspace_local_image_library_identity_immutability_and_failed_insert_preserve_original`
- `workspace_local_image_normalizer_preserves_original_shape_and_bounded_pixels_without_a_file_or_dialog`
- `workspace_local_import_intent_refuses_same_identity_logical_rebind_and_a_replaced_controller`
- `workspace_local_import_intent_survives_only_expected_revision_change_and_has_one_post_dialog_deadline`
- `workspace_local_picture_reference_is_profile_metadata_and_preserves_the_inventory_picture_wire_shape`
- `workspace_local_picture_regions_use_separate_managed_keys_and_keep_original_world_clips`
- `workspace_picture_cache_rejects_oversized_partial_pixels_and_releases_its_owned_repeat_texture`
- `workspace_picture_display_codec_refuses_span_overrides_and_preserves_common_fallback`
- `workspace_picture_picker_keeps_native_editor_selection_and_geometry_independent`
- `workspace_picture_placement_preserves_aspect_native_size_and_clipped_tile_uv`
- `workspace_picture_policy_requalifies_exact_native_item_permissions_archive_source_and_privacy_without_fetches`
- `workspace_picture_references_refuse_paths_nil_and_non_native_grid_keys`
- `workspace_picture_regions_keep_span_continuity_and_never_paint_world_or_unknown_geometry`
- `workspace_picture_upgrade_retains_display_colors_original_grid_and_scoped_removal`

## Changed MCP controls and offline smoke

After the full/build gates, use only isolated synthetic owned fixtures. Exercise
native controller and character opt-in/preferences, native Ask Deny/AllowOnce and
source/policy retirement where existing fixtures genuinely support them. No user
accounts/credentials or grid login; missing real source means NOT RUN, never a
fake authority bypass. The production-control CPU fixtures may be captured with
the documented AURORA_MCP_VISUAL_DIR/LANGUAGE overrides and filter
`mcp_cpu_visual_fixture -- --ignored --test-threads=1 --nocapture`; inspect actual
frozen-argument tail/actions/notifications, in both affected styles. These three
inert fixtures keep native dispatch disconnected and are component evidence only.
Do not mix their filtered outcomes into the full audit.

Ordinary default viewer startup must not create host OR character MCP discovery/
pipes. Enabling host must not enable a character. Do not turn on real-grid actions
or test spending/uploads. Read-only workspace projection omits private references;
MCP apply/previous must remain unsupported. Native old user workspaces still work.
Standalone Vulkan startup and passive discovery observations are startup evidence
only. Preserve Japanese missing-glyph limitation unless actually corrected/tested.

## Targeted offline/native acceptance

Use existing isolated development fixtures, owned PIDs and automation title/border;
never user accounts, credentials, saved-account selection or live-grid login. Use
actual native authority; if unavailable, record NOT RUN rather than fabricate it.
Do not repeat an unrelated screenshot matrix. Changed manager controls need a
normal/narrow/short check in both styles; existing CPU evidence is component-only.

1. In an authoritative existing synthetic context only: Save/Update captures
   opted-in additional Inventory geometry/open/minimized state and eligible loaded
   folders. Preview does not open/close native windows or wear HUDs. Commit respects
   source/restrictions and preserves unrelated editor selections/dependent windows.
   If the fixture cannot establish a native source, this remains human acceptance.
2. Display targets survive changed monitor origin/order when geometry matches;
   unavailable/changed topology uses common fallback. Fill/Fit/Stretch/Center/Tile/
   Span and brightness never paint over actual world. Keep normal editor picker
   and hover previews functional beside the separate workspace Inventory picker.
3. Native local dialog import/cancel, then reopen original profile; verify bounded
   image retention and no external path serialized. Change source/close manager/
   take another workspace action while a dialog is open: no late catalogue apply.
   A full/corrupt synthetic library refuses import without replacing original bytes.
   Use synthetic files only, never inspect the user's image directories.
4. Existing host movement/resize/switch/responsiveness, custom viewport/picking/
   mouselook and first/reopen Monitor checks remain important acceptance gaps.
   Record observed timings, styles, DPI and unavailable cases. No deliberate GPU
   loss; driver-reset recovery and XR stay unresolved unless separately verified.

Grid/RLVa, creator rights, real HUD additions, actual source changes and human
workspace acceptance remain Sodie's. A package pass is not proof of native host,
live inactive rendering, functional MCP or GPU reliability.

Return a sanitized GitHub-accessible result link via Slick plus exact source,
commands/exits, totals/failures, coverage summary, version/package/hash identities,
actual native observations/screenshots and unrun cases. Keep raw context/logs/
audits private. Verify source/artifact/worktree preservation before ending.
