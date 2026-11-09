# Windows request — advanced workspace contexts and MCP descriptor regression

Prepared for ONE relay through Slick to the EXISTING Foldurora Windows Builder
on Cumputer, thread01a1af77-a465-71c3-be08-cd9d1b927f83. The5f990 run is complete
and the Builder was last confirmed idle. Check current history/status first;
if this exact request is already delivered/running, do not duplicate it. No new
Builder, old-source rerun or source change in the build worktree is requested.
Delivery/acknowledgment/start is not claimed merely by publishing this document.

Repository: https://github.com/DrCum/Foldurora.git
Branch: `fix/mcp-descriptor-regression-20261109`
Commit: `a903e16c01db412b0a7fd77886851be3145e1852`
Tree: `a338a4b88a8a4e660efa59ad71d18878dc603e91`
Cargo.lock SHA256: `27c0735eab8d015661ef42be7652e6561928aa9e48e570d30e451e7dc7981497`
PR29: https://github.com/DrCum/Foldurora/pull/29 (stacked after28/27/25/24/23).
Source/remote identity verified and frozen. A documentation commit containing
this request is NOT the build source. Host/worker protocolv2 requires matching
executables; never mix old f08 workers.

## Evidence and correction

5f990 remains FAILED:8935passed/1failed/58ignored, zero filtered,37 summaries;
1149/1150 required passes. Parser/format/strict Clippy and theme guard passed.
MCP `ipc::tests::wrong_secret_and_stale_incarnation_are_refused` hit Windows
sharing violation32 at synthetic descriptor std::fs::write, AFTER wrong-secret
refusal and pipe connection. Later stale/preservation assertions did NOT run.
The actual conflicting handle was not identified; read/delete/no-write reader
sharing is consistent with the failure, not a proven identification. Original
report/logs/artifacts stay preserved:
https://github.com/DrCum/Foldurora/blob/2cbeca99372f6ab1c8127b21399cd8479e8e0584/docs/FOLDURORA_WINDOWS_VALIDATION_5F99030.md
Monitor feature gates, builds, versions, packaging, CPU/native smoke were NOT RUN.

Only the MCP test mutation changes: private create/sync/close/atomic replacement
in the live case; every original authentication/refusal/preservation assertion
remains. New deterministic cases cover an open original reader over replacement
(Windows must also refuse in-place write with32), and SAME-file-identity changed
contents surviving original Publication cleanup. Production sharing, ACL/owner,
link/reparse, transport, authentication and functional MCP behavior are unchanged.
Coordination notice: https://github.com/DrCum/Foldurora/pull/28#issuecomment-6739054196 .
Owner acknowledgment is pending; no competing functional MCP work is included.

The combined source also adds solid padding colors, stable-record Update,
explicitly reviewed additive HUD restore, bounded fetched main-tree expansion
and four-Hz optional capture/modified sampling. New HUD/expanded contexts compare
the original native grid namespace. Preview never wears/navigates; restrictions
and current source win. Additional Inventory windows, image backgrounds, live
inactive Monitor integration and functional MCP remain unfinished.

Linux evidence: MCP package34/0/0 (32unit+2 Linux-only stdio), strict MCP Clippy0;
viewer parent e2fc qualified all-target Clippy0 with existing Linux dead-code
allowance, viewer code unchanged by this test-only correction. Source components,
localization and genuine compiled-but-unrun cases are distinguished in source
feature docs. These are not a full workspace/Windows/native pass. Full Linux
viewer linking remains resource blocked; terrain/software-GPU is unresolved.

## Owned worktree, commands and stop gates

Reuse the clean owned camera-6a3009f-windows worktree, existing tooling and
persistent DEV-profile graph used for5f990. Preserve all prior worktrees/heads,
packages, profiles, test binaries and failed evidence. No routine clean, new
cache/profile, dependency/tool upgrade, Windows ACL/security/long-path policy
change, source repair, install, merge, release, credentials or grid login.
Return needed source fixes to the lead. Record exact commands, actual exit codes,
stdout/stderr, tool versions and source/clean state. A failure stops downstream
build/package/smoke; do not waive it or rerun unchanged to hope it disappears.
Capture the launch parent token/integrity/TEMP and child overrides for this run;
label unobserved child facts as unobserved, never substitute an old/different token.
Keep raw context/logs/checklists PRIVATE; sanitized source/result summaries only.

```powershell
if ((git rev-parse HEAD).Trim() -ne 'a903e16c01db412b0a7fd77886851be3145e1852') { throw 'Wrong source commit' }
if ((git rev-parse 'HEAD^{tree}').Trim() -ne 'a338a4b88a8a4e660efa59ad71d18878dc603e91') { throw 'Wrong source tree' }
git status --short
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\check-scripts.ps1
cargo fmt --all -- --check
cargo clippy --locked -p aurora-mcp --all-targets -- -D warnings
cargo test --locked -p aurora-mcp --no-fail-fast
```

This early, small MCP package gate targets the Windows-specific failure before
paying for the full viewer group. Verify the original named case and BOTH new
cases below each ran/passed in this stage. The held-reader case must execute its
Windows error32 branch. Package-selected feature resolution is not the full
workspace graph. Both Linux-only IPC guards and Linux-only stdio remain unchanged;
report Windows's actual totals rather than treating those absent cases as passes.

After early gates pass, run ONE unfiltered combined full workspace suite:

```powershell
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked --no-fail-fast
```

Coverage audit uses ONLY this full-run raw result set, not early filtered/package
runs. Preserve all failure names/panics, all summaries and zero filtered. Do not
infer total passes from source. Every full-suite case must pass in addition to
all required-name matches. No viewer/sidecar/package if any full case fails.

Then run the previously UNRUN nondefault renderer gates, with process-scoped
AURORA_RENDER_THREAD=1 and AURORA_GPU_VALIDATION=1; preserve/restore inherited
values. Only owned synthetic windows/devices, no grid or deliberate GPU loss.
Filters overlap: report actual names/results, not an added unique total. The
Monitor filter has10 expected author cases; any mismatch/failure is evidence,
not permission to adjust it. These tests do not enable production live rendering.

```powershell
cargo clippy --locked -p aurora-render --features monitor-prototype --all-targets -- -D warnings
cargo test --locked -p aurora-render --features monitor-prototype --lib monitor -- --nocapture
cargo test --locked -p aurora-render --features monitor-prototype --lib render_thread::tests -- --nocapture
cargo test --locked -p aurora-render --features monitor-prototype --lib session_fence::tests -- --nocapture
cargo test --locked -p aurora-session preview_ -- --nocapture
```

Build/package the ordinary viewer and sidecar WITHOUT monitor-prototype:

```powershell
cargo build --locked -p aurora-viewer -p aurora-mcp --features aurora-viewer/xr-static-loader
.\target\debug\aurora-viewer.exe --version
.\target\debug\foldurora-mcp.exe --version
.\target\debug\foldurora-mcp.exe --help
.\scripts\package.ps1 -Exe .\target\debug\aurora-viewer.exe -McpExe .\target\debug\foldurora-mcp.exe -Out 'C:\Users\Marty\..Projects\Aurora Viewer\Builds\Foldurora-a903e16c01db\Foldurora-a903e16c01db-windows-x64'
```

Use existing ZIP/manifest verification and packaging tooling. Viewer version must
identify a903e16c0, not an old cached worktree. Preserve a mismatch and use only
the proven normal Cargo version refresh, no overrides/clean. Sidecar should report
foldurora-mcp0.1.0; verify FOLDURORA_MCP.md and NOTICE-MCP.txt are packaged. Return
EXE/sidecar/ZIP bytes and SHA256, manifest and extracted hashes (must match).
No production listener/discovery should appear from ordinary default startup.

## Source-bound full-run audit —1184 requirements

Apply the Builder's existing source-bound coverage procedure to this candidate.
Retain the1150 requirements audited in the completed5f990 run and add the34
PUBLIC SOURCE additions below, yielding1184. The original failed
wrong_secret_and_stale_incarnation_are_refused remains required, together with
all unrelated private-store/long-path/slot/archive/Contacts/restriction cases.
Coverage inputs and their identifying metadata remain with the Builder; none
are supplied or published here. If those inputs are unavailable, report that
specific blocker through Slick without substituting a weaker list. Do not
publish the inputs, their metadata, or raw test evidence. No source guard or
security assertion is waived by the coverage procedure.

- `descriptor_atomic_replacement_preserves_open_reader_and_validates_private_successor`
- `native_workspace_hud_review_buttons_cannot_retarget_a_replaced_nonce`
- `native_workspace_update_is_a_source_bound_request_without_mutating_the_painted_record`
- `original_publication_cleanup_preserves_changed_descriptor_with_the_same_file_identity`
- `workspace_background_choices_roundtrip_without_asset_or_path_fields`
- `workspace_background_full_or_unknown_world_never_fills_over_scene`
- `workspace_background_padding_partitions_canvas_without_world_or_corner_overlap`
- `workspace_background_pending_choice_requires_exact_binding_and_account`
- `workspace_background_preview_keeps_committed_choice_and_source_loss_hides_it`
- `workspace_background_previous_keeps_scoped_target_instead_of_old_record_lookup`
- `workspace_background_unknown_kind_extra_fields_and_invalid_channels_are_refused`
- `workspace_expanded_actual_archive_mask_and_fetched_ancestry_gate_capture_and_restore`
- `workspace_expanded_folder_references_are_bounded_unique_and_canonical`
- `workspace_expanded_folder_schema_accepts_only_uuid_references_and_explicit_empty`
- `workspace_expanded_native_apply_touches_only_qualified_scope_and_preserves_edit_selection`
- `workspace_expanded_native_owner_refuses_search_other_windows_and_overflow`
- `workspace_expanded_scope_distinguishes_absent_and_empty_and_keeps_old_layouts_readable`
- `workspace_hud_catalogue_is_opt_in_private_and_rejects_duplicate_item_references`
- `workspace_hud_lane_has_one_deadline_and_does_not_retry_failed_dispatch`
- `workspace_hud_lane_serializes_native_requests_and_requires_observed_wear`
- `workspace_hud_matching_keeps_extra_worn_items_and_never_changes_points`
- `workspace_hud_preview_baseline_and_previous_preserve_only_the_scoped_additive_set`
- `workspace_hud_projection_obeys_add_locks_and_hides_locked_worn_names_and_showinv`
- `workspace_hud_projection_refuses_foreign_link_trash_library_archived_and_unknown_items`
- `workspace_hud_projection_refuses_grid_mismatch_against_original_native_login_namespace`
- `workspace_hud_projection_uses_real_worn_item_and_native_point_without_asset_access`
- `workspace_hud_reference_schema_refuses_asset_payloads_unknown_fields_and_future_points`
- `workspace_hud_references_are_bounded_unique_original_items_at_hud_points`
- `workspace_hud_review_is_one_shot_and_scope_changes_retire_approval`
- `workspace_hud_runtime_inventory_outfit_generation_does_not_replay_or_drop_native_flight`
- `workspace_hud_runtime_retires_consent_on_focus_binding_rules_and_archive_publication`
- `workspace_last_capture_sampling_is_bounded_forceable_and_source_reset`
- `workspace_source_gap_reloads_presentation_without_replaying_consumed_startup`
- `workspace_update_keeps_id_name_favorites_and_startup_and_rejects_invalid_capture_atomically`

Match exact full names or unique final identifiers as in the existing audited
runner. Require one PASS each in the full unfiltered run, zero missing/duplicate/
nonpassing outcomes. Source-bind the derived private audit to this SHA/tree;
return its matching summary through Slick; keep the audit and raw rows private.
Feature-enabled renderer outcomes are a separate audit, not substituted default
coverage. The pure component passes do not waive the genuine World/UI cases.

## Offline/native tests and expected observations

Use existing isolated development AURORA_UPDATE_TEST_ROOT/demo fixtures, owned
PIDs, automation title/border, default glass/theme, Vulkan and host diagnostics.
No saved-account selection, user credentials, real-grid login or input to other
apps. Use existing fixtures only; if an actual source cannot be established,
report NOT RUN, never relax source/privacy gates or fabricate acceptance.

Existing f08 Friends/textbox CPU captures are preserved evidence of their
unchanged components, not current native acceptance. Do not repeat an unrelated
capture matrix for this group. The full suite still executes the textbox input
regression; an optional fresh capture must have a concrete changed-layout reason.

1. Native workspace manager in both styles at normal/narrow/short sizes: shared
   controls, scrolling and new HUD section fit/reach. Offline/unavailable/archive-
   MemoryOnly HUD or folder contexts must remain unavailable, not acquire native
   wear/persistence authority. No unexpected network/appearance action.
2. With an existing authoritative synthetic fixture ONLY, Save/Update preserves
   stable name/id/favorite/startup. Preview/Cancel/Previous and geometry-only
   replacement keep scoped graphics/camera/background values, unrelated settings
   and four-Hz advisory sampling. Solid padding fills only outside actual world,
   not photo/XR. Source gaps reload presentation without replaying startup.
   Missing actual native source means NOT RUN, not a new fake source bypass.
3. Fresh Open Monitor must show its first guarded frame; close/minimize/reopen,
   expired/replaced source privacy. Label remains LastActive, not LiveInactive.
   Host move/resize/snap/management should avoid hide/show churn, focus theft or
   orphan workers. Record responsiveness/timings (target under100ms for the
   tested workload), mixed DPI/negative origins if available, and unrun cases.
4. Hosted left/right/asymmetric viewport affects image/picking/mouselook and
   keeps constrained controls in the world while ordinary panels use spare
   space. Original switching/camera/Monitor, driver reset and XR stay open unless
   actually verified; do NOT induce device loss or substitute CPU captures.

Human/grid checklist remains Sodie’s: capture selected eligible HUDs, Preview
without wear, decline then approve missing items on different points, keep extra
worn items, retire future work on focus/source/resize/locks/manager close and
observe actual wear/timeout. Save opposite expanded ordinary loaded trees,
Switch/Previous/explicit-empty/old records, hidden/restricted/unknown ancestry,
search/explorer/other owners and overflow. No unauthorized names, fetches or
wears. Exercise per-account startup/favorites, Update and opposite backgrounds/
camera/graphics. Real grid/RLVa/driver-reset/XR acceptance is not delegated.

Return exact source/branch/tree/clean state and Cargo.lock hash; commands, exits,
totals, failure names/logs; full default/feature coverage audits; executable
versions; package/manifest paths, sizes/hashes/ZIP identity; CPU/native screenshots,
per-case observations/timings and NOT RUN stages; original launch-context limits;
preservation checks for prior artifacts/worktrees/heads. No source fixes in place,
public raw evidence, Windows security changes, merge, installation or release.
This is a combined checkpoint; overall porting/upstream work remains unfinished.
