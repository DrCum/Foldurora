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
  resident IM conversations. One CPU Shared Chat tool explicitly selects among
  the five live character sources and their initialized conversations. It needs
  integrated tests/native acceptance before publication; groups,
  offers and other interactive actions are not exported. Controls require a live
  authenticated worker and actual native permission metadata. A distinct private
  bootstrap key authenticates both socket peers; chat text never enters IPC
  files, logs or process arguments. IO is separate from control, and native
  restrictions/account/lease are checked again at submission. Uncertain sends
  are not automatically retried, and policy gaps purge cached text/titles.
- Each character/conversation retains its own bounded draft and editor Undo
  state under its exact source and native policy. The host owns one canonical
  composer per source; this first stage exclusively uses the shared window.
  Future monitor or per-character views must borrow that state rather than copy
  drafts, Undo or pending sends. Ordinary Close drops the native surface while
  the host retains the model; reopening restores it into a fresh context.
  Closed and minimized states still process
  policy/receipt updates. Lost authority discards affected text and Undo history;
  host shutdown ends all retention. No draft files or restart persistence are added.
- Chat buttons select and focus the Shared Chat window. Its Send as selector
  shows the slot label alongside the actual account and grid; world switching
  never retargets it. An unavailable selected sender stays disabled until an
  explicit available selection. Source/conversation browsing neither activates
  the world nor acknowledges unread. Enter sends literal text, Shift+Enter adds
  a line, and Ctrl/Command+Enter also sends. Held-key repeats, IME confirmation,
  popup/modal input and target changes cannot submit a send. Delivered edits
  before navigation/Close stay with the old sender; ambiguous input after a
  navigation boundary is suppressed. A queued Cut is ignored on the close-only
  pass so that discarding clipboard output cannot delete the draft.
- Reduced-resolution monitors are a later stage; protocol/budget models or an
  uninvoked renderer backend alone are not that feature.

## Compact controls

The default host leaves the world the full client width below one 38-point
control strip. It contains the five-profile selector, manual Open login,
Activate, source-bound Chat, running-character tabs, character management and
Close. Long names truncate; character tabs condense to their slot number before
using a bounded horizontal scroller. Hover text retains the profile, logged-in
account, grid and session state.

Committing a Ready live profile selection requests the ordinary fenced switch.
Selecting a free profile still requires an explicit Open login and manual login;
selecting a busy or unready session exposes management. The selected profile is
not an acknowledgment: only the exact confirmed hosted worker is highlighted.
Character management reveals the search, saved accounts, Rename, session details
and Detach actions. Its deliberate drawer resize follows the same viewport
handoff as an ordinary host resize. Notices and errors use a fixed status badge
with full hover text; tabs remain visible and world geometry stays unchanged.
Close still requires confirmation and leaves sessions running in their own viewer
windows.

Native Windows acceptance must check compact drop-downs/tooltips over the world,
keyboard access, narrow/enlarged-UI controls, management open/close, and the same
title-bar drag, switch and detach paths as the earlier host. The layout does not
add Attention or preview monitors. Shared Chat is a separate source stage,
with compilation, synthetic captures and native acceptance still pending.

The Shared Chat behavior follows the pinned reference's
[`saveDraft` / `refreshChat` / `queueChat`](https://github.com/DrCum/Folderstorm/blob/abbe3cb0ca66ee2484f277219f6a27706fab7fc6/tools/session-host/main.cpp#L2980)
and [`ComposerReturn`](https://github.com/DrCum/Folderstorm/blob/abbe3cb0ca66ee2484f277219f6a27706fab7fc6/indra/newview/fssessionchatbehavior.h#L11).
The following read stage adds native unread counts and an explicit **Mark
conversation read** action for Nearby and existing resident IMs. It captures the
exact source, permission revision and native incoming/read observation. A newer
arrival or native read rejects that old observation; the user must review the
current count and click again. Uncertain operations are queried, never replayed.
Reading a conversation does not answer or remove its pending offers. Its native
UI retains its existing reading behavior in both styles; this stage adds no
claim that all passive managed rendering is prevented from acknowledging read.
Chat V2 peers are required; older chat peers fail closed. Inactive New IM,
Friends, contacts, groups, automatic host reading and Attention remain absent.
The read stage's compiled, synthetic visual and native acceptance is pending.

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
