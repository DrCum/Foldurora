# Corrected native prerequisite — 6451f7ae

ONE request through Slick to the EXISTING Foldurora Windows Builder on Cumputer,
thread `01a1af77-a465-71c3-be08-cd9d1b927f83`. Before dispatch, check its current
history/availability and whether this request was already received. Do not send
directly or duplicate it. DAB completed once and remains inconclusive; this is
materially changed source, not an unchanged retry. No new run is claimed yet.

This is the explicitly authorized separate native prerequisite within the ONE
combined Inventory/host feature group. It is not an Inventory-only product build
or the final combined package. **Independent source review is the first gate.**
Production host styling, popup routing, real session/GPU/input authority and human
acceptance remain open; this source must not enable them.

## Pinned source and changes

- Repository: https://github.com/DrCum/Foldurora.git
- Branch: `fix/native-frame-foreground-preflight-20261110`
- Source: `6451f7aeeda035b76ef9d06c8911573aa68d15e0`
- Tree: `5c30f3bb47ad92a2948e3f0cabcd1967e8b7e57c`
- Canonical Git-LF Cargo.lock SHA-256:
  `e04f6814529802ba6469b9455294aabfcfbec989d0e271698b3723a04d682e32`
- DAB comparison: `dab73b2b70c1a2e35ffc331e5c4cc7b2c9b96415`.

The third synthetic app had granted foreground eligibility only to the worker.
The worker now verifies actual foreground and exact host registration, then calls
checked AllowSetForegroundWindow for its exact controller PID before pausing.
This is ordinary process-specific activation eligibility, not an ACL/registry/
security-token/global foreground-policy change. No ASFW_ANY, simulated Alt/input,
AttachThreadInput, foreign-window modification or repeated activation request.
The source gap may explain DAB, but its actual cause is still unidentified.

The controller records ONE SetForegroundWindow BOOL and observes actual host
foreground and existing guarded exposure over its own message-pump turns, for
at most 250 ms and before pause+900 ms. The same 1.8-second pause, original actual
host/exposure assertions, 200-ms cover bound and later tests remain. No late or
refused activation is passed. A 256-record memory trace observes native identity,
thread/order/activation and exposure entry/exit, printing only after timed work
and cleanup. Dropped records prevent final PASS. SetForegroundWindow's LastError
is not treated as an authoritative cause.

Formatting and strict small MSVC-target metadata-harness Clippy passed for this
pin. That is not Windows linking, native execution or independent code review.
Inventory, ordinary host/worker/session/renderer/MCP sources, protocol and lock
are unchanged from DAB. Popup preparation58fe4041 remains separate and unwired.

## Preserve and prepare

Reuse the clean owned camera-6a3009f-windows worktree, existing DEV cache and
MSVC/Rust1.99.1 tooling. Preserve DAB source, its retained example and raw result,
Monitor520/radio/98b/a903 packages, existing caches, profiles, logs/screenshots,
worktrees and branch heads. DAB's retained example is 397312 bytes, SHA-256
64abcb4f6b05e03354ec2e51bc3798256ad4ba42fb013b088b9ec89c460f16fb.
Verify that retained copy before overwriting target/debug/examples output.

Stop for a busy/dirty owned worktree or unavailable desktop. No replacement
Builder/worktree/cache, broad cleanup, tool/dependency update, source repair,
permissions/policy change, credentials, grid login, installation, merge or release.
Preserve actual parent/child observations separately; never infer a child token
or TEMP from its parent. Raw evidence stays local/private.

Branch tip may be a later documentation-only descendant. Verify that the exact
pin is present and has the requested tree; do not reject it merely because tip
is newer, and do not validate the documentation tip instead.

```powershell
git status --short
git fetch origin fix/native-frame-foreground-preflight-20261110
git merge-base --is-ancestor 6451f7aeeda035b76ef9d06c8911573aa68d15e0 FETCH_HEAD
if ($LASTEXITCODE -ne 0) { throw 'Pinned source is not in fetched branch' }
```

## Gate 0 — independent read-only diff review

An agent independent of the lead author must review the concrete correction.
The existing Builder may do this itself, before compiling/running. If it cannot,
return this exact blocker through Slick; do not silently waive the gate or run
natively. Return actionable findings for lead integration/new pin; do not fix
source in the Builder worktree.

```powershell
git diff --check dab73b2b70c1a2e35ffc331e5c4cc7b2c9b96415 6451f7aeeda035b76ef9d06c8911573aa68d15e0
git diff --unified=40 dab73b2b70c1a2e35ffc331e5c4cc7b2c9b96415 6451f7aeeda035b76ef9d06c8911573aa68d15e0 -- crates/aurora-viewer/examples/native_frame/proof.rs crates/aurora-viewer/src/session_host/frame_guard.rs
```

Review exact-PID/registration and actual-foreground checks before eligibility
handoff, one-request/strict pause deadline, trace boundedness/reentry/lifetimes,
unchanged coverage/input assertions and checked cleanup. Confirm the trace
cannot create authority or alter window order. The target host belongs to the
calling controller thread; generic async different-target-queue behavior alone
is not an explanation for DAB. Record review scope and findings/no findings.

## Gates 1–2 — build and ONE native invocation after review clears

```powershell
git switch --detach 6451f7aeeda035b76ef9d06c8911573aa68d15e0
if ((git rev-parse HEAD).Trim() -ne '6451f7aeeda035b76ef9d06c8911573aa68d15e0') { throw 'Wrong source' }
if ((git rev-parse 'HEAD^{tree}').Trim() -ne '5c30f3bb47ad92a2948e3f0cabcd1967e8b7e57c') { throw 'Wrong tree' }
git status --short
cargo fmt --all -- --check
cargo clippy --locked -p aurora-viewer --example host_native_frame -- -D warnings
cargo build --locked -p aurora-viewer --example host_native_frame
Get-FileHash -Algorithm SHA256 ./target/debug/examples/host_native_frame.exe
./target/debug/examples/host_native_frame.exe
```

Verify canonical Git-LF and CRLF-normalized lock without changing Git settings.
Check the production host mod.rs/worker.rs/native.rs, viewer manifest, session,
renderer and MCP trees match DAB; Monitor remains enabled and protocol unchanged.
No AURORA_* override, source/profile/grid authority or new GPU policy is needed.
The three synthetic processes act only on windows they create. The example has
no product viewer version or package. Do not borrow Monitor520's identities.

Use the owned active local desktop and innocuous background, with no competing
interaction/automation. Record real window DPI/theme/desktop context. A failure
stops downstream work; no source/environment fix, native retry, focus injection,
ASFW_ANY/queue attachment/foreground-policy workaround or assertion waiver.

Require exit0, the final checked-cleanup PASS, zero dropped trace records and all
native assertions. Specifically require checked worker-to-controller eligibility;
recorded SFW BOOL and actual host foreground+guarded exposure before the stated
live-pause deadline; both minimized/restored black samples before worker resume;
second pending target; stale PID/registration refusal; detach/hide and normal
child exits. Initial observations from DAB are not passes for this pin. Preserve
all missing/unreached cases if it again returns inconclusive.

The native trace must retain request-before/after, caller/host/foreground thread
facts, activation/position and exposure observations as actually produced.
If the API refuses or actual foreground remains elsewhere, report those facts
without selecting an unproven cause. GUI-query failure is unobserved information,
not active/focus==0 proof. Do not publish raw window/GUI metadata or logs.

No full workspace suite, Monitor matrix, GPU gate, CPU capture matrix, viewer/
sidecar product build or packaging is requested in this preflight. Unchanged
PowerShell/C# compile checks belong to DAB and are not relabeled new passes.
PR44's interactive Normal/Region/Clear and visual caption/snap acceptance remain
NOT RUN; they are not replaced by source review or a native API return.

## Return through Slick

Return ONE sanitized GitHub-accessible result with exact source/tree/lock,
independent-review outcome, commands/exits, actual toolchain/desktop/DPI/theme
and observed parent/child context; executable path/bytes/SHA-256 and retained
copy verification; all assertion scopes, SFW BOOL, eligibility outcome, timing,
trace completeness and any inconclusive/failing/unrun cases; normal vs forced
child cleanup and source/artifact/cache preservation. Raw logs/metadata stay
private. No product executable version or ZIP: report NOT PRODUCED.

Preserve the immutable DAB inconclusive report:
https://github.com/DrCum/Foldurora/blob/91e1900b9fa08eee65bb5bcb99db9e08c006de3c/docs/FOLDURORA_HOST_FRAME_PREFLIGHT_DAB73B2B.md
Even a successful corrected example is partial prerequisite evidence only. The
lead still must join actual same-context popup surfaces/input/IME, exact native/
lease/GPU authority and both placement paths, independently review the production
change, then freeze ONE combined Inventory/host full suite/audit/XR+Monitor build
and verified package. Hosted/UI/grid/RLVa, privacy recovery, GPU-reset and XR
acceptance remain open. No production enablement or overall completion follows
from this preflight.
