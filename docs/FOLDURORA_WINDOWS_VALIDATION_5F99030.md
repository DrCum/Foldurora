# Foldurora Windows validation for 5f99030a7daa

The unfiltered Windows workspace suite failed in the MCP discovery descriptor fixture. The build stopped at that gate. The workspace theme guard that failed on a611 passed here.

## Source and checks

- Repository and branch: `DrCum/Foldurora`, `fix/workspace-theme-integration-20261109`.
- Commit: `5f99030a7daabb2879563c998828a2beb2f7099d`. Tree: `36cb2e1e0e0bec083d19ed993c90810b1be52dd2`.
- The owned Windows worktree was detached at the pinned commit and clean before and after validation. Cargo.lock SHA-256: `00762d1a8752003a27c2a757472942bf4131b8a3dfa2f6faa1851f9e892b3b77`.
- Reused the existing x64 Windows MSVC toolchain and dev target. The runner observed a medium-integrity, non-admin parent and matching TEMP, TMP and .NET temp under user-local temp. These are parent observations. Full launch context remains local.
- Validation ran on 2026-11-09. [Pinned source](https://github.com/DrCum/Foldurora/commit/5f99030a7daabb2879563c998828a2beb2f7099d) · [Windows request](https://github.com/DrCum/Foldurora/blob/54ca46d1c778ce0aa236a6ea101506acf8e38814/docs/FOLDURORA_CONTEXT_WINDOWS_REQUEST_20261109.md)

| Command | Exit | Result |
| --- | ---: | --- |
| `powershell.exe -NoProfile -ExecutionPolicy Bypass -File ./scripts/check-scripts.ps1` | 0 | Passed |
| `cargo fmt --all -- --check` | 0 | Passed |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | 0 | Passed |
| `cargo test --workspace --locked --no-fail-fast` | 101 | 8,935 passed, 1 failed, 58 ignored, 0 filtered, 37 summaries |

The single unfiltered run exported 8,994 named rows. The source-bound audit used the exact 1,134-name a611 baseline and 16 new requirements. Of 1,150 requirements, 1,149 appeared exactly once as passing rows. The sole unmet requirement appeared once as failed. The connection-cap and link/mode checks are gated by `cfg(unix)`. The synthetic stdio integration target is gated by `cfg(target_os = "linux")`. [Unix fixture guards](https://github.com/DrCum/Foldurora/blob/5f99030a7daabb2879563c998828a2beb2f7099d/crates/aurora-mcp/src/ipc/tests.rs#L176-L217) · [Linux stdio guard](https://github.com/DrCum/Foldurora/blob/5f99030a7daabb2879563c998828a2beb2f7099d/crates/aurora-mcp/tests/synthetic_mcp_stdio.rs#L1-L3)

## Failure for the lead

`aurora-mcp` reported 27 passed and 1 failed. In `ipc::tests::wrong_secret_and_stale_incarnation_are_refused`, the in-place `std::fs::write` of the synthetic discovery descriptor at line 122 returned Windows error 32, a sharing violation. The initial wrong-secret assertion and subsequent client connection precede that line. The panic prevents the stale-incarnation and replacement-preservation assertions from running. [Failing fixture](https://github.com/DrCum/Foldurora/blob/5f99030a7daabb2879563c998828a2beb2f7099d/crates/aurora-mcp/src/ipc/tests.rs#L103-L129)

The accept loop rereads the descriptor. The Windows descriptor read handle permits read and delete sharing, so a concurrent open can conflict with the fixture's in-place write. This is a source-based explanation consistent with the failure. The run did not identify the conflicting handle. [Accept loop](https://github.com/DrCum/Foldurora/blob/5f99030a7daabb2879563c998828a2beb2f7099d/crates/aurora-mcp/src/ipc.rs#L312-L335) · [Windows share flags](https://github.com/DrCum/Foldurora/blob/5f99030a7daabb2879563c998828a2beb2f7099d/crates/aurora-mcp/src/ipc/windows.rs#L224-L238)

## Downstream status and retained evidence

The Monitor prototype Clippy, Monitor, render-thread, session-fence and preview gates were **NOT RUN**. The ordinary viewer and sidecar build, version checks, package and ZIP were **NOT RUN**. No package or executable identity is attributable to this commit. CPU captures and native smoke were **NOT RUN**. Workspace, camera, Monitor and host movement/geometry/input acceptance remain open. Driver-reset recovery, XR, Linux terrain and Japanese glyph coverage received no new evidence.

Prior artifact metadata matched for 2,001 files. Hashes matched all 37 retained cache outputs. Registered worktrees and local branch heads matched the expected inventory, and the prior a611 checkout was retained by ref. The frozen source was left clean. No rerun or environment change was used to clear the failure.

Raw evidence and the exact private checklist remain in the builder's local `Builds/Foldurora-5f99030a7daa/logs`. These hashes identify the retained files.

| Local evidence | Bytes | SHA-256 |
| --- | ---: | --- |
| `full-tests.stdout.log` | 870008 | `a4f74d3015e5a1cbbdf078a1d9bd749eb1fdbbf4e191a37b6ec69ac81cfe4676` |
| `full-tests.stderr.log` | 144584 | `aa4187aa931c5e87c4fcb1c947f574faacdfcd71677f535f06b5109c4e37654d` |
| `all-test-names.json` | 2404942 | `d2385ce47a292f6ca6772c77b216406f0970a54a5a80efc838958864f44a55b0` |
| `required-name-audit.json` | 688098 | `4a65995066d1f1098752e57c763e88792d387c1bb28a619cab3b966a26dcf0e4` |
| `final-report.json` | 13107 | `d9dc2c662f5baf88636c86258e6ac9974005320636a6cedc783a03f2792d11f6` |
