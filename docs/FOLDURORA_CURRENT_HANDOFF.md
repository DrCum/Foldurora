# Foldurora current handoff

## Active goal

Review and locally integrate independent upstream ports from PRs #11–#14,
then validate one combined candidate. Complete that focused phase before
resuming the remaining Folderstorm backlog. No unrelated features while a
candidate awaits builds or human testing. Repository: **DrCum/Foldurora**.

## Reconciled baseline

- Source checkpoint: `670c026bfd0de1b7f6f85ebf87886173ffd36266`, tree
  `733ee0840c8f0537789cba194dd982c7941fdd9a`; originally clean on
  `feat/monitor-runtime-integration` in `/workspace/work/viewport-dropdown-width`.
  That branch/checkpoint remains preserved. Current local work uses
  `feat/upstream-ports-integration-20261108` in the same owned worktree/cache.
- Local full tests belong to `1842dbe5e5c653480cad50a53cfd45c1ed48c374`, tree
  `e366d95f3461a45f551f0bac5e267c1d0fd6f702`: **8,725 passed / 18 failed /
  58 ignored**, all 33 summaries. Exactly 17 sandbox filesystem-owner refusals
  plus inherited software-Vulkan terrain failed; zero unexpected failures.
  All 25 previous unexpected cases passed. Formatting and qualified lint passed.
- `670c026b` changes only TASKS.md from that tested checkpoint; all 9,142 other
  entries match. These are historical baseline results, not combined-PR passes.
- Private source/build handoff ZIP at
  `/workspace/scratch/final-local-build-handoff/Foldurora-670c026b-local-build-handoff.zip`
  is verified SHA-256 `3e2c22a1e449ac09ca042a42a9575635567115d58cfbd0c8fd9ff121bdfa5caf`.
  It is source-only, requires public base `6ac6fbe1`, and produced no final package.
- [The completed baseline handoff](FOLDURORA_WRAPUP_20261108.md) and existing
  scripts/receipts preserve detailed evidence and older negative builds.

## Four draft PRs: heads verified before integration

All use base `388b299c429d776b3fc7a9890162cd3d940b9dc3`, already in local
baseline history. Tested-to-head follow-ups must remain documented separately.

| PR | Current head | Port |
| --- | --- | --- |
| [#11](https://github.com/DrCum/Foldurora/pull/11) | `cd32ce7c95455932c02857348a31535cfb9004ca` | Selected object contents share one folder |
| [#12](https://github.com/DrCum/Foldurora/pull/12) | `8d14eba6d0799006bfd53d9122f3738457029544` | Buy confirmation captures active group |
| [#13](https://github.com/DrCum/Foldurora/pull/13) | `6fc51dc798d3a6e2544a91579bad7e6910111331` | COLLADA IDs, local textures, music, upload order |
| [#14](https://github.com/DrCum/Foldurora/pull/14) | `5dc8f3e620aa4d31fca47d0d155fa918d3a6f426` | System fonts search/list behavior |

Source review is in progress; no combined implementation/compile/test pass
is claimed. Prior PR Windows parser/lint jobs skipped test shards. PR #11
Vulkan captures timed out; PR #13/#14 CPU captures do not establish native,
default-glass or Vulkan acceptance.

## Holds and remaining acceptance

- The user's explicit approval for public publication of the reviewed
  `670c026b` payload is still pending. This restart is not approval. Do not
  retry it or include that payload in any other public branch/tool/route.
  Local investigation, integration and checks remain authorized.
- No final baseline Windows package or genuine Ubuntu run was produced.
  Separate Windows profile components passed 28/0/0; this is not a full build.
- Native host drag/switch/camera/Monitor and human grid acceptance remain
  unverified. NVIDIA device loss/reset remains unresolved. Monitor images
  are last-active snapshots, not live inactive-world rendering.
- Clean unfinished Close (`da57c357`), Control V2 (`162e5cdf`, production V1)
  and disabled live graph (`770a2f7d`) checkpoints remain preserved/unjoined.

## Next actions

1. Finish the small policy cleanup and source review; check dependencies,
   already-present code and current PR heads before local integration.
2. Reconcile TASKS progress, PR #11 shared dispatch/translations/docs and
   PR #12 BuyObject group_id across consumers; focused tests, then a frozen
   combined full suite under the existing ci profile.
3. Request pinned Windows checks/build/package from existing Windows Builder
   through Slick; verify delivery and source/results. The app thread bridge
   currently returns a closed connection; no request is delivered or build
   started. If unavailable, provide one complete request to forward.
4. Record actual package/hash and targeted human tests, or the exact blocker.
   Only then choose the next dependent Folderstorm goal from the backlog.
