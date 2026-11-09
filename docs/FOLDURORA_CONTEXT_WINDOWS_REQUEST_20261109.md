# Windows request — combined workspace contexts and themed-control correction

**Prepared, GitHub source published; delivery/acknowledgment/start not yet verified.**
Destination: the existing Foldurora Windows Builder on Cumputer, thread
`01a1af77-a465-71c3-be08-cd9d1b927f83`, through Slick. Slick reported the a611 run
complete and Builder idle. Verify its current request history before relaying
this request once. Do not duplicate a running/delivered request or replace the
Builder. No old d0d/a611/f08/bc20/48042 rerun is requested.

Repository: `https://github.com/DrCum/Foldurora.git`
Branch: `fix/workspace-theme-integration-20261109`
Commit: `5f99030a7daabb2879563c998828a2beb2f7099d`
Tree: `36cb2e1e0e0bec083d19ed993c90810b1be52dd2`
Review: https://github.com/DrCum/Foldurora/pull/23 (stacked after #22 and #20).
The source branch stays frozen. A documentation commit containing this request
is not the build target.

## Corrected failure and included feature group

The a611 Windows run passed parser/format/strict Clippy, but the unfiltered suite
failed: **8920 passed, 1 failed, 58 ignored**, zero filtered, 37 summaries. All1134
required cases passed; that does not waive the failure. Builds, versions,
packaging and smoke were NOT RUN. Preserve its exact negative evidence:
https://github.com/DrCum/Foldurora/blob/f901066de67e40a59caf518a8e411405bf796708/docs/FOLDURORA_WINDOWS_VALIDATION_A6119664.md

All four workspace checkboxes now use the existing shared themed control. No
guard, exception, test or security/permission rule changed. The unchanged full
source-tree appearance scan passes as a focused std-only component; its separate
real-egui scrollbar test still belongs to the full suite. Viewer all-target
Linux ci Clippy passes with the existing dead-code allowance. Source is compiled,
not full-suite/package/native accepted. Full Linux viewer linking remains
resource-blocked; the old software-GPU terrain issue stays unresolved.

This cumulative group also includes opt-in loaded-folder restoration (fresh
main Inventory/archive/RLVa gates), native graphics-quality snapshots and guarded
avatar-relative camera context, after startup/favorites. Preview/save projection
and Previous preserve their group scopes. Folder3, graphics-codec3 and camera3
actual-source component cases and13 localization cases passed in their slices;
component fixtures do not establish App/RLVa/IO/native behavior. Real test types
compile. All15 added default-suite cases below must run on Windows.

PR21's renderer foundation is included behind nondefault `monitor-prototype`.
It has no production App caller: ordinary viewer/host still uses LastActive,
not LiveInactive. Source/privacy/aggregate authority, inactive scene preparation,
async native export/freshness, cold-memory headroom and host promotion joins
remain unfinished. Check its feature-enabled API separately, never enable a live
account to simulate this missing integration. The functional MCP service remains
with its independent author; production enablement is still absent/disabled.

## Ownership and exact commands

Reuse the clean owned `camera-6a3009f-windows` and compatible persistent dev caches.
Preserve all previous source/worktrees, packages, profiles and failed evidence.
No routine clean, dependency/tool upgrade, Windows ACL/security/long-path change,
source fix in the frozen checkout, install, merge, release, credentials or grid
login. Return source fixes to the lead for integration and renewed validation.
Host/worker protocol v2 requires matching executables; do not mix f08 workers.

Verify exact commit/tree and tracked state before checks. Record every command,
actual exit code and stdout/stderr separately using existing runner tooling.
Stop and return failures before packaging; no filtering, waiver or token/policy
change to make a fixture pass. Run unfiltered default suite once; named coverage
comes from its raw rows, not repeated filtered runs.

```powershell
if ((git rev-parse HEAD).Trim() -ne '5f99030a7daabb2879563c998828a2beb2f7099d') { throw 'Wrong source commit' }
if ((git rev-parse 'HEAD^{tree}').Trim() -ne '36cb2e1e0e0bec083d19ed993c90810b1be52dd2') { throw 'Wrong source tree' }
git status --short
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\check-scripts.ps1
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked --no-fail-fast
```

Run these separate renderer-feature gates with `AURORA_RENDER_THREAD=1` and
`AURORA_GPU_VALIDATION=1` scoped to these test processes (restore prior environment
values afterwards). The owned test may create/show its own synthetic native
window; no grid, saved accounts or input to other applications. Retain adapter,
driver, failure logs and per-case outcomes. Existing author software-Vulkan
results are focused evidence, not this Windows result. The Monitor filter has10
expected cases including the rendezvous regression; report actual names/totals.
Filters overlap, so do not add them as unique coverage. Failure remains a gate.

```powershell
cargo clippy --locked -p aurora-render --features monitor-prototype --all-targets -- -D warnings
cargo test --locked -p aurora-render --features monitor-prototype --lib monitor -- --nocapture
cargo test --locked -p aurora-render --features monitor-prototype --lib render_thread::tests -- --nocapture
cargo test --locked -p aurora-render --features monitor-prototype --lib session_fence::tests -- --nocapture
cargo test --locked -p aurora-session preview_ -- --nocapture
```

Then build/package the ordinary viewer and sidecar, without the prototype feature:

```powershell
cargo build --locked -p aurora-viewer -p aurora-mcp --features aurora-viewer/xr-static-loader
.\target\debug\aurora-viewer.exe --version
.\target\debug\foldurora-mcp.exe --version
.\target\debug\foldurora-mcp.exe --help
.\scripts\package.ps1 -Exe .\target\debug\aurora-viewer.exe -McpExe .\target\debug\foldurora-mcp.exe -Out 'C:\Users\Marty\..Projects\Aurora Viewer\Builds\Foldurora-5f99030a7daa\Foldurora-5f99030a7daa-windows-x64'
```

Use existing ZIP/manifest verification. Verify EXE version identifies `5f99030a7`,
not an old cached worktree. Preserve any mismatch and use the proven normal Cargo
version-regeneration procedure, no override/clean. Return EXE/sidecar/ZIP bytes and
SHA256; ZIP-extracted hashes must match. Sidecar should identify foldurora-mcp0.1.0;
verify FOLDURORA_MCP.md and NOTICE-MCP.txt are packaged. No production MCP listener
or discovery descriptor should appear when the viewer starts.

## Exact source-bound coverage gate

Reuse the already saved private a611 `logs/combined-windows-requirements.json`:
103687bytes, SHA256 `911795508163054c0dbbd5e4f0308d98cdb3dd638ac0e20e76ed26ebd3dd8df7`.
Preserve its original bytes and source identity. It contains1134 names and already
correctly excludes Unix-only IPC and Linux-only stdio cases by actual source cfg.
Do not change those source guards. Derive a new source-bound list, add/deduplicate
these16 names, and require **1150** default-suite passing matches:

- `workspace_previous_only_restores_folder_when_the_switch_included_it`
- `workspace_graphics_preview_union_keeps_rollback_but_previous_uses_final_switch_scope`
- `workspace_camera_preview_union_rolls_back_and_previous_tracks_only_final_target`
- `workspace_folder_restore_is_optional_validated_and_scoped_to_its_record`
- `workspace_folder_references_roundtrip_inside_account_scoped_catalogue`
- `workspace_graphics_preview_save_projection_and_cancel_preserve_unrelated_preferences`
- `workspace_camera_preview_save_projection_does_not_persist_candidate_or_unrelated_preferences`
- `workspace_camera_snapshot_is_bounded_and_refuses_unknown_or_future_data`
- `workspace_camera_preferences_preserve_unrelated_mouselook_and_constraint_choices`
- `workspace_camera_comparison_tolerates_only_native_zoom_rounding`
- `workspace_graphics_snapshot_restores_only_native_quality_and_current_profile_identity`
- `workspace_graphics_snapshot_refuses_unknown_fields_oversize_and_future_versions`
- `workspace_graphics_codec_accepts_known_defaults_but_refuses_unknown_nested_fields`
- `workspace_graphics_codec_checks_every_array_element_and_shape`
- `workspace_graphics_codec_refuses_excessive_nesting`
- `ui::theme_guard::windows_take_their_appearance_from_the_theme`

Match exact full names or unique final identifiers as in the verified a611 audit.
Require exactly one passing outcome per requirement and report missing, duplicate
or nonpassing matches. All other full-suite tests must pass too: an audit is not
a waiver for a failing case. Retain all12 old unrelated Windows regressions,
long-path publication, shared-store/slot/Monitor isolation, chat and Contacts.
No private checklist/raw test logs are to be published. No large transfer is
needed; if the retained base file is missing or its hash differs, report that
precise blocker through Slick instead of substituting another list.

Export full unfiltered names and summaries, zero filtered; report actual totals,
not expected totals inferred from source. Prototype feature cases are audited
separately with their actual source guards. All existing Linux-only connection
capacity/security/stdio cases remain required on Linux, not silently waived.

## Owned offline/native smoke and remaining human acceptance

Use existing isolated development test-root/demo fixtures, automation title and
border, default theme, Vulkan and `AURORA_HOST_DIAGNOSTICS=1`. Interact only with
owned PIDs/windows and synthetic data. Do not select saved accounts or log in.
If current-source native workspaces cannot be established by an existing offline
fixture, report NOT RUN; do not alter its source guard to fabricate acceptance.

- Workspace manager at normal/narrow/short sizes in both styles: shared themed
  capture/favorites controls and scroll reachability. Favorite/rename/delete
  keeps stable identity; strip obeys viewport/native source. Default startup is
  off; named/last restores once per logical login and does not repeat on source
  gaps or parking/activation. Different account/grid never inherits a record.
- Save opposite graphics/camera contexts; Preview, Cancel, Previous and replace
  with a geometry-only record. Save unrelated FPS/mouselook preferences during
  Preview, restart: committed context wins, unrelated changes remain. Removed/
  changed named quality loses stale label while snapshot still applies. No
  frame-pacing, account, hardware or environment data is imported.
- Folder capture/restoration only in visible authorized main Inventory. Closed,
  archived, foreign, Library/trash, missing or restricted folders must not be
  opened or revealed; no deferred hidden navigation. Include root/item parent
  mapping and source change between paint and action where fixtures permit.
- Camera capture/target switch refuses free, object/alt-focus, mouselook or script
  control. New mode/authority during Preview cancels pending commit. Rollback
  respects new native locks, never changes camera mode or activates a worker.
  Actual script/grid restrictions remain Sodie's human checklist.
- Fresh Monitor Open must show from the first guarded frame without a cached
  image; close/minimize/reopen and expired/replaced source privacy. It still says
  LastActive; do not claim live inactive rendering.
- Host drag/resize/snap/minimize/management: no fit/hide/show loop or focus theft;
  latest placement wins. Check negative origins/mixed DPI if available, detached
  placement, no orphan/inaccessible worker, one actual world render owner and
  held-key/PTT release. Controls/chat/chooser response target under100ms in the
  measured workload; record timings, not assumed improvement.
- Hosted left/right/asymmetric world margins affect image/picking/mouselook;
  restrict-controls scopes toolbars/photo while ordinary panels use spare space.
  Keep original host switching/camera/Monitor acceptance open if source fixture
  is unavailable. CPU captures are component evidence, not default-glass/native
  acceptance. Native GPU reset recovery and XR remain open; do not induce loss.

Return exact source/tree/clean state; commands/exits/logs/totals/failure names;
source-bound default/feature audits; versions; manifest/package paths, sizes and
hashes/ZIP identity; per-case native observations/screenshots/timings and NOT RUN
cases; original token/TEMP context for any new security failure; preservation
result. No Windows security/ACL changes, source changes or public raw evidence.

Background/HUD workspace parity, live inactive transport, functional MCP and the
remaining AFK roadmap are still source work. No merge, installation or release
is authorized. This request is a combined checkpoint, not overall completion.
