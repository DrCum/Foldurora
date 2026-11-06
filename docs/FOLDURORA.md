# Foldurora workspace and character preview

This preview brings the first two Folderstorm priorities into Aurora's Rust
viewer: a world viewport for a window spanning several displays, and a native
way to open and switch between characters. Each character keeps its own viewer
process, world and network session. This avoids moving live simulator state
between renderers.

The integration branch is `feat/overnight-integration`. The source comparison
uses [Folderstorm's tested 82373 tree](https://github.com/DrCum/Folderstorm/tree/abbe3cb0ca66ee2484f277219f6a27706fab7fc6)
and starts from Aurora commit `ae2fd1016ec83304719fa131312e2ac69c8f37a6`.
This is a test candidate; Windows display/focus and human grid acceptance are
still required before the preview is accepted. Checkpoint pull requests remain
drafts while that validation is pending, as authorized for this preview.

## Feature inventory

| Folderstorm capability | Aurora foundation | This pass |
| --- | --- | --- |
| World viewport across displays | One native winit window; world targets composed before egui | Custom world rectangle with consistent rendering, camera, HUD and input coordinates |
| Several character sessions | Saved-account carousel; separate network/world per process; cross-process GPU fair sharing and background frame cap | Native Characters panel, existing-session focus and explicit opening of another account's login |
| Named workspaces, bars and window arrangements | Saved native floater geometry and toolbar controls | Follow-up after viewport and character acceptance |
| Assistant inventory, appearance, camera and workspace tools | Native typed inventory, appearance and camera actions | Follow-up: a bounded permission-aware adapter; no generic input injection |
| Bulk inventory review/history/limited undo | Native inventory mutations and progress tracking | Follow-up after the adapter and acknowledged-operation model |
| Inventory explorer, filters and outfit gallery | Already present, including independent tab filters and outfit JSON backup | Retain native behavior; compare only remaining differences |
| Gesture boards and shared posing | Native gesture manager, poser and IK tools | Later feature-specific work; collaborative posing is a separate protocol |
| Settings migration | Different typed settings and account stores | Map selected compatible values explicitly in a later opt-in importer |
| Hosted character view, combined chat and inactive-character monitors | Separate process windows in this preview | Later work; no hosted world or combined chat in this pass |

The existing OpenGL performance patches are not applicable to Aurora's Vulkan
renderer. The port follows user-visible behavior and uses Aurora's render
targets, typed actions and background workers.

## Multi-monitor testing

Stretch the main native window across the displays, then use the custom world
viewport controls in Preferences. The remaining area holds normal Inventory,
chat, map and other floaters. Those floaters remain inside this native window.
They are not separate operating-system windows.

Check these cases in the offline demo first, then in the user's normal session:

1. Put the world in the left and right halves, then use nonzero top and left
   margins together. Resize the window and check the camera's aspect ratio.
2. Move Inventory and chat into the space outside the world. Use their buttons,
   scrolling and text fields. Clicking, scrolling or dropping there must not
   select, zoom, move or edit the world underneath.
3. In the world area, click an object, orbit with Alt, use a context menu and
   open Build. HUD text, HUD editing handles and their hit areas must line up.
4. Enter and leave mouselook, hide and show the UI, and take photos with and
   without UI. The ordinary viewport must return afterward.
5. Try unequal display sizes, negative desktop coordinates and different DPI
   scales. Remove or disable the secondary display, restart, and confirm the
   main window can be recovered on an attached display.

A native window has one current DPI scale. A spanning-window implementation
does not provide independent per-display scaling for its floaters. Separate
native panel windows would require a further event, input and render-surface
implementation.

## Character testing

Use the Characters panel to select an existing session or open a saved
account's login window. Opening another character does not log out the first.
The new window uses the ordinary login and two-factor prompts; the user logs
in. No password, two-factor code or login session token belongs in a process
argument or in the switcher's discovery records.

1. Open two different saved accounts on the same grid, then switch between
   their native windows. Confirm the selected name, world, Inventory and chat
   all belong to that window's account.
2. Attempt to open the same saved account twice. Check the existing or starting
   session is used, and a stale/crashed record does not block reopening.
3. Switch while movement keys or push-to-talk are held. Release them, type in
   the other window, and confirm no old key or microphone state carries over.
4. Minimize the target and switch back to it. Test focus after Alt-Tab, while a
   dialog is open, and with each window on a different display.
5. Receive an IM and complete a teleport in the background, then switch back.
   Confirm normal session maintenance continued. Repeat with a custom grid
   account that has the same login name as another saved account.
6. Close one viewer normally, then crash a synthetic demo viewer. Confirm live
   rows disappear and other characters stay open.

These are separate native viewer processes, not one hosted world, shared chat
or unified voice. Global Aurora preferences and layout keep their existing
shared-file behavior; account-specific data uses Aurora's existing account
paths. Avoid editing global preferences simultaneously while validating this
preview. The saved-account catalogue already merges changes under an OS lock.

## Build and verification

Every feature uses a separate worktree and small commits. The integration
branch combines only reviewed checkpoints; the main branch and the unrelated
credential-transport PR stay outside the integration work. A failed feature
checkpoint remains on its feature branch instead of being included in the
candidate.

On a Windows development machine, in a separate worktree:

```powershell
./scripts/check.ps1
cargo build --locked -p aurora-viewer --features xr-static-loader
./scripts/package.ps1 -Exe target/debug/aurora-viewer.exe -Out .scratch/Foldurora-preview
```

The preview uses the default development profile. Performance comparisons use
matching release builds, the same world, graphics, render scale and world pixel
dimensions, with `AURORA_PROFILE=1`. Record frame time and its tail, memory and
switch recovery; FPS from the software Vulkan cloud environment is not a GPU
benchmark. Aurora's existing background cap and GPU budget sharing remain in
effect for real simultaneous sessions.

The artifact-only Windows workflow packages a commit-bound ZIP and checksum.
It does not create a version tag, GitHub release or updater feed. Its runner
must be available in this fork; absence of a run is a pending build, not a
successful artifact. Test evidence and exact remaining limitations accompany
the candidate handed to the user.

The Linux cloud environment uses software Vulkan. Its existing terrain
triplanar GPU precision test has failed in the base checkout; that result is
tracked separately from feature tests and Windows hardware acceptance. No
grid session is automated. The user has explicitly authorized checkpoint draft
pull requests for this preview; human Windows and grid validation remain pending
and nothing is merged automatically. Local-computer actions require the user's
separate computer-use approval; cloud Linux and hosted Windows CI do not use that
computer.
