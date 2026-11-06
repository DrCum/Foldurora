# Foldurora session host

## Persistent character profiles checkpoint

The source now provides five persistent slots in the profile selector above
Open login. Select a profile, then open its ordinary manual login; saved-account
Open actions also use that selected free profile. Rename changes its label,
with at most 64 UTF-8 bytes and no control characters. An empty name restores
Character 1 through Character 5. The actual ready account and grid remain shown
separately. Renaming never changes login, session or chat ownership.

Each slot retains its own settings/account files and private cache after the
viewer exits. New slots begin with defaults. Earlier generated profiles and
ordinary viewer settings are preserved and are not imported. The namespace
still belongs to the canonical executable location: replacing that executable
retains it, while a different portable/worktree location has separate slots.
Only one controller owns the installation's registry at once. A detached
viewer keeps its profile occupied until the process exits; close that normal
viewer before reopening its profile. No unrelated viewer is adopted.

The private registry is bounded, versioned and atomically replaced. If it is
missing while stable profile directories exist, startup refuses to adopt them;
restore the original registry before reopening. Corrupt/newer/nonprivate
registries are refused without rewriting their data or permissions. No profile
deletion, reset, migration, restart command or automatic login is supplied.

Source tests cover storage identity, retention, lock/controller loss, stale
bootstrap refusal, registry loss/version/privacy and actual egui profile
selection/rename. Integrated Linux tests and Clippy passed for the new behavior;
the full run retains the documented software-GPU terrain precision failure.
This checkpoint still needs Windows
acceptance: change one slot's preferences, close its viewer, reopen it and
verify retention; repeat with another slot and custom names. Verify a second
controller is refused, detached/live profiles remain occupied, startup errors
preserve data, and every reopened login has new session/chat authority. Grid
validation belongs to the human.

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
- Controller startup checks and syncs a separate `aurora-session-host-*.log`
  before detaching only its own Windows console. Managed Windows workers start
  with `CREATE_NO_WINDOW`; no shared or foreign console window is hidden or
  closed. Help/version output and ordinary viewer startup remain unchanged.
  Log or startup failures stay visible, including after console detachment.
- Each worker keeps its own world, network and GPU state. IO and launches stay
  off the viewer frame path, and queues/messages have explicit bounds.
- Private transport binds installation, controller, worker nonce, account,
  grid, session generation, sequence and lease. Window handles alone do not
  authorize commands. Invalid replies must not renew a lease. A worker that has
  never reported status has a fixed five-minute startup deadline; its first
  valid receipt arms the ordinary fifteen-second lease. No status means no
  activation, and outgoing heartbeats cannot extend that startup deadline.
- The five explicitly selected profile slots persist across launches. Preserved
  pre-slot bootstraps retain their old per-session/generated profile behavior.
  Existing user settings and passwords are not copied into those profiles. The shared saved-account
  catalogue is used as metadata; private cache keys avoid the shared OS entry.
- Windows transport, bootstrap-claim and profile-lock files receive an explicit
  current-user owner and private DACL at creation. A token's default owner can
  be a group, even under a private directory. Existing files keep their original
  security policy and are rejected if their owner or ACL does not match.
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
- The native inactive-chat path is connected for literal Nearby and existing
  resident IM conversations, with its CPU tool window bound to one immutable
  source. It needs integrated tests/native acceptance before publication; groups,
  offers and other interactive actions are not exported. Controls require a live
  authenticated worker and actual native permission metadata. A distinct private
  bootstrap key authenticates both socket peers; chat text never enters IPC
  files, logs or process arguments. IO is separate from control, and native
  restrictions/account/lease are checked again at submission. Uncertain sends
  are not automatically retried, and policy gaps purge cached text/titles.
- Each popout retains separate bounded Nearby/existing-IM drafts and editor Undo
  state under its exact source and native policy. Ordinary Close drops the native
  surface while the host retains session UI state; reopening the same live source
  restores it into a fresh context. Closed and minimized states still process
  policy/receipt updates. Lost authority discards affected text and Undo history;
  host shutdown ends all retention. No draft files or restart persistence are added.
- Reduced-resolution monitors are a later stage; protocol/budget models or an
  uninvoked renderer backend alone are not that feature.

## Acceptance before recommending a build

Automated checks must include identity/replay/lease rejection, stale and
old-surface frame proofs, uncertain-grant rollback, quiescence before another
login, profile locks and path ownership, bounded transport, idle CPU repaint
behavior and source-bound control actions. Run formatting, strict workspace
Clippy and workspace tests, then build the normal viewer at the exact source.

Native Windows checks must confirm that `--help` and `--version` still print in
an existing terminal, that the terminal remains available after a host launch,
and that a debug host plus its managed workers adds only the host's taskbar
entry after manual login/hosting. Check the distinct host log and visible
startup failures before treating console detachment as accepted.

Native Windows checks must prove the taskbar and Z-order behavior, switching,
host-control focus, held-key/PTT release, resizing, mixed DPI, display removal,
worker crash and host-death recovery. Chat checks must cover inactive-account
Nearby/IM source selection, source-specific drafts, native restrictions/mutes,
policy changes with old host-only cached history, reconnect uncertainty without
replay, and composing in an owned tool window while the active world stays
visible. No chat action may promote a world or redirect text after a switch.
 Closing a viewer can open a confirmation;
distinguish an unconfirmed Close request from a hung confirmed Quit. Preserve
the earlier standalone/viewport test packages while this work is validated.

Grid acceptance belongs to the human. Automated fixtures must use synthetic
accounts and owned processes without contacting a real grid or reading login
credentials. A passing state machine or software-rendered capture does not
establish physical NVIDIA/Vulkan coexistence.
