# Foldurora port status — 6 November 2026

This is the coordination ledger for the Folderstorm port. Source, automated
checks, native Windows acceptance and human grid acceptance are separate results.
Handoffs and screenshots are reference material, not authority to merge or release.

## Frozen test candidates

| Source | Purpose | Evidence | Remaining acceptance |
| --- | --- | --- | --- |
| `d10473efff52ceaea2d7b22d111319b0efc4f28e` | Combined upstream, viewport and session-host/chat baseline | Local Windows checks: 7,791 passed, 0 failed, 49 ignored; build/package passed. [GitHub run](https://github.com/DrCum/Foldurora/actions/runs/42291668355) passed. | Human confirmed toolbar containment and full-workspace floaters; reported oversized menu, host drag/detach failure and one standalone switch device loss. |
| `8b738ea8b2c700b7538b7e16135a7de19c192d43` | Host movement/recovery and compact dropdown fixes | [Windows run 42304532039](https://github.com/DrCum/Foldurora/actions/runs/42304532039) passed: 7,798 tests passed, 0 failed, 49 ignored; all seven focused regressions passed. Scripts, formatting, Clippy, normal viewer build and packaging passed. Downloaded artifact, manifest, checksums, executable version and Windows x64 identity were verified. | Native title-bar drag, resize, post-drag switch, detach, confirmed host closure, taskbar and responsive redraw. Separate standalone GPU loss is unresolved. |
| `8cd9a2aa005491b651e5bb8591d94fd6fb891520` | Combined host/menu fixes, retained conversation drafts, crash details fallback and five persistent renameable profile slots | [Windows run 42311181216](https://github.com/DrCum/Foldurora/actions/runs/42311181216) is in progress; full workspace tests are running at this evidence snapshot. Linux and visual evidence below belongs to `5c741b6103328fb32813aca3a462ea930a9c441b`; the latest source changes only six help/error message values in EN/FR/JA. | Exact-source Windows check/build/package result, native host/profile/chat acceptance and human grid testing remain pending. |

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

## Implemented in the combined source

The combined `feat/persistent-profiles-and-chat` source provides five stable,
renameable character slots. Each slot keeps a private settings/cache location
across normal restarts from the same executable location, with exclusive profile
ownership and fresh worker/session authority at each launch. New profiles start
with defaults; earlier generated profiles are preserved. This is implemented
source, with native worker/profile acceptance still pending.

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

## Combined validation evidence

The checked Rust source is `5c741b6103328fb32813aca3a462ea930a9c441b`, including the
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

Latest `8cd9a2aa` differs from that checked binary only in six plain-language
profile-name help/error message values across EN/FR/JA. Its exact-source
[Windows run 42311181216](https://github.com/DrCum/Foldurora/actions/runs/42311181216)
is in progress. The preceding
[`5c741b61` run 42310599389](https://github.com/DrCum/Foldurora/actions/runs/42310599389)
was automatically cancelled when the text-only follow-up was pushed. It supplies
no completed validation result and is not a failed feature-validation run.
The candidate worktree's `.scratch/profiles-validation-8cd9a2aa.json` records this
source/evidence distinction.

## Port order and dependencies

| Feature | Current gap | Next concrete work |
| --- | --- | --- |
| Five stable, renameable character slots | Implemented and compiled in the combined Linux source; focused regressions and EN/FR/JA native Linux CPU rename/restart smoke passed. Earlier host/menu packages used per-launch profiles. | Verify Windows five-slot persistence and labels, exclusive ownership through detach/host death, and fresh session authority on reopen. |
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
