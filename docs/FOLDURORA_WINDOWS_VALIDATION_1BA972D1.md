# Foldurora Windows validation for 1ba972d1419b

Strict workspace Clippy stopped the combined Windows run with eight `aurora-viewer` diagnostics. PowerShell parsing, formatting, strict MCP Clippy and the early Windows MCP suite passed. The full suite, prototype checks, build, package and smoke were **NOT RUN**.

## Source

- Source branch `feat/mcp-advanced-integration-20261109`, commit `1ba972d1419b22ee918515b12ecd5e6d42c4ca45`, tree `5cda6cd40d64c9355bf381fa4689532f79c52515`.
- Canonical Git-LF Cargo.lock SHA-256 `c869d88f1f837f302fde6ac52dfaa5e7ff0a325feac9ddf6f77da5954a7e852a`. The CRLF checkout normalized to the same digest. The worktree was clean before and after validation.
- Existing x64 Windows MSVC tooling and dev target. Parent launcher had a medium-integrity non-admin token and matching temporary directory observations. Child token and effective temporary directory were unobserved. [Pinned source](https://github.com/DrCum/Foldurora/commit/1ba972d1419b22ee918515b12ecd5e6d42c4ca45) · [Pinned Windows request](https://github.com/DrCum/Foldurora/blob/296a492af3717e4f710959145376060549ce9d7f/docs/FOLDURORA_COMBINED_WINDOWS_REQUEST_1BA972D1.md)

## Actual commands and exits

| Command | Exit | Result |
| --- | ---: | --- |
| `powershell.exe -NoProfile -ExecutionPolicy Bypass -File ./scripts/check-scripts.ps1` | 0 | Passed |
| `cargo fmt --all -- --check` | 0 | Passed |
| `cargo clippy --locked -p aurora-mcp --all-targets -- -D warnings` | 0 | Passed |
| `cargo test --locked -p aurora-mcp --no-fail-fast` | 0 | 68 passed, 0 failed, 0 ignored, 0 filtered, 4 summaries |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | 101 | Eight unused/dead-code diagnostics from the viewer binary |
| `cargo test --workspace --locked --no-fail-fast` | | **NOT RUN** |

The retained stale-incarnation regression, held-reader atomic replacement, and same-file-identity cleanup passed in the early MCP run. The held-reader case executed the Windows error 32 assertion. Two Unix IPC cases and four Linux SDK stdio cases were outside the early Windows run. Three Unix-only native upload-handle cases are separate Linux coverage. [Descriptor fixtures](https://github.com/DrCum/Foldurora/blob/1ba972d1419b22ee918515b12ecd5e6d42c4ca45/crates/aurora-mcp/src/ipc/tests.rs) · [Linux stdio guard](https://github.com/DrCum/Foldurora/blob/1ba972d1419b22ee918515b12ecd5e6d42c4ca45/crates/aurora-mcp/tests/synthetic_mcp_stdio.rs#L1-L3) · [Unix upload guard](https://github.com/DrCum/Foldurora/blob/1ba972d1419b22ee918515b12ecd5e6d42c4ca45/crates/aurora-viewer/src/app/mcp/inventory/uploads/files.rs#L303-L304)

## Viewer diagnostics for the lead

| Diagnostic | Source |
| --- | --- |
| Unused variable `opened`. Its identity comparison is under `cfg(unix)`. | [creative/pose.rs:279](https://github.com/DrCum/Foldurora/blob/1ba972d1419b22ee918515b12ecd5e6d42c4ca45/crates/aurora-viewer/src/app/mcp/creative/pose.rs#L278-L285) |
| Unused function `inventory_continuation` | [mcp.rs:493](https://github.com/DrCum/Foldurora/blob/1ba972d1419b22ee918515b12ecd5e6d42c4ca45/crates/aurora-viewer/src/app/mcp.rs#L493) |
| Unread `PendingKind::ContentSave` fields `asset` and `script` | [inventory.rs:83](https://github.com/DrCum/Foldurora/blob/1ba972d1419b22ee918515b12ecd5e6d42c4ca45/crates/aurora-viewer/src/app/mcp/inventory.rs#L81-L87) |
| Unread `PendingKind::Running` fields `object` and `item` | [inventory.rs:91](https://github.com/DrCum/Foldurora/blob/1ba972d1419b22ee918515b12ecd5e6d42c4ca45/crates/aurora-viewer/src/app/mcp/inventory.rs#L90-L96) |
| Unread `PendingKind::PutObserve.operation` | [inventory.rs:105](https://github.com/DrCum/Foldurora/blob/1ba972d1419b22ee918515b12ecd5e6d42c4ca45/crates/aurora-viewer/src/app/mcp/inventory.rs#L103-L107) |
| Unconstructed `PendingKind::TaskMutation` | [inventory.rs:122](https://github.com/DrCum/Foldurora/blob/1ba972d1419b22ee918515b12ecd5e6d42c4ca45/crates/aurora-viewer/src/app/mcp/inventory.rs#L122) |
| Unused function `continuation` | [inventory.rs:515](https://github.com/DrCum/Foldurora/blob/1ba972d1419b22ee918515b12ecd5e6d42c4ca45/crates/aurora-viewer/src/app/mcp/inventory.rs#L515) |
| Unread `OwnerEdit.typed` | [shared_pose.rs:62](https://github.com/DrCum/Foldurora/blob/1ba972d1419b22ee918515b12ecd5e6d42c4ca45/crates/aurora-viewer/src/world/shared_pose.rs#L62) |

## Downstream status

The 1,356-case checklist was assembled from the retained baseline and pinned additions. No full-run audit occurred. This includes the Windows long-ordinary-path and new managed workspace image cases, which have no run evidence for this candidate. The alternate MCP workspace backend's two Context and one HUD-queue tests are unintegrated. They received no passes or waivers.

The Monitor prototype filters, ordinary viewer/sidecar versions and package identity, MCP CPU visual fixtures, default Vulkan startup, native opt-in/Ask and discovery observations, Advanced Workspace and hosted Monitor/geometry smoke were **NOT RUN**. There is no new package or executable hash. The existing a903 package remains separate.

The initial preparation wrapper passed two installed Git paths as one executable path when starting lockfile verification. Preparation continued from the successful fetch with the first Git executable on PATH. No candidate check had started when the wrapper was corrected.

Preservation checks matched metadata for 2,349 prior artifacts and hashes for 153 retained cache outputs. Worktrees and local branch heads matched their expected inventories, and the prior a903 checkout is retained by ref. Raw logs, audit inputs, and launch context remain in the local builder directory.
