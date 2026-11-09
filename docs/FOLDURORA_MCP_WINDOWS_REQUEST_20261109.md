# Windows request — host repair and disabled MCP foundation

**Source published and verified; request not delivered or started.** The exact
candidate below is available from GitHub. The thread reader returned
`connection closed before server request was answered`; no outbound relay tool
is currently exposed. Check Slick/Builder history before forwarding once.

Supersedes the undelivered standalone dd086 host request. Do not dispatch both.
This integrates MCP foundation 0cbc3cf1 with the host repair. The source branch
stays frozen at d0d8f8f5; the separate documentation branch holding this request
is not the build target.

Destination: existing Foldurora Windows Builder on Cumputer, thread
`01a1af77-a465-71c3-be08-cd9d1b927f83`, through Slick or Sodie. Do not duplicate
f08/bc20/48042 builds or create a replacement builder.

Repository: `https://github.com/DrCum/Foldurora.git`
Branch: `feat/host-mcp-integration-20261109`
Commit: `d0d8f8f5ad73f177160882a4543252e6941d661d`
Tree: `af882184e494d378b9523c9f90c3686dd003a2dc`
Base: `f08c22e2b0875b7d9b22a1dfe8742c1884321ff5`

Reuse clean owned `camera-6a3009f-windows` and its compatible persistent dev
cache. Preserve source/worktree registrations, f08 and older packages, the
known-good build and unresolved-failure evidence. Do not clean, upgrade tools
or dependencies, change Windows security/ACL/long-path policy, install, merge,
release or log into a grid. Return source corrections to the lead; do not fix
source in the frozen build checkout. Host protocol v2 requires matching host
and worker executables; do not combine these binaries with f08 workers.

After checking that this request is not already running, fetch the explicit branch, verify the
full commit/tree and clean tracked state, and run the following once, recording
actual exit codes and stdout/stderr separately. Stop and return compile/test
failures before packaging; do not filter or waive them. Existing dev caches are
preferred over starting another ci dependency graph on Windows.

```powershell
if ((git rev-parse HEAD).Trim() -ne 'd0d8f8f5ad73f177160882a4543252e6941d661d') { throw 'Wrong source commit' }
if ((git rev-parse 'HEAD^{tree}').Trim() -ne 'af882184e494d378b9523c9f90c3686dd003a2dc') { throw 'Wrong source tree' }
git status --short
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\check-scripts.ps1
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked --no-fail-fast
cargo build --locked -p aurora-viewer -p aurora-mcp --features aurora-viewer/xr-static-loader
.\target\debug\aurora-viewer.exe --version
.\target\debug\foldurora-mcp.exe --version
.\target\debug\foldurora-mcp.exe --help
.\scripts\package.ps1 -Exe .\target\debug\aurora-viewer.exe -McpExe .\target\debug\foldurora-mcp.exe -Out 'C:\Users\Marty\..Projects\Aurora Viewer\Builds\Foldurora-d0d8f8f5ad73\Foldurora-d0d8f8f5ad73-windows-x64'
```

Use the existing package/ZIP verification procedure in that output directory.
Confirm the final EXE version identifies `d0d8f8f5a`, hash EXE and ZIP, and
verify the ZIP-extracted EXE hash equals the packaged EXE. Also return the
sidecar version (expected `foldurora-mcp 0.1.0`), bytes and SHA256 and verify its
ZIP-extracted hash. Verify `FOLDURORA_MCP.md` and `NOTICE-MCP.txt` are packaged.
No MCP listener should start in the viewer: enablement/profile/approval UI is
unfinished and the production authority is default-disabled. The previous f08
version-cache mismatch must not recur unnoticed: preserve a mismatched binary
and use the already proven normal Cargo version regeneration procedure, with
no source/version override or clean-cache rebuild. Return its commands too.

## Coverage gate from the existing private checklist

Reuse the exact privately retained `combined-source-requirements.json` from f08:
4,799,090 bytes, SHA256
`8afdac85d57a2ddea9e7adc4c1c8150b15ba25fdbde2e4852205c753fcf8e75e`.
It was already restored/verified in the completed f08 build. No new large file
transfer is needed. Start from `required.windows.required_names` (1,087 names).
Remove these superseded contracts if present:

- `native_geometry_refresh_requiesces_even_at_the_same_rect_and_rejects_superseded_proofs`
- `fitted_workers_share_full_render_and_toolbar_geometry_until_detached`

Add these exact names and deduplicate:

- `native_translation_keeps_render_grant_and_requires_exact_latest_placement_receipt`
- `native_resize_completes_each_fence_and_coalesces_without_granting_obsolete_geometry`
- `native_translation_refuses_surface_host_activation_and_lease_changes`
- `native_geometry_coalescing_does_not_renew_an_unacknowledged_placement_deadline`
- `native_geometry_queued_after_grant_waits_for_its_exact_receipt_before_the_next_placement`
- `cpu_painter_clips_host_hole_but_still_uploads_frees_and_covers_old_pixels`
- `host_paint_regions_cover_popup_union_once_and_skip_only_the_inward_world_hole`
- `fitted_workers_keep_character_world_and_toolbar_geometry_after_detach`

After the host-repair changes above, the list has **1,094 required names**.
The second removed name was absent from the original list. Add the following
29 MCP unit cases and five viewer-adapter cases:

- `authority::tests::approval_expires_without_grant_and_capacity_is_bounded`
- `authority::tests::ask_remains_pending_until_real_approval_then_delivers_exactly_once`
- `authority::tests::client_scoped_refs_and_approval_results_cannot_be_reused_by_another_peer`
- `authority::tests::disabled_by_default_and_independent_host_profile_metadata_action_authority`
- `authority::tests::every_binding_component_and_stable_slot_replacement_require_new_authorization`
- `authority::tests::final_delivery_checks_freshness_and_provider_gaps_require_new_opt_in`
- `authority::tests::invalid_projections_do_not_refresh_freshness_or_expose_partial_state`
- `authority::tests::list_filters_unauthorized_profiles_and_projection_never_exports_bindings`
- `authority::tests::missing_native_permission_omits_names_and_later_privacy_revocation_omits_them`
- `authority::tests::profile_ask_cannot_be_overridden_by_global_allow_or_action_allow`
- `authority::tests::repeated_lists_reuse_bounded_refs_and_disconnect_retires_every_operation`
- `authority::tests::revoked_pending_or_approved_read_never_retains_payload`
- `authority::tests::shutdown_invalidates_approvals_and_old_refs_even_after_reenablement`
- `authority::tests::stopped_profile_grant_does_not_authorize_next_launch_or_login`
- `ipc::tests::actual_transport_discovery_persistent_calls_and_owned_cleanup`
- `ipc::tests::framing_serialization_stops_at_bound`
- `ipc::tests::idle_connection_survives_frame_deadline_for_human_approval_polling`
- `ipc::tests::malformed_and_oversized_frames_disconnect_without_dispatch`
- `ipc::tests::only_four_connections_are_admitted_until_a_slot_closes`
- `ipc::tests::service_shutdown_cancels_connected_clients`
- `ipc::tests::truncated_disconnect_does_not_dispatch_and_listener_survives`
- `ipc::tests::wrong_secret_and_stale_incarnation_are_refused`
- `sdk::tests::admission_precedes_sdk_task_creation_and_duplicate_ids_close`
- `sdk::tests::cancellation_retires_connection_instead_of_leaking_sdk_dropped_response`
- `sdk::tests::handshake_versions_exclude_initialization_free_protocol`
- `sdk::tests::missing_descriptor_or_closed_transport_never_delivers_queued_placeholder`
- `sdk::tests::only_closed_read_tool_arguments_are_accepted`
- `sdk::tests::output_bound_is_enforced_during_serialization`
- `sdk::tests::unadvertised_notification_flood_never_reaches_sdk_tasks`
- `session_host::mcp::tests::host_mcp_broker_projection_defaults_off_and_omits_raw_account_grid_names`
- `session_host::mcp::tests::host_mcp_closing_stopped_and_registry_failure_retire_publication`
- `session_host::mcp::tests::host_mcp_invalid_broker_projection_revokes_prior_authority_immediately`
- `session_host::mcp::tests::host_mcp_login_replacement_and_unreachable_source_require_new_grants`
- `session_host::mcp::tests::host_mcp_native_display_requires_exact_current_binding_and_fresh_permission`

Final gate: **1,128 required names**. Match every requirement to exactly one
passing full-suite row using the previous exact-full-name/unique-identifier
method; report missing/nonpassing/duplicate rows. Preserve all other requirements,
including the 12 former non-Profile-Copy failures, long-path publication, private
Monitor retirement, slot isolation, Contacts and Chat. Do not run focused filters
merely to produce duplicate coverage. Export all unfiltered names and summaries.
The Linux-only mode/symlink test and two Linux-only stdio subprocess tests are
not Windows requirements; record their absence by source cfg, not as failures
silently filtered out. All Windows-enabled MCP tests must run in the full suite.
Runtime totals are measured results, not inferred from definition counts.

Cloud combined checks: 148 focused Cargo passes (32 MCP, 103 session, 13 i18n),
plus five actual-source adapter component passes; zero failed/ignored. Scoped
viewer/MCP all-target Clippy passed with the existing Linux dead-code allowance,
and formatting passed. The production viewer adapter compiles in that check.
Full Linux link is resource-blocked; Windows tests and packaging are unverified.
Foundation source is not a functional host MCP feature: no native enablement,
profile opt-in, privacy-display or approval UI, character connection or action
tool is implemented. Test-only approvals must never become a production bypass.

## Owned offline/native smoke

MCP transport unit fixtures must remain isolated; do not point the sidecar at
real saved-account discovery or change ACLs/settings to make a fixture pass.
The viewer must start with no new MCP listener/discovery descriptor.

Use the existing isolated development test-root/demo workflow and automation
border/title, `AURORA_HOST_DIAGNOSTICS=1`, default theme and Vulkan. Interact
only with owned PIDs/windows and synthetic data. Do not select saved accounts
or log in. If an actual hosted synthetic world/Monitor cannot be established
with existing fixtures, report that case NOT RUN and leave it to Sodie; do not
fabricate native acceptance from component images or relax source checks.

- Fresh Open monitor must show from its first guarded frame, including with no
  cached preview image. Test close/reopen and minimize/reopen. Verify sender
  identity and fresh authority; expired/replaced source must not reveal pixels.
- Drag an active host continuously, then resize, snap, minimize/restore and
  toggle management. Test negative desktop origins and mixed DPI if available.
  Position-only moves preserve the same renderer/worker, with no repeated
  fit/hide/show/focus. Resizes finish at the latest rectangle. No orphaned or
  inaccessible worker may remain after detach/host close.
- Host controls, settings, character chooser and Shared Chat remain responsive
  at small, maximized and spanning sizes. Record paint/input/geometry timings;
  ordinary control response target is under100ms in the recorded workload.
- Hosted left-half/right-half/asymmetric world margins apply to the image,
  picking and mouselook. Restrict-controls determines toolbar/photo placement;
  ordinary panels use spare client space. Test workspace apply/preview/cancel
  and confirm standalone placement is preserved after detach.
- Preserve safe different-character activation, held-key/PTT release, exact
  current placement, no focus theft during geometry, and one world render owner.

Capture affected compact host/Monitor views using existing CPU fixtures where
applicable, and distinguish those images from native desktop acceptance. Grid
switching/chat/restriction acceptance is Sodie’s. Driver-reset recovery, XR,
Linux terrain and prior Japanese fixture glyph limitations remain unresolved
unless specifically verified. Do not induce GPU loss.

Return exact source/tree/clean state; commands, exit codes, test totals and all
failure names/logs; requirement audit; viewer/sidecar versions; package/EXE/sidecar paths, bytes and
SHA256; ZIP/EXE identity check; per-native-case observations/timings/screenshots
and NOT RUN cases; and preservation result. No public raw-log/evidence upload.
A passing build is not native or human acceptance. Report failures as failures.
