# Foldurora workspace and character preview

This preview brings the first two Folderstorm priorities into Aurora's Rust
viewer: a world viewport for a window spanning several displays, and a native
way to open and switch between characters. Each character keeps its own viewer
process, world and network session. This avoids moving live simulator state
between renderers.

The first checkpoint is `feat/overnight-integration` at `62cdd4ea`; its verified
build remains available. `feat/workspace-followup` adds idempotent repeated Open
feedback, readable character captions and gentle live display recovery without
moving that checkpoint. The source comparison
uses [Folderstorm's tested 82373 tree](https://github.com/DrCum/Folderstorm/tree/abbe3cb0ca66ee2484f277219f6a27706fab7fc6)
and starts from Aurora commit `ae2fd1016ec83304719fa131312e2ac69c8f37a6`.
This is a test candidate; Windows display/focus and human grid acceptance are
still required before the preview is accepted. Checkpoint pull requests remain
drafts while that validation is pending, as authorized for this preview.

## Feature inventory

| Folderstorm capability | Aurora foundation | This pass |
| --- | --- | --- |
| World viewport across displays | One native winit window; world targets composed before egui | Custom world rectangle with consistent rendering, camera, HUD and input coordinates; optional viewport-bound toolbars and camera/snapshot controls with ordinary floaters free across the native window; Windows recovery policy; live X11 recovery remains unaccepted |
| Several character sessions | Saved-account carousel; separate network/world per process; cross-process GPU fair sharing and background frame cap | Native Characters panel, existing-session focus, explicit manual-login opening, repeated-Open feedback and readable account/grid/phase captions |
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

Stretch the main native window across the displays, then open Preferences →
Layout → first tab → **Multi-monitor world viewport**. Enable **Use a custom
world viewport**, use **Left half** or **Right half**, or adjust
the four percentage margins. **Full window** restores the ordinary view.
The remaining area holds normal Inventory,
chat, map and other floaters. Those floaters remain inside this native window.
They are not separate operating-system windows.

To keep the controls beside the world, enable **Keep toolbars and camera controls
inside the world viewport** in the same section. Top/bottom/side bars, their
menus, and camera/snapshot controls then stay in the configured world rectangle,
including its top/left offset. Inventory, Preferences, World Map, mini-map, chat,
Characters and other ordinary floaters can still use the full native window.
They can remain wherever they were saved; enabling this option does not move
them out of the world area or trap them inside it. Leave it off for full-window
toolbar and camera-control placement too. The setting is off by default and
survives placement presets and restarts.

Camera/snapshot controls fit or scroll within a small viewport so their title,
close button and controls stay reachable, even when ordinary floater settings
permit partial offscreen placement. Snapshot output is a separate choice:
selecting full-frame output does not move these controls outside the configured
viewport. Desktop mouselook retains the configured world viewport, including
when its interface is hidden. Photo output, VR and hiding the interface outside
mouselook can render full-frame without changing the saved control-placement
preference. Native cursor confinement/capture still requires separate acceptance.
Ordinary floaters
retain full-workspace anchors and sizes, including wide geometry retained before
the earlier blanket restriction fitted them to the viewport. An explicit manual
resize replaces the retained size. The native title bar, system dialogs and
separately detached native windows remain operating-system windows.

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
6. On Windows, change or remove a display while the viewer is running. If
   the whole normal window becomes inaccessible, expect a recovery attempt after
   at least three stable seconds. Its size is preserved when it fits; an oversized
   window requests 80% of the nearest surviving display, subject to native minimum
   sizes. Check that it does not gain focus.
7. Repeat with a partly visible span, a held native drag, or a minimized,
   maximized, fullscreen or VR window. It must stay untouched. After an excluded
   state ends, recovery gets a fresh grace period; an already reachable layout
   clears the pending recovery.
8. Toggle **Keep toolbars and camera controls inside the world viewport** with
   Inventory, Preferences, World Map, mini-map, chat and Characters open. Keep
   ordinary windows outside the world area and use their buttons, scrolling,
   menus and keyboard focus there. Top/bottom/side bars, toolbar menus and
   camera/snapshot controls must stay inside it. Ordinary window menus must
   remain usable beside their windows. Toggle back; check saved positions and
   sizes, including a previously fitted wide floater, minimize/restore and restart.
9. Repeat with nonzero top/left margins, fractional DPI/interface zoom and both
   toolbar layouts. Make the viewport very narrow: camera/snapshot controls must
   fit or scroll inside it while Preferences remains usable across the full
   workspace. Enable partial offscreen movement for ordinary floaters and select
   full-frame snapshot output; neither may let camera/snapshot controls escape.

The corrected selective scope still requires automated, offline visual and human
native-window acceptance; the earlier blanket-interface behavior is not its
validation evidence. `AURORA_DEMO_VIEWPORT=ui-tools-left|ui-tools-right|ui-tools-inset|ui-tools-toggle`
opens the relevant controls and ordinary windows without a grid login; see
[TESTING.md](TESTING.md) for capture frames and the toggle/restart checks.

A native window has one current DPI scale. A spanning-window implementation
does not provide independent per-display scaling for its floaters. Separate
native panel windows would require a further event, input and render-surface
implementation.

Live recovery arms only after a display topology/DPI change; ordinary manual
placement does not arm it. Missing geometry or unknown/stale native input state
prevents movement. Windows physical-display acceptance remains required. The
owned Xvfb/RandR fixture exposed stale winit monitor lists, so live X11 recovery
is not accepted at this checkpoint; fresh sampling is isolated on a separate
branch. macOS and Wayland leave the window alone when reliable native state is
unavailable.

## Character testing

Live testing of source `81c167e2` confirmed the display-spanning viewport but
reported graphics-device loss in older background character windows. Automated
checks had passed; they did not establish safe real-world GPU coexistence.
The contention mitigation pauses an unfocused world before drawing when another
live viewer shares the graphics card, using the existing **Pause background 3D
on a shared or overloaded card** preference. Chat, sound and the session keep
running. Focus resumes the world immediately. Shared-only pause also ends after
four seconds after the first observed empty peer snapshot; a transient registry
read failure or a long frame gap cannot immediately resume a heavy frame.
Known different GPUs are excluded; unknown GPU identities count
conservatively. Photos, VR, capture/profile runs and the setting's opt-out retain
their exemptions. The underlying reset cause remains unconfirmed; a person
still needs to accept simultaneous sessions on the affected hardware.

Restart **all** participating viewers from the replacement build before testing
the mitigation. A newly launched child cannot update the pause policy inside
an older process that is still running. Keep the earlier ZIPs separately for
comparison; they have not passed this simultaneous-session acceptance.

Open **Me → Characters**, or use the users icon on the login screen. Use the
Characters panel to select an existing session or open a saved
account's login window. Opening another character does not log out the first.
The new window uses the ordinary login and two-factor prompts; the user logs
in. No password, two-factor code or login session token belongs in a process
argument or in the switcher's discovery records.

1. Open two different saved accounts on the same grid, then switch between
   their native windows. Confirm the selected name, world, Inventory and chat
   all belong to that window's account. Check Alt-Tab/taskbar captions show the
   selected account, short grid label and login/connecting/online phase; unread
   and render-pause indicators still appear. A custom `--title` remains in force.
   Custom grids include a short identifier so endpoints sharing a name or host
   remain distinguishable without displaying private URL details.
2. Attempt to open the same saved account twice. Check the existing or starting
   session is used, repeated clicks report Opening or a neutral waiting notice,
   rather than a false failure,
   and a stale/crashed record does not block reopening. Once ready, the existing
   window is focused; genuine launch and filesystem errors remain errors.
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
Some existing account files are keyed by avatar UUID alone; conversation
transcripts also include the grid. Known same-avatar cross-grid launches need
special handling rather than assuming all account data is grid-isolated. The
switcher refuses Open login when that avatar is known to be live on another
grid. Distinct avatars can run concurrently; manually started viewers retain
the existing storage behavior.

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

The viewport reduces the world 3D targets. The native presentation surface,
interface and HUD depth resources still cover the full native window.

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
