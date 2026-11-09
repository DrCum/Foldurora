# Foldurora Windows validation for a6119664

**Failed at the unfiltered Windows workspace suite on November 9, 2026.** The corrected strict Clippy gate passed. The suite exited 101 on one theme guard failure.

- Repository `DrCum/Foldurora`, branch `feat/advanced-workspaces-20261109`, draft PR #20
- Frozen source `a6119664d9f576925c66e4bac20bbf049d0fdca9`
- Tree `70c1a3d01fc569d6c203f17e04d618a59232d772`
- Base `f08c22e2b0875b7d9b22a1dfe8742c1884321ff5`
- Rust and Cargo 1.99.1 on `x86_64-pc-windows-msvc`, reused default dev target
- Cargo.lock SHA-256 `4e38421f8cbdeb23b23e0a2b99ca2badec13c7290dabfea0b4da069a88d5c839`

[Pinned Windows request](https://github.com/DrCum/Foldurora/blob/6b390b6dabc4e2daf979af382f6a20d4da2de661/docs/FOLDURORA_WORKSPACE_WINDOWS_REQUEST_20261109.md)

| Command | Exit | Measured result |
| --- | ---: | --- |
| `powershell.exe -NoProfile -ExecutionPolicy Bypass -File ./scripts/check-scripts.ps1` | 0 | Passed |
| `cargo fmt --all -- --check` | 0 | Passed |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | 0 | Passed |
| `cargo test --workspace --locked --no-fail-fast` | 101 | **8,920 passed, 1 failed, 58 ignored**, zero filtered, 37 summaries, 8,979 named rows |

The only failing test was `ui::theme_guard::windows_take_their_appearance_from_the_theme`. The guard rejected direct `egui::Checkbox::` at `src/ui/workspaces.rs:112` and requires a shared themed control. The failing viewer target measured 7,441 passed, 1 failed and 20 ignored. [Offending control](https://github.com/DrCum/Foldurora/blob/a6119664d9f576925c66e4bac20bbf049d0fdca9/crates/aurora-viewer/src/ui/workspaces.rs#L112) · [Guard assertion](https://github.com/DrCum/Foldurora/blob/a6119664d9f576925c66e4bac20bbf049d0fdca9/crates/aurora-viewer/src/ui/theme_guard.rs#L402)

### Coverage audit

All **1,134 required names** matched exactly one passing row, with 55 full-name matches and 1,079 unique final-identifier matches. The audit includes the 28 portable MCP unit cases, five viewer MCP adapter cases and all seven new workspace cases, including the actual UI and IO cases. The passing audit does not waive the failing unfiltered suite.

The two Unix-only IPC cases `only_four_connections_are_admitted_until_a_slot_closes` and `links_foreign_modes_and_descriptor_overflow_fail_closed`, and the two Linux-only stdio cases `synthetic_mcp_stdio_host_authority_end_to_end` and `synthetic_stdio_negotiates_older_protocol_and_unknown_version_fallback`, produced no Windows rows under their existing source guards. The connection-capacity case remains a Linux requirement. [IPC guards](https://github.com/DrCum/Foldurora/blob/a6119664d9f576925c66e4bac20bbf049d0fdca9/crates/aurora-mcp/src/ipc/tests.rs#L176) · [Stdio platform guard](https://github.com/DrCum/Foldurora/blob/a6119664d9f576925c66e4bac20bbf049d0fdca9/crates/aurora-mcp/tests/synthetic_mcp_stdio.rs#L3)

### Downstream gates

**NOT RUN** for this candidate.

- Viewer and MCP sidecar build, viewer version, sidecar version/help, package and ZIP identity.
- Compact Host and Monitor CPU captures.
- Isolated Vulkan startup and MCP pipe/descriptor observation.
- Native workspace favorites, strip, startup source binding, preview and logout persistence.
- First-frame Monitor opening, reopen, minimize and private source authority.
- Host drag, resize, snap, minimize, management, DPI, control timing and world margin checks.
- Switching, held-key/PTT release, focus, single render ownership and detach.

Native and human host/workspace/Monitor acceptance, driver-reset recovery, XR, Linux terrain and the Japanese glyph limitation remain open.

### Local evidence identity

The candidate checkout was clean before and after. The prior d0d8 HEAD has a retained ref, and the branch/worktree checks matched their expected state. Preservation checks matched the metadata for 1,945 prior artifact files and the hashes of 19 saved cache outputs. This handoff contains measured summaries and hashes. The raw logs and private checklist are retained locally.

| Local file under `logs` | Bytes | SHA-256 |
| --- | ---: | --- |
| `all-test-names.json` | 2,400,225 | `34e1fb3f40c5b3eaf39418e9dcc64e0d42e9ec8292a880e8668488ad8ea205ff` |
| `combined-windows-requirements.json` | 103,687 | `911795508163054c0dbbd5e4f0308d98cdb3dd638ac0e20e76ed26ebd3dd8df7` |
| `full-tests.stderr.log` | 290,539 | `8bcdfb1ef46c574d5122cbd50cddd2a840bab7f3f9ff78830537476c802f07ef` |
| `full-tests.stdout.log` | 868,652 | `30de064a358d69473cc6cff15959da009be88a4b37eb013d009923fbc277052b` |
| `required-name-audit.json` | 678,491 | `7889fa689955315bf2d3100106472d586ecfa85c880c8acfd72efce2a9ed9234` |
| `final-report.json` | — | `43ff984b896154e29228f80567a23cb953a774c674ca948c86e1fd81e1a0b32b` |
