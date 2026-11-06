# Foldurora implementation handoff 6 November 2026

Work is frozen at the user's request to take over agent orchestration. Main,
PR #1, earlier test packages and all worktrees are preserved. Nothing has been
merged, tagged or released. This handoff distinguishes built checkpoints from
unfinished source; it is not approval for grid testing by an agent.

## What is ready and what remains

| Capability | State at handoff |
| --- | --- |
| Multi-monitor workspace and character launcher | Implemented in PRs #2–#3. The user confirmed multi-monitor works. Separate viewer windows remain the standalone behavior. |
| Character GPU coexistence | PR #4 proactively parks background rendering. The user confirmed switching without the reported crashes. All participating viewers must use the new build. Root cause of the earlier device loss is not established. |
| Optional toolbar restriction | PR #5 shipped the wrong scope. The current correction keeps ordinary windows on the full workspace and restricts toolbars and camera/photo controls. See the exact correction checkpoint below; old #5 packages do not validate it. |
| Mouse look in a configured viewport | Current correction keeps desktop world rendering, reticle and capture anchor in that viewport. Raw relative/absolute sensitivity stays unchanged. Physical Windows input acceptance is pending. |
| One session host | Draft PR #8 implements a CPU controller, separately logged-in worker processes and Windows fitted top-level presentation. Exact Windows build verified; actual taskbar, adoption and focus acceptance is pending. |
| Inactive-character chat | M2 is wired source with a verified Windows checkpoint and Linux CPU fixtures. Nearby and existing direct conversations only. Native popup/focus and real account acceptance are pending. |
| View-only reduced-resolution monitors | M3 is unfinished, uncompiled source. Capability remains **false**. Do not enable it by flipping a flag. |
| Upstream synchronization | 16 selected changes are isolated and Windows-built. They are not combined with host/chat. Conflicting or risky changes are recorded for later review. |

## Exact branches and checkpoints

All PR links are in [DrCum/Foldurora](https://github.com/DrCum/Foldurora).
Branches are stacked; merging every PR independently into main would be wrong.

| PR or checkpoint | Branch | Exact source |
| --- | --- | --- |
| [#1](https://github.com/DrCum/Foldurora/pull/1), preserved | `fix/credential-transport` | `9f389441cfbafaa42c968a35324cc08fda0bb734` |
| [#2](https://github.com/DrCum/Foldurora/pull/2), workspace/launcher | `feat/overnight-integration` | `62cdd4ea911983cbdfee3708317f42ec00b8e480` |
| [#3](https://github.com/DrCum/Foldurora/pull/3), captions/recovery | `feat/workspace-followup` | `81c167e27e9a2fcce692ff166b27edd0be1b9c62` |
| [#4](https://github.com/DrCum/Foldurora/pull/4), GPU mitigation | `fix/character-gpu-coexistence` | `5e46f785bdaaed5db40a2f1558d86b69d6dbc386` |
| [#5](https://github.com/DrCum/Foldurora/pull/5), original UI scope | `feat/viewport-ui-integration` | `f504342231006423d392224f1225bc08e9e32fb5` |
| [#6](https://github.com/DrCum/Foldurora/pull/6), 9 upstream candidates | `sync/upstream-safe-20261106` | `5cf34c58778651cd2cb8e942aa88ddab4000a48b` |
| [#7](https://github.com/DrCum/Foldurora/pull/7), 13 upstream candidates | `sync/upstream-safe-b962b2a7` | `8fbb2f6878bbc571f1c5e9619f17407190c93178` |
| [#8](https://github.com/DrCum/Foldurora/pull/8), host M1 | `feat/session-host-integration` | `d87d11df89ec15ff0688cf3605f8f70d5e98b391` |
| [#9](https://github.com/DrCum/Foldurora/pull/9), viewport/cursor correction | `fix/viewport-toolbars-integration` | `989a08f3fca3719230f397beeb6da710ab4a80e2` |
| M2 exact Windows checkpoint | `feat/session-host-chat-integration` remote | `480176d037eb576119775e8d67aa2df065aa86ae` |
| M2 final Linux guard correction | `handoff/session-host-chat-20261106` remote | `32b298392c7d40afd2d8cc96d0658041b69be985` |
| 16 upstream candidates, no PR yet | `sync/upstream-safe-7afa101c` | `a17185fd9b5d9eee1706d5634eba371a894d7711` |
| Disabled preview worker/barrier | `handoff/session-preview-suspension-20261106` remote | `213d8e498ea8b1e15e121f7186c50a799c9e2b0a` |
| Disabled preview renderer precursor | `handoff/session-preview-renderer-20261106` remote | `a449faa78bd0b80f8a39b07b2cb25347a59eb167` |

Main was held at `ae2fd1016ec83304719fa131312e2ac69c8f37a6`.
The original `/workspace/Foldurora` checkout stays at
`7b52b689c6df523cf2c778eddd950d1854ca784e` on `work`.

### Current viewport correction

Base is exactly PR #5, including the GPU coexistence follow-up. UI source
`0300b253cda3bb8849e6319394a74146cf026763` and follow-up
`a03cf5dfaf481221225602812a008eba733b71fd` were independently reviewed.
Root cherry-picked the latter as `ad0a963d`. Cursor source
`b79199f3f12428351488a2da5f74dae56fab920a` is root alias `26c94b8e`.
`22527b0d` only admits this isolated branch to existing Windows build CI.
Root `989a08f3` fixes the compiler-reported popup alignment array lifetime;
the initial `ad0a963d` combined Clippy run stopped on that error before tests.
Do not duplicate original and cherry-picked aliases.

Egui keeps the full native canvas. Chrome, its menus and camera/snapshot
controls have explicit viewport bounds; Inventory, Settings, Map, mini-map,
chat and other floaters retain full-workspace positions and input. Canonical
wide sizes/anchors survive option toggles. Snapshot output choice is independent
of where its controls sit. Editing viewport margins does not abort held input
on an unmoved Settings window. Desktop mouselook retains configured physical
viewport geometry, including drag-to-mouselook and viewport edits while held.
Alt, focus loss, ordinary camera dragging and VR retain their existing paths.

Formatting and strict Linux workspace/all-target Clippy passed at `989a08f3`
with the existing platform dead-code allowance. Focused regressions are
running. [Exact Windows run 42280773023](https://github.com/DrCum/Foldurora/actions/runs/42280773023)
is running; no final artifact or native acceptance is claimed yet.

At 19:44 UTC, Linux test-binary compilation was still active. Focused test
execution, the full suite, the normal build and offline viewport captures have
not completed for this source. Prepared scenarios and source review do not
substitute for those checks or human native acceptance.

### Host and chat boundaries

Launch the host with `aurora-viewer.exe --session-host`. Open a login window,
log in manually, then Activate. The CPU-painted controller manages independent
world/network workers. Windows adoption uses fitted, unowned borderless
top-level windows and reversible taskbar styles, not cross-process parenting.
Other platforms retain separate viewer windows. Actual keyboard focus is
separate from active rendering authority. Revoke, GPU completion, target-surface
preparation and grant proofs gate switching; submitted completion is not proof
of physical display scanout.

Host profiles are per launch/session generation, not persistent per character.
Minimize parks rendering; restore needs explicit Activate. Inline rendering/XR
is unsupported. Window/device recreation detaches rather than automatically
re-adopting. Closing or losing the host restores ordinary workers instead of
logging characters out. These native paths still need acceptance.

M2 adds authenticated private loopback chat and CPU-owned tool windows, bound
to the exact host/worker/account/grid/session/conversation/draft. It supports
Nearby and existing resident IMs, without group chat, new-recipient lookup,
offers, payments, gesture execution or chatbar commands. Source/lease and
privacy changes purge stale state; RLVa and mute policy is rechecked on send.
The underlying native send API returns no delivery proof, so its receipt is
**Uncertain**, never a claim that the message was delivered. No automatic resend.
Chat focus must not resume a parked full-resolution world.

Final M2 `32b29839` adds a terminal-zero chat IO revision guard to Windows-built
`480176d0`. Linux ran the 11 relevant actor tests, strict Clippy, normal build,
and 36 real CPU-painter captures in English/French/Japanese, normal and 150%
narrow layouts. Every capture was inspected. These fixtures exercise actual
egui events and painting, not native windows, sockets or a grid. Exact `32b29839`
has no repeated full suite or Windows build; do not transfer `480176d0` results
to that later source. Its decoded Linux binary hash is
`8a107129d90cfd4b7829160544cbfdb4d7a7355c144bdb983fe824d5af8b755c`.

## Verified build artifacts and evidence

These are separate feature cohorts, not one combined executable. Download
the named portable archive from the linked Actions artifact; retain earlier
packages for comparison. Authenticated GitHub access may be required.

| Source | Verified Windows tests | Build and artifact |
| --- | --- | --- |
| Host M1 `d87d11df` | 7,656 passed, 0 failed, 47 ignored; all 90 new and 54 retained named regressions | [Run 42268877155](https://github.com/DrCum/Foldurora/actions/runs/42268877155), [artifact 13533112206](https://github.com/DrCum/Foldurora/actions/runs/42268877155/artifacts/13533112206) |
| Chat M2 `480176d0` | 7,708 passed, 0 failed, 48 ignored; all 142 new and 54 retained named regressions | [Run 42266902733](https://github.com/DrCum/Foldurora/actions/runs/42266902733), [artifact 13531138507](https://github.com/DrCum/Foldurora/actions/runs/42266902733/artifacts/13531138507) |
| Upstream16 `a17185fd` | 7,637 passed, 0 failed, 48 ignored; 54 retained and 2 added audio regressions | [Run 42263229823](https://github.com/DrCum/Foldurora/actions/runs/42263229823), [artifact 13528889147](https://github.com/DrCum/Foldurora/actions/runs/42263229823/artifacts/13528889147) |

All three rows passed actual raw formatting/strict Clippy/full-test/build/package
outcomes, and exact PE version/source/manifests/checksums were verified. The
workflow uses `continue-on-error`; green step colors alone are insufficient.
[Detailed receipts](handoff/windows-build-evidence.json) retain the identities.
Inner portable ZIP SHA-256 values, in the same order:

```text
d26baa6cd6e3366a9018f4e8f7d5d242ccd3d566a6a9f8f6b19b713ad9d1d3b4
3236651479bffcd4ee871bb68c2110e652ea10229a3e9296929f51d40234d845
4efe42d73e58da2cde2c1ed1358dd10c6004fa6b82c856fd70defd3363abe824
```

Linux full suites: host at `47556cce` had 7,682 passed / 1 failed / 47 ignored;
chat at `480176d0` had 7,734 passed / 1 failed / 48 ignored. Both failures are
the unchanged terrain GPU precision assertion, `0.59764713` versus `0.5984283`.
The final host's only subsequent change was a Windows title-borrow lint fix.
Final Linux empty-host smokes verified version/log durability, confirmation and
normal exit with no workers/GPU. They do not validate native adoption.

PR #8 Windows/Linux compiler checks and secret scanning passed. Its macOS REST
job never obtained a runner/steps, including one retry, and was abandoned;
**no macOS compilation, tests or runtime acceptance occurred**. PR CI subsets
do not substitute for the full source-bound portable-build receipts.

Synthetic empty-host screenshots are on `captures` at
`6e7ce3b24aefbe20429871c86801d0c0aa6fc3de`, paths `pr-8/host-empty-cpu-d87.png`
and `pr-8/host-close-cpu-d87.png`. The 36 M2 PNGs and exact Linux binary archive
remain in root `.scratch`; see the workspace inventory below. Do not publish
user crash screenshots or logs containing private account data.

## Preview monitors remain disabled

The goal is view-only 320×180, 480×270 or 640×360 CPU popouts requesting
0.5, 1, 2 or 5 snapshots per second, with exact-source chat launch. Independent
small GPU targets and asynchronous readback must not reactivate full-resolution
background rendering. Those rates are requests, not measured sustained output.

Read [worker dependency order and pending wiring](handoff/PREVIEW_WORKER.md)
and [caller and popout contracts](handoff/PREVIEW_CALLER.md) before integration.
The worker branch contains reviewed source for purpose-separated authentication,
bounded IO, exact source/device/cohort suspension and actual ordered GPU drain.
It holds Activate/new-login warmup/latest viewport re-Prepare until every
participant acknowledges the matching GPU fence. Loss or timeout is not a
completion proof. The 63-bit challenge cap and exact-source disable cleanup are
included in `213d8e49`.

The aggregate 5 FPS reservation/reclamation model is committed but **not declared
or wired into the broker**. Budget uncertain or disconnected participants until
exact apply/drain evidence permits clean reclamation. Add real broker actor
tests proving no Prepare/spawn before all GPU acknowledgments; the proposed actor
fixture was not implemented, and its preparatory wrappers were removed.

App/cached Scene/CPU popout integration is **unreviewed, uncompiled WIP** in
the renderer worktree and in the complete
[gzipped source patch](handoff/preview-caller-ui-wip.patch.gz). SHA-256:
`6b23d11abf7c109da1e3c4a55118bac1a521bb8c1441db17db5902b14603f8bd`.
Decompress the `.patch.gz` before applying; the hash above identifies the
uncompressed patch. It includes all three new source files and tracked edits; apply only to
its recorded `a449faa7` base or reconcile explicitly with the combined cohort.
Do not duplicate `afae5458` / `98c2299d`; M2 already contains the chat dependency.

Pending gates: coherent source review, aggregate admission before init/pose
work, strict compile/tests, small-target Vulkan readback and cost/cadence/memory
measurements, stale-image blanking, capture protection/native recreation,
all-participant suspension and Windows owned-tool/focus/input acceptance.
The 100 ms admission ceiling and 128 MiB reserve are prototype policy, not a
driver-reset or performance guarantee. Capability stays false through handoff.

## Upstream changes

The 16-candidate `a17185fd` branch is preserved and Windows-built, with a frozen
ledger in its `docs/UPSTREAM_REVIEW_20261106.md`. Compared with PR #7 it adds the
wgpu git pin, a fetch comment and microphone-close behavior. Wgpu remains the
same package versions/dependencies; the selected revision changes descriptor
allocation fallback. [Clean source simulations](handoff/UPSTREAM_M2_16_SCREEN_20261106.txt) on frozen M2 left 82 protected
chat/bootstrap/profile/UI files byte-identical. No actual host/chat/upstream
integration or combined runtime claim follows from that simulation.

The newest reviewed official commit was
`4a613cd3e6e3cdc493bd3f98d97d60ec59e31e08` (async logging). It is **deferred**
for host/chat: actual `logging.rs` conflict and checked host-log durability/drain
semantics require deliberate review. It is clean only on the standalone upstream
candidate branch. Do not import it on that basis or extract partial changes.
There are 18 host/chat deferrals through this point; the frozen `a17185fd`
ledger has the earlier 17 by design. The fourth-review record is preserved in
the workspace inventory. No conflict was silently resolved.

## Workspace inventory and safe resumption

| Cloud path | Contents and state |
| --- | --- |
| `/workspace/work/overnight-integration` | Root default development target; current viewport correction. Only one heavy Cargo owner. Logs under `.scratch/viewport-hotfix-*`; M2 captures `.scratch/chat-popout-32b29839-{en,fr,ja}` and provenance `.scratch/host-chat-visual-summary-32b29839.json`; M2 gzip `.scratch/checkpoints/32b298392c7d/aurora-viewer.gz`. |
| `/workspace/work/viewport-toolbar-fix` | Clean UI source `a03cf5df`, branch `fix/viewport-toolbars-only`; already integrated by aliases above. |
| `/workspace/work/viewport-mouselook-cursor` | Clean cursor source `b79199f3`; already integrated. |
| `/workspace/work/session-host-worker` | Clean worker `213d8e49`; disabled preview. Detailed handoff copied into this document set. |
| `/workspace/work/multi-monitor-workspace` | Renderer `a449faa7` plus preserved dirty WIP. Patch and notes copied into this document set; do not reset it. |
| `/workspace/work/windows-build` | Windows artifact receipts and CPU-host smoke evidence under `.scratch`; all owned smoke resources stopped. |
| `/workspace/work/host-windows-private-acl` | `.scratch/UPSTREAM_M2_16_SCREEN_20261106.md`, `.scratch/UPSTREAM_FOURTH_REVIEW_20261106.md`, raw JSON/reproducers. |
| `/workspace/work/upstream-latest-review` | Clean 16-candidate source and frozen conflict ledger. |
| `/workspace/work/foldurora-handoff` | This documentation-only branch, `docs/foldurora-handoff-20261106`, based on frozen main. |

Use these next tasks independently; the user now chooses who resumes them:

1. Finish exact-source Windows/offline and physical viewport acceptance first.
   Verify ordinary windows can be moved/resized outside the world viewport;
   toolbars, camera/photo controls and their menus remain inside. Exercise
   both toolbar styles, option on/off, nonzero X/Y, fractional DPI, toggle/restart
   and saved geometry. In mouselook test relative and remote/absolute movement,
   Alt release, focus loss, viewport edit/resize, drag transitions and escape.
2. Integrate the accepted viewport correction onto M2 in an isolated branch.
   M1/M2 currently inherit the original UI scope bug. Preserve the root's M2
   compile/lint/native ACL/console repairs; do not replace it with the older
   worker author base. Then repeat exact combined checks and native host/chat
   acceptance. Validate taskbar/Z-order, held movement/PTT release, host focus,
   two manual logins, resize/mixed DPI/display removal, worker/host loss and
   exact-account Nearby/existing-IM privacy/send behavior.
3. Assemble and review disabled M3 in dependency order, wire aggregate admission,
   add actor integration coverage, then compile/test/measure before enabling.
   Preserve the independently owned viewport correction in shared files.
4. Integrate the selected upstream changes only in a separate reviewable cohort
   after feature acceptance. Recheck newer upstream changes; preserve risky
   conflicts for the user rather than guessing resolutions.

Read `AGENTS.md`, `ARCHITECTURE.md` and `TASKS.md` in the selected checkout.
Source `/workspace/.setup-tools/activate.sh` before Linux Cargo. Use only the
worktree's default development target and `cargo test --workspace` (filters
allowed). Linux strict Clippy retains the existing `-A dead-code` allowance;
Windows strict checks do not. Keep captures on `captures`. Do not trim other
worktrees' caches or run `new-task.ps1`, which can clean unrelated task caches.

The proven Windows repo is
`C:\Users\Marty\..Projects\Aurora Viewer\Foldurora`; preserve its clean
`work\pr1-build` at `9f389441`. Use a fresh sibling worktree at an exact source,
add `C:\Users\Marty\.cargo\bin` to PATH, then use
`cargo build --locked -p aurora-viewer --features xr-static-loader` and
`scripts\package.ps1`. Rust/Cargo 1.99.1 and VS 2026 x64 were verified earlier.
Use the local computer only for Windows-specific work under existing approval.

Only the human logs into a grid or validates real accounts. Keep main and PR #1
untouched; no merge or release is authorized by this handoff. No further
implementation tasks are being initiated. There is no supported tool here for
the user's banked usage-limit reset, so none was applied.
