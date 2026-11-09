# Foldurora Windows validation for 47955237fd40

The frozen radio checkpoint passed strict workspace checks, the full Windows test suite and the 1,390-case coverage audit. The ordinary viewer and sidecar portable ZIP was built and verified. Both changed CPU captures and isolated Vulkan startup passed within their fixture scopes.

## Source and Windows gates

- Branch `feat/upstream-radio-integration-20261109`, commit `47955237fd40dd61e4fab8ca9a8a19078ef74e7d`, tree `0a16570f3944a4f1e1e9e9391cb0ae009321c1fa`.
- Canonical Git-LF Cargo.lock SHA-256 `e04f6814529802ba6469b9455294aabfcfbec989d0e271698b3723a04d682e32`. The CRLF checkout normalized to that digest. The owned worktree was clean.
- Existing x64 MSVC dev target, rustc and Cargo 1.99.1. The launcher observed a medium-integrity non-admin parent and consistent temporary directory settings. Child token and effective temporary directory were unobserved. [Pinned source](https://github.com/DrCum/Foldurora/commit/47955237fd40dd61e4fab8ca9a8a19078ef74e7d) · [Pinned request](https://github.com/DrCum/Foldurora/blob/50dc448fa139fd4c01a1c2058a1abfc72f285728/docs/FOLDURORA_RADIO_WINDOWS_REQUEST_47955237.md)

| Command | Exit | Actual result |
| --- | ---: | --- |
| `cargo fmt --all -- --check` | 0 | Passed |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | 0 | Passed |
| `cargo test --workspace --locked --no-fail-fast` | 0 | 9,175 passed, 0 failed, 62 ignored, 0 filtered, 37 summaries |

All 1,390 requirements passed exactly once, with zero missing, duplicate or nonpassing requirements. The audit uses only the 9,237 named rows from the single unfiltered suite. The 24 additions passed, comprising 11 new offline playlist tests and 13 existing media-consent and staff cases. The playlist fixtures use owned loopback and include real MP3 decode, origin/redirect restrictions, cancellation and document budgets. The unfiltered run recorded `playlist::tests::lightman_live_probe`, `stream::tests::issue_1628_live_probe`, `device_tests::opens_default_device`, `device_tests::switches_output_device` and `device_tests::captures_default_microphone` as ignored. Unix IPC/upload and Linux stdio cases are platform-excluded. The alternate MCP workspace backend's three exclusive cases remain unintegrated. [Playlist fixtures](https://github.com/DrCum/Foldurora/blob/47955237fd40dd61e4fab8ca9a8a19078ef74e7d/crates/aurora-audio/src/playlist.rs)

Renderer, session and MCP Git trees match 98b. The lockfile delta is only an `aurora-audio` edge to the existing `serde_json` package. All versions and renderer dependency entries are unchanged. The nondefault Monitor prototype gates were **NOT RERUN**. Their 62 distinct passes belong to the prior checkpoint as qualified evidence. Default workspace tests ran in the new suite. [98b validation](https://github.com/DrCum/Foldurora/blob/039313a7d421147dab46113791162e103ab89a35/docs/FOLDURORA_WINDOWS_VALIDATION_98B30BE2.md)

## Ordinary dev package

`cargo build --locked -p aurora-viewer -p aurora-mcp --features aurora-viewer/xr-static-loader` exited 0. The viewer matched its source stamp on the first build. Both `--version` probes, sidecar `--help` and `scripts/package.ps1` exited 0. The staged package and ZIP contain `aurora-install.json`, `foldurora-build-info.json`, `FOLDURORA_MCP.md`, `NOTICE-MCP.txt`, `LICENSE` and `NOTICE.md`. Built, staged and ZIP-entry hashes match for both executables. The package excludes `monitor-prototype`.

| Artifact | Version | Bytes | SHA-256 |
| --- | --- | ---: | --- |
| `Foldurora-47955237fd40-windows-x64.zip` | | 74801349 | `6801a34d03b9d4b4f54d5d76eaa1e05e402acac5debb01b132d23813c7323aaa` |
| `aurora-viewer.exe` | `Aurora Viewer 0.9.3-beta.20261106.1+524.g47955237f` | 177553408 | `693c6174ca9924e60cc73d98280dbb120d34d8d6a84d3c31f0d3f9562a8b4d95` |
| `foldurora-mcp.exe` | `foldurora-mcp 0.1.0` | 3467776 | `616d3935bebf601b6ff3be5a7c54b785dd9812d0398ffdf82eda0272016e2a3c` |

Package, manifests and receipts remain local under `Builds/Foldurora-47955237fd40`.

## Changed CPU captures

| Command | Exit | Result |
| --- | ---: | --- |
| `cargo test --workspace --locked character_mcp_cpu_visual_fixture -- --ignored --test-threads=1 --nocapture` | 0 | 1 passed, 9,236 filtered, 23 summaries, 7 inspected PNGs |
| `cargo test --workspace --locked inventory_hover_picture_only_capture_both_presentations -- --test-threads=1 --nocapture` | 0 | 1 passed, 9,236 filtered, 23 summaries, 2 inspected PNGs |

The scoped English capture overrides preserved the inherited environment. The fixtures used synthetic data and inert dispatch. Character default showed the disabled control. Both review flows showed the frozen argument tail after scrolling. Enter left approval pending. Explicit pointer Deny returned `permission_denied`. Allow once queued one action with the original arguments. Both outcomes recorded no native execution. [Character capture fixture](https://github.com/DrCum/Foldurora/blob/47955237fd40dd61e4fab8ca9a8a19078ef74e7d/crates/aurora-viewer/src/app/mcp/ui/visual.rs)

The two flows had identical action metadata. The clip ran from `(65,331)` to `(613,651)`. Rectangles below are top-left through bottom-right in CPU fixture coordinates.

| Stage | Deny bounds → intersection | Allow once bounds → intersection | Full visibility, both | Center visibility, both |
| --- | --- | --- | --- | --- |
| Before scroll | `(75,627)–(195,655)` → `(75,627)–(195,651)` | `(203,627)–(353,655)` → `(203,627)–(353,651)` | No | Yes |
| After scroll | `(75,623)–(195,651)` unchanged | `(203,623)–(353,651)` unchanged | Yes | Yes |
| Frozen tail | `(75,623)–(195,651)` unchanged | `(203,623)–(353,651)` unchanged | Yes | Yes |

Aurora and Classic Inventory showed the cached synthetic picture with text tips disabled. The Classic footer showed real icons and legible labels without the previous fallback-label overlap in the CPU fixture. Native footer behavior was not exercised. No additional CPU matrix ran. [Hover capture fixture](https://github.com/DrCum/Foldurora/blob/47955237fd40dd61e4fab8ca9a8a19078ef74e7d/crates/aurora-viewer/src/ui/inventory/hover.rs)

## Standalone native startup and scope

- **PASS, standalone offline startup.** The verified package copy ran the isolated default-theme Vulkan demo on Windows build 26200 with an RTX 3060 and NVIDIA driver 610.88. It captured frame 650 at 1280 × 800 and 96 DPI and exited 0 after 24.13 seconds. The inspected image showed the synthetic world and offline status. No new matching host/character MCP pipe appeared in 24 passive samples, and the isolated tree contained no controller descriptor.
- **NOT RUN, real playback and live authority.** Public station playback, switching/cancelling a real resolution, human consent/denial and live Team decoration/privacy were outside the owned fixtures. Native hosted movement/switching, viewport and picking/mouselook, login-bound workspaces, native MCP Ask/source retirement and hosted LastActive Monitor require an authoritative source the standalone fixture did not provide. Live inactive Monitor, GPU reset, XR and Japanese glyph coverage received no new evidence.

Preservation matched metadata for 3,140 prior artifact files and hashes for 186 retained cached outputs. Worktrees and local branch heads matched their expected inventories. The previous 98b checkout is retained by ref. The 98b and a903 ZIPs independently matched their recorded SHA-256 after the run. Raw logs, coverage inputs, images, action metadata and launch context stay local.
