# Windows native prerequisite — Inventory/host group

Prepared for Slick to relay ONCE to the EXISTING Foldurora Windows Builder on
Cumputer, thread `01a1af77-a465-71c3-be08-cd9d1b927f83`. Check current history and
whether already delivered before dispatch. Current app relay is unavailable to
the lead; preparation/publication is not delivery, acknowledgment or execution.
Do not repeat the completed Monitor520 build. Do not start two per-feature runs.

This is a small native prerequisite within ONE combined Inventory/host group,
**not a final viewer build request**. Native placement is a prerequisite in the
reviewed plan. Inventory is integrated but the production host repair still
needs its popup/input/authority join. Do not build/package that unfinished repair
or call this source host-fixed. Return this preflight first so the lead can
finish production integration and freeze the eventual combined product gate.
No unfiltered suite, Inventory-only package or repeated capture matrix now.

## Exact source

- Repository: https://github.com/DrCum/Foldurora.git
- Branch: `feat/inventory-host-frame-integration-20261109`
- Commit: `dab73b2b70c1a2e35ffc331e5c4cc7b2c9b96415`
- Tree: `7e034cd93da34686608bfded83b8e27def7bfaeb`
- Canonical Git-LF Cargo.lock SHA-256:
  `e04f6814529802ba6469b9455294aabfcfbec989d0e271698b3723a04d682e32`

Reuse the clean owned camera-6a3009f-windows worktree and compatible DEV caches,
existing Rust/MSVC tooling. Preserve all prior worktrees, branch heads, logs,
profiles, cached outputs and verified Monitor/radio/98b/a903 packages before
checkout. Do not change Windows settings, ACLs, security, long-path policy,
source, tooling/dependencies or credentials. No cleaning, grid login, install,
merge or release. If busy/dirty or native desktop unavailable, return that exact
blocker; do not replace Builder or create another cache/workspace.

## Sequential commands and expectations

Record exact commands, exits, output and parent/child context actually observed.
Verify canonical Git blob and CRLF-normalized lock without changing Git settings.
Any failed/inconclusive gate stops; return evidence without fixes, unchanged
reruns, focus injection, waivers or suppression.

```powershell
if ((git rev-parse HEAD).Trim() -ne 'dab73b2b70c1a2e35ffc331e5c4cc7b2c9b96415') { throw 'Wrong source' }
if ((git rev-parse 'HEAD^{tree}').Trim() -ne '7e034cd93da34686608bfded83b8e27def7bfaeb') { throw 'Wrong tree' }
git status --short
powershell.exe -NoProfile -ExecutionPolicy Bypass -File ./scripts/check-scripts.ps1
powershell.exe -NoProfile -STA -File ./scripts/probe-host-frame.ps1 -CompileOnly
cargo fmt --all -- --check
cargo clippy --locked -p aurora-viewer --example host_native_frame -- -D warnings
cargo build --locked -p aurora-viewer --example host_native_frame
Get-FileHash -Algorithm SHA256 .\target\debug\examples\host_native_frame.exe
.\target\debug\examples\host_native_frame.exe
```

The example starts only its own synthetic host/worker/foreground-app processes;
it uses no viewer/profile/grid/MCP transport or real GPU authority. The inherited
Monitor dependency remains enabled; do not change the manifest or strip it.
XR-static loader remains an eventual combined package requirement; this example
has no renderer/runtime and does not establish XR acceptance. No AURORA_* override
or production authority is required. It deliberately pauses only its own child
UI for 1.8 seconds and has a 20-second controller deadline plus checked cleanup.

Require exit 0 AND final `PASS ... checked child cleanup`. Require all preceding
native assertions, including exact third-process identity, both pending-target
samples, unblocked worker-focus input baseline, cover-only refusal with unchanged
focus/geometry, activation while paused, minimized and restored old-location
black samples completed before the worker can resume, stale PID/registration
refusal, and normal exits. Observe native cover call durations below 200 ms;
later pixel samples are NOT instantaneous compositor proof. OS foreground
refusal/locked desktop is inconclusive and must not be forced or called a pass.

If the owned desktop is available, optionally run the original PR44 probe:
`powershell.exe -NoProfile -STA -File ./scripts/probe-host-frame.ps1`.
Inspect its own Normal -> Region -> Clear controls, native caption/buttons/
border and OS snap policy on an innocuous background. Record each observation
and DPI/theme/Windows/local-or-remote context. Do not mark a DWM HRESULT, query
result or synthetic example as visual production acceptance. If interactive
inspection is unavailable, say NOT RUN. No user grid/account interaction.

## Return and next gate

Return one sanitized GitHub-accessible result through Slick: exact source/tree/
canonical lock; commands/exits; actual Windows/toolchain/DPI/theme and observed
launch context (never infer a child token from parent); example executable path,
bytes and SHA-256; every assertion and failed/inconclusive/unrun case; normal vs
forced child cleanup; private logs/captures and preservation outcomes. The proof
has no viewer version or product package: report those NOT PRODUCED, rather than
borrowing Monitor520's identities. Raw logs and native screenshots stay private.
No final combined source/request or product package is claimed by this preflight.

The eventual combined product request must retain the Builder's 1,413-case
Monitor declarations and exact source-bound full-run audit, reconcile the retired
singleton-toggle case against the user's new behavior, include the eight new
ordinary Inventory/clipboard/workspace regressions and changed Folder Jump/
workspace-owner gates, and add the actual production presentation tests. Keep
all unrelated security/permission tests. The large Inventory profile remains
ignored unless explicitly requested; report performance honestly with no new
cap. Preserve Monitor/XR features, strict workspace gates, the separate GPU
export gate, source-stamped viewer/sidecar build and built/staged/ZIP hashes,
required MCP/licensing documents, and scoped native/human acceptance. That
product stage awaits completed production source and a NEW exact pin, not edits
by Builder to this frozen checkpoint.
