# Foldurora port status — 6 November 2026

This is the coordination ledger for the Folderstorm port. Source, automated
checks, native Windows acceptance and human grid acceptance are separate results.
Handoffs and screenshots are reference material, not authority to merge or release.

## Frozen test candidates

| Source | Purpose | Evidence | Remaining acceptance |
| --- | --- | --- | --- |
| `d10473efff52ceaea2d7b22d111319b0efc4f28e` | Combined upstream, viewport and session-host/chat baseline | Local Windows checks: 7,791 passed, 0 failed, 49 ignored; build/package passed. [GitHub run](https://github.com/DrCum/Foldurora/actions/runs/42291668355) passed. | Human confirmed toolbar containment and full-workspace floaters; reported oversized menu, host drag/detach failure and one standalone switch device loss. |
| `8b738ea8b2c700b7538b7e16135a7de19c192d43` | Host movement/recovery and compact dropdown fixes | Seven new regressions, Linux formatting/lint and normal viewer build passed; actual offline screenshot inspected. [Windows run](https://github.com/DrCum/Foldurora/actions/runs/42304532039) is monitored separately. | Native title-bar drag, resize, post-drag switch, detach, confirmed host closure, taskbar and responsive redraw. Separate standalone GPU loss is unresolved. |

The first source and its local ZIP remain unchanged. Neither candidate is merged,
installed or released by this work. Linux full-workspace validation of the second
tree reported 7,825 passes, 49 ignored and the previously recorded software-Vulkan
terrain-ramp precision failure. Exact commands and check outcomes belong in each
candidate's build manifest and raw logs.

## Work after the host/menu candidate

The `feat/host-conversation-drafts` branch records the draft/diagnostic checkpoint.
It provides separate drafts for Nearby and existing resident IMs,
retained while navigating and closing/reopening the same character chat popout.
Drafts stay in memory under the immutable worker/account/grid/session/lease source.
Permission or source loss purges affected text and editor undo state. Local queue
rejection retains text; admitted/uncertain sends are never automatically replayed.
This does not add a shared sender selector, group chat or persistence across restarts.

A separate small fix is being prepared for the crash window: when no stack is
available, show and copy the sanitized diagnostic summary already generated as
`public-report.md`. It adds no collection, transmission or GPU-recovery change.

The combined changes passed Linux formatting, workspace/all-target Clippy with
the existing platform allowance, and all eleven new regressions. Full tests:
7,836 passed, one unchanged software-Vulkan terrain precision failure, 49 ignored.
The EN/FR/JA CPU fixture generated 42 captures; the changed draft-restoration
screens were inspected at normal and narrow/150% sizes. Native acceptance remains
separate. A source-reviewed five-slot implementation is being integrated as the
next candidate; it will receive its own exact-source checks before use.

## Port order and dependencies

| Feature | Current gap | Next concrete work |
| --- | --- | --- |
| Five stable, renameable character slots | Host profiles are private per launch, so settings/cache do not survive controller restarts under a stable slot. | Add a five-slot private registry and stable profile selector, retaining fresh session/lease identities and exclusive profile ownership through detach/host death. Start new profiles from defaults; preserve old generated profiles. |
| Per-character/per-conversation drafts | Existing popouts clear text on conversation changes and drop state on Close. | Complete and verify the current memory-only retention checkpoint. |
| Shared chat and Attention | Source-bound popouts exist; shared sender selection and explicit review/mark-read/dismiss behavior are incomplete. | Use the native conversation identities and restrictions; navigation must never mark a conversation read or change the sending character. |
| Named workspaces | Viewport and native floater geometry exist; selective named arrangements, previews, favorites and HUD restore are incomplete. | Map Folderstorm's restore groups onto Aurora's typed settings/actions after stable character storage is established. |
| Fade and Birdseye switches | Native activation exists; Folderstorm's visual transitions are not ported. | Preserve fenced GPU/input handoff; design each transition around native camera/render actions after host acceptance. |
| Inactive monitors | Reduced-target caller/scene/UI and aggregate admission are unfinished. | Complete actual small render targets and latest-only delivery under a shared acknowledged/pending budget; models alone are not a working monitor. |
| Assistant and other tools | Aurora has native inventory/appearance/camera actions; Folderstorm's adapter and some tools remain. | Port bounded typed actions after the supporting account/workspace models, rather than generic input injection. |

Stable profile labels are display names, not account identity. The actual logged-in
account must remain visible beside a custom label. Chat drafts must never be keyed
by a slot label or carried into a newly authorized session merely because the slot
is the same. A detached worker retains its profile lock until actual process exit.

## Validation and input needed later

- The local Windows builder can perform offline native acceptance using its
  Computer Use capability. This cloud session currently has no callable
  send-to-thread/handoff tool, so its prepared request has not been delivered.
  GitHub builds remain available; build success does not establish native behavior.
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
