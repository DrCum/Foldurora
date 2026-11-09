# Windows request — combined Advanced Workspace images

Prepared for ONE relay through Slick to the existing Foldurora Windows Builder
on Cumputer, thread `01a1af77-a465-71c3-be08-cd9d1b927f83`. The a903 run completed
successfully and Builder was last verified idle. Check current request history
before delivery; do not duplicate an existing/running request. The lead's direct
status connection is closed. Publishing this document does not establish delivery,
acknowledgment or execution. No replacement Builder or unchanged-source rerun.

Repository: https://github.com/DrCum/Foldurora.git
Branch: `feat/workspace-local-backgrounds-20261109`
Commit: `da8ee12fd417ecb62eac22d40cb4b91d8da94e86`
Tree: `497c1f4f63314fd68630d1e81204fafae981b802`
Cargo.lock Git-LF SHA256: `27c0735eab8d015661ef42be7652e6561928aa9e48e570d30e451e7dc7981497`
PR35: https://github.com/DrCum/Foldurora/pull/35, stacked on PR34/31/30/29.
Source and remote tree are verified/frozen. A documentation commit containing
this request is NOT the build source. Use matching host/worker binaries.

## Scope and prior evidence

This closes the source group for additional native Inventory window capture,
geometry-only display background targets, six-mode Inventory image backgrounds
and bounded private local-image import. Existing HUD review, expanded loaded
folders, graphics/camera/startup/favorites, native source and security safeguards
remain. Local images use immutable account/grid IDs with normalized copies at
most256px, not live external paths or grid uploads. Quota is128 images/32MiB,
without automatic GC. Folder/graphics bounded limitations remain documented.

A903 Windows source a903e16c01db412b0a7fd77886851be3145e1852 passed parser/format/
strict lint, one full8970/0/58 suite, all1184 required names once, separate renderer
prototype gates and ordinary viewer/sidecar packaging. Its isolated Vulkan startup
passed; hosted/login-bound workflows were NOT RUN. This new source is outside
that package. Preserve all older failed evidence and known-good artifacts.
https://github.com/DrCum/Foldurora/blob/03a6024fb0b96827c8eb277168b0e3e84e52ff7f/docs/FOLDURORA_WINDOWS_VALIDATION_A903E16C.md

New source slices pass qualified Linux viewer all-target ci Clippy, formatting,
13 localization cases, unchanged appearance scan and bounded actual-source model/
geometry components. Nine genuine new local-image cases compile but remain unrun;
others from PR30/31/34 also await the combined Windows gate. Isolated codec linking
was blocked by existing image/moxcms E0460. Full Linux viewer linking remains
resource blocked; historical software-GPU terrain remains unresolved. No full
Linux pass or new Windows/native acceptance is claimed.

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
unchanged to hope it clears. Run ONE unfiltered full suite for this related group:

```powershell
if ((git rev-parse HEAD).Trim() -ne 'da8ee12fd417ecb62eac22d40cb4b91d8da94e86') { throw 'Wrong source commit' }
if ((git rev-parse 'HEAD^{tree}').Trim() -ne '497c1f4f63314fd68630d1e81204fafae981b802') { throw 'Wrong source tree' }
git status --short
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\check-scripts.ps1
cargo fmt --all -- --check
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

After the full gate succeeds, build/package ordinary viewer+sidecar WITHOUT
monitor-prototype, using the existing packaging and ZIP verification tooling:

```powershell
cargo build --locked -p aurora-viewer -p aurora-mcp --features aurora-viewer/xr-static-loader
.\target\debug\aurora-viewer.exe --version
.\target\debug\foldurora-mcp.exe --version
.\target\debug\foldurora-mcp.exe --help
.\scripts\package.ps1 -Exe .\target\debug\aurora-viewer.exe -McpExe .\target\debug\foldurora-mcp.exe -Out 'C:\Users\Marty\..Projects\Aurora Viewer\Builds\Foldurora-da8ee12fd417\Foldurora-da8ee12fd417-windows-x64'
```

Viewer must identify da8ee12fd, sidecar0.1.0. Preserve any cached-version mismatch;
use only normal Cargo version regeneration, no overrides/clean. Verify built,
staged and extracted viewer/sidecar hashes match, manifests and required MCP/
licensing documents are included. Return executable versions, package/ZIP path,
byte sizes, SHA256 and verification outcomes. No listener/discovery is expected
from ordinary default startup. A903's prototype code is unchanged in this source;
retain its separate exact-source evidence rather than repeat its unrelated GPU
matrix. No production inactive-rendering or functional MCP enablement is claimed.

## Required full-run coverage —1221 public-source cases

Retain the1184 requirements audited once in a903, add these37 public source test
identifiers, and source-bind the derived audit to this SHA/tree. Match exact full
names or unique final identifiers, exactly one PASS each from THIS full run alone,
zero missing/nonpassing/duplicates. Earlier components or filtered rows cannot
substitute. Coverage inputs and identifying metadata stay private with Builder;
if unavailable, report through Slick without substituting a weaker gate. Do not
publish inputs, metadata, raw rows or logs. New source additions:

- `native_workspace_additional_capture_emits_only_a_current_typed_source_request`
- `native_workspace_display_background_edits_are_typed_and_topology_change_retires_the_target`
- `native_workspace_image_chooser_emits_captured_display_target_without_mutating_private_background`
- `native_workspace_images_corrupt_library_is_not_repaired_or_overwritten_by_an_import`
- `native_workspace_images_kind_keeps_fixed_identity_paths_and_all_legacy_limits`
- `native_workspace_images_missing_load_and_cancelled_publication_never_create_a_store`
- `native_workspace_images_publish_load_exact_pixels_and_refuse_identity_or_immutable_id_replacement`
- `native_workspace_local_import_busy_control_emits_only_one_current_source_request`
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
