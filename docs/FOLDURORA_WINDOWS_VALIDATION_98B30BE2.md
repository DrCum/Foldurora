# Foldurora Windows validation for 98b30be2ff08

The focused diagnostics, unfiltered workspace suite and separate Monitor prototype gates passed. The ordinary dev viewer and sidecar portable ZIP was built and verified. The targeted CPU captures and standalone Vulkan startup passed within their fixture scopes.

## Pinned source and checks

- Branch `fix/workspace-windows-regressions-20261109`, commit `98b30be2ff08da897b625a7014c00fa5d5bbbb03`, tree `939c21690dbc923b66109153a1da758e9752d520`.
- Canonical Git-LF Cargo.lock SHA-256 `c869d88f1f837f302fde6ac52dfaa5e7ff0a325feac9ddf6f77da5954a7e852a`. The CRLF checkout normalized to the same digest. The owned build worktree stayed clean.
- Existing Windows x64 MSVC dev target, rustc and Cargo 1.99.1. The runner observed a medium-integrity non-admin parent and consistent temporary directory settings. Child token and effective temporary directory were unobserved. [Pinned source](https://github.com/DrCum/Foldurora/commit/98b30be2ff08da897b625a7014c00fa5d5bbbb03) · [Pinned request](https://github.com/DrCum/Foldurora/blob/04725fb8b2c5420ba1292cf6910d72845ee18ad9/docs/FOLDURORA_CORRECTED_WINDOWS_REQUEST_98B30BE2.md)

| Command | Exit | Actual result |
| --- | ---: | --- |
| `powershell.exe -NoProfile -ExecutionPolicy Bypass -File ./scripts/check-scripts.ps1` | 0 | Passed |
| `cargo fmt --all -- --check` | 0 | Passed |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | 0 | Passed |
| `cargo test --workspace --locked every_character_tool_has_one_native_adapter_or_closed_runtime_disposition -- --nocapture` | 0 | 1 passed, 9,224 filtered, 23 summaries |
| `cargo test --workspace --locked mcp_workspace_closed_disposition_returns_unsupported_without_preparation_or_native_approval -- --nocapture` | 0 | 1 passed, 9,224 filtered, 23 summaries |
| `cargo test --workspace --locked workspace_picture_ -- --nocapture` | 0 | 9 passed, 9,216 filtered, 23 summaries |
| `cargo test --workspace --locked --no-fail-fast` | 0 | 9,164 passed, 0 failed, 61 ignored, 0 filtered, 37 summaries |

The 1,366 required cases all passed exactly once, with no missing, duplicate or nonpassing requirement. The audit uses only 9,225 named rows from the single unfiltered run. The four-kind Windows long-path case, managed-image corruption/immutability/no-create cases, descriptor regressions, save correlation, hover/control and shared texture interest cases passed. Focused rows did not contribute to the audit. The separate prior 68-case MCP package gate was not repeated. MCP, session and renderer trees match the preceding ec0 source. Unix IPC/upload-handle and Linux stdio cases stay outside Windows coverage. The alternate MCP workspace backend's two Context cases and one HUD-queue case remain unintegrated.

## Focused diagnostics

The catalogue test covered 153 tools with one disposition each. Three workspace tools use the native adapter. Apply and Previous are closed. The Runtime case verified one read preparation, then `unsupported_capability` for both mutations with no additional preparation, ready action or approval. [Catalogue test](https://github.com/DrCum/Foldurora/blob/98b30be2ff08da897b625a7014c00fa5d5bbbb03/crates/aurora-viewer/src/app/mcp.rs#L579-L615) · [Runtime test](https://github.com/DrCum/Foldurora/blob/98b30be2ff08da897b625a7014c00fa5d5bbbb03/crates/aurora-viewer/src/app/mcp/workspaces.rs#L168-L215)

Both native World image-policy cases passed. The ordinary fixture reached Ready at every named transition. The malformed case reached Limited when archive folder 2 was published and returned to Ready after actual Library provenance repair. The archived item stayed refused while the unrelated ordinary item recovered. The policy, permission, RLVa and no-fetch assertions ran. These traces belong to this corrected invocation. The earlier ec0 panic did not identify its transition. [Native image tests](https://github.com/DrCum/Foldurora/blob/98b30be2ff08da897b625a7014c00fa5d5bbbb03/crates/aurora-viewer/src/workspace/images/tests.rs)

```text
workspace image transition=malformed Library with empty archive bypass status=Ready turns=1 generation=1
workspace image transition=initial ordinary metadata status=Ready turns=1 generation=1
workspace image transition=copy revoked status=Ready turns=1 generation=2
workspace image transition=malformed Library after archive folder2 published status=Limited turns=1 generation=1
workspace image transition=modify revoked status=Ready turns=1 generation=3
workspace image transition=Library provenance repaired, archive retained status=Ready turns=1 generation=2
workspace image transition=transfer revoked status=Ready turns=1 generation=4
workspace image transition=permissions restored status=Ready turns=1 generation=5
workspace image transition=archive folder2 published status=Ready turns=1 generation=5
workspace image transition=archive cleared status=Ready turns=1 generation=5
```

## Separate Monitor prototype gates

Each command ran with child-only `AURORA_RENDER_THREAD=1` and `AURORA_GPU_VALIDATION=1`. Inherited values were preserved.

| Command | Exit | Passed | Failed | Ignored | Filtered |
| --- | ---: | ---: | ---: | ---: | ---: |
| `cargo clippy --locked -p aurora-render --features monitor-prototype --all-targets -- -D warnings` | 0 | | | | |
| `cargo test --locked -p aurora-render --features monitor-prototype --lib monitor -- --nocapture` | 0 | 10 | 0 | 0 | 465 |
| `cargo test --locked -p aurora-render --features monitor-prototype --lib render_thread::tests -- --nocapture` | 0 | 19 | 0 | 0 | 456 |
| `cargo test --locked -p aurora-render --features monitor-prototype --lib session_fence::tests -- --nocapture` | 0 | 4 | 0 | 0 | 471 |
| `cargo test --locked -p aurora-session preview_ -- --nocapture` | 0 | 30 | 0 | 0 | 78 |

There were 62 distinct crate/test names. The shared test was `aurora-render::render_thread::tests::monitor_reservation_under_contention_does_not_prepare_or_consume_previous_result`. The native-window and GPU target fixtures ran. Adapter and driver were unobserved in the prototype logs.

## Ordinary dev package

`cargo build --locked -p aurora-viewer -p aurora-mcp --features aurora-viewer/xr-static-loader` exited 0. The package excludes `monitor-prototype`. The viewer stamp matched on the first build. Both `--version` probes, sidecar `--help` and `scripts/package.ps1` exited 0. The package and ZIP contain `aurora-install.json`, `foldurora-build-info.json`, `FOLDURORA_MCP.md`, `NOTICE-MCP.txt`, `LICENSE` and `NOTICE.md`. Built, staged and ZIP-entry hashes matched for both executables.

| Artifact | Version | Bytes | SHA-256 |
| --- | --- | ---: | --- |
| `Foldurora-98b30be2ff08-windows-x64.zip` | | 74781242 | `348fad299ccce543869f66130c7faedf3d1c24ac6729cbf9c08d87c8c67b5e68` |
| `aurora-viewer.exe` | `Aurora Viewer 0.9.3-beta.20261106.1+522.g98b30be2f` | 177525760 | `63e17af0322ad70d78b667df795362b7d7541a2b6e9959d7ea136915ff22c7a8` |
| `foldurora-mcp.exe` | `foldurora-mcp 0.1.0` | 3467776 | `616d3935bebf601b6ff3be5a7c54b785dd9812d0398ffdf82eda0272016e2a3c` |

Package, manifests, raw logs, screenshots and source-bound receipts are local under `Builds/Foldurora-98b30be2ff08`.

## Offline observations and scope

- **PASS, MCP CPU component fixtures.** The three ignored capture fixtures ran explicitly in English with inert dispatch and produced 14 inspected PNGs. Host and character controls were off in their respective defaults. Host pending and review controls were visible. Both character review scroll captures revealed the full argument tail and Deny/Allow once. The before-scroll images show partial button-bottom clipping. Both Aurora and Legacy notification captures showed the exact selected review and left the missing-review card area empty.
- **PASS, hover/control CPU component fixtures.** Each focused capture exited 0 and produced two inspected images. The Preferences images show separate Friends Only and keep-after-teleport controls plus the independently surfaced Inventory picture preference. Aurora and Classic Inventory both show the synthetic picture with text tips disabled. The Classic CPU image has fallback icon labels overlapping footer text. Native footer rendering was not inspected. Child capture environment overrides preserved inherited values. The unfiltered optional capture row alone produced no image evidence.
- **PASS, standalone Vulkan startup only.** The verified package copy ran the isolated default-theme offline demo on Windows build 26200, captured frame 650 at 1280 × 800 and 96 DPI, and exited 0 after 24.35 seconds. The startup log reported an RTX 3060 with Vulkan driver 610.88. The inspected frame shows the synthetic world and offline status. No new matching MCP pipe appeared in 24 passive samples and no controller descriptor was found in the isolated tree. Host and character transports share the observed pipe prefix.
- **NOT RUN, login-bound native controls and workspaces.** The standalone fixture did not provide native authority for prelogin control interaction, host/character MCP opt-in, Ask or source retirement, or both-style workspace manager layout, Save/Update, HUD/additional Inventory, display topology, picture import/cancel/persistence or archive/RLVa workflows. The native store tests cover their synthetic assertions independently.
- **NOT RUN, hosted and human acceptance.** Host movement/resize/switch, viewport margins and picking/mouselook, responsiveness, first/reopened LastActive Monitor, live inactive rendering, GPU reset, XR and grid acceptance remain open. Japanese glyph coverage received no new observation.

Preservation checks matched metadata for 2,751 prior artifact files and hashes for 186 retained cache outputs. Worktrees and local branch heads matched their expected inventories, the preceding ec0 checkout is retained by ref, and the a903 package and earlier failures were preserved. Raw evidence stays local.
