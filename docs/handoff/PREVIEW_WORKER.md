# Session-host worker handoff

Stopped at the user's orchestration boundary. No further work is running in
this worktree or the profile child. No Cargo, native viewer, grid, credentials,
push, merge or release action was performed by this agent.

- Worktree: `/workspace/work/session-host-worker`
- Branch: `feat/session-preview-suspension`
- Exact head: `213d8e498ea8b1e15e121f7186c50a799c9e2b0a`
- Git status: clean at handoff.
- Frozen chat base `a5d708af471f390e5b6f320e001e02fbf8c4a641` and all earlier
  branches remain preserved. This branch does not include root's later M2
  compile/lint/ACL/console repairs; integrate onto the root cohort, not over it.
- `transport::PREVIEW_CAPABLE` remains **false**.

## Ordered preview dependencies

Use the local aliases below OR their originals, never both.

| Local commit | Original / purpose |
| --- | --- |
| `5e008044d298ba2bbe084140d21f4d1fa7b437d4` | `31fdc1cd`: small renderer backend |
| `a362f5fd137a3799920a00eef3bfdc3f823e83f0` | `d5da523c`: completed GPU query evidence |
| `b4eab6922ade5660efe82db542a7e245ce13ea54` | worker standby GPU authority/rearm |
| `e9afa88897df5a4353080464b8b304b79352caf2` | `2d7e14bf`: shared purpose-bound auth |
| `ab5a110169fbd28009fbaeaaa557b059a5308770` | `5715c528`: preview DTO/mailbox/cache |
| `3ab36fa6301f0ee54dd68f22572d1ce6d43ce91a` | `4ffe335a`: dedicated preview IO |
| `da243f1fd04daab9a921bc8d41515bd9579dd0e3` | private bootstrap, sockets, broker, worker hooks, disabled |
| `aa8b13daa165ae24fa0b931485b72b22e9b91e0a` | `64c709f7`: exact cohort barrier model |
| `ab7d3562badd59867641c72849f804b48ac0bad8` | `dfdedb80`: suspension wire/mailbox/init-order tests |
| `5ddec58d31180641d8bdf34e9e91d6f6141a5a0a` | `23f01e7f`: suspension IO forwarding |
| `fed4084c943c847e139a37df6e153ef7b6d8a712` | actual broker holding + worker GPU drain |
| `4f0dd295a4d858bda85e57b3d6f8d87679c5dd6f` | `28539bb7`: retire authority before metadata lock |
| `d88eeae36561c8b682788e7578f23ffc02696b38` | `45bbd78d`: pure aggregate policy ledger |
| `93384df38d71fc7729ad8366de32e990b7b10115` | `7ed09174`: explicit verified tombstone reclamation |
| `213d8e498ea8b1e15e121f7186c50a799c9e2b0a` | shared 63-bit nonce bound + exact-source disable cleanup |

Separate chat fix: `0560e8e2e16931cd609f8522f6f347ae84593e6f` keeps
chat IO revision zero terminal. Root owns its integration and tests.

## Truly connected source

- Dedicated private preview key, endpoint-selected distinct Chat / PreviewControl /
  PreviewImage HMAC purposes, small control and separate bounded binary-image
  sockets, worker IO, and latest-only host cache.
- Fresh source/control/actual child PID/device metadata, fresh completed query
  evidence, strict local challenge age and conservative rounding. Cached query
  statistics cannot renew admission.
- Exact GPU suspension messages and atomic retirement before producer/metadata
  lock contention. Old post-resume commands cannot poison the current token.
- Worker GPU suspension gates world/UI submission until its actual ordered
  fence completes. Input/native ownership remains truthful. Pending init writes
  precede that fence through the existing renderer rendezvous; callbacks cannot
  create later optional GPU submissions.
- Actual broker holds Activate, latest viewport re-Prepare and new-login warmup
  until every exact cohort GPU acknowledgment. It reconciles the complete live
  source/device roster again before consuming readiness. Loss, timeout and
  cancellation are not quiescence. Geometry moves park the old fitted world
  first; cancel may leave it Standby requiring Activate. No-preview M1 route is
  unchanged.
- Critical suspension/proof TCP frames survive ordinary latest status replacement.
- Exact-source `set_preview(source, None)` queues even if opening eligibility
  disappeared; it cannot hide a later login or clear uncertain GPU authority.

## Pending; do not enable

1. `preview_policy.rs` is **pure and not declared/wired** in the broker. Wire its
   combined 5 FPS exact-rate reservations BEFORE optional init/pose work. Keep
   max acknowledged/unacknowledged/requested history until exact native apply /
   actual GPU drain. Loss/reconnect never frees uncertain history.
2. Use explicit verified hidden/apply/drain/zero-charge reclamation before
   clearing a peer's ever-authorized flag or pruning its clean exited Child.
   Existing M1 Child retention otherwise limits process churn. Unknown/undrained
   sources must retain bounded recovery state, not be silently evicted.
3. Multi-monitor author's App/Scene/CPU window/UI/locales hooks remain a separate
   worktree/checkpoint. They are not in this branch. Whole App/popup/cohort source
   integration has not been cleared, compiled or run.
4. No production Broker actor integration fixture was implemented. Only its
   isolated branch and read-only design existed at stop. Preparatory synthetic
   process wrapper, module declaration and visibility changes were REMOVED; no
   missing test file/reference or production process abstraction remains.
5. Add actual broker no-Prepare/no-spawn-before-all-GPU-acks integration coverage,
   then formatting, strict Clippy, workspace tests, actual small-target GPU and
   native focus/taskbar/geometry/policy acceptance at one exact integrated tree.
   Keep the 100ms GPU threshold and 5 FPS cap documented as prototype policy,
   not performance guarantees.

## Evidence limits

Individual auth, framing, DTO/mailbox, IO, pure barrier, worker GPU gate, broker
holding, pure policy/reclamation and final cap/disable deltas received independent
source reviews. The whole preview feature has **not** received enablement clearance.
Formatting and whitespace checks passed. All preview tests in this range remain
unrun here; there is no native or GPU performance acceptance from this agent.

The child is stopped, clean at branch `test/session-preview-actors`, head
`fed4084c943c847e139a37df6e153ef7b6d8a712`. Its previously delivered branches and
commits above are preserved. Parent/root has separate M1/M2 test/build/fixture
evidence; do not attribute that evidence to this preview branch.
