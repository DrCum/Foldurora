# Foldurora Windows native-frame preflight for dab73b2b70c1

**Inconclusive.** The static checks and example build passed. The one native run exited 1 because the synthetic controller window did not become foreground after its explicit host activation request while the worker was paused. Both synthetic children exited normally. The final checked-cleanup PASS marker was absent. [Foreground assertion](https://github.com/DrCum/Foldurora/blob/dab73b2b70c1a2e35ffc331e5c4cc7b2c9b96415/crates/aurora-viewer/examples/native_frame/proof.rs#L647-L663)

## Exact source and execution

- Branch `feat/inventory-host-frame-integration-20261109`, commit `dab73b2b70c1a2e35ffc331e5c4cc7b2c9b96415`, tree `7e034cd93da34686608bfded83b8e27def7bfaeb`.
- Canonical Git-LF Cargo.lock SHA-256 `e04f6814529802ba6469b9455294aabfcfbec989d0e271698b3723a04d682e32`. The CRLF checkout normalized to the same digest. The source stayed clean and the manifest retained its Monitor dependency.
- The first preparation wrapper stopped before checkout because the fetched branch tip was the newer request-document commit. The pinned source was independently verified as an ancestor with the requested tree. Preparation resumed from that successful fetch and checked out the pin. No native invocation had started. [Pinned source](https://github.com/DrCum/Foldurora/commit/dab73b2b70c1a2e35ffc331e5c4cc7b2c9b96415) · [Frozen request](https://github.com/DrCum/Foldurora/blob/c3d3bc611a415bf15dd736df2d2ff0f4da6bd53a/docs/FOLDURORA_HOST_FRAME_PREFLIGHT_REQUEST_DAB73B2B.md)
- Windows build 26200, active local console with the default input desktop, and dark app/system theme preferences. The synthetic host window measured 96 DPI. All three synthetic processes passed their per-monitor-v2 DPI setup checks. The observer used read-only desktop and window queries. The launcher observed a medium-integrity non-admin parent and consistent temporary directory settings. Child token and effective temporary directory were unobserved.
- Existing Windows x64 MSVC tooling and dev target with rustc 1.99.1 and Cargo 1.99.1. There were no added `AURORA_*` overrides.

| Command | Result |
| --- | --- |
| `powershell.exe -NoProfile -ExecutionPolicy Bypass -File ./scripts/check-scripts.ps1` | Exit 0. Windows PowerShell 5.1 parser passed. |
| `powershell.exe -NoProfile -STA -File ./scripts/probe-host-frame.ps1 -CompileOnly` | Exit 0. Embedded C# compiled. |
| `cargo fmt --all -- --check` | Exit 0. |
| `cargo clippy --locked -p aurora-viewer --example host_native_frame -- -D warnings` | Exit 0. |
| `cargo build --locked -p aurora-viewer --example host_native_frame` | Exit 0. |
| `Get-FileHash -Algorithm SHA256 ./target/debug/examples/host_native_frame.exe` | Succeeded. Built and retained copy hashes matched. |
| `./target/debug/examples/host_native_frame.exe` | Exit 1. Foreground assertion inconclusive. Elapsed 3.12 seconds. |

The example at `work/camera-6a3009f-windows/target/debug/examples/host_native_frame.exe` is 397312 bytes with SHA-256 `64abcb4f6b05e03354ec2e51bc3798256ad4ba42fb013b088b9ec89c460f16fb`. An identical copy is retained under `Builds/Foldurora-dab73b2b70c1`.

## Native assertions actually observed

| Scope | Outcome |
| --- | --- |
| Registration and identity | Duplicate worker registration refused without replacing the constraint. Exact spawned worker PID and distinct third-process ownership passed. |
| Initial placement and pending target | Exact submitted native target passed. The shown target stayed covered before acknowledgement. The first pending sample verified controller-owned hit target and opaque pixels. The second pending sample was **NOT RUN**. |
| Exposed world and popup | The exact acknowledgement exposed the world. The worker stayed independently unowned. Host, popup and worker passed the no-global-topmost checks. The exposed world outside the popup and the popup over the world matched their native hit targets and expected pixel colors. |
| Third app and worker foreground | The distinct owned third app became foreground and stayed above the workspace in both hit testing and the pixel sample. The subsequent worker foreground request passed. |
| Paused-worker cover and input | The pause receipt was fresh. The unblocked exact worker-focus input baseline passed. The cover kept worker focus, closed the input gate with focus and geometry unchanged, and took **3.720 ms**, below the 200 ms bound. Exact worker input reopened after synthetic authority returned. This measured the native call, with later compositor samples separate. |
| Host foreground while paused | **INCONCLUSIVE.** Immediately after `SetForegroundWindow(host)`, `GetForegroundWindow()` did not equal the owned host. The invocation did not identify why. The paused-host activation assertion was not reached. |
| Subsequent cover, minimize/restore and retirement | **NOT RUN.** Second cover measurement, minimized/restored old-location black samples before worker resume, subsequent placement and pending target, stale PID/registration refusal, and detach/hide checks were not reached. |
| Child cleanup | The outside app and worker both exited normally with code 0. No forced cleanup was logged. The controller returned the foreground error and did not emit the final PASS marker. |

No focus was injected to recover the foreground check. There was one native invocation. The optional interactive PR44 Normal/Region/Clear, native caption/border/buttons and snap inspection was **NOT RUN**. The C# compile check does not exercise that UI. No screenshot was produced. The native hit and pixel observations are recorded in the private logs.

## Scope and preservation

Product viewer version and package are **NOT PRODUCED**. The unfiltered suite and prior Monitor matrix were **NOT RUN**. Production host placement/input, egui routing, real GPU/session authority, hosted recovery and visual acceptance remain open.

Preservation checks matched metadata for 3,773 prior artifacts and hashes for 236 retained cached outputs. Of those, 156 byte-identical outputs reused existing verified copies. Worktrees and branch heads matched their expected inventories and the Monitor520 checkout is retained by ref. The Monitor520, radio, 98b and a903 ZIPs independently matched their recorded SHA-256 after the run. Raw logs and launch context stay local under `Builds/Foldurora-dab73b2b70c1/logs`.
