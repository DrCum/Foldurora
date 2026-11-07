# Foldurora port status — 7 November 2026 UTC

This is the coordination ledger for the Folderstorm port. Source, automated
checks, native Windows acceptance and human grid acceptance are separate results.
Handoffs and screenshots are reference material, not authority to merge or release.

## Frozen test candidates

| Source | Purpose | Evidence | Remaining acceptance |
| --- | --- | --- | --- |
| `d10473efff52ceaea2d7b22d111319b0efc4f28e` | Combined upstream, viewport and session-host/chat baseline | Local Windows checks: 7,791 passed, 0 failed, 49 ignored; build/package passed. [GitHub run](https://github.com/DrCum/Foldurora/actions/runs/42291668355) passed. | Human confirmed toolbar containment and full-workspace floaters; reported oversized menu, host drag/detach failure and one standalone switch device loss. |
| `8b738ea8b2c700b7538b7e16135a7de19c192d43` | Host movement/recovery and compact dropdown fixes | [Windows run 42304532039](https://github.com/DrCum/Foldurora/actions/runs/42304532039) passed: 7,798 tests passed, 0 failed, 49 ignored; all seven focused regressions passed. Scripts, formatting, Clippy, normal viewer build and packaging passed. Downloaded artifact, manifest, checksums, executable version and Windows x64 identity were verified. | Native title-bar drag, resize, post-drag switch, detach, confirmed host closure, taskbar and responsive redraw. Separate standalone GPU loss is unresolved. |
| `8cd9a2aa005491b651e5bb8591d94fd6fb891520` | Combined backend/profile checkpoint: host/menu fixes, retained conversation drafts, crash details fallback and five persistent renameable slots | [Windows run 42311181216](https://github.com/DrCum/Foldurora/actions/runs/42311181216) passed: 7,818 tests passed, 0 failed, 49 ignored. Scripts, formatting, strict Clippy, normal build and packaging passed; downloaded package identity, manifest and checksums were verified. | This checkpoint has a known profile-popup margin overlap corrected by `70c1b5cd`. Native host/profile/chat and human grid acceptance remain pending. |
| `70c1b5cda927547b28e5a5ff9713e861771aaee7` | Latest corrected candidate, `fix/profile-popup-bounds`: bounds the profile popup including its frame and wraps occupied-profile help inline | [Windows run 42315300611](https://github.com/DrCum/Foldurora/actions/runs/42315300611) is in progress. Linux formatting, Clippy, the new outer-popup regression and normal build passed. Fresh EN/FR/JA Linux CPU UI and sequential demo-worker evidence is recorded below. | Exact-source Windows check/build/package result and offline native Windows host/profile/chat acceptance remain pending; human grid and physical GPU acceptance are separate. |

Keep these sources and their packages separate. No candidate is merged, installed
or released by this work. The runtime branch remains frozen while this ledger is
updated on `docs/foldurora-progress-20261106`; documentation changes do not replace
or restart its validation run.

The verified host/menu [Windows artifact 13548583765](https://github.com/DrCum/Foldurora/actions/runs/42304532039/artifacts/13548583765)
contains `Foldurora-test-8b738ea8b2c7-windows-x64.zip`. Its manifest names the exact
`8b738ea8` source, successful checks/build/package and pending human, Windows GPU,
multi-monitor and grid validation. The executable and PE version strings agree on
`0.9.3-beta.20261106.1+110.g8b738ea8b`; its machine identity is AMD64/PE32+.

- Downloaded artifact SHA-256: `899ca31cfe316081bdf39483166afa1c2331ad0a2b2c8756362be73114199325`.
- Inner portable ZIP SHA-256: `1b1b7f46ee8810d4b83de7c7bee05210d7e8971c2b2ad830b2da3f78dc46fb7c`.
- Executable SHA-256: `533a3b92217288bf7cac2e366585623c95e9e52fce88511e1a94b5dafe1380bd`.

The retained receipt is the candidate worktree's
`.scratch/windows-ci-42304532039/receipt.json`, alongside the downloaded manifest,
artifact and raw logs. An earlier CLI download was blocked; the later connector
download and content verification completed successfully. Native behavior is not
established by those checks.

The verified backend/profile [Windows artifact 13550927031](https://github.com/DrCum/Foldurora/actions/runs/42311181216/artifacts/13550927031)
contains `Foldurora-test-8cd9a2aa0054-windows-x64.zip`, built for
`x86_64-pc-windows-msvc` in the development profile with `xr-static-loader`.
The manifest names exact source `8cd9a2aa`, successful checks/build/package and
pending human, GPU/multi-monitor, grid and performance acceptance. Executable
and PE version strings agree on `0.9.3-beta.20261106.1+118.g8cd9a2aa0`, with
AMD64/PE32+ identity. All seven host/menu and 20 applicable profile/draft/diagnostic
regressions passed; the Unix-only linked-path regression is absent as expected.

- Downloaded artifact SHA-256: `aba9afbcebdcda0a0f3be6407a2f30a4ec43aa9297a6febd4684f6a0efd3dbcd`.
- Inner portable ZIP SHA-256: `5bbf8afa07904521eda9ea7598f37c2b9b55764d984527a64efae39909fa90ba`.
- Executable SHA-256: `086e0724ac515cf9cc050eeee4c36c6c4cae49c71de1593c8490ee1d4028fe5b`.

Its retained receipt is `.scratch/windows-ci-42311181216/receipt.json` in the
candidate worktree. The known popup-margin follow-up is not included in this
verified package; use the separate `70c1b5cd` candidate for the corrected UI.

## Implemented in the combined source

The combined source provides five stable, renameable character slots.
Each slot keeps a private settings/cache location
across normal restarts from the same executable location, with exclusive profile
ownership and fresh worker/session authority at each launch. New profiles start
with defaults; earlier generated profiles are preserved. This is implemented
source, with native Windows worker/profile acceptance still pending.

It also provides separate drafts for Nearby and existing resident IMs,
retained while navigating and closing/reopening the same character chat popout.
Drafts stay in memory under the immutable worker/account/grid/session/lease source.
Permission or source loss purges affected text and editor undo state. Local queue
rejection retains text; admitted/uncertain sends are never automatically replayed.
The selected conversation, draft and receipt survive ordinary popout Close/reopen;
a failed native window creation preserves cached UI for another explicit attempt.
This does not add a shared sender selector, group chat or draft persistence across
host restarts.

A small crash-window fix also shows and copies, when no stack is available,
the sanitized diagnostic summary already generated as
`public-report.md`. It adds no collection, transmission or GPU-recovery change.

The latest UI correction keeps the profile popup's full frame within the roster,
including narrow windows and long labels. Occupied-profile guidance now wraps
inline beside the disabled Open action. The earlier popup's outer margin crossed
the world cutout and could be clipped by the native Windows region; backend
profile ownership and session authority are unchanged by this correction.

## Combined validation evidence

The earlier checked Rust source is `5c741b6103328fb32813aca3a462ea930a9c441b`, including the
source-reviewed five-slot implementation from
`d08441453aa756b7b5668a4d5f21edf20e036d04`. Its Linux full-workspace run reported
**7,846 passed, 1 failed, 49 ignored**. The failure is the unchanged software-Vulkan
terrain-ramp precision case (`0.59764713 != 0.5984283`); this is not an all-green
workspace result. All **21 new profile/draft/diagnostic regressions passed**.
Formatting and workspace/all-target Clippy passed with the existing Linux
`-A dead-code` allowance. The actual normal viewer binary built in 4m31s and
reports `0.0.0+1491.g5c741b610`.

The EN/FR/JA CPU chat fixture generated **42 captures**. The six changed
draft-restoration screens were inspected at normal and narrow/150% sizes. These
drive the actual chat UI and software painter with synthetic data; they do not
open a native chat window or prove Windows behavior. Retained evidence is under
`.scratch/chat-drafts-{en,fr,ja}/` in the candidate worktree.

The actual CPU crash-helper window was also opened with a synthetic device-loss
record, and `.scratch/crash-details-fallback.png` was inspected. The fallback
summary displayed successfully. A separate egui clipboard regression verifies
the copy payload; native Windows clipboard behavior and a real GPU crash remain
untested by this fixture.

The actual native Linux CPU controller/profile UI smoke passed in EN/FR/JA on
that same `5c741b61` binary. All five numbered slots were visible, localized
Rename dialogs fit and Japanese glyphs rendered. A short ASCII label was typed
and saved through the UI in each locale. Confirmed normal closure followed by a
restart from the same executable location and private XDG roots retained each
label in both the UI and its owned registry. All six host exits were successful;
the driver and Xvfb wrapper finished. No Open login action, workers, world
renderer, grid or ordinary profile data were used. The receipt is
`.scratch/host-profile-visual/RESULTS.md`, with six inspected captures per locale
under `.scratch/host-profile-visual/run-ieeu1k9Y/{en,fr,ja}/`.

Xvfb's stale front buffer required two Expose flushes after an action before
captures reflected the current UI. An earlier apparent Rename failure was
withdrawn after correcting that observation protocol; no viewer defect or fix
is claimed from that false alarm. This smoke does not establish Windows hosted
world, taskbar, GPU coexistence or human grid behavior.

Backend checkpoint `8cd9a2aa` differs from that checked binary only in six
plain-language profile-name help/error message values across EN/FR/JA. Its
exact-source Windows package is verified above. The preceding
[`5c741b61` run 42310599389](https://github.com/DrCum/Foldurora/actions/runs/42310599389)
was automatically cancelled when the text-only follow-up was pushed. It supplies
no completed validation result and is not a failed feature-validation run.
The candidate worktree's `.scratch/profiles-validation-8cd9a2aa.json` records this
source/evidence distinction.

Latest `70c1b5cd` Linux full-workspace validation reported **7,847 passed, 1 failed,
49 ignored**, including the new regression that measures the actual popup Area
under normal/narrow widths, zoom, long labels and removal of stale guidance.
Formatting and workspace/all-target Clippy passed with the existing Linux
`-A dead-code` allowance. The normal viewer built in 2m46s, reporting
`0.0.0+1494.g70c1b5cda`; its SHA-256 is
`ab4a01e7f149fc8559d37d54b3bd040dc6c70941a978273184e609e43b1c13ff`.

The one Linux failure remains the unchanged terrain-ramp test. The exact
`0.59764713` result is consistent with 8-bit UNORM sampling precision rather
than the ideal-float reference `0.5984283`. The test and renderer are unchanged
across these candidates. The exact driver cause and any production visual impact
are unproven; no tolerance or renderer change was made and the workspace result
is not all green. The Windows test can return successfully without a usable
adapter, so its automated result alone is not physical GPU acceptance.

Fresh `70c1b5cd` native Linux CPU UI checks passed in EN/FR/JA, including current
plain-language help, all five slots, actual Rename/Save and normal restart
persistence. The popup frame stayed inside the roster at 1240×800 and 760×600,
including a 64-character ASCII label. A faint one-level RGB shadow extended two
columns beyond the frame; this is not a zero-outside-pixel claim. A simulated
private slot-5 lock showed wrapped help and disabled Open, then normal
availability after release; this case launched no worker. All seven hosts exited
normally and all owned driver/display processes finished. Receipts are
`.scratch/host-profile-visual/RESULTS-70c1b5cd.md` and
`run-NdHjIHbH/validated-summary.json` beneath that evidence root.

A separate real application IO smoke used sequential offline demo workers on
`70c1b5cd`, with private XDG roots, low graphics, 960×640 windows, a 15 FPS cap
and software Vulkan. Slot 1's actual Clock seconds preference changed from on to
off, saved to its private JSON, and remained off after confirmed Quit, actual PID
exit and reopening the same slot. Its cache-key file retained the same path and
metadata; only metadata was inspected. Slot 2 then retained its separate default
on setting and distinct cache path/key-file inode while slot 1 stayed off.
Slots 3–5 gained no settings files. Explicit Linux Detach left the viewer usable
and the profile occupied until its actual process exit. All three workers and
both controllers exited normally; no forced cleanup or device-loss marker was
needed. The receipt is
`.scratch/host-profile-visual/RESULTS-WORKER-70c1b5cd.md`, with
`worker-run-tKsGOMYz/validated-worker-summary.json` beneath that evidence root.
This establishes slot-1 retention and sequential slot-1/2 separation, not five
simultaneous sessions, sealed-cache content reuse, Windows embedding/taskbar,
physical GPU coexistence, audio/voice or real accounts. No grid, credentials or
ordinary profile data were used.

The latest actual crash-helper capture, `.scratch/crash-details-70c1b5cd.png`,
also displayed the no-stack summary from a synthetic report. It does not
reproduce a real graphics reset. The earlier 42 chat captures remain source
`5c741b61`; that chat component is unchanged through `70c1b5cd`, but those images
are not captures of the latest complete UI.

## Selected visual evidence

Seven reviewed synthetic PNGs are retained on the dedicated `captures` branch,
preserving its existing images and history. The
[capture README and provenance](https://github.com/DrCum/Foldurora/blob/b3e9771643403f51fac92ef441d30935d422ee21/foldurora/20261106-70c1b5cd/README.md)
records source versions, fixture methods and limits. These raw links are pinned
to that capture commit; they do not depend on the cloud `.scratch` directories.

| Image | Raw PNG |
| --- | --- |
| `70c1b5cd`: Japanese five-slot selector | [Open](https://raw.githubusercontent.com/DrCum/Foldurora/b3e9771643403f51fac92ef441d30935d422ee21/foldurora/20261106-70c1b5cd/01-ja-selector-open.png) |
| `70c1b5cd`: narrow popup with a 64-character label | [Open](https://raw.githubusercontent.com/DrCum/Foldurora/b3e9771643403f51fac92ef441d30935d422ee21/foldurora/20261106-70c1b5cd/02-en-narrow-long-label-popup.png) |
| `70c1b5cd`: narrow occupied slot, simulated fixture lock | [Open](https://raw.githubusercontent.com/DrCum/Foldurora/b3e9771643403f51fac92ef441d30935d422ee21/foldurora/20261106-70c1b5cd/03-en-narrow-simulated-occupied-slot5.png) |
| `70c1b5cd`: real demo slot 1 after preference restart | [Open](https://raw.githubusercontent.com/DrCum/Foldurora/b3e9771643403f51fac92ef441d30935d422ee21/foldurora/20261106-70c1b5cd/04-slot1-reopened-clock-preference.png) |
| `70c1b5cd`: real sequential demo slot 2 default | [Open](https://raw.githubusercontent.com/DrCum/Foldurora/b3e9771643403f51fac92ef441d30935d422ee21/foldurora/20261106-70c1b5cd/05-slot2-clock-preference-default.png) |
| `70c1b5cd`: actual helper with a synthetic crash report | [Open](https://raw.githubusercontent.com/DrCum/Foldurora/b3e9771643403f51fac92ef441d30935d422ee21/foldurora/20261106-70c1b5cd/06-crash-details-synthetic.png) |
| Earlier `5c741b61`: restored Nearby draft, unchanged chat component | [Open](https://raw.githubusercontent.com/DrCum/Foldurora/b3e9771643403f51fac92ef441d30935d422ee21/foldurora/20261106-70c1b5cd/07-chat-nearby-draft-restored-5c741b61.png) |

## Port order and dependencies

| Feature | Current gap | Next concrete work |
| --- | --- | --- |
| Five stable, renameable character slots | Implemented and compiled; `8cd9a2aa` Windows package verified. Corrected `70c1b5cd` Linux UI rename/restart and actual sequential slot-1/2 IO passed. Earlier host/menu packages used per-launch profiles. | Verify corrected Windows package, five-slot persistence and labels, exclusive ownership through detach/host death, and fresh session authority on reopen. |
| Per-character/per-conversation drafts | Memory-only navigation and Close/reopen retention is implemented and passed synthetic checks. | Native popout focus/reopen and human grid routing/restrictions remain to be checked. |
| Shared chat and Attention | Source-bound popouts exist; shared sender selection and explicit review/mark-read/dismiss behavior are incomplete. | Use the native conversation identities and restrictions; navigation must never mark a conversation read or change the sending character. |
| Named workspaces | Viewport and native floater geometry exist; selective named arrangements, previews, favorites and HUD restore are incomplete. | Map Folderstorm's restore groups onto Aurora's typed settings/actions after stable character storage is established. |
| Fade and Birdseye switches | Native activation exists; Folderstorm's visual transitions are not ported. | Preserve fenced GPU/input handoff; design each transition around native camera/render actions after host acceptance. |
| Inactive monitors | Reduced-target caller/scene/UI and aggregate admission are unfinished. | Complete actual small render targets and latest-only delivery under a shared acknowledged/pending budget; models alone are not a working monitor. |
| Assistant and other tools | Aurora has native inventory/appearance/camera actions; Folderstorm's adapter and some tools remain. | Port bounded typed actions after the supporting account/workspace models, rather than generic input injection. |
| Profile import/migration | Stable new slots exist; migration of old generated profiles or ordinary viewer preferences is not implemented. | Decide explicit import/copy behavior later; keep credentials, writable caches and live-session authority separate. Moving installations does not silently migrate profiles. |

Stable profile labels are display names, not account identity. The actual logged-in
account must remain visible beside a custom label. Chat drafts must never be keyed
by a slot label or carried into a newly authorized session merely because the slot
is the same. A detached worker retains its profile lock until actual process exit.
The stable namespace remains tied to the canonical executable path; persistence
tests must restart the host from the same location. Another portable folder is a
separate installation for these profiles.

## Local Windows agent next steps

The local builder can use its Computer Use capability and existing desktop
permission for offline acceptance. This cloud session has no callable
send-to-thread/handoff tool; the prepared request remains undelivered. The
following recipe is durable even if its `.scratch` copy is unavailable. If the
desktop or existing permission is unavailable, report that boundary without
touching unrelated windows.

1. Prefer exact source `70c1b5cda927547b28e5a5ff9713e861771aaee7` from
   [Windows run 42315300611](https://github.com/DrCum/Foldurora/actions/runs/42315300611)
   once its package is ready. Verify manifest checks, source/version suffix and
   SHA-256 before testing. Preserve the separate `8b738ea8` focused package and
   `8cd9a2aa` backend/profile checkpoint; do not mix their receipts. Keep the
   same executable location throughout persistence tests. If building locally,
   use its own default-dev worktree, normal build-slot rules, `scripts/check.ps1`,
   `cargo build --locked -p aurora-viewer --features xr-static-loader` and
   `scripts/package.ps1`. No cleanup scripts, installation, release or source fixes.
2. Use fresh owned test data and a fresh PowerShell process with `AURORA_DEMO=1`,
   `AURORA_GRAPHICS_PRESET=low`, `AURORA_WINDOW_SIZE=960,640` and
   `AURORA_FPS_LIMIT=15`. Leave login/exit/capture/profile/forced-loss overrides
   and `AURORA_LOG_NAME` unset; do not use `AURORA_UPDATE_TEST_ROOT`. Before
   starting, check only whether
   `%APPDATA%\Aurora\AuroraViewer\config\demo\login_accounts.json` exists.
   If this would change unrelated prior data, stop and report it; do not read,
   rename or delete that data. Use only a fixture whose ownership is established.
   Launch `--session-host --language en` with a source-tagged
   `Test host native 70c1b5cd [AUTOMATION - DO NOT TOUCH]` title and the repository's
   automation border helper. Track the owned executable PIDs/HWNDs and the host's
   actual child workers; worker titles are not inherited. Never select saved
   accounts, enter credentials or submit a grid login.
3. Open two generic demo workers and identify them by owned process/session,
   since visible synthetic names can match. Switch A/B/A. Drag the host's actual
   native title bar with real down/move/up, first a short distance, then away and
   back; release promptly. Capture settled positions, switch again and resize.
   Verify input/cutout alignment, release of held movement/camera input across a
   switch, usable host chrome and compact searchable menus/submenus. Physically
   release all input after the check. Stop on device loss or a persistently
   hidden/unreachable worker; preserve the failing checkpoint rather than retrying.
4. Detach A, then normally close the host with its Detach sessions and close
   action. Both workers must remain visible, responsive ordinary windows with
   native captions/taskbar access and correct client contents immediately and
   after lease fallback. Normally Quit only these owned workers, confirm prompts
   and verify actual PID exit. Forced host loss is a separate case; an
   unconfirmed Close is not exit evidence.
5. Rename two of the five profiles to synthetic labels. Change a harmless real
   preference in slot 1, normally Quit, wait for actual worker exit and reopen
   that slot from the same executable path; its setting must persist with fresh
   session authority. Slot 2 must retain its separate setting/cache namespace.
   Restart the controller and confirm labels persist. A second controller at
   that location must refuse concurrent registry ownership. A detached worker
   must keep its profile occupied through host closure/replacement until the
   worker's actual confirmed exit, then make it available. Inspect only owned
   settings and cache-file metadata, never key bytes or cache contents.
6. Where demo supplies Nearby and an existing resident IM, verify independent
   drafts across navigation and popout Close/reopen, without changing sender when
   switching worlds. Record any missing native-IM fixture as a limit. Check
   crash-details fallback only with a separate synthetic report, without inducing
   a GPU hang. An existing sanitized `d10473ef` `public-report.md` may be read
   for build/GPU/driver/window/loss fields; do not reopen that old real report's
   helper, because automatic sending may be enabled. Keep raw logs/bootstrap
   authentication files private. Return exact source/version, scoped PID mapping,
   screenshots and pass/fail per case. Real-grid routing, restrictions, unread
   behavior, voice and held PTT require the human.

## Validation and input needed later

- The local Windows builder can perform offline native acceptance using its
  Computer Use capability. This cloud session currently has no callable
  send-to-thread/handoff tool, so its prepared request has not been delivered.
  The request is retained as `.scratch/WINDOWS-NATIVE-RETEST-REQUEST.txt` in the
  candidate worktree. Preparation is not delivery or a completed retest. GitHub
  builds remain available; build success does not establish native behavior.
- Only the human performs real-grid login and verifies actual Nearby/IM routing,
  restrictions, unread semantics, voice and held movement/PTT behavior.
- If the standalone graphics loss recurs, retain the report folder and identify
  which viewer/process was active and which switch was requested. The crash label
  detects graphics-device loss; it does not establish the operating-system cause.
- No immediate preference blocks new stable slots starting from defaults. Importing
  an old generated profile, copying ordinary viewer preferences, or sharing slots
  across moved installations needs an explicit later choice; no migration is planned
  as an automatic side effect.

Preserve failing checkpoints and exact-source receipts. Do not use a retry,
unconfirmed Close, or a green build alone as acceptance of host recovery.
