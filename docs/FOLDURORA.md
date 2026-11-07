# Foldurora workspace and character preview

## Native unread, explicit Read and shutter candidate

The separate `feat/native-read-integration` candidate follows frozen Shared Chat
source `1ef87bf6`. It adds native Nearby and existing resident-IM unread metadata
and an explicit **Mark read** action for the selected source/conversation. Native
badge counts are independent of exported history-line order. Selecting a source,
conversation or active world does not issue this action, and read updates do not
replace a draft or its Undo/IME state.

Read admission captures the source, permissions and observed native incoming/read
revision. A later arrival or native read makes that observation stale; it cannot
clear newer messages. Receipts contain no chat bodies and remain immutable;
uncertainty queries a receipt without replaying the mutation. Controls reflect
native readiness, privacy and mute rules. Both native chat styles retain their
existing read-on-draw timing/defaults. This slice does not establish passive-read
safety, add automatic host reading, Attention, new conversations or monitors.
The separate chat wire protocol is V2 and rejects older chat peers; the control
protocol stays V1. Use a matching executable for controller and workers.

The same candidate includes the completed primary shutter-click follow-up below.
All seven targeted old-behavior regressions were reproduced by runtime assertions
in a compiled full-workspace negative control. Final fixed tests, lint, build,
package and native/human acceptance belong to this candidate's own manifest and
coordination ledger; earlier binary receipts do not transfer.

Human-managed grid acceptance should compare native and Shared Chat counts,
explicitly mark one selected Nearby/existing IM conversation read, and confirm
that a new arrival or native read racing the gesture produces a stale result.
Navigate while a receipt is pending; completion must retire the captured operation
without clearing another draft. Change privacy/mute/source eligibility and verify
text/Undo removal, then check reconnect uncertainty without automatic replay.
Only the human logs in or tests on a grid.

## Shared Chat integration candidate

The `feat/shared-chat-integration` branch applies the first Shared Chat surface
on frozen camera source `6a3009f2`, retaining all of that candidate's camera
production changes. The separate author checkpoint `e6b27e4e` was based on
`388b299c` and does not itself contain the camera corrections. Use the exact
combined commit and version from its own package manifest. Runtime checkpoint `7b2298e8` passed all 139 host/chat checks and seven camera
regressions. Full Linux tests reported 7,925 passed, one inherited terrain GPU
precision failure and 51 ignored; thirty EN/FR/JA production CPU captures passed
required-text/privacy checks. The final package's exact-source build/lint/test
outcomes belong to its manifest and the coordination ledger. Native Windows and
human grid acceptance remain pending. Earlier camera/upstream receipts do not
establish these results.

In controller mode, a character's **Chat** action opens or focuses one shared
CPU window and explicitly selects that character as sender. Its selector shows
the profile and actual account/grid identity. Choosing a character/conversation
neither activates its world nor marks messages read. A source that disappears
stays unavailable until explicitly changed; it never silently chooses another
sender. Only already supported Nearby and existing resident IM conversations
can send, under their current native permissions.

Each exact source owns its drafts, Undo and receipts independently of the
window. Closing/reopening, world switching, ordering or renamed labels must not
move text to another character. Policy/authentication/session loss purges affected
state even while closed. Admission and late receipts update their captured
source; uncertain messages are never replayed. Enter sends from the freshly
focused composer; Shift+Enter adds a line. IME composition, held keys/buttons,
navigation and Close cannot turn into a send to a different target. Closing
retains proven delivered edits before its navigation/focus boundary.

Native acceptance: with human-managed logins, open Chat from two different
characters and confirm the same window changes its explicit Send as selection.
Type distinct drafts in each character and in Nearby/an existing IM, switch and
reopen, then send one controlled test from each source. Switching the active
world must leave the chosen sender unchanged. Check duplicate/renamed labels,
minimized/closed drafts, source disconnection, Enter/Shift+Enter, IME, and a rapid
source switch/Close. Only the human logs in or tests on a grid. Offline synthetic
CPU captures and actual-egui tests are separate automated evidence.

Friends/New IM creation, groups/conferences, native unread/read acknowledgment,
Attention and render monitors remain subsequent stages. The native protocol is
unchanged by this first shared surface. See [the host guide](SESSION_HOST.md).

## Photo viewfinder interaction follow-up

The source-only `fix/shutter-same-pass-sol` follow-up starts from exact frozen
`1ef87bf6`. The offline native case on that binary recorded no export after a
first shutter click, then a 640×360 export after another click following a fresh
hover. Its captures show the same unobscured shutter and MF settings. The log
contains a photo plan/save only for the later click. The case did not record
native event-to-egui-frame boundaries, so coalescing remains a possible cause
of that observation, not a demonstrated native diagnosis.

Source inspection found that a primary down/up pair in one egui pass never set
the shutter's held state and therefore emitted neither focus nor release.
The follow-up accepts egui's completed primary click with focus followed by
release. Held presses and off-button cancellation retain their existing path;
native Enter is already event-driven. Real-egui regressions drive the production
shutter through repeated layout passes and the full viewfinder at ordinary and
fractional scale with both toolbar scopes. Their compiled results and a fresh
native shutter/export check belong to this new candidate. Frozen `1ef87bf6` and
`6a3009f2` receipts remain evidence for those exact earlier binaries only.

The `fix/camera-controls-integration` candidate follows frozen `388b299c`; it is
separate from that verified upstream package and the earlier `f5ae21d5` photo
package. Use the exact commit and version in the new build's manifest. Validation
belongs to that source; the sections below retain earlier checkpoints.

The reported right-half scene remains bounded. This follow-up gives the vertical
dock its own narrow area at the selected control canvas's right edge, bounds the
status header to its actual row, and keeps the guide button and lens ring out of
other controls' mouse areas. Small canvases scroll rather than form negative
bounds. Decorative controls are clipped to their selected canvas; scene guides
and AF overlays still follow the world. The existing toolbar restriction setting
continues to choose world-only or full-workspace controls.

Wheel input over the lens ring, bottom values and camera pad belongs to those
controls. Their outer containers retain scrollbars/touch access without also
scrolling for the same wheel gesture. The dock and ordinary drawers keep their
own scrolling. No camera movement, focal/aperture semantics, renderer, photo
source/output aspect or worker authority is changed.

For the native retest:

1. Use ordinary camera mode with **Free camera** off. In the viewfinder, **Tab**
   opens the quick menu containing that toggle; it defaults off. Over the scene,
   plain wheel should move the ordinary camera, while Alt+wheel adjusts the
   underlined bottom value. Select a middle focal length such as 50 mm so limits
   do not hide a change. Plain wheel retains its existing free-camera exception.
2. Select the focal value, then try wheel over the ring and its minus/plus
   buttons. Open/close Settings, change aperture/focus/a look, then reselect focal
   and repeat. The selected underlined value determines the ring's function.
3. Change the guide from the upper-left control. The upper-right status row must
   not intercept it. Narrow windows may put status on a separate row.
4. Take/save a photo, wait for preview work to finish, and repeat the controls.
   Check both toolbar restriction choices, the right-half world and UI scaling.
   Other floaters continue to use the native workspace.

Automated coverage uses the actual egui viewfinder with duplicate layout passes,
real pointer/wheel events, narrow and very short canvases, fractional DPI/zoom,
drawer/dial changes, control clipping, and restoration after a short viewport.
The corrected regression fails against the old production header's oversized
mouse area; separate checks reproduced the blocked guide and simultaneous
ring/container wheel action before their fixes. Final compiled/runtime results
are recorded in the coordination ledger and candidate receipts, not inherited
from earlier packages. Native Windows and human grid acceptance remain pending.

## Selective upstream catch-up — 7 November 2026 UTC

Author branch `sync/upstream-hud-alpha-20261107` follows exact pushed candidate
`8750450f0161e03fe6ef48194aa58b28d49a2cb9` on
`feat/foldurora-upstream-integration`. That candidate includes author checkpoint
`sync/upstream-safe-20261107`, starting from frozen photo/branding source
`f5ae21d535da3a338bace51ffb03af7beeb040c6`. The upstream snapshot is
[`576dd54a707677bf5b93dd696142cc17fbdef270`](https://github.com/Aurora-Viewer/Aurora-Viewer/commit/576dd54a707677bf5b93dd696142cc17fbdef270),
fetched from `Aurora-Viewer/Aurora-Viewer` main into
`refs/upstream-review/20261107`. The existing `upstream/main` review checkpoint
remains `c6e52d1ffe06`; the shared ancestor is `ae2fd1016ec8`.

The upstream side has 58 commits after that ancestor. Seventeen were already
imported, including photo-roll source `6ff95fb6dc9d` as `ff23f303e29c` (all six
per-file changes match without diff context, although its full-context patch ID
differs). This batch adds sixteen complete upstream commits individually with
`-x`, in the order below, leaving 25 for the explicit follow-up plan. The first
thirteen retain upstream order; two bounded additions follow them without
rewriting those commits, followed by one dependency-unblocked masked-HUD fix.
This is a
selective integration, not a merge of the whole upstream branch.
The sections below retain earlier snapshots; this table and remaining-work plan
supersede their older candidate/deferral status.

| Upstream | Fork commit | Change |
| --- | --- | --- |
| `0dbede28efa1` | `a5de54015426` | Login/session details, explicit critical-message consent and renderer metadata |
| `ed0ae3678bfd` | `723c55f4f22c` | Progressive UI-picture refinement and same-account portrait saving |
| `19645b9935bc` | `1de7fc286743` | RLVa Trash purge exception with worn-item refusal retained |
| `8607d6760503` | `23ab89bf99b4` | Favorites membership/count use each record's own flag |
| `50cdd26ebe8d` | `5583653988a6` | Bounded retry when a switchable-graphics layer hides all adapters |
| `fccd1629a67c` | `b643af68f33a` | Photo-isolation indirect draws and occlusion metadata use the same ordered list |
| `74d0a4edb36c` | `57bb69ccba29` | Atomic account-local teleport history, preserving unreadable/malformed data |
| `1311651d6ee4` | `2a127f02b6b5` | Shared numeric-name ordering across lists |
| `19ca7bcd5047` | `d318bbb8477c` | Explicit IM-toast selection focuses its ordinary App conversation |
| `58200ec8dddf` | `b06e105ad812` | Bounded retry for wanted meshes after repeated failure |
| `84199e610bcb` | `ae221e32f59a` | Distinct menu icons and removal of an inactive placeholder |
| `9751891878a3` | `30af203f7bda` | Blocked-person hover row keeps its layout gap |
| `576dd54a7076` | `7426f56b62ad` | Outfit faux-bold text uses the scaled row origin |
| `c6e52d1ffe06` | `1936ec42d178` | Inventory hover master preference, intentionally off by default |
| `05180f17a58b` | `b3d4512ecb2d` | Creators' permissions guidance, followed by the explicit fork qualification |
| `07ff8662d325` | `4ef11364915a` | Restore masked-HUD mip alpha monotonically and regenerate affected coverage-cache entries |

All production and non-tracking-document patches applied without conflict,
including as a complete sequence against the exact base. Five `TASKS.md`
conflicts retained the fork's sections and the upstream patch's changed entries;
unchanged context about unimported media consent/uninstall was not introduced.
No production source was manually resolved or partially extracted. The existing
host/profile/chat authority, viewport/photo scope and branding remain present.

The hover preference intentionally adopts upstream's ordinary default change:
new or previously missing `show_hover_tooltips` is false, so text/picture hover
cards require opting in. The existing thumbnail preference remains, gated by
that master switch. The permissions-guidance commit is preserved intact with
`-x`; the following fork note in `AGENTS.md` distinguishes available helpers from
the five deferred API targets. It does not assume those implementations exist.

The masked-HUD patch was re-screened on exact `8750450f` after the UI-image/cache
refinement import removed its former production-source conflict. Only tracking
text required reconciliation. Alpha coverage can now increase mip alpha but
never reduce it; old coverage code 3 entries regenerate as code 4. Cache
`VERSION=5`, pick-mask planes and profile IO remain unchanged. Included tests and
the existing `mask-cutoff`/`mask-cutoff-zoom` demo modes require validation on
the final combined source; this follow-up does not inherit its parent's checks.

Source/dependency review and `git diff --check` passed at the author checkpoint.
Compiled checks and runtime evidence belong to the exact candidate's manifest
and the coordination ledger. Imported task descriptions retain historical
**upstream** tests and human observations; they do not establish new Foldurora
checks or grid acceptance. Validation of this integrated source includes
login/consent reset, same-source images, rights/Favorites/sort,
teleport-history IO, mesh retry, adapter-retry plans and photo isolation. Affected
AMD/NVIDIA hardware and human grid behavior remain unverified. The history
writer's 500 ms shutdown bound preserves atomic files, not guaranteed latest-save
completion. Keep the frozen base and earlier test packages unchanged.

### Remaining upstream integration plan

These 25 commits are deliberately outside this batch. A clean text patch alone
does not establish compatible behavior or complete dependencies.

| Group | Commits | Concrete integration work |
| --- | --- | --- |
| Picking and selection | `c6ffeeb705aa`, `5e65df245f52`, `7851d13e61b3`, `037a71b4d351` | The own-avatar correction can stand alone. Integrate alpha click-through with its corrective invisible/Mask-face follow-up, preserving fork ray/epoch guards; reconcile selection tests and verify HUD/worn/world plus offset/fitted/photo coordinates. |
| Cache/rights and texture operations | `026651a845eb` → `316e5e067499` → `1ae9a375f069` → `3ce55eba155e` | Preserve profile-before-key initialization and private stores. Validate UUID-name migration, distinct worker keys, restart, sealed/plain demo sources and texture permissions. Replace the explicit pending-target qualification in `AGENTS.md` only when those APIs exist. |
| Selected object-contents copy | `54df367c7019` | No identified dependency on the cache/rights batch. On `8750450f`, production source is text-clean; the non-tracking conflict is only `ARCHITECTURE.md`, alongside `TASKS.md`. Reconcile those documents and retain source-rights, selected-item, no-copy, worn-item and approval regressions. |
| Browser native file pickers | `ae10170eb635` | Add owned-modal/source visibility and host-park handling; validate browser replacement/close and switching with a pending picker. |
| Media consent and camera migration | `db6209901611` → `275e3808b951` | Report the actually shown World media-consent Modal to host blocking without deadlocking inactive workers. Validate per-profile consent and deliberate camera-default migration; README follows working runtime support. |
| Runtime updates | `e37ac4180630` | Resolve published-fork release-feed identity and multi-worker installation ownership; test loopback/stale-generation/notice/contended-install behavior. Development gating does not settle published behavior. |
| GPU startup | `e1e44ed74bce` | Extend the imported adapter fix with the XR constructor argument while retaining the fork viewport argument, worker startup ordering and recovery recreation. |
| GPU budget, recovery and peer pause | `38bab7286b56`, `f7d31f563e95`, `eb875bc9f55c` | `38bab728` is now text-clean, but changes GPU-budget behavior and still needs a dedicated behavioral/memory validation plan. Preserve host display invalidation, fresh surfaces, proactive pause/hysteresis and isolated registries; assess defaults and shared-device memory behavior together. |
| Async logging | `4a613cd3e6e3` | Preserve checked durable controller startup/shutdown and sink-error propagation; ordinary queued-write drain is not a checked HostLog barrier. |
| FSR | `f114ef820877` → `bb4d4fc7f49c` → `1d0118d479ad` | Reconcile native canvas, bounded world, scaled rendering and photo outputs without weakening device/surface fences. Add FSR documentation only after the feature works. |
| Reflection probes | `a44ef04e1796` | Substantial CPU/WGSL ABI, culling and scan changes raise default slots from 32 to 256. Validate memory/quality, constrained GPUs, photo outputs and inactive-worker packets in a dedicated renderer batch. |
| Uninstall | `3d11510600b2` | Define fork installation/profile scope before exposing shared OS registration/cache/credential removal to private workers. No uninstall action is authorized by this source review. |
| Release news and task archive | `1378da4254f1`, `3bf52dacbc4a` | Reconcile only truthful implemented-feature claims and preserve fork task ownership; do not advertise deferred FSR/consent features or archive unfinished fork work. |

## Profile picker boundary follow-up

Branch `fix/profile-popup-bounds` follows the combined profile/chat source
`8cd9a2aa005491b651e5bb8591d94fd6fb891520`. Visual review found that the
selector's outer frame crossed the roster/world boundary by its popup margins.
Windows removes the world rectangle from the controller's native window, so
those extra pixels would be clipped. The picker now respects its inner width
budget, and occupied-profile guidance wraps inside the roster instead of using
a tooltip over that cutout. Long names remain truncated.

The added regression measures the actual popup Area, including its frame,
against the roster/world boundary at narrow sizes, fractional zoom and after
reopening narrower. This changes host UI geometry only. Check outcomes and exact
source accompany the candidate; native Windows acceptance remains pending.

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
when its interface is hidden. Opening photo mode also keeps its live scene,
guides, masks, frozen preview and focus marks in that world rectangle, even
when control restriction is off. The chosen photo dimensions/aspect remain
separate: clean exports frame the bounded live view at that output aspect;
Interface exports keep the whole window and its workspace overlays, with the
scene in its original relative position. Interface output needs matching window
proportions and a size that can be drawn in one pass; incompatible requests show
an actionable message. Resizing the native canvas or changing interface scale
during an Interface capture cancels it so it can be retaken. VR and hiding the
interface outside mouselook retain their explicit full-scene behavior. Native
cursor confinement/capture still requires separate acceptance.
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
10. With control restriction both on and off, open the photo panel and
    viewfinder. Keep ordinary chat/Inventory windows outside the world. Check
    rule-of-thirds guides, crop masks, held previews, focus points and face
    brackets at each world edge; none may spill into the workspace. Save clean
    portrait and wide photos and compare their edges with the live guide.
    Save an Interface photo with Window aspect and confirm the complete native
    canvas, workspace floaters and bounded scene keep their proportions. Try a
    large Interface output, a nonmatching aspect and a resize/zoom during capture;
    expect a clear message without a partial or silently clean image. Restore
    the default full-window viewport and repeat ordinary photography.

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
