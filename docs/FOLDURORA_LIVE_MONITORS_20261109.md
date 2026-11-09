# Live inactive Monitor integration

The native worker now samples an explicitly opened, visible Monitor without
requiring that character to have been active first. This joins the preserved
PR21 offscreen foundation to the existing native source, scheduling and sealed
pixel transport. It does not grant active presentation/input or MCP workspace
mutation capability. Source implementation is separate from native acceptance.

## Behavior and bounds

- Real managed Standby only: exact native worker window/source, fresh control
  lease, Online/ready World, no promotion/drain/trim/exit prompt. Active, detached,
  unsupported and unproven native contexts cannot use the inactive branch.
- Explicit visible demand and native size/rate receipt, not roster presence.
  Source privacy, mutes, RLVa names/location/nearby and visual masks, locked HUDs,
  teleport/loading, photos, renderer loss and XR retain their existing denials.
- Half/One/Two/Five FPS, globally paced 10 FPS total; no catch-up. Scene preparation
  runs only after a render-owner slot is reserved, at fitted small dimensions
  within 320x180 / 480x270 / 640x360. Normal network/chat/agent ticks continue.
- Reduced small-view quality: at most 128 m and 12 avatars, 512 texture resolution,
  one sample/probe, no shadows/reflections, editor overlays, particles, labels,
  HUD/UI or photo rendering. Existing world visibility/mute/complexity policy is
  used. Saved graphics/camera and the real window size are not changed.
- One small backend per worker, two persistent outputs, latest-only leases.
  A process-wide allocation lease follows the backend, GPU callbacks and borrowed
  Frames across renderer/device replacement; a new renderer cannot renew it.
  Initial resources use reduced settings before allocation. The previous default
  shadow/probe texture pair alone was about 96 MiB; the reduced pair is about
  5 MiB. These are two calculated payloads, not total/measured VRAM.
- Additional conservative view credit: 128 MiB per accepted/pending/uncertain
  source, 512 MiB across the host (at most four configured preview workers).
  Existing CPU/staging/IO pools are separate. EOF, hide and pixels do not release
  credit. A native Stop acknowledgment waits for GPU completion, borrowed output
  retirement and destruction of the small backend on its owner. Process exit is
  the other release proof. This credit is not a claim about driver total usage.
- Cold live allocation additionally requires a real driver budget sample with
  256 MiB free; subsequent requests require 64 MiB. Unknown/low headroom refuses
  the sample. GPU allocation/driver failures retain their normal failure paths.
- Offscreen export reuses the bounded asynchronous capture service. Capture kind
  is sealed into the original queue request and carried through actual mapping.
  Full small-texture UVs are sampled while the original native source boundary,
  viewport crop, surface epoch and fitted geometry remain associated metadata.
  Active packets cannot consume inactive claims; serialized kinds cannot be
  relabeled. Opaque sRGB bytes, credit, timeout and publication revocation remain.
- Bounded recovery limitation: an uncertain native reservation after metadata
  loss is held until owned process exit. Existing privacy/teleport source closure
  can trigger this condition; clearing the restriction alone may not resume
  sampling, so restart that worker. No automatic rebind or implicit reservation
  release is added. This is a remaining Folderstorm behavior gap.
- Pixel wire is V5; older V1-V4 peers are refused. Bootstrap/auth versions stay
  unchanged. No ordinary full-window, photo or blocking readback fallback exists.
- Captions distinguish inactive world samples from LastActive images and show
  bounded age. A retained sample is never a frame/input ownership proof.

## Reference and validation

Folderstorm reference: DrCum/Folderstorm
`abbe3cb0ca66ee2484f277219f6a27706fab7fc6`,
`indra/newview/fssessionworker.cpp::renderMonitor` (around line1641), SHA256
`30564e0c1884a387a2877b2393eb0f8ff866b828`.
The port uses Aurora's queue owner, small projection, shared scene resources and
strict native/privacy fences rather than Folderstorm's OpenGL snapshot path.

Focused pure component checks cover actual production policy/wire modules and
capture queue/origin metadata. Qualified aggregate Linux lint checks compile the
App/World/queue joins but do not run their tests. Full Linux linking is blocked
by the fixed cloud disk; terrain/ownership history remains unresolved.

Windows must run one combined unfiltered suite, audit required names, execute
both the real small-world/retirement fixture and explicit synthetic offscreen
color/crop GPU gate, then build/package the ordinary viewer/sidecar. Native
hosted acceptance remains separate: never-active character, Stop/reopen, changing
source, privacy/teleport and promotion/rollback, host move/resize, custom viewport,
responsiveness and Monitor chat. Driver reset, XR and real-grid/RLVa acceptance
remain unverified. The separate radio and older 98b/a903 results are not passes
for this changed renderer/source group.
