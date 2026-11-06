# Foldurora workspace and character preview

## Five persistent profiles integration candidate

Branch `feat/persistent-profiles-and-chat` combines the host/menu fixes,
conversation-draft retention and crash details with five stable character
profiles. The profile selector above **Open login** chooses the settings/cache
slot; **Rename** changes only its label. Empty labels display Character 1 through
Character 5. The actual logged-in account and grid remain shown separately.

Profiles belong to the canonical executable location. Closing and reopening the
host at that location keeps each slot's settings and private cache. Replacing
the executable there keeps the namespace; another portable/worktree directory
starts a separate group. Earlier generated profiles and ordinary settings remain
untouched. New slots start with defaults; no automatic migration is performed.

A detached viewer keeps its profile occupied until its process exits. Closing
the controller alone does not make that profile free. A replacement worker gets
fresh session/chat authority, so reopening the same slot cannot inherit old
drafts or receipts. Only one controller owns this installation's private registry.
See [the profile and acceptance guide](SESSION_HOST.md) for corruption/ownership
refusals and the required native close/reopen, contention and persistence checks.

This source is a test candidate. Its own build manifest and logs record compiled
checks; native Windows and human grid acceptance remain separate.
Linux full-workspace tests reported 7,846 passes, 49 ignored and the unchanged
terrain precision failure. All 21 new profile/draft/diagnostic regressions passed,
as did formatting and Clippy with the existing Linux dead-code allowance.

## Conversation drafts and usable crash details

Branch `feat/host-conversation-drafts` follows the frozen host/menu source
`8b738ea8b2c700b7538b7e16135a7de19c192d43`. Its chat popouts retain separate
Nearby and existing resident IM drafts when navigating, and retain session UI
state through an ordinary Close/reopen. Each window remains bound to its exact
character, grid, session and lease. Switching the active world does not choose
the sender. Retention is memory-only and ends with authority or host-session loss.

Every snapshot validates parked drafts against their own complete native policy.
Revoked/missing permissions purge affected text and Undo state even while the
popout is closed or minimized. Ordinary history truncation does not erase an
unaffected draft. Failed local queue admission retains text; admitted sends
consume only the matching frozen draft, and uncertain sends are never replayed.
The current one-request receipt behavior remains; this is not shared-chat,
group-chat, Attention or persistence-across-restart parity.

For a crash without a recovered stack, Technical details now shows and copies
the sanitized summary already saved in `public-report.md`, including available
build/GPU/loss information. A recovered raw trace keeps its existing display
and privacy warning. This changes diagnostic visibility, not GPU recovery or the
unresolved cause of the standalone switching crash.

See [the coordination ledger](FOLDURORA_PORT_STATUS.md) for frozen candidates,
check receipts, remaining ports and input needed later. Native Windows popout
focus, close/reopen and clipboard behavior still need acceptance. Only the human
tests real-grid routing and native restrictions; automated fixtures use synthetic
accounts and do not contact a grid.

At this checkpoint Linux formatting and workspace/all-target Clippy passed with
the existing `-A dead-code` platform allowance. Full tests reported 7,836 passes,
49 ignored and the same software-Vulkan terrain-ramp precision failure as before.
All eleven new draft/cache/diagnostic regressions passed. The isolated CPU fixture
passed in EN/FR/JA, generating fourteen captures each; the restored-draft screens
were inspected at normal size and a narrow 150% display scale.

## Host movement and dropdown follow-up

Branch `fix/host-move-and-dropdowns` starts from combined Windows source
`d10473efff52ceaea2d7b22d111319b0efc4f28e`. That earlier source passed its local
Windows checks (7,791 tests, zero failures, 49 ignored), but human testing found
an expanding main menu and a controller-move/detach recovery failure. Those
results do not validate this follow-up; its own check outcomes and exact source
belong in its build manifest.

Searchable toolbar menus keep their content width, with available vertical
room for long results. Host movement now suspends old placement and requires
fresh rendering proof through the normal handoff. Detached windows force a real
visibility transition, then regain their taskbar entry and request redraw.
If a modal or unavailable worker prevents refresh, the safe fallback restores
ordinary windows. Minimizing during an outstanding refresh may also detach;
this is distinct from ordinary minimize/restore requiring Activate.

Linux validation: 7,825 workspace tests passed and 49 were ignored. The only
failure was the previously recorded software-Vulkan terrain ramp precision
check (`0.59764713` versus `0.5984283`); it is outside these changes. All seven
new menu/host regressions passed. Formatting and workspace/all-target Linux
Clippy passed (`-D warnings -A dead-code`, retaining the existing platform
allowance). Demo capture and Windows results must be recorded separately;
these checks do not establish native recovery.

Native acceptance must cover two demo workers: drag the controller repeatedly
and back to its original position, switch after the drag, resize, then test
minimize/restore, explicit detach, normal host Close and host loss separately.
Recovered windows must be visible, responsive and taskbar-listed, at their real
client size without stale smaller contents and black margins. Test held-input
release and exact source-specific chat without replay after these transitions.
The separate graphics-device-loss report during standalone character switching
remains unproven; this follow-up is not a claim to fix that driver/device failure.

## Current combined Windows candidate — 6 November 2026

Branch `build/windows-integration-20261106` combines final host/chat source
`32b298392c7d40afd2d8cc96d0658041b69be985` and the complete viewport correction
`c5076565a3777d27c436fe59d9713907533be0e8`. Main and the earlier checkpoint
branches remain unchanged. Its local Windows checks passed (7,791 tests,
zero failures, 49 ignored), as did build and portable packaging. Human testing
confirmed toolbar containment and freely placed floaters, and found the menu
and host bugs addressed by the follow-up above. Native acceptance is incomplete.
Its exact source, check outcomes and build identity accompany each portable ZIP
in `build-manifest.json` and the workflow logs.

The controller and inactive-account Nearby/existing resident IM popouts are
included. Preview monitors remain unavailable: their uncompiled caller/Scene/UI
patch and aggregate-admission work are not part of this source. Worker profiles
remain private per launch, and chat retains its existing source-bound single
draft and uncertain-send behavior. These are not complete workspace/chat parity.

Fitted workers use their full native rectangle for world rendering, toolbars
and mouselook cursor capture. Their saved standalone viewport is restored after
detach. Two new regressions cover nonzero saved X/Y margins and the ordinary,
fitted and detached transitions. Shared transport, chat identity, private-file
ownership, checked host logging and GPU/input handoff code remain unchanged.

### Selected upstream changes

All seventeen commits below were imported intact with provenance and without
upstream conflict resolutions. The first sixteen were selected in the earlier
isolated upstream checkpoint; the final photo-roll change is new in this review.

| Original upstream commit | Change |
| --- | --- |
| `2944d8016715` | Shared demo-source reads |
| `e8faf65ef9ec` | Source rights for settings/script/item editors |
| `493b519114c3` | Inventory item names in IK projects |
| `cae37649b415` | Shared dialog-name rules |
| `6da2115bd3fe` | Shared asset-ID visibility rules |
| `b4041f7a529f` | OpenSim inventory script-save result |
| `ddb8a69c735e` | Neighbour-region terrain edges |
| `224b01d79cda` | Media-filter exception for worn objects |
| `d981d28600a2` | Login chat import follows saving preferences |
| `b30f7982a118` | Linux stack snapshot stack selection |
| `d37702436d6b` | Session-scoped bridge channels and gaze/RLVa rules |
| `a86f1a3411af` | Land permissions for passage and portals |
| `b962b2a759b2` | Appearance camera and A-Pose controls |
| `85d488a95e23` | Pinned wgpu Vulkan descriptor-allocation fix |
| `113afd8062db` | Asset-fetch ceiling explanation |
| `7afa101cdc6e` | Background close for a stuck microphone |
| `6ff95fb6dc9d` | Per-account/session photo-roll copies and stale-session sweep |

Upstream review is frozen at `c6e52d1ffe06e59176a0d1ef951147d6b4a4340e`.
New changes deferred from this snapshot:

| Original upstream commit | Reason |
| --- | --- |
| `38bab7286b56` | GPU-memory budgeting conflicts with renderer/settings; requires joint ownership and budget review |
| `f114ef820877` | FSR 3.1 conflicts with renderer sizing; full native output dimensions require viewport integration |
| `1d0118d479ad` | README depends on the deferred FSR implementation |
| `74d0a4edb36c` | TASKS conflict; self-contained runtime change can be reviewed in a later explicit resolution |
| `037a71b4d351` | Selection-test and TASKS conflicts; picking/selection behavior needs joint review |
| `f7d31f563e95` | Locale conflicts and changed GPU-recovery defaults overlap worker detach/recreation behavior |
| `c6e52d1ffe06` | Clean text, but turns off existing inventory hover details and thumbnails by default; preserve current behavior for this candidate |

The preceding eighteen deferrals, including asynchronous logging `4a613cd3`,
remain deferred. Their reasons are preserved in the
[earlier upstream ledger](https://github.com/DrCum/Foldurora/blob/a17185fd9b5d9eee1706d5634eba371a894d7711/docs/UPSTREAM_REVIEW_20261106.md)
and [host logging review](https://github.com/DrCum/Foldurora/blob/dc17eb2706ef40f712974544786876cf8952ab0b/docs/handoff/UPSTREAM_FOURTH_REVIEW_20261106.txt).
Only the feature-branch merge needed explicit resolutions: combine workflow
branch lists, retain the corrected viewport task entry, and share the fitted
world override across render/control/cursor paths. No upstream patch was split
or forced through a conflict.

### First native checks for this candidate

Use isolated demo profiles first. Launch `aurora-viewer.exe --session-host` to
test the controller; ordinary launch tests the standalone viewport. Verify
nonzero X/Y viewport margins, toolbar/camera placement and freely movable
Inventory/Preferences windows, then mouselook and detach with saved margins.
Reproduce normal host Close/Cancel/Quit and forced host loss separately, on owned
demo processes, while recording worker visibility/geometry and taskbar state.
The earlier visibility trace was inconclusive. Test normal-cap frame pacing with
worker focus and host-control focus; the previous 15-FPS fixture was intentional.
Held movement/PTT, resize, mixed DPI, display removal and source-specific chat
acceptance remain pending. Only the human performs real-grid login and tests.

## Earlier preview checkpoints and general usage

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
| Hosted character view, combined chat and inactive-character monitors | Separate process windows in the tested mitigation path | Opt-in controller work is tracked in [the host guide](SESSION_HOST.md); active hosting, previews and inactive chat are not accepted yet |

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

Toolbar dropdowns and their submenus keep their normal content width inside
the allowed viewport; they do not stretch to fill it. Long menus and filtered
search results can use the available height and scroll. Check the searchable
main menu after its first frame, open a submenu by hovering, and repeat after
changing the viewport and after detaching a hosted worker.

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
