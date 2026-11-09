# Independent Aurora hover/control ports — 9 November 2026

Reviewed Aurora main d174ae17e895bbb399e28165757c3b99e8168952, after
b817c5748fab00a9b516786bfb0a3745b1ef8fbf. Owned branch
fix/upstream-hover-controls-20261109 starts at
388b299c429d776b3fc7a9890162cd3d940b9dc3
(feat/foldurora-upstream-integration). Native acceptance remains pending.

Read the current policy on main f78d2814, published handoff on
docs/workspace-context-build-20261109 at
96cc5693d58e64fc5c519576141a5e8a7b127ff3, and plan/status/integration
documents on frozen combined source 1ba972d1419b22ee918515b12ecd5e6d42c4ca45.
Refreshed all published non-capture branches and open PRs and checked source
signatures and provenance. PR11/26/32 are excluded. PR36 already incorporates
PR18/19/26/32; PR11–16 are in the lead stack.

## Changes

| Aurora source | Port commit | Behavior |
| --- | --- | --- |
| 686a125d4900d3e5ca974a64ed7365929eca8c84 (#3046) | 26b23a5255e4dbb0ab0879627dc26873d0eff212 | Pictures work with Inventory text tips off. Both switches remain visible in Inventory preferences and main Preferences. |
| ea14f2174a1672db9c6d2504cc998dbe47c875aa (#3047) | 2d5fdefd5cbab6c68ebe5edb5ae7a989796e7ce7 | Region arrival resends the owning App's Always Run, temporary keyboard run or joystick run state. |
| 1f02818d5dc9e533cafc65b0a296b861302a57fd (#2535) | 31cfb232cb72f2371ca5ff1576d4ac6bd002ddb1 | Controls exposes the existing account's Friends Only state using World::set_friends_only, disabled before login. Teleport persistence stays separate. |

Hover migration uses **Foldurora settings version 33**, from 32; Aurora uses 35.
Earlier files keep pictures only where text tips previously made them visible.
Fresh defaults keep both off; a later explicit pictures-only choice survives
repeated sanitization. Intervening Aurora graphics migrations are not imported.
Future ports must adapt those version guards rather than blindly reusing 33/34.

The existing asset resolver, @viewtexture, delay and drag rules remain shared.
Hover never fetches an inventory tree or selects, opens, copies, moves, exports
or wears items. No-copy/worn/confirmation rules are unchanged. Tests cover both
Inventory layouts, selection preservation and restriction after a cached image.
The upstream Preferences test insertion was adapted around unrelated missing
font-test context; those font changes are not repeated.

Folderstorm reference was read at abbe3cb0ca66ee2484f277219f6a27706fab7fc6:
LLInventoryPanel::handleToolTip (indra/newview/llinventorypanel.cpp),
the sendWalkRun arrival (indra/newview/llviewermessage.cpp), and
FSRenderFriendsOnlyPersistsTP (skins/default/xui/en/panel_preferences_move.xml).
The extra Friends Only row and independent picture/text preferences are Aurora
additions; Foldurora's existing behavior and permissions remain authoritative.

## Deferred upstream input

| Source | Overlap or dependency |
| --- | --- |
| 7525518cec4f018744e0b650e77953fc6fe341aa | Avatar-only tips need an RLVa privacy audit: existing avatar_tip reads canonical cached display/username/group data directly, while World::avatar_name separately checks rlva_can_name. No avatar-only code is included. |
| 3afebccf9604e603ce6c1d98a64aac426a232f5c | Recent emoji/GIF state spans composers, persistence and Shared Chat ownership. |
| 286b2ca91da5789e952740acd1f0b5618fc3d518 | Region Tracker adds account state, polling, world/map/toolbar/window integration. |
| cc604479ee80b65083bd8a295e2eae7dcd6eaaa5 | Parcel-radio intent and login/teleport/media-consent lifecycle overlap host/MCP and shelved background-audio work. |
| d174ae17e895bbb399e28165757c3b99e8168952 | Alpha sorting changes scene/render ordering and GPU lists alongside protected Monitor/renderer work. |
| e978c86b6afd7da2b1bf38bf49f77ace0da87161 | Aurora README performance/marketing rewrite needs separate Foldurora documentation/image coordination. |

## Integration and acceptance

Coordinate app.rs (arrival and Preferences state/result only), settings version,
options.rs/Controls, one matching EN/FR/JA help key, TESTING.md and this note/TASKS.
MCP's Controls edits are in different rows; native account ownership stays inside
its App. Rerun combined guards after integration. Slick is not callable here:
no coordination delivery or Builder start is claimed.

Frozen-source commands/totals/logs and actual CPU images are recorded in the PR.
Focused checks cover Inventory hover, Preferences, Friends Only world behavior,
neighboring deletion/clipboard restrictions and focus, Controls/App, theme,
localization and lint/parser/formatting. The arrival resend is compiled and
source-reviewed; simulator acceptance is pending. Full combined/native validation
belongs to the next Dev Agent checkpoint. Do not retry PR11's timed-out Vulkan
captures or PR32's unassigned macOS jobs for this port.

CPU captures use opaque surfaces and egui test fonts, 1280×900 at 1×. They do not
establish native glass/GPU/final-font or grid acceptance. Human checks: both
Inventory layouts with text off/pictures on, selection preservation and restriction
after cache; existing no-copy/worn refusal/confirmation; Friends Only agreement
with menu/toolbar, prelogin disabled state and account/teleport persistence;
walk/run after teleport and border crossing, temporary/joystick run and RLVa.
Only the human logs into a grid. No merge, install or release.

## Lead combined integration

PR38 frozen input `ef01cebfc1a756b16ee86a847fb470bf15c24091` is integrated
after the MCP Windows correction `ea63e22896b5a0b05cc51e9b1c0631f39d0351c6`.
Two Preferences-test insertion conflicts were resolved by retaining all existing
Reset/Undo and installed-font regressions, then adding the independent hover
module and Friends Only case. TASKS progress is additive. No existing directly
annotated test definitions are removed; all six new cases are Windows-enabled.

The parent settings version is32, so the adapted33 migration is appropriate.
The shared hover cache, streamer Hover/Background interests and native Friends
Only setter are unchanged from the parent. Existing image background ownership,
RLVa qualification, per-App source ownership, MCP consent and unsupported
workspace-mutation boundaries remain. No Monitor production caller is enabled.
The arrival resend is ordered after the existing arrival handling; simulator
walk/run continuity remains a human acceptance case.

The next Windows run validates this combined group once: corrected strict
workspace lint, unfiltered workspace suite and1,364-name source audit, then
nondefault Monitor gates, ordinary build/package and targeted offline smoke.
The earlier1ba run stopped at lint; no downstream pass is inferred. The a903
package and its separate measured evidence remain preserved.

Combined-source checks: formatting0; qualified viewer all-target ci Clippy0
(existing Linux dead-code allowance, no new suppression); localization13 passed
and unchanged production theme scan1 passed. All six new viewer cases compile
but remain unrun locally because full viewer linking is resource blocked. The
author's focused225-pass evidence remains separate, not relabeled as this source.
No new full Linux/Windows/package/native pass is claimed.
