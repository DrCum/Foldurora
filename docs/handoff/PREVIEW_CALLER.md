# Preview caller and CPU popup handoff

**Frozen at the user handoff boundary.** Branch `feat/session-preview-worker`,
HEAD `a449faa7` (reviewed capture precursor). The remaining App, cached Scene,
registry and CPU popup integration is **unreviewed, uncompiled WIP**. Preserve
the dirty worktree or use the self-contained source patch
`preview-caller-ui-wip.patch.gz` (decompress before applying) beside this file; it includes the three
new source files as well as tracked changes. The inventory is
`session-preview-wip-inventory.txt`. No further implementation is underway.

Status: source work only. Preview capability remains false. No Cargo, viewer,
GPU helper or native monitor acceptance has run in this worktree for this stage.
The parent owns the combined default target and serialized build/test jobs.

## Reviewed source checkpoints

- `31fdc1cd` (local alias `4d90d484`): persistent small renderer targets,
  independent view history, async one-flight readback and ordered fence gates.
- `d5da523c`: actual completed timestamp-query telemetry and lazy preparation.
- `5715c528`: bounded source-bound image/control contracts, cohort matching
  contracts and rate-aware immutable image deadlines.
- `dfdedb80`: suspension wire/mailbox and deterministic initialization ordering
  regressions. Local `afae5458` between these is only the chat UI dependency
  `98c2299d`; M2 already contains it and must not pick it twice.
- `28539bb7`: atomic suspension retirement before all locks, persistent 63-bit
  replay floor and canonical exact scope. The barrier allocator must use the
  exported `MAX_SUSPEND_CHALLENGE`.
- `a449faa7`: exact own-window creation capture intent and unique native epoch
  through startup and GPU recreation. Unknown/protected provenance denies.
  Compile this precursor with the actual caller, which consumes its API.

The worker author's `fed4084c` owns the authenticated lanes, transport actor,
global all-participant GPU-drain barrier and worker fence API. Root assembles
the coherent cohort; do not blindly cherry-pick either branch's dependencies.
Its typed `None` retirement follow-up was source-reviewed after the popup's
close seam was reported; pick the worker's final follow-up with this WIP. A
disable must remain enqueueable even after opening eligibility disappears.

## Caller behavior

`app/host_preview.rs` runs before the parked worker's early return. The App
rechecks the bridge's submission gate after the hook so a newly started suspend
fence cannot fall through into a normal world/UI packet.

The source is immutable installation/host/worker/session/generation/lease;
device, window, policy and vision epochs also scope the retained frame. A new
source/vision/visibility policy retires the old producer generation and requires
a new authenticated control challenge. The separate image connection closes on
retirement, causing the broker to purge already received pixels.

Cached PID/device/LUID identity can answer a fresh metadata challenge while
suspended. GPU samples, optional service/init/render/readback and Scene work
cannot. A suspension response comes only from the bridge's actual ordered GPU
completion and exact still-pending request. Late init callbacks have no Queue
authority; their writes were ordered before that fence.

A preview requires a previously accepted ordinary world frame and the matching
consumed Standby completion fence. Never-activated sessions remain not ready.
The view is a 16:9 camera with persistent 320x180, 480x270 or 640x360 targets,
not a reduced copy of the native window or a photo capture. No UI/picks/input
forwarding run. Fresh physical memory headroom, exact all-peer cohort identity,
new challenge-bound active GPU completion timing and the overload/VRAM guards
remain mandatory. The 100ms ceiling is a conservative prototype heuristic,
not evidence of driver-reset prevention.

`scene/host_preview.rs` accepts only committed cached assets with a bounded
8192-object slot range, 32 palette slots and 65,536 faces. Existing transforms,
known animations, current avatar bounds and read-only impostor planning can
advance. It does not run normal asset synchronization/streaming, register
unknown animations, acquire foot-support geometry, grow mover palettes,
consume dirty feeds or change texture wants. Changed assets or larger scenes
remain unavailable; activating a world may resolve assets but cannot remove
these explicit prototype limits.

## CPU popup behavior

`session_host/preview_window.rs` uses the existing egui CPU painter/softbuffer
owned tool window. It creates no Vulkan context. Its image sense is hover only;
only local requested size/cadence controls accept input. Every existing popup
action uses the exact source, including hide/close/retry, and cannot retarget to
a later login in the same worker process.

Requested cadence choices are 0.5, 1, 2 and 5 snapshots/second. Actual delivery
can be lower because fresh GPU/memory samples, the aggregate cohort budget,
readback and cold initialization can defer work. A rate limit is not a GPU
overload guard. The sampler cadence must be measured before claiming these
requested rates are sustained.

Live/control authority expires after 750ms. Each accepted image keeps its own
immutable host-challenge deadline: requested frame period plus 750ms, up to
2750ms at 0.5 FPS. Status-only heartbeats cannot renew it. The popup schedules
its own bounded deadline redraw, blanks on source/policy/state loss and does
not rely on receiving another broker snapshot. It sleeps when unchanged;
image heartbeats do not force software repaint of the large main controller.

Barrier status/cancel uses the existing header height. It does not shift the
active native viewport or hide the current acknowledged world merely because
a pre-action barrier is waiting. Cancel abandons the held action, not a missing
GPU acknowledgment; retry starts fresh typed challenges.

## Root validation still required

- Compile/strict Clippy/fmt and full workspace tests for the coherent combined
  renderer, DTO, barrier, worker, caller, popup and aggregate-admission sources.
- Run the ignored small-target Vulkan readback helper and measure cold/warm
  initialization, target bytes, readback latency and actual sustained cadence.
- Exercise full normal-frame protections and no-preview M1 behavior; verify
  no normal full-resolution packet is produced by a parked preview request.
- Exercise queued initialization followed by suspend/Prepare/new login,
  delayed callbacks, stale proofs, membership changes and timeout/cancel.
- Windows: validate the actual default unprotected native window provenance
  path and positive protected rejection, recreation, owned tool/taskbar/Z-order
  and no movement/key/click forwarding. Never alter protection/window styles
  just to make the native getter succeed.
- Validate hide/close/minimize, config changes, source/vision/lease changes,
  broker stalls and image/control backpressure visibly blank old pixels.
- Use synthetic/demo data only. Grid validation belongs to the human.

The current user toolbar/camera/snapshot confinement hotfix is separately owned
by `windows_private_acl`; its shared layout files must not be overwritten by
this M3 integration.
