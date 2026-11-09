# Foldurora Windows validation for ec0bcb52e8e4

The unfiltered Windows workspace suite stopped the candidate with two required viewer failures. PowerShell parsing, formatting and strict workspace Clippy passed.

## Pinned source and launch

- Branch `feat/upstream-hover-controls-integration-20261109`, commit `ec0bcb52e8e4f7062bbb714f10bf2cbc7ca41e09`, tree `65a0efe8d1287047ff430782efd74d20339af7c8`.
- Canonical Git-LF Cargo.lock SHA-256 `c869d88f1f837f302fde6ac52dfaa5e7ff0a325feac9ddf6f77da5954a7e852a`. The CRLF checkout normalized to the same digest. The build worktree stayed clean.
- Existing Windows x64 MSVC dev target, rustc and Cargo 1.99.1. The launcher observed a medium-integrity non-admin parent and matching temporary directory settings. Child token and effective temporary directory were unobserved. [Pinned source](https://github.com/DrCum/Foldurora/commit/ec0bcb52e8e4f7062bbb714f10bf2cbc7ca41e09) · [Pinned Windows request](https://github.com/DrCum/Foldurora/blob/a18cf58040540f35f315eb6d672dcde6cbfe5071/docs/FOLDURORA_COMBINED_WINDOWS_REQUEST_EC0BCB52.md)

| Command | Exit | Actual result |
| --- | ---: | --- |
| `powershell.exe -NoProfile -ExecutionPolicy Bypass -File ./scripts/check-scripts.ps1` | 0 | Passed |
| `cargo fmt --all -- --check` | 0 | Passed |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | 0 | Passed |
| `cargo test --workspace --locked --no-fail-fast` | 101 | 9,160 passed, 2 failed, 61 ignored, 0 filtered, 37 summaries |

The separate MCP package gate was skipped as requested. The MCP and session Git trees and lockfile blob match 1ba byte for byte. Its 68-case early run remains qualified prior-source evidence. The current audit uses only the 9,223 named rows from the one full workspace run. Of 1,364 required cases, 1,362 passed exactly once. Each of the two unmet requirements appeared exactly once as a failed row.

## Failures for the lead

- `app::mcp::catalogue_tests::every_character_tool_has_one_native_adapter_or_closed_runtime_disposition` panicked at `app/mcp.rs:599`. The catalogue found zero dispositions for `workspace_apply`, where the invariant expects one. The independent native-capability refusal test passed. The read-only adapter advertises List, Status and Preview. The apply/previous bridge remains unsupported. [Failing invariant](https://github.com/DrCum/Foldurora/blob/ec0bcb52e8e4f7062bbb714f10bf2cbc7ca41e09/crates/aurora-viewer/src/app/mcp.rs#L579-L612) · [Read-only adapter and refusal test](https://github.com/DrCum/Foldurora/blob/ec0bcb52e8e4f7062bbb714f10bf2cbc7ca41e09/crates/aurora-viewer/src/app/mcp/workspaces.rs#L10-L12)
- `workspace::images::tests::workspace_picture_policy_requalifies_exact_native_item_permissions_archive_source_and_privacy_without_fetches` panicked at `workspace/images/tests.rs:18` with `workspace image ordinary native projection did not settle`. The shared pump did not observe Ready within its 256 turns. The same helper is used at several transitions, and the panic did not identify which call or the underlying cause. [Fixture and pump](https://github.com/DrCum/Foldurora/blob/ec0bcb52e8e4f7062bbb714f10bf2cbc7ca41e09/crates/aurora-viewer/src/workspace/images/tests.rs#L8-L96)

The six new hover/control tests and both save-kind/item/object correlation tests passed. The hover preferences row ran with the default environment and produced no visual evidence. The retained descriptor regressions, exact Windows long-path fixture (all four store kinds), dedicated managed-image corruption/immutability/no-create cases, and shared hover/background texture-interest case passed. The two Unix IPC tests, Linux SDK stdio cases and three Unix upload-handle cases are platform-excluded. The alternate MCP workspace backend's two Context and one HUD-queue cases remain unintegrated. [Long-path fixture](https://github.com/DrCum/Foldurora/blob/ec0bcb52e8e4f7062bbb714f10bf2cbc7ca41e09/crates/aurora-viewer/src/session_host/profile.rs#L934-L982) · [Unix upload guard](https://github.com/DrCum/Foldurora/blob/ec0bcb52e8e4f7062bbb714f10bf2cbc7ca41e09/crates/aurora-viewer/src/app/mcp/inventory/uploads/files.rs#L303-L304)

## Downstream status and preservation

The Monitor prototype gates, ordinary viewer and sidecar build, version probes, package, MCP/hover CPU captures, default Vulkan startup and all targeted native offline smoke were **NOT RUN**. No executable version, ZIP or SHA-256 was produced for this source. The known a903 package and 1ba negative evidence remain distinct. Host, native workspace, MCP authorization, live inactive Monitor, GPU reset, XR and the Japanese glyph limitation received no new native evidence.

Prior artifact metadata matched for 2,542 files and all 172 retained cached outputs matched their hashes. Worktrees and local branch heads matched their expected inventories. The 1ba checkout is retained by ref. Raw logs, detailed audits and launch context stay local under `Builds/Foldurora-ec0bcb52e8e4/logs`.
