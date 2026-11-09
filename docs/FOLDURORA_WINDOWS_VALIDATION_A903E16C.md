# Foldurora Windows validation for a903e16c01db

The Windows default and nondefault prototype gates passed. The ordinary viewer and sidecar portable ZIP was built and verified locally. The isolated native startup capture passed. The login-bound workspace and hosted interaction cases remain open.

## Source and default gates

- Source branch: `fix/mcp-descriptor-regression-20261109`.
- Commit: `a903e16c01db412b0a7fd77886851be3145e1852`. Tree: `a338a4b88a8a4e660efa59ad71d18878dc603e91`.
- Canonical Cargo.lock SHA-256: `27c0735eab8d015661ef42be7652e6561928aa9e48e570d30e451e7dc7981497`. The initial verifier compared the Windows CRLF checkout bytes to the canonical LF hash. The Git blob and the LF-normalized checkout independently matched the pin. No source or Git settings were changed to resolve the comparison.
- The owned build worktree was clean before and after validation. Existing x64 MSVC tooling and the persistent dev target were reused. The observed launcher had a medium-integrity non-admin token and consistent temporary directory resolution. Child token and effective temporary directory were unobserved. [Pinned source](https://github.com/DrCum/Foldurora/commit/a903e16c01db412b0a7fd77886851be3145e1852) · [Windows request](https://github.com/DrCum/Foldurora/blob/f305efdc55dc0d6d8f71ea9849b1f9e748e8df7a/docs/FOLDURORA_ADVANCED_CONTEXT_WINDOWS_REQUEST_A903E16C.md)

| Command | Exit | Actual result |
| --- | ---: | --- |
| `powershell.exe -NoProfile -ExecutionPolicy Bypass -File ./scripts/check-scripts.ps1` | 0 | Passed |
| `cargo fmt --all -- --check` | 0 | Passed |
| `cargo clippy --locked -p aurora-mcp --all-targets -- -D warnings` | 0 | Passed |
| `cargo test --locked -p aurora-mcp --no-fail-fast` | 0 | 30 passed, 0 failed, 0 ignored, 0 filtered, 4 summaries |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | 0 | Passed |
| `cargo test --workspace --locked --no-fail-fast` | 0 | 8,970 passed, 0 failed, 58 ignored, 0 filtered, 37 summaries |

The original stale-incarnation fixture and both added descriptor fixtures passed in the early Windows MCP run and the full workspace run. The passing held-reader case executes the Windows-only assertion for sharing violation 32 and validates the replacement while the original reader stays open. The same-file-identity cleanup case also passed. The two Unix-gated IPC cases and the Linux-gated stdio integration cases were outside Windows coverage. [Descriptor regressions](https://github.com/DrCum/Foldurora/blob/a903e16c01db412b0a7fd77886851be3145e1852/crates/aurora-mcp/src/ipc/tests.rs#L103-L202) · [Linux stdio guard](https://github.com/DrCum/Foldurora/blob/a903e16c01db412b0a7fd77886851be3145e1852/crates/aurora-mcp/tests/synthetic_mcp_stdio.rs#L1-L3)

The source-bound audit matched 1,184 of 1,184 requirements exactly once as passing from the single full-run result set. Early package and feature runs contributed zero rows to that audit. Raw command logs, coverage inputs and detailed audits remain local.

## Separate Monitor prototype gates

Each child received `AURORA_RENDER_THREAD=1` and `AURORA_GPU_VALIDATION=1`. The inherited values were preserved.

| Command | Exit | Passed | Failed | Ignored | Filtered |
| --- | ---: | ---: | ---: | ---: | ---: |
| `cargo clippy --locked -p aurora-render --features monitor-prototype --all-targets -- -D warnings` | 0 | | | | |
| `cargo test --locked -p aurora-render --features monitor-prototype --lib monitor -- --nocapture` | 0 | 10 | 0 | 0 | 465 |
| `cargo test --locked -p aurora-render --features monitor-prototype --lib render_thread::tests -- --nocapture` | 0 | 19 | 0 | 0 | 456 |
| `cargo test --locked -p aurora-render --features monitor-prototype --lib session_fence::tests -- --nocapture` | 0 | 4 | 0 | 0 | 471 |
| `cargo test --locked -p aurora-session preview_ -- --nocapture` | 0 | 30 | 0 | 0 | 73 |

There were 62 distinct crate/test names. The Monitor and render-thread filters both passed `render_thread::tests::monitor_reservation_under_contention_does_not_prepare_or_consume_previous_result`. The Monitor filter included the real GPU target and native-window world packet fixtures. The prototype logs did not emit adapter or driver identity. The separate standalone smoke logged its own Vulkan adapter and driver.

## Ordinary dev package

`cargo build --locked -p aurora-viewer -p aurora-mcp --features aurora-viewer/xr-static-loader` passed with exit 0. The ordinary package excludes `monitor-prototype`. Both version probes and sidecar `--help` exited 0. The viewer stamp matched on the first build. `scripts/package.ps1` exited 0 with both executables. The staged package includes `FOLDURORA_MCP.md`, `NOTICE-MCP.txt`, `aurora-install.json` and `foldurora-build-info.json`. Each executable's built, staged and ZIP-entry SHA-256 matched.

| Artifact | Version | Bytes | SHA-256 |
| --- | --- | ---: | --- |
| `aurora-viewer.exe` | `Aurora Viewer 0.9.3-beta.20261106.1+511.ga903e16c0` | 171362816 | `083df4a8c97cd9b385e3c76f4ef4b7a4b8f6eb6c38ffce7d1b75248b3af175d7` |
| `foldurora-mcp.exe` | `foldurora-mcp 0.1.0` | 3039232 | `f925d6c0ecea78b1b46780366f51904d984b89bbfa522292699d00a21c46d90b` |
| `Foldurora-a903e16c01db-windows-x64.zip` | | 72634257 | `fc3e5c27669a455ef2aab8ac022d3f5ae0910855a09c3df1b6fd919a532363e7` |

Package, logs and source-bound receipts are local under `Builds/Foldurora-a903e16c01db`.

## Offline and native scope

- **PASS, standalone startup.** The verified package copy ran the isolated offline demo with the default theme and Vulkan, captured one 1280 × 800 frame at 96 DPI, and auto-exited 0 after 24.43 seconds. The inspected image shows the synthetic world, People, Conversations and the offline demo indicator. Twenty-four passive MCP pipe prefix samples saw zero new matching names. The owned fixture contained zero controller descriptors.
- **NOT RUN, workspace manager and source-bound contexts.** The capture did not exercise manager layout in both styles or establish an authoritative source for Save/Update, Preview/Cancel/Previous, backgrounds, camera/graphics, HUD, expanded folders, archive, persistence or source changes.
- **NOT RUN, hosted Monitor and host behavior.** The standalone fixture did not establish matching host/workers and a LastActive frame source. First-open/reopen/privacy, movement and geometry, mixed DPI, responsiveness, switching, held input and viewport margins remain open. GPU reset, XR and grid/RLVa acceptance remain open.
- **NOT RUN, fresh CPU capture.** Prior captures were retained. No unrelated matrix was repeated.

Prior artifact metadata matched for 2,074 files and all 129 retained cached outputs matched their recorded hashes. Prior worktrees and branch heads matched their expected inventories. The previous 5f990 checkout is retained by ref. The package and screenshot remain local.
