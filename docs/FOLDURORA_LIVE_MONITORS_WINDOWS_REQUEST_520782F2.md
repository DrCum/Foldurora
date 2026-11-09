# Windows request — live inactive Monitor checkpoint

Queue ONE run through Slick to the EXISTING Foldurora Windows Builder on
Cumputer, thread01a1af77-a465-71c3-be08-cd9d1b927f83, AFTER the active radio run
finishes. FIRST check its current history and whether this request was already
received. Do not interrupt, replace, duplicate or rerun the radio candidate.
Preparation/publication is not acknowledgment, execution or a package result.

Repository: https://github.com/DrCum/Foldurora.git
Branch: `feat/live-inactive-monitors-20261109`, draft PR46
https://github.com/DrCum/Foldurora/pull/46
Commit: `520782f2998150a26c74c51e18eccc7c4994c737`
Tree: `bcded0890dbdb359d44db28c399956fa2fb1ccd5`
Canonical Git-LF Cargo.lock SHA256:
`e04f6814529802ba6469b9455294aabfcfbec989d0e271698b3723a04d682e32`
Use this SOURCE, not a later documentation commit. Reuse the owned
camera-6a3009f-windows worktree, existing DEV caches and tooling. No new cache,
clean, dependency/tool upgrade, Windows ACL/security/long-path-policy change,
source repair, credentials, grid login, install, merge or release. Preserve the
radio/98b/a903 artifacts, prior failures, profiles, logs/screenshots, worktrees
and branch heads. Builder fixes return to lead for integration and revalidation.

## Source and actual evidence

Stacks on frozen radio47955237. The normal viewer dependency now explicitly
includes aurora-render/monitor-prototype; this is the production caller join,
not another disabled-renderer prototype package. Do not strip that dependency
feature or build with a changed manifest. Sidecar workspace Apply/Previous stay
unsupported; native consent and all source/account/privacy/permission fences hold.

Open visible native Standby demand can render never-activated characters at
fitted small dimensions. Source and physical geometry, kind-specific V5 claims,
credited asynchronous export, reduced cold resources, per-process allocation
leases across renderer replacement, aggregate reservations, and actual GPU/
backend retirement before Stop ACK are joined. Saved settings/window geometry
and active/input proofs are separate. See pinned source documentation:
https://github.com/DrCum/Foldurora/blob/520782f2998150a26c74c51e18eccc7c4994c737/docs/FOLDURORA_LIVE_MONITORS_20261109.md

Formatting and qualified aggregate all-target Linux ci Clippy passed (existing
Linux platform dead-code allowance; Windows MUST be strict). Actual source
components:33 policy/wire cases and16 capture/cadence/lifetime cases passed;
16 includes10 capture-queue cases, not an additional independent10. Native App/
World/IO/GPU tests were compiled, not run. Fixed cloud disk prevents full Linux
linking; previous terrain/ownership and ENOSPC evidence remain unresolved.
No test definitions were removed. The previously optional Monitor tests become
part of this candidate's default workspace run. No Windows/GPU/package pass is
claimed for this source. Preserve earlier negative evidence and qualified passes.

Known bounds/limitations:512MiB host view credit,128MiB per configured source,
at most four previews, separate from existing image pools; pending/uncertain
charges remain. Driver headroom must be known (cold256MiB/subsequent64MiB), else
samples refuse. An uncertain source/metadata reservation can require worker exit,
including after privacy/teleport source closure; automatic recovery is unfinished.
Older V1-V4 image peers fail closed. Driver-reset/XR/native hosted acceptance stay
open. Do not relax any boundary or use a full-window/photo fallback for a pass.

## Sequential gates

Record exact source, commands, child environment and exits/stdout/stderr. Verify
clean checkout and canonical Git-LF lock (account for CRLF without altering Git
settings). A failure stops downstream stages; return it without waivers, source
repairs or unchanged reruns. Preserve old source/artifacts before checkout.

```powershell
if ((git rev-parse HEAD).Trim() -ne '520782f2998150a26c74c51e18eccc7c4994c737') { throw 'Wrong source' }
if ((git rev-parse 'HEAD^{tree}').Trim() -ne 'bcded0890dbdb359d44db28c399956fa2fb1ccd5') { throw 'Wrong tree' }
git status --short
powershell.exe -NoProfile -ExecutionPolicy Bypass -File ./scripts/check-scripts.ps1
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked --no-fail-fast
```

For that ONE full suite use child-only AURORA_RENDER_THREAD=1 and
AURORA_GPU_VALIDATION=1, preserving inherited values for restoration. The real
small-world/retirement fixture now runs unfiltered. No separate early MCP gate,
old fixed filters or repeated Monitor/render-thread/session-prototype matrix.
Record actual adapter/driver only if identified by that invocation; otherwise
unobserved. Do not borrow identity from a later standalone startup.

The coverage audit requires **1413** exactly-one passing outcomes from this full
run alone: retain the radio request's1390 declarations (98b1366 +24 public source
identifiers), then add the23 below. Audit declaration sets, never reuse old passing
rows. If the radio run failed, its declaration list is still input, not a pass.
The existing Builder already holds the98b baseline/private audits and the complete
radio request. No new private packet is required. Reconcile exact full names or
unique leaf identifiers with source, fail missing/duplicate/nonpassing cases, and
retain raw results privately. All full-suite failures still block packaging.

Radio declaration reference:
https://github.com/DrCum/Foldurora/blob/50dc448fa139fd4c01a1c2058a1abfc72f285728/docs/FOLDURORA_RADIO_WINDOWS_REQUEST_47955237.md
- `live_monitor_gpu_reservation_survives_visibility_eof_and_unacknowledged_stop_until_native_release`
- `live_native_pending_rechecks_exact_source_boundary_lease_and_deadline_after_sampling`
- `live_native_policy_never_activated_source_requires_exact_independent_inactive_authority`
- `live_native_policy_rejects_missing_expired_foreign_or_physical_mismatch_and_renews_without_capture_churn`
- `live_preview_before_first_activation_has_no_fabricated_active_grant`
- `live_preview_cannot_relabel_an_active_claim_or_mismatched_stamp`
- `live_preview_captured_body_refuses_an_active_queue_result_even_when_every_geometry_field_matches`
- `live_preview_io_inactive_guard_cannot_borrow_active_authority_or_revive_after_retirement`
- `live_preview_needs_explicit_inactive_eligibility_not_retained_history`
- `live_preview_revocation_cancels_pending_and_retires_borrowed_pixels_on_privacy_loss`
- `live_preview_stream_never_active_policy_roundtrips_fragmented_authenticated_pixels_then_revokes_them`
- `monitor_borrowed_queue_handle_never_closes_owner_but_cannot_outlive_its_authority`
- `monitor_busy_worker_does_not_prepare_scene_or_consume_uploads`
- `monitor_cadences_drop_missed_ticks_without_catchup`
- `monitor_cancelled_before_submission_and_after_callback_never_delivers`
- `monitor_character_lifetimes_and_close_are_isolated`
- `monitor_hidden_minimized_closed_and_promotion_keep_drain_reservation`
- `monitor_process_resource_lease_survives_cancellation_replacement_and_borrowed_callback_lifetimes`
- `monitor_reservation_under_contention_does_not_prepare_or_consume_previous_result`
- `monitor_resize_invalidates_old_frames_and_does_not_accelerate_rate`
- `monitor_small_gpu_targets_are_persistent_bounded_and_have_no_readback_usage`
- `monitor_world_packets_keep_surface_size_and_never_prove_presentation`
- `viewport_capture_inactive_origin_and_original_crop_survive_the_actual_map_queue`

Ten of these are preserved formerly nondefault Monitor tests, thirteen are new
default regressions. The source has no removed tests. Existing platform exclusions
(Unix IPC/upload handles/Linux SDK stdio) remain; unsupported alternate MCP
workspace backend cases are still unintegrated/unpassed. Existing real-network
and hardware probes remain ignored. Do not run public radio/station probes.

After the full suite passes, run ONE new explicit ignored GPU export gate with
the same child render-thread/GPU-validation settings:

```powershell
cargo test --workspace --locked viewport_gpu_live_offscreen_export_keeps_native_crop_identity_full_texture_uv_and_opaque_color -- --ignored --nocapture
```

Require exactly one passing matched test, no failure. It tests actual small GPU
textures through the capture owner: original offset native rectangle metadata,
full small-texture UVs, all four RGBA/BGRA linear/sRGB formats, opaque channels,
ordinary-frame refusal and publication revocation. Its synthetic texture/source
fixture does not establish native window/source authority. It is ignored in the
full run, then separately executed; do not mix its row into the1413 audit.
Preserve actual diagnostics; no test skip, validation suppression or policy change.

## Build/package after all gates

```powershell
cargo build --locked -p aurora-viewer -p aurora-mcp --features aurora-viewer/xr-static-loader
.\target\debug\aurora-viewer.exe --version
.\target\debug\foldurora-mcp.exe --version
.\target\debug\foldurora-mcp.exe --help
```

The ordinary build now INCLUDES the Monitor feature through its dependency.
Require the viewer's version stamp to match520782f29, including staged and ZIP
executables. Refuse a cached other-worktree stamp. Preserve a mismatch and use
only the existing normal Cargo version-regeneration procedure if needed; no
source/version override or broad clean. Stop if identity cannot be established.

Package with the existing script into a new owned destination, for example:

```powershell
.\scripts\package.ps1 -Exe .\target\debug\aurora-viewer.exe -Out '<owned Builds>\Foldurora-520782f29981\Foldurora-520782f29981-windows-x64'
Compress-Archive -Path '<owned package>\*' -DestinationPath '<owned package>.zip'
Get-FileHash -Algorithm SHA256 '<owned package>.zip'
```

Resolve placeholders to the existing owned Builds directory and report actual
commands/paths. Include viewer, sidecar, manifests, FOLDURORA_MCP.md and existing
license/notices. Verify built/staged/ZIP-entry SHA256 equality for both executables,
exact source and lock identity, versions, ZIP length/hash and manifest contents.

## Offline/native scope and return

Run one isolated standalone default-theme Vulkan demo startup from the verified
package using the existing owned AURORA_UPDATE_TEST_ROOT/AURORA_DEMO fixture,
owned title/automation border/PIDs and frame650 exit procedure. Preserve inherited
environment and prior fixtures. Inspect the frame/log for startup/exit0, record
actual adapter/backend/driver/DPI, and perform existing passive unexpected MCP
pipe/descriptor checks. This is startup only, not hosted live Monitor acceptance.
Do not invent an online/source grant or use credentials to make an offline case
appear native. No unrelated CPU/DPI/localization screenshot matrix is requested.

If the existing OFFLINE native host fixture cannot actually prove Ready/Standby
source authority, mark hosted sampling NOT RUN. Do not change its phase/authority
for a screenshot. Grid login and interactive acceptance belong to Sodie.

Human checklist for later:
1. Open a Monitor for a logged-in inactive character that has never been active;
   verify sampled world/caption/age and chat, then change size/rate, hide/reopen.
2. Switch/promote/exchange, move/resize the host and test custom viewport/picking/
   mouselook. Old frames must stop on source change; Monitor never grants input.
3. Close/reopen and exercise privacy/teleport boundaries: immediate refusal and
   old image retirement; record restart-required uncertain recovery honestly.
4. Verify headroom denial/ordinary recovery, responsiveness and multiple Monitor
   admission. Do not force GPU reset. Driver-reset and XR acceptance remain open.

Return a sanitized GitHub-accessible report with exact source/tree/lock, actual
commands/exits/configuration, full totals/ignored/failing names,1413-case full-run
audit, separate GPU-gate result, logs and actual adapter identities/unobserved facts;
versions, package path/bytes/SHA256 and executable hash verification; startup/native
observations and NOT RUN cases; source/worktree/artifact/cache preservation.
Raw logs/private audit input/screenshots stay local/private. A package success is
not native hosted, real-grid/RLVa, GPU-reset or XR acceptance. No merge/install/
release. Keep candidate source frozen and report completion through Slick.
