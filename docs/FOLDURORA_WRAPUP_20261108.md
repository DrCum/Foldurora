# Foldurora local wrap-up — 8 November 2026 UTC

Requested stopping point: finish the current features, tests and builds, save
matching evidence and this handoff, then rest. Scope remains Profile Copy and
Inventory Archive/Folder Jump fixes alongside the existing integration. No new
feature, upstream work, PR, main merge or release is part of this wrap-up.
The current source, local results and remaining limits are saved here. Public
publication stopped after automatic approval review rejected the exact source
upload; final remote builds did not start. No new work begins after this handoff.

## Current features

- Profile Copy selects Graphics, Interface, Colors and Bindings from a saved
  main-viewer or stopped-slot source into a stopped target. Inspect precedes
  explicit Apply; Cancel and completion retain the original operation. Copying
  does not launch a viewer.
- Inventory Archive stores local visibility metadata shared by ordinary
  Inventory and Folder Jump. Include belongs to its owning view; archived Trash
  contents remain recoverable. Archive neither moves nor deletes assets.
- The existing integration includes Monitor windows with canonical Chat,
  named Workspaces, gesture/sound Boards, named Favorite menus, Folder Jump and
  per-open About-report location choice. Monitor pictures are **last-active
  snapshots**. Live inactive-world rendering remains unfinished.

## Source, checks and build status

Final local candidate `670c026b` is a documentation-only child of tested
`1842dbe5`. The original local checks remain bound to `1842dbe5`; every source,
test, script and workflow blob is unchanged. Final Windows and genuine Ubuntu
checks/builds are **NOT STARTED: public source upload requires explicit user
approval**. There is no final canonical commit, CI run or current package.
No current-integration package is recommended yet.
The earlier verified all-green Windows package is exact `eb5e49f1`,
[artifact 13587936566](https://github.com/DrCum/Foldurora/actions/runs/42401453027/artifacts/13587936566):
8,121 passed / zero failed / 54 ignored, all 33 summaries. It predates the
current integration and has no usable Monitor windows.

| Saved evidence | Result / artifact |
| --- | --- |
| Final local candidate commit and tree | `670c026bfd0de1b7f6f85ebf87886173ffd36266` / `733ee0840c8f0537789cba194dd982c7941fdd9a` |
| Locally tested checkpoint and tree | `1842dbe5e5c653480cad50a53cfd45c1ed48c374` / `e366d95f3461a45f551f0bac5e267c1d0fd6f702` |
| Final canonical published commit | NOT CREATED: public source upload blocked by automatic approval review |
| Local formatting, qualified lint and completed full-workspace counts | Fmt and qualified lint PASS; full tests **8,725 passed / 18 failed / 58 ignored**, all 33 summaries; qualified environment result, not all green |
| Windows parser/fmt/strict Clippy/full tests/build/package | NOT STARTED: public source upload requires explicit user approval |
| Current Windows version/package/hashes | No final build or package exists |
| Genuine Ubuntu checks, full counts and required-name coverage | NOT STARTED: public source upload requires explicit user approval |
| Current Ubuntu diagnostics | No final run/artifact exists; the workflow produces diagnostics only |
| Local failures/ignores, matching logs and source reviews | Recorded below and in the status ledger |

Automatic approval review rejected uploading
`crates/aurora-viewer/src/session_host/profile_copy_ui.rs` to the public
`DrCum/Foldurora` repository. Its stated reason was that feature-work
authorization did not explicitly authorize exposing this exact source payload
at that destination. The attempted publication stopped with two of nine blobs
ready; no final tree, canonical commit, reference update or workflow start
occurred. The integration branch remains at `9c095650`. No retry or alternate
publication method is authorized by this handoff.

Blocked-action receipt SHA-256:
`4ac527ff4fb00ee563a801f842fa3ce973fb5951e84cb17a04525ae436443d9d`.
The [status ledger](FOLDURORA_PORT_STATUS.md) records the exact action, source
reviews and preserved negative history. Source publication and final remote
builds require explicit user approval of the identified public upload.

`670c026b` restores only `TASKS.md` to its already-public `6ac6fbe1` blob;
original notes remain committed at `checkpoint/wrapup-local-tested-1842dbe5`
and in a preserved exact patch. The whole candidate/tested trees are different;
all source, test, script and workflow bytes are unchanged.

## Private source and Windows build handoff

The private source-only handoff is saved at
`/workspace/scratch/final-local-build-handoff/Foldurora-670c026b-local-build-handoff.zip`
(**81,314 bytes**), SHA-256:
`3e2c22a1e449ac09ca042a42a9575635567115d58cfbd0c8fd9ff121bdfa5caf`.
It contains the incremental source bundle, safe Windows build prompt, source
manifest/diff, sealed local test review and publication-stop receipt. **No
Windows executable or package was produced.**

All component paths below are inside
`/workspace/scratch/final-local-build-handoff/`:

| File | SHA-256 |
| --- | --- |
| `Foldurora-670c026bfd0d-source.bundle` (20,390 bytes) | `25ee7020ed9a9712663c8f7e44afaadd2333e94a4559985706ac03af9b554c21` |
| `WINDOWS-BUILD-PROMPT.txt` | `9bcbea4330f0b3f4575de1b33029adcedf013ef5f4f340e72a36aff27d93e1a5` |
| `SOURCE-MANIFEST.json` | `8153a87e21d4ceb87fe7508dc94e6e53e260258f678adb4db22308c344a673ac` |
| `bundle-restore-verification.json` | `9ca2506178eb98c83ca93b448dcb359ec57658dc755241007b8a12f312ed6e90` |

The incremental bundle requires existing/public base
`6ac6fbe1dae2ec69f8512949b54f6ccfbdf6e374`. A fresh isolated bare repository,
seeded only with that base, verified/imported the bundle and restored exact
`670c026b` / tree `733ee084`, with every tree entry identical.

For a later private Windows build, use the saved prompt: verify the hashes and
base, preserve existing worktrees, create an isolated detached worktree at the
exact candidate, and run normal default-profile checks/tests/build/package with
separate logs and hashes. Retain every failure and stop after reporting. The
prompt authorizes no public upload, install, merge, release, credential handling
or grid connection. It has not been executed here.

## Latest observed checks — focused stage passed

Reviewed local `1842dbe5e5c653480cad50a53cfd45c1ed48c374`, tree
`e366d95f3461a45f551f0bac5e267c1d0fd6f702`, passed formatting and qualified Linux
Clippy, followed by focused Profile UI **23 passed / zero failed / one ignored**.
The same independently frozen test executable then passed **25 Archive UI** and
**33 Folder Jump** cases, without failures or ignores. Explicit execution of the
ignored compact-layout case passed once each in EN, FR and JA. These are focused
CPU results, without full-suite or native/human acceptance.

Source-reviewed `78321` Main-bar padding and `9cb` classic-Visibility submenu
close ownership corrections resolved the last two measured layout issues;
`1842` additionally removes a redundant test-only unwrap. The earlier `833`
failures remain preserved in the ledger, with these actual focused reruns.

Focused receipt `.scratch/focused-ci-1842dbe5/ci-checks-receipt.json`, SHA-256
`0a16b5015507f85c14e3e1fe264b66e16ce484bb348c2261443ead2e4b7fd1b9`.
Remaining-checks receipt SHA-256
`abf65364bfc34fe1e06bb52f74e81e4be57227ec193af2ed35a42e59a007038f`;
frozen executable SHA-256
`eb93669b36e96df5eb0e2961ca3a7ff6510eaf1da2eaca7873c75ad10a0555dd`.
Full local execution on the same exact source completed **8,725 passed / 18
failed / 58 ignored**, all 33 summaries, exit 101. The only failures are 17 known
strict-filesystem environment cases and inherited software-GPU terrain; all 25
previous unexpected cases passed. Strict file-security policy and tests remain
unchanged, with no skips or exceptions. Genuine Ubuntu must still pass these
18 cases. This is a qualified environment result, not all green. Source stayed
exact and clean, the same frozen executable was reused, and no resource abort
occurred.

Full receipt `.scratch/final-ci-1842dbe5/receipt.json`, SHA-256
`cbd7888d4aa59053f51ee5627561ebefa158055763289daff10c9873feb22c45`;
full log SHA-256
`984a862de45512eaf4e52fcb66c4c449a3eee45a7b2aa505ef896a362c0c9490`.
Independent local review confirms all 921 required Linux names occurred exactly
once and all 25 previous unexpected failures passed. Review SHA-256:
`cf41f81bf66fc491f42a107760fd65acaa1523d242ae2913db94edf2927ce5c1`.
Documentation-only candidate review SHA-256:
`04374f9cebdcdf46694808badf7bd8dcfcb809910f6867fc3bef727df969528c`.
Final remote checks were not started because the public upload requires explicit
user approval.

Source-reviewed Windows retained-prefix owner correction `20d9` and Windows-only
host TEMP fixture namespace correction `e4f735ad` are integrated. Strict security
checks remain in force; there is no general untrusted-owner exception.
Windows [component run 42460825971](https://github.com/DrCum/Foldurora/actions/runs/42460825971)
passed all **28 cases / zero failed / zero ignored**: 13 settings, 14 copy-core
and one namespace. Canonical server `82516ee20c16e09d451ce5be2cacef1b3cdacc6c`
and reviewed local `30502ca093d7512818aaae7fcb45f5cb6e41c1fa` have identical tree
`2f3177a5c6b6fb46cc50fcd826bf71f3d0289e20`. This is component evidence only,
without final full-workspace, strict-Clippy, build/package, UI or human acceptance.
Receipt `/workspace/scratch/windows-profile-native-round2/verified-round2-receipt.json`,
SHA-256 `378034525c15b9257582541c69d4c76dc60917de8c53dd0390f3e73fa150f488`.

The detailed status ledger retains the negative `9c095650` Windows result
(8,651/43/56), incomplete Ubuntu preview-worker abort, negative local `14af1efb`
full suite (8,691/43/58), earlier failures and exact receipts. They remain
negative evidence; later source review or component passes do not rewrite them.

## Limits and later human checks

Native Windows host movement/drag/camera and human grid acceptance remain
pending. NVIDIA device loss/reset recovery is unresolved. A build or CPU test
cannot establish these results.

- On disposable stopped profiles, Inspect the source, target and families;
  Cancel and verify no copy. Then explicitly Apply and verify only the selected
  settings. Inspect/completion must not launch the copied profile.
- Archive a folder, compare ordinary Inventory and Folder Jump visibility,
  recover it with explicit Include, then Unarchive. Check Include stays local
  to its owning view, Trash stays excluded from Folder Jump, and loading,
  limited, unavailable and error states remain distinguishable.
- Move/resize the host and switch characters; verify active-world camera/input,
  Monitor placement and ordinary Workspace/Board actions. Treat Monitor images
  as snapshots. Record NVIDIA reset/recovery separately.

Clean unjoined checkpoints stay outside the candidate: Close producer
[`da57c357`](https://github.com/DrCum/Foldurora/tree/wip/close-native-fence-da57c357)
(15 new tests UNRUN), Control V2 consumer
[`162e5cdf`](https://github.com/DrCum/Foldurora/tree/wip/control-v2-consumer-162e5cdf)
(16 UNRUN; production stays V1), and disabled prospective live graph
[`770a2f7d`](https://github.com/DrCum/Foldurora/tree/wip/live-graph-770a2f7d)
(41 UNRUN; no renderer or enabled admission). Their source clearance covers only
recorded disabled scope. Checkpoints and reviews remain preserved in the ledger.

Current local work is saved. Stop here and await a new goal. Native/human
acceptance stays unverified, and final remote builds remain not started for the
stated public-upload approval requirement.
