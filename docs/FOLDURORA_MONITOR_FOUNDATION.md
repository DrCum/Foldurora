# Monitor offscreen renderer foundation

This is a **disabled renderer prototype**, not live inactive-character Monitor
support. No viewer, host capability, preference, protocol or production caller
enables Cargo feature `aurora-render/monitor-prototype`. The existing native
Monitor continues to show guarded LastActive snapshots. Do not enable the new
API for live accounts until the integration gates below are complete.

## Source and coordination

Base: `d0d8f8f5ad73f177160882a4543252e6941d661d`, the published combined
host/MCP candidate. Main was `f78d28147773ae9b2873bc64a0da212853f5c085`.
The implementation owns `feat/monitor-offscreen-foundation` and
`/workspace/work/monitor-foundation`. The PR pins its final source head and
validation commands; source candidates stay frozen during checks.

Inspected main AGENTS, the published current-handoff pointer, all 97 available
remote refs, open PRs, and the lead's current chat. The lead's private current
handoff worktree is not mounted here; the published pointer is not a substitute
for that unavailable file. The lead's November 9 Monitor audit identified
presentation-coupled dimensions, absent inactive scene preparation and switching
risks, then moved to independent workspace work. PR #20 subsequently froze that
work at `a6119664d9f576925c66e4bac20bbf049d0fdca9`. This renderer slice leaves
that source and the Windows candidates untouched. Coordination notes are on
PRs #10 and #20; no direct chat-send/Builder relay tool was available.

The preserved `handoff/session-preview-renderer-20261106` at `a449faa7` and
`handoff/session-preview-suspension-20261106` at `213d8e49` contain an older,
unintegrated preview prototype and suspension work. Read their
`docs/handoff/PREVIEW_CALLER.md` and `PREVIEW_WORKER.md` on the documentation
handoff branch. In particular `5e008044` already explored an independent
small backend, asynchronous readback and admission. This slice uses the
current renderer's newer external-texture path and keeps GPU leases local;
it does not import that old cohort's transport, caller WIP or budget claims.
The synthetic FrameParams test fixture is adapted from that archived source.
Do not integrate both implementations as two competing preview owners.

Folderstorm references, all at
`abbe3cb0ca66ee2484f277219f6a27706fab7fc6`:

- [Small snapshot target](https://github.com/DrCum/Folderstorm/blob/abbe3cb0ca66ee2484f277219f6a27706fab7fc6/indra/newview/llviewerwindow.cpp#L6268-L6439): reject full-window fallback.
- [Monitor worker](https://github.com/DrCum/Folderstorm/blob/abbe3cb0ca66ee2484f277219f6a27706fab7fc6/indra/newview/fssessionworker.cpp#L1641-L1675): low-rate inactive scene sampling.
- [Frame lane](https://github.com/DrCum/Folderstorm/blob/abbe3cb0ca66ee2484f277219f6a27706fab7fc6/indra/newview/fssessionframepipe.h#L10-L85): latest-only delivery, skip busy samples.

No OpenGL implementation was copied. Folderstorm's synchronous pixel conversion
and snapshot buffer reallocation are intentionally absent from the live design.
Repository licensing and source attribution are retained.

## Implemented boundary

`Renderer::try_render_monitor_prototype` reserves the existing render-thread
rendezvous before invoking a caller's scene-preparation closure. Busy means the
closure never runs: no journal is consumed, no scene work is queued, and no
wait is introduced into network/chat ticking. The closure receives actual
320x180, 480x270 or 640x360 dimensions for projection, CPU culling and LOD.
It must use the character's existing scene and renderer stores. A future App
adapter still needs to implement and validate that preparation.

The queue owner lazily constructs a separate external `Backend` using the
same device, queue, adapter and scene GPU handles. It retains its independently
sized depth/world/effect targets and its own culling, exposure and temporal
state. Two small output textures are persistent. At most one sample is queued
or on the GPU. A reader holding both leases stalls future delivery rather than
allocating a third output. Resize waits for old leases, so rapid size changes
cannot accumulate target generations. This prototype retains its backend while
hidden; releasing idle view memory is a remaining policy decision.

The presentation backend and native window dimensions never change for a
preview. External views now also reject missing output instead of acquiring
a surface. Preview packets have no UI, HUDs, picks, photo request, viewport
capture or input timestamp. RLVa pixel effects remain on FrameParams; removing
HUDs is **not** permission to bypass mandatory/locked HUD restrictions. Such
sources must be denied by the future existing-policy adapter.

The preview uses single-sample rendering with temporal AA, AO, screen-space
reflections, occlusion, shadows, planar reflections and water effects disabled.
This is temporary per-view quality, not persisted character graphics settings.
Presentation motion history is cut on return from the prototype. The backend
constructor still has expensive default initialization before applying this
quality; optimizing cold initialization and inventorying all allocations remain
required before enablement. Lower resolution does not bound asset, scene,
shadow/probe, shader or memory costs by itself.

The existing photo path rebuilds targets before/after the image and waits for
readback. `render_to_texture` alone expects presentation-sized configuration.
The new view uses the common world encoder but neither of those caller paths.
Output textures have no copy-source usage; live submission does no pixel map,
readback or CPU image allocation. Delivery occurs only after
`on_submitted_work_done`. A caller must retain its GPU lease until consumer GPU
use completes. This same-device lease is not a native host transport format.

The local scheduler accepts 0.5, 1, 2 and 5 FPS and drops missed periods. It
never builds a backlog, accelerates a configuration change, or releases an
in-flight reservation merely because a view was hidden. Revocation invalidates
already delivered leases and late callbacks. Normal rendering, resize, renderer
failure and `request_session_fence` retire Monitor demand. The ordinary ordered
session fence drains the same queue; Monitor completion never populates
`session_presented`, and cannot grant input. The live worker must additionally
revoke at the *beginning* of accepted promotion, before its native effects.

## Required before live enablement

1. Wire the existing exact source/lease/native ownership and privacy/RLVa
   checks before configuration, preparation, submission and display. Deny
   mandatory HUD vision, secure/unresolved names, capture restrictions,
   teleport, photo, XR, loss and unknown state. Reuse current shared helpers;
   a Config is no authority. Bind a producer to one character and source epoch.
2. Connect inactive scene preparation independently of the presentation frame:
   correct camera aspect, CPU/GPU culling, animation, committed asset lifetime,
   upload/mirror journals and no mutation of foreground camera/settings. Bound
   CPU scene/asset preparation as well as pixel work; keep networking/chat on
   the existing tick. The test exercises synthetic sky/world packets, not a
   logged-in animated region.
3. Reuse the actual host aggregate ledger. `aurora-session::admit_previews`
   already reserves max(acknowledged, pending) and handles hidden sources;
   `preview_projection/demand.rs` currently admits RememberActive captures,
   not live inactive render authority. Include all peers, pending policy,
   initialization and uncertain/draining history. A per-renderer 5 FPS cap
   does not establish aggregate GPU safety or fairness.
4. Adapt GPU output to the existing sealed source/claim/latest-frame image
   transport with bounded **asynchronous** staging, immutable freshness
   deadlines and native image retirement. Never turn this into per-frame photo
   readback. Current API exposes only same-device GPU leases; it does not
   deliver pixels to another worker/host process.
5. Hook hide/minimize/close and every policy/source change to revocation.
   On promotion stop admission before native effects; drain accepted render,
   initialization and export work; invalidate old images and require a fresh
   real foreground presentation at the existing boundary before input. Test
   actual broker/worker races, re-Prepare, reconnect and five-source churn.
6. Audit all view allocations including constructor peaks, lazy pipelines,
   probes, shadows, motion/debug overrides and readbacks. Add fresh device
   headroom/admission and failure backoff; release idle resources without
   recreating full targets per sample. Run real GPU loss and recovery.
7. Freeze the combined feature group and run current AGENTS full validation,
   native Windows tests and the existing builder/package gate. This draft's
   focused checks are not a full workspace or release-candidate pass.

## Measurement and acceptance

No performance improvement is claimed. There is no comparable run of two
ordinary full windows versus an inactive Monitor versus a host without Monitor.
Software Vulkan regression checks are correctness evidence, not hardware
benchmarks. Requested output payload for two RGBA8 textures is 460800,
1036800 or 1843200 bytes; these figures exclude every other allocation and are
not measured VRAM. No live readback is requested by this prototype, which is
not an end-to-end transport-bandwidth measurement.

After the authority/scene/transport joins, use the same machine, accounts,
camera, region, warmed assets, window sizes, settings and interval for all three
baselines. Record foreground frame-time distributions (including p95/p99),
completed timestamp-query preview GPU time, process CPU, actual device VRAM and
allocation peaks, exported/readback bytes, achieved/dropped frames and switch
latency. Include all sizes/rates, hidden/minimized views, simultaneous demands,
cold starts and asset-heavy scenes. Retain raw data and failed runs.

Windows/human acceptance remains required for focus/input, minimize/restore,
rapid switching, multiple monitors/DPI, device reset, native privacy restrictions
and logged-in scene correctness. Only the user performs grid login. No merge,
release, installation, credential or security-setting changes are part of this
slice. No Windows build has been dispatched for it.
