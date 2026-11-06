# Foldurora session host

This is an opt-in implementation in progress. The current source assembles a
CPU controller, ordinary manual-login workers, private profiles, bounded local
transport and GPU completion fences. The Windows presentation adapter is
implemented; integrated compilation, runtime checks and physical Windows
acceptance are still being completed. No complete host test build is accepted yet.

Run `aurora-viewer.exe --session-host` to select the controller mode once using
an explicitly identified host test build. Ordinary launches retain the existing
viewer path. The controller never submits a login, transports a password or
moves a live simulator session into another renderer.

## Intended behavior and stages

1. One visible taskbar host after ordinary manual login, with one separate
   viewer process per character. Revoke held movement, cursor and push-to-talk;
   wait for previous GPU work to complete; prepare a frame on the target's
   current surface; grant only after that exact frame completes. Host controls
   may take keyboard focus while the active world retains render ownership.
2. Read-only monitor windows render actual small targets at 0.5, 1, 2 or 5 FPS.
   Bounded latest-only frames and an aggregate budget must include acknowledged
   and pending policies. Hiding a monitor releases its demand. Full-resolution
   background rendering followed by image resizing is not an implementation of
   this stage.
3. A monitor's chat window sends as its bound inactive account and conversation
   through native restrictions. Drafts and replies keep their source session;
   changing the active world must not retarget text. Offers, permissions and
   payments remain native interactive actions, not text commands.

The [Folderstorm help guide](https://folderstorm.com/help) describes the behavior
being compared. The pinned implementation reference is
[`abbe3cb0ca66`](https://github.com/DrCum/Folderstorm/tree/abbe3cb0ca66ee2484f277219f6a27706fab7fc6):
`tools/session-host/main.cpp`, `indra/newview/fssessionworker.cpp` and
`fssessionmonitormodel.h`. Its worker stays an unowned borderless top-level
window fitted to the host area; it is not a cross-process child window.
Foldurora uses its own V1 protocol and Aurora's renderer; no legacy wire
compatibility is claimed.

## Current boundaries

- The controller uses egui and softbuffer CPU painting, without another Vulkan
  world renderer. Unchanged status receipts must not cause continuous repaints.
- Each worker keeps its own world, network and GPU state. IO and launches stay
  off the viewer frame path, and queues/messages have explicit bounds.
- Private transport binds installation, controller, worker nonce, account,
  grid, session generation, sequence and lease. Window handles alone do not
  authorize commands. Invalid replies must not renew a lease. A worker that has
  never reported status has a fixed five-minute startup deadline; its first
  valid receipt arms the ordinary fifteen-second lease. No status means no
  activation, and outgoing heartbeats cannot extend that startup deadline.
- Profiles are isolated **per launch/session generation**. They do not yet keep
  stable per-character settings across restarts. Existing user settings and
  passwords are not copied into those profiles. The shared saved-account
  catalogue is used as metadata; private cache keys avoid the shared OS entry.
- GPU fence completion proves submitted work completed and presentation was
  issued. It does not prove physical display scanout. Inline rendering and XR
  cannot currently provide this host fence contract.
- Activation, host-owned viewport cutouts, detach recovery and resize handoff
  need integrated native checks. Minimize parks the active world; restoring the
  host currently requires explicit Activate. Device/window recreation detaches
  to an ordinary worker rather than automatically re-adopting it. Other
  platforms leave unsupported presentation disabled. Requesting fullscreen
  first detaches the fitted worker so host geometry is not saved as the user's
  ordinary window placement.
- Reduced-resolution monitors and inactive-account chat are later stages;
  protocol/budget models alone are not those features.

## Acceptance before recommending a build

Automated checks must include identity/replay/lease rejection, stale and
old-surface frame proofs, uncertain-grant rollback, quiescence before another
login, profile locks and path ownership, bounded transport, idle CPU repaint
behavior and source-bound control actions. Run formatting, strict workspace
Clippy and workspace tests, then build the normal viewer at the exact source.

Native Windows checks must prove the taskbar and Z-order behavior, switching,
host-control focus, held-key/PTT release, resizing, mixed DPI, display removal,
worker crash and host-death recovery. Closing a viewer can open a confirmation;
distinguish an unconfirmed Close request from a hung confirmed Quit. Preserve
the earlier standalone/viewport test packages while this work is validated.

Grid acceptance belongs to the human. Automated fixtures must use synthetic
accounts and owned processes without contacting a real grid or reading login
credentials. A passing state machine or software-rendered capture does not
establish physical NVIDIA/Vulkan coexistence.
