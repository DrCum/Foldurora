# Session host native frame: investigation and implementation plan

Status: reviewed investigation/probe for the lead to implement. This change does
not repair the production viewer. Windows reproduction and the replacement
presentation path remain pending.

## Evidence and cause

The supplied screenshots identify Aurora Viewer
`0.9.3-beta.20261106.1+522.g98b30be2f (Dev)`. Investigation starts at the matching
public source, **98b30be2ff08da897b625a7014c00fa5d5bbbb03**, branch
`fix/workspace-windows-regressions-20261109`, rather than `main` (which does not
contain this host). The first screenshot shows an active hosted world with a
classic-looking caption, malformed caption controls and a thick square border.
The second, reported while dragging, shows the normal accented Windows frame
and the host's empty-session cover. Neither screenshot alone establishes a
driver fault or the exact Windows/DPI configuration.

The source explains that correlation:

| Path at the pinned source | Effect |
| --- | --- |
| `session_host/worker.rs`, `native::fit` and `native::stay_below_host` | Keep the independent worker immediately **below** the host. |
| `session_host/native.rs`, `Region::set` | Apply `SetWindowRgn` to the host's top-level HWND: full outer rectangle minus the acknowledged world, plus popup islands. |
| `session_host/mod.rs`, `window_event` | Move/resize/DPI events call `close_region`, invalidate geometry, then wait for current presentation acknowledgement. |
| `session_host/native.rs`, `Region::close` | Clear the custom region with `SetWindowRgn(hwnd, NULL, TRUE)`. |

Keeping the caption's pixels inside the region is insufficient. Microsoft's
[visual-style documentation](https://learn.microsoft.com/en-us/windows/win32/controls/cookbook-overview#when-visual-styles-are-not-applied)
explains that Windows treats a top-level window with a non-null region as a
specialized window and does not apply normal visual styles. Its
[Windows 11 corner documentation](https://learn.microsoft.com/en-us/windows/apps/desktop/modernize/ui/apply-rounded-corners)
also excludes windows using regions from normal corner rounding, including
manual opt-in. Thus the region explains both the change when hosting and the
restoration when geometry invalidation removes it. This is a source-backed
diagnosis; native confirmation on the reporting PC is still required, especially
for the exact corrupted button pixels.

`DWMNCRP_ENABLED` explicitly requests non-client rendering, but its
[documented contract](https://learn.microsoft.com/en-us/windows/win32/api/dwmapi/ne-dwmapi-dwmncrenderingpolicy)
does not promise normal corners for a shaped window. Repainting, forcing the
accent color, repeatedly resetting frame styles, or adding a rounded HRGN does
not remove the architectural cause. Do not call a policy-only experiment a
complete fix. Clearing the region alone also fails: the opaque host would hide
the worker beneath it.

## Small offline confirmation

Run [the probe](../scripts/probe-host-frame.ps1) in a fresh Windows 10 (1703+)/11
PowerShell process on the same local desktop/theme as the report:

```powershell
powershell.exe -NoProfile -STA -File scripts/probe-host-frame.ps1
```

1. Check that **Normal** already has the expected native frame. If it does not,
   this run cannot isolate the reported transition; record the Windows build,
   theme, DPI, remote/local desktop and baseline appearance first.
2. Click **Region**, leaving the window restored (neither snapped nor maximized).
   The hole exposes the desktop. Compare caption buttons, border and corners.
3. Click **Clear**. Check that the normal native frame returns. Repeat the cycle.
4. Optionally try **Forced DWM** and record whether it changes any symptom. A
   successful HRESULT or `NcRenderingEnabled=true` is not proof of correct pixels
   or normal corners. Clear again afterward.
5. Moving/resizing clears the probe region intentionally; click **Region** again
   after settling. Close to print JSON observations, which may be saved locally
   by redirecting stdout to a new file outside the source checkout.

The probe creates only its own WinForms window. It performs the same Win32
full-outer-rectangle-minus-client-hole operation, without winit, softbuffer,
Vulkan, a worker, grid login or profile access. It records immediate and delayed
native geometry, DPI, style flags, region type and DWM HRESULT/state; the delayed
sample allows a message-loop turn but is not a compositor/pixel fence. A
`GetWindowRgn` result of 0 means no application region **or** a query error, not
independent proof of successful restoration. API failures and an unthemed
baseline make a run inconclusive. The explicit policy result must be checked,
even in Normal/Clear. No screenshots or uploads are automatic. Take optional
captures against an innocuous desktop; the hole exposes what is behind it.

This isolation probe does not reproduce production popup routing, worker
ownership, winit messages, GPU fences or the exact timing of dragging. It cannot
establish that a proposed production repair works.

## Recommended implementation

Keep the decorated top-level host rectangular, with no application HRGN. Fit the
acknowledged, borderless worker **above** that host and below its owned popup/tool
surfaces. The worker must remain an unowned top-level window in its own process.
Adapt the existing Folderstorm behavior rather than adding cross-process parents:

- Pinned reference: `DrCum/Folderstorm`,
  **abbe3cb0ca66ee2484f277219f6a27706fab7fc6**,
  [`fssessionworker.cpp::fitSurface` and `embed`](https://github.com/DrCum/Folderstorm/blob/abbe3cb0ca66ee2484f277219f6a27706fab7fc6/indra/newview/fssessionworker.cpp#L603).
  `fitSurface` inserts the worker directly above the host using the preceding
  Z-order neighbor and `SWP_NOACTIVATE`; `embed` preserves independent lifetime.
- Aurora's intentional difference: preserve its exact binding/lease/geometry
  receipts, GPU completion proofs, input gate and rollback rules. Folderstorm's
  placement code is a behavioral reference, not sufficient authority validation.
- Aurora currently paints egui menus/tooltips on the same surface as the host.
  Those pixels would sit underneath an above-host worker. Therefore popup and
  cover handling must land with placement changes, before removing the region.

Implement as one related presentation change, with these reviewable stages:

1. **Prove relative placement offline.** Exercise two separately owned synthetic
   processes with a normal host, borderless worker and host-owned popup/cover.
   Use real native handles and message loops. Prove that host activation does
   not obscure the world, that another app stays above the workspace, and that
   an unresponsive worker can be covered immediately. Do not enable a new
   production path until these invariants work.
2. **Replace both placement paths together.** Update `native::fit`,
   `native::stay_below_host`/`native_order` and their callers in `worker.rs`.
   Query/revalidate the exact host PID/handle and current geometry before placing
   only the worker's own HWND. Preserve existing visibility and foreground-family
   checks. Handle an already-correct neighbor, host activation, a topmost neighbor,
   host minimization/destruction, HWND reuse and native API failure. Never make
   the worker globally topmost, force focus on translation, modify a foreign
   window, or attach thread input. Keep separate explicit activation focus.
3. **Provide owned popup and cover surfaces.** Refactor `native::Overlays`,
   `software.rs` and host rendering to present menus/tooltips above the worker
   using bounded host-owned surfaces. Use the existing theme and exact painted
   bounds, outward rounding for hit targets and the current conservative overflow
   behavior. Retain pointer/wheel/keyboard, outside-click/Escape dismissal and
   existing focus-family checks; prevent clicks through a visible menu to the
   world. Include management-panel expansion and popup closure on focus loss.
   Owned tools (Chat, Attention, Monitor, review) must retain their current order
   and source identity.
4. **Replace the cover guarantee before deleting HRGN.** `close_region` currently
   covers a stale/pending world immediately without waiting for another process.
   Supply an opaque, host-owned client cover with equivalent immediate visual
   and input protection. Revoke/park/hide the worker through the existing guarded
   protocol as needed and wait for the matching receipts before exposing it
   again. A modal, pending activation, stale geometry, excess/invalid overlays,
   minimized host or failed native operation must never reveal an unacknowledged
   world. A delayed/dead worker must not prevent the cover or emergency cleanup.
   Do not fabricate GPU quiescence or input ownership to make a frame visible.
5. **Remove top-level region changes.** Replace `Region::set/close` only after
   the prior stages work together. Preserve `presented_worker` and
   `confirmed_active` authority checks and update CPU paint-region optimization
   so it skips only a currently acknowledged, visibly occluded world. Keep the
   normal native caption/decorations throughout attach, movement and detach;
   normal OS policy controls their appearance.

Presentation semantics must not silently interoperate with the old
below-host/HRGN path. Review `aurora-session` protocol negotiation and worker
launch/version checks; use an explicit version/capability boundary if mixed
binaries are possible, failing closed for an unsupported pair. Do not weaken
the existing v2 lease/receipt checks. Coordinate shared host render/event,
worker/native, popup/cover and protocol files with the lead's current source.
The base above identifies the diagnosis, not permission to overwrite newer work.

Do not substitute per-pixel top-level transparency (also excluded from normal
Windows 11 rounding), cross-process `SetParent`, `GWLP_HWNDPARENT`, global
topmost placement, injected input or an unreviewed new GPU/compositor path.

## Required validation for the implementation

Use one frozen combined candidate for the feature group's native/full checks.
Run focused session/host geometry, placement, popup/input, cover/rollback and
foreground-family regressions between stages. Include assertions that the
decorated host has no application region and that visibility never precedes
the matching grant/placement acknowledgement. Add failure cases for stale PID,
HWND reuse, lease expiry, hide/placement failure and an unresponsive worker.
Retain the independent-worker crash/host-exit recovery behavior.

On the reporting Windows PC (and Windows 10 if supported), compare the same
restored window before attach, attached, during move/resize, settled, and detached:

- Native title, accent, minimize/maximize/close buttons, border/shadow, system
  menu, resize hit tests and Windows snap behavior follow OS policy. Windows 11
  intentionally uses square corners when maximized/snapped and in some remote/VM
  contexts; those are not failures by themselves.
- Dropdowns/tooltips crossing the world boundary remain visible and interactive.
  Check close confirmation, management panel, Shared Chat, Attention, Monitor,
  native file dialogs, and both viewer UI styles where controls overlap.
- Move/resize/snap, minimize/restore, mixed DPI and negative-origin monitors keep
  geometry aligned; rapid switching/translation retains the exact active source.
  Foreground changes among toolbar, worker, tools and another app do not cause
  repeated raising, obscured content, focus theft or accidental world input.
- Delayed/unresponsive worker, revoked grant, host close/crash and detach preserve
  immediate cover and eventual independent viewer recovery. No idle full-frame
  repaint loop or unnecessary Vulkan surface recreation is introduced.

Grid login, real accounts and live GPU acceptance belong to the user. Offline
fixtures must not contact a grid or access credentials. A successful build or
`NcRenderingEnabled=true` is not visual acceptance.

## Handoff and current validation

This PR adds the investigation and standalone probe only. Production Rust,
dependencies, protocol and existing validation candidates are unchanged.

- Local PowerShell 7 parser and embedded C# compilation are checked separately
  from native execution. `scripts/check-scripts.ps1` prints a legacy 5.1 success
  label even on Linux; only its PowerShell 7 parse is established here.
- Native Windows 5.1 compilation, probe execution, screenshot comparison and all
  production repair/build/package acceptance remain pending. No repaired binary
  is claimed or requested by this investigation.
- Before a lead implementation is ready, run this probe on its frozen PR source:
  `powershell.exe -NoProfile -File scripts/check-scripts.ps1`, then
  `powershell.exe -NoProfile -File scripts/probe-host-frame.ps1 -CompileOnly`,
  then the interactive STA command above. Return exact repository/branch/SHA,
  commands/outcomes, Windows build/theme/DPI/local-or-remote context, the JSON
  and visual Normal/Region/Clear comparison (plus optional Forced DWM result).
  No viewer features, package, grid or installer are required for this probe.
- The lead should pin a separate implementation candidate and combine its normal
  full suite/strict Windows lint and package verification with the feature-group
  checkpoint. Include the native scenarios above and return test totals/failure
  logs, exact executable version, package path and SHA-256. Preserve the current
  known-good package and earlier failed-run evidence.

Use this document as a focused implementation handoff; the lead continues to
own the existing current-handoff record. Keep the PR draft until the lead has
chosen/integrated the repair and the relevant Windows results are available.
