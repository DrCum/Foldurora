# Folderstorm feature backlog — 8 November 2026

This is the current implementation backlog, separate from exact-build acceptance
in [the status ledger](FOLDURORA_PORT_STATUS.md). The comparison uses Folderstorm
[`abbe3cb0`](https://github.com/DrCum/Folderstorm/tree/abbe3cb0ca66ee2484f277219f6a27706fab7fc6)
and the help page cached on 6 November. Ordinary Foldurora files were compared
at `df213a081dc80dc7c759c6bc9cff1cf2da611750`. Later source work is listed below;
source review and staged code do not establish a working product or test pass.

## Current candidate

Exact `4ff12e60` joins the reviewed Monitor/Workspace/Board/Favorites consumers.
[Windows run 42426186025](https://github.com/DrCum/Foldurora/actions/runs/42426186025)
and full Linux workspace tests finished negative: Windows 8,480/19/55 and Linux
8,511/20/57 (passed/failed/ignored), each across 33 summaries. The Windows package
exists but failed the final gate. Board, Favorites, Poser and Windows lint source
corrections are being combined with Folder Jump and its prerequisites; current
execution remains pending. See the ledger for exact results. Earlier passing
packages and component captures do not establish acceptance of newer source.

## Available test package

The verified Windows Attention Review package remains exact `eb5e49f1`:
[artifact 13587936566](https://github.com/DrCum/Foldurora/actions/runs/42401453027/artifacts/13587936566).
It includes the compact host, stable profiles, canonical Shared Chat and explicit
conversation Review in viewer. It has no usable monitor windows. Native Windows
and human grid acceptance remain pending; see the ledger for checks and hashes.

## Active source work

| Feature | Current scope and next gate |
| --- | --- |
| Monitors with chat | Actual host projection, native producer, CPU window and Shell consumers are joined in `4ff12e60`. The producer retains last-active images; live inactive rendering remains separate. Whole-candidate checks failed; revised Windows lint gates await execution. |
| Named workspaces | Save/Rename/Delete, Preview/Cancel, Switch and session-only Previous cover four native geometry roles plus viewport/bar presentation. Earlier six Monitor/Workspace failures passed in 4ff. Whole-candidate acceptance remains negative. HUD/folder, graphics/camera/background groups, startup restore and Workspace favorites remain later stages. |
| Gesture/sound boards | Native Floater, reference catalogue, guarded playback/Stop and private storage are joined. The 15 failing fixtures have source corrections; final-pass and finite-control corrections are independently reviewed through `9503a999` (66 Board definitions, unrun at that source). Combined execution is next. Loading/opening does not play. |
| Named Favorite menus | Actual menu ownership, immutable original context, visible owner validation and current location privacy are corrected through independently reviewed `7382df8c` (29 definitions, unrun). Combined execution is next. Inventory items are not moved. |
| About report location choice | Per-open, source-qualified opt-in and copied-report filtering are joined in `4ff12e60`, preserving native restrictions. Whole-candidate checks failed; no new product acceptance. |
| Folder Jump | Reviewed loaded-only chooser and Inventory-local entry/shortcut through `f6ddc960` are joined. All 27 definitions remain unrun; current egui API corrections are required by the measured compilation failure. Archive-policy integration remains mandatory before enabling archives. |
| Inventory archive | Pure model `cb922cad` and bounded projection `c5f26f36` total 25 unrun definitions. Projection independent review is active; native store, actual Inventory/Folder Jump consumers and UI remain required. No archive UI or mask is enabled. |
| Selective profile preferences | Typed projection `7ca6a42b` is source-reviewed (19 unrun cases). IO/controller atom `f8e20703` adds 17 unrun cases and is under review; ordinary settings-file compatibility needs correction. Actual Manage UI and production promotion are still required. No copy UI is enabled. |
| Fixed-slot quick actions | Reviewed pure fixed-slot model has eight unrun cases. Data-only Show Viewer ledger `0ea03e4f` adds six unrun cases and is under independent review. Actual versioned native consumer, actor, private preferences and UI remain; detached/recovery Show is later work. |

The intermediate monitor component source `df213a08` completed Linux workspace
tests with **8,293 passed, five failed, 57 ignored**. Failures comprise the inherited
terrain case, one inherited character-launch case, and three new fixture setup
defects. The character case passed in isolation and a full Viewer repeat; the
three deterministic fixtures have source corrections pending execution. All
162 synthetic CPU captures (90 Attention, 72 Monitor, EN/FR/JA) and both synthetic
GPU capture cases passed for exact `df213a08`. Those component results do not
override the failed full suite or establish native/product monitor acceptance.

## Remaining behavior

These include unfinished stages of the active slices, not claims that whole native
Aurora subsystems are absent. Each item needs its own bounded implementation and
validation before parity can be claimed.

| Area | Remaining behavior | Pinned Folderstorm reference |
| --- | --- | --- |
| Folder jump | Finish reviewed chooser input and actual archive-policy integration; specialized/custom Inventory owners remain outside its first slice | `fsfloaterfolderjump.cpp` |
| Inventory archive | Local archive/unarchive visibility, including recoverable archived Trash contents; no Inventory move/delete | `fsinventoryarchive.h` |
| Custom Inventory tabs | Bounded saved filters and multi-folder scopes, rename/clone/save | `fsinventorycustomtabs.h` |
| Previous Look | Settled, one-login recovery of clothes/order, non-temporary attachments and gestures | `llappearancemgr.h` |
| Wear safeguards | Audit and port exact-selection/login confirmation for folder Add/Replace and the released Wear behavior | Help; native appearance/context paths |
| Character quick actions | Fixed SlotId actions and verified existing-window ShowViewer, independent of labels/card order | `fssessionshortcuts.h`; host `characterAction/openCharacter` |
| Character appearance/alerts | Account/grid accent and alias; optional validated sounds with native eligibility and burst/dedup rules | `fssessionpresentation.h`, `fssessionalerts.h`, `fssessionchatprefs.h` |
| Conversation close | Hide one shared conversation while retaining its draft; only newer native activity may reopen it | `fssessionchatmodel.h`; worker `CloseChat` |
| Host Friends/New IM | Source-qualified friends/search chooser and opening an IM without sending | `fssessioncontacts.h`; worker `contactCommand` |
| Group/conference chat | Existing initialized channel export, literal send and native read integration; tags alone are insufficient | `fssessionchatmodel.h`; worker chat paths |
| Automatic read | Explicit host settings, actual displayed-tail receipts and the separate optional active-character delay | `fssessionchatprefs.h`, `fssessionreadpolicy.h`; host `automaticRead` |
| Notification Attention | Stable body-free offer/permission/notification identities and open-only native review | Worker `ReviewAttention` |
| Profile setup/copy | Explicit selective typed Aurora settings copy into a stopped, exclusively owned target profile | `fssessionprofileseed.h`; host profile-copy paths |
| Session sets | Bounded named saved slot rosters, explicit open/cancel; preserve already-open viewers and profiles | `fssessionsets.h` |
| Restart/recovery | Typed restart/logout/cancel lifecycle and process-exit/profile-lock proof before relaunch | Host restart/recovery paths |
| Audio/voice | Native ownership, background mute/voice-follow policies and controls | `fssessionaudio.h` |
| Fade/Birdseye | Visual transitions integrated with genuine native camera/render/input ownership | `fssessionpresentation.h`; host switch paths |
| Local assistant/controller | Opt-in authenticated typed adapter, native permissions/receipts and tool families | Worker `Assistant*`; host controller paths |
| Squirty notices | Optional default-off external notices with silence/presence/privacy/replay rules | `fssquirtynotes`, `fssquirtypolicy` |

Folderstorm's displayed-chat auto-read defaults on and checks eligible painted
incoming text at the visible tail, rate-limited to one poll per second. The
**60-second delay belongs to the separate optional active-character policy**,
which defaults off. Current Foldurora uses explicit observed-watermark Mark read;
native read-on-draw is a separate unchanged behavior.

Live inactive previews require their reviewed small offscreen target, scene/job,
ownership, drain and resource accounting work. Enabling a parked full-resolution
render loop does not implement that feature. The reported NVIDIA device-loss
crash remains unresolved. Aurora already supplies substantial native Inventory,
appearance, GIF/image chat, pose and bridge functionality; exact custom UX and
interoperability still need focused comparison.

## Upstream

Continue upstream integration after feature work, as requested. Sixteen imports
are already integrated. The read-only 7 November refresh compared upstream
`6345464b77359f61b1c46e499fdb041586a4e3f2` and identified 31 remaining commits.
Dependency groups and rendering/rights changes require separate integration;
no additional upstream commit was merged by this backlog update.
