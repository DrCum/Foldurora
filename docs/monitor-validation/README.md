# Monitor prototype validation and lead handoff

Frozen implementation: `504ef73c4ca368d5dc565262fafcfc5ae10a0a42`.
Tree: `50c0ed1db845ae8384e21f463728827dadc79909`.
Base: `d0d8f8f5ad73f177160882a4543252e6941d661d`.
The later evidence commit changes documentation/logs only; the PR identifies
both that final head and this tested source. The lead should integrate into its
next owned feature-group branch, preserving all frozen Windows candidates.
There is no renderer/Cargo.lock overlap with PR #20's `a6119664` source.

Linux, Rust 1.99.1 / Cargo 1.99.1, existing `ci` profile and reused
`CARGO_TARGET_DIR=/workspace/Foldurora/target`. One heavy process at a time.
Source checkout stayed frozen during these checks; no dependency versions,
security settings, credentials, grid login, ignored tests or CI gates changed.
Winit was already locked; the renderer gains a test-only dependency edge.

| Command (all from the frozen source) | Result |
| --- | --- |
| `cargo fmt --all --check` | exit 0; empty output |
| `git diff --check d0d8f8f5 HEAD` | exit 0; empty output |
| `cargo clippy --profile ci --locked -p aurora-render --all-targets -- -D warnings -A dead-code` | exit 0 |
| Same Clippy with `--features monitor-prototype` before `--all-targets` | exit 0 |
| `cargo test --profile ci --locked -p aurora-render --features monitor-prototype --lib monitor -- --nocapture` | **10 passed, 0 failed, 0 ignored**, 465 filtered |
| Same render test command with filter `render_thread::tests` | **19 passed, 0 failed, 0 ignored**, 456 filtered |
| Same render test command with filter `session_fence::tests` | **4 passed, 0 failed, 0 ignored**, 471 filtered |
| `cargo test --profile ci --locked -p aurora-session preview_ -- --nocapture` | **29 unit + 1 integration passed**, 0 failed/ignored; other cases filtered |

These filters are explicit, focused regressions, not a full suite. One of the
Monitor cases also runs in the rendezvous filter; do not add the rows as unique
coverage. The Linux dead-code allowance is the repository's existing allowance,
not a new suppression. [Exact serial command outcomes](commands.json).

The evidence-only head preserves raw Cargo output, including terminal blank
lines. Its unrestricted `git diff --check d0d8f8f5 HEAD` reports four log-file
EOF blank lines (exit 2); the same check with
`-- . ':(exclude)docs/monitor-validation/*.log'` passes. No source, test or
Cargo.lock content differs from the tested implementation commit.

The Monitor GPU command ran under:

```sh
source /workspace/.setup-tools/activate.sh
export CARGO_TARGET_DIR=/workspace/Foldurora/target
export AURORA_RENDER_THREAD=1
export AURORA_GPU_VALIDATION=1
xvfb-run -a -s '-screen 0 1280x800x24 -nolisten tcp' \
  cargo test --profile ci --locked -p aurora-render \
  --features monitor-prototype --lib monitor -- --nocapture
```

The environment selects Mesa llvmpipe software Vulkan. The test uses a real
native surface, real Renderer/queue owner, existing uploaded triangle geometry,
scene records, world encoder and completion callbacks. It verifies all three
small output dimensions, real draw counters, native surface dimensions unchanged,
revocation at promotion, no false session presentation proof, and a subsequent
real foreground presentation satisfying a fresh boundary. A separate real GPU
target test covers texture reuse, two-lease capacity, no COPY_SRC/COPY_DST usage,
and delivered-frame revocation/device-loss flags. The other tests cover rate
limits/missed ticks, latest-only delivery, hide/minimize/close suspension model,
resize, cross-lane isolation, late cancellation and queue contention/lifetime.

The missing optional OpenXR runtime prints loader diagnostics in the GPU log;
ordinary Vulkan initialization proceeds and tests pass. This is not XR coverage.
The complete run took 85.48 seconds including cold software-renderer setup;
this number is not a foreground/preview performance comparison.

## Preserved failures

Development iterations were uncommitted and are not attributed to the frozen
source. Their raw logs remain here, including dependency compile output:

- `monitor-check-initial.log`: exit 101, wrong `AntiAliasing::Off` name;
  corrected to the renderer's `None` variant.
- `monitor-compile-tests.log`: exit 101, no `FrameParams::default`;
  replaced by an explicit synthetic fixture adapted from the archived prototype.
- `monitor-tests-first.log`: exit 101, **7 passed / 1 failed**, world test
  timed out at admission. A borrowed queue owner closed the rendezvous on drop.
  Fixed ownership and added a regression that verifies both borrowed-handle
  drop and original-owner closure. Final GPU test passes without skipping it.

No unresolved failure was observed in the final scoped commands. The base's
Windows Clippy failure and unrun full tests/build/package remain independent:
[original builder evidence](https://github.com/DrCum/Foldurora/blob/7491bdee09fac83c80566416086f9c1e4f3a6fe5/docs/FOLDURORA_WINDOWS_VALIDATION_D0D8F8F5.md).
PR #20 carries the lead's correction; this renderer branch does not duplicate it.
No earlier full-suite result is claimed for this source.

## Review and integration request

Keep draft and keep live capability disabled. Review the architecture and
remaining gates in [the foundation note](../FOLDURORA_MONITOR_FOUNDATION.md),
especially authority joins, cached scene preparation, mandatory HUD denial,
aggregate reservations, asynchronous export/freshness/color format, cold init
memory, temporal isolation and ordered promotion. Reconcile the preserved old
preview cohort with this current-renderer prototype before choosing one owner.

Apply the implementation commit and this documentation commit to the next owned
integration branch, not the frozen `a6119664` build checkout. Rerun the affected
checks after integration/conflict resolution. When the related group is complete,
freeze its full new SHA and run the required unfiltered workspace/aggregate
checks and the existing Windows Builder/package process. Do not reinterpret the
scoped passes above as that checkpoint. No standalone viewer package was built
for a prototype with no production caller; no build request was dispatched.

For a scoped native renderer check, the existing Windows Builder can use:

- Repository `https://github.com/DrCum/Foldurora.git`, branch
  `feat/monitor-offscreen-foundation`, source
  `504ef73c4ca368d5dc565262fafcfc5ae10a0a42`, tree above. Fetch and verify clean
  tracked state and exact source. Reuse its owned worktree/compatible cache.
- `cargo fmt --all --check`;
  `cargo clippy --profile ci --locked -p aurora-render --features monitor-prototype --all-targets -- -D warnings`;
  `cargo test --profile ci --locked -p aurora-render --features monitor-prototype --lib monitor -- --nocapture`;
  repeat the test with `render_thread::tests` and `session_fence::tests` filters;
  `cargo test --profile ci --locked -p aurora-session preview_ -- --nocapture`.
  Set `AURORA_RENDER_THREAD=1` and `AURORA_GPU_VALIDATION=1`. The real world test
  is compiled on Windows as well as Linux; it opens only a synthetic local window.
- Return exact source/tree, commands/exit codes, test names/totals, full failure
  logs, adapter/driver and test executable path/version/hash. Stop on failures
  and send source fixes back for integration; do not edit a frozen candidate.
  Viewer/sidecar packaging belongs to the combined group request, not this
  renderer-only check. Preserve existing packages and reproduction evidence.
- No merge, release, installation, grid login, credentials, Windows security
  policy changes or new Builder instance. Actual Windows run and physical GPU
  acceptance are still pending; a prepared request is not confirmed delivery.
