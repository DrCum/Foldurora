# Foldurora Windows validation for 520782f29981

The Monitor-enabled Windows candidate passed the full workspace suite, the separate GPU export gate and the ordinary viewer/sidecar package checks. Standalone Vulkan startup passed. Hosted live Monitor acceptance remains open.

## Source and commands

- Branch `feat/live-inactive-monitors-20261109`, commit `520782f2998150a26c74c51e18eccc7c4994c737`, tree `bcded0890dbdb359d44db28c399956fa2fb1ccd5`.
- Canonical Git-LF Cargo.lock SHA-256 `e04f6814529802ba6469b9455294aabfcfbec989d0e271698b3723a04d682e32`. The CRLF checkout normalized to the same digest. The lockfile blob matches the radio checkpoint and the owned worktree stayed clean.
- Existing x64 MSVC dev target with rustc and Cargo 1.99.1. The launcher observed a medium-integrity non-admin parent and consistent temporary directory settings. Child token and effective temporary directory were unobserved. [Pinned source](https://github.com/DrCum/Foldurora/commit/520782f2998150a26c74c51e18eccc7c4994c737) · [Pinned Windows request](https://github.com/DrCum/Foldurora/blob/9d880db77c9986bf412eacd1d39216b6a0629318/docs/FOLDURORA_LIVE_MONITORS_WINDOWS_REQUEST_520782F2.md)

| Command | Exit | Actual result |
| --- | ---: | --- |
| `powershell.exe -NoProfile -ExecutionPolicy Bypass -File ./scripts/check-scripts.ps1` | 0 | Passed |
| `cargo fmt --all -- --check` | 0 | Passed |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | 0 | Passed |
| `cargo test --workspace --locked --no-fail-fast` | 0 | 9,198 passed, 0 failed, 63 ignored, 0 filtered, 37 summaries |
| `cargo test --workspace --locked viewport_gpu_live_offscreen_export_keeps_native_crop_identity_full_texture_uv_and_opaque_color -- --ignored --nocapture` | 0 | 1 passed, 0 failed, 0 ignored, 9,260 filtered, 23 summaries |

Both test invocations used child-only `AURORA_RENDER_THREAD=1` and `AURORA_GPU_VALIDATION=1`. Inherited parent values were preserved. The separate earlier MCP, catalogue/image and nondefault prototype matrices were outside this run.

## Full-run audit and GPU result

All 1,413 requirements passed exactly once from the 9,261 named rows in the single unfiltered suite. No requirement was missing, duplicate or nonpassing. The 23 additions include ten preserved formerly nondefault tests and thirteen new cases. The real small-world and retirement test `monitor_world_packets_keep_surface_size_and_never_prove_presentation` ran and passed. Libtest reported it still running at 60 seconds before the pass. This is fixture timing evidence. [Small-world and retirement fixture](https://github.com/DrCum/Foldurora/blob/520782f2998150a26c74c51e18eccc7c4994c737/crates/aurora-render/src/monitor/tests.rs#L239-L350)

The explicit export case was ignored in the full run and passed once when invoked separately. Its separate row contributed zero rows to the 1,413-case audit. Its source assertions exercised RGBA8 and BGRA8 in linear and sRGB formats, the original offset crop and boundary metadata, all four corners of the small texture, opaque alpha, refusal of an inactive claim by ordinary export, and invalidation of the publication guard. Adapter, backend and driver were unobserved in both test invocations. The standalone launch below has its own separately measured identity. [GPU export fixture](https://github.com/DrCum/Foldurora/blob/520782f2998150a26c74c51e18eccc7c4994c737/crates/aurora-render/src/viewport_capture_gpu.rs#L1689-L1782)

The two public station probes and three audio device/microphone probes remained ignored. Unix IPC/upload and Linux SDK stdio cases are platform-excluded. The alternate MCP workspace backend's three exclusive cases remain unintegrated, and workspace Apply/Previous remain unsupported.

## Monitor-enabled dev package

`cargo build --locked -p aurora-viewer -p aurora-mcp --features aurora-viewer/xr-static-loader` exited 0. The viewer manifest enables `aurora-render/monitor-prototype`; the actual renderer compiler invocation included both `monitor-prototype` and `xr-static-loader`. The viewer matched its pinned stamp on the first build. Both `--version` probes, sidecar `--help` and `scripts/package.ps1` exited 0. The packager received both executables via `-Exe` and `-McpExe`. The staged and ZIP contents include `aurora-install.json`, `foldurora-build-info.json`, `FOLDURORA_MCP.md`, `NOTICE-MCP.txt`, `LICENSE` and `NOTICE.md`. Both executables' built, staged and ZIP-entry hashes matched. [Viewer dependency](https://github.com/DrCum/Foldurora/blob/520782f2998150a26c74c51e18eccc7c4994c737/crates/aurora-viewer/Cargo.toml#L33)

| Artifact | Version | Bytes | SHA-256 |
| --- | --- | ---: | --- |
| `Foldurora-520782f29981-windows-x64.zip` | | 74819659 | `6868063c75d7d0928c0468f5f89697e7b8965f01f4d28375f4611be010b919ce` |
| `aurora-viewer.exe` | `Aurora Viewer 0.9.3-beta.20261106.1+525.g520782f29` | 177644032 | `b7f87a5c01785a0065ef898d6f7d8b7b4f4c672f4fa35b514f933ee1f27182bd` |
| `foldurora-mcp.exe` | `foldurora-mcp 0.1.0` | 3467776 | `a008768de6ef8a5753a4a8e8d83e2ce9c7fc015b02e7a7051b0cbe503d2393d0` |

The verified package, manifests and receipts are local under `Builds/Foldurora-520782f29981`.

## Offline observations and limits

- **PASS, standalone startup.** The packaged viewer copy ran the default-theme offline demo on Windows build 26200 with Vulkan on an RTX 3060, driver NVIDIA 610.88. It captured frame 650 at 1280 × 800 and 96 DPI and exited 0. Smoke elapsed time was 32.97 seconds. The inspected screenshot showed the synthetic world and offline indicator. There were zero new matching MCP pipes in 24 passive samples and zero controller descriptors in the owned isolated tree.
- **NOT RUN, hosted live sampling.** The isolated startup did not establish a managed Ready/Standby source. Host-worker profile selection excludes updater-test installations. No native authority was supplied for never-active samples, timing, open/reopen or retirement on privacy/teleport and promotion. Hosted movement, resize, viewport/picking/mouselook, login-bound workspaces and functional MCP interactions also remain open. No CPU capture matrix ran. Real-grid/RLVa, real radio, GPU reset, XR and Japanese glyph coverage received no new acceptance evidence. [Host profile guard](https://github.com/DrCum/Foldurora/blob/520782f2998150a26c74c51e18eccc7c4994c737/crates/aurora-viewer/src/session_host/profile.rs#L40-L44)
- **Source bounds.** Host credit is 512 MiB, with 128 MiB per configured source and at most four previews. Existing image pools are separate. Pending or uncertain reservations remain charged. Known driver headroom is required, with 256 MiB cold and 64 MiB subsequent thresholds. Unknown headroom refuses sampling. Privacy or teleport closure can leave an uncertain reservation that requires worker exit before recovery. These limits were retained and do not establish hosted recovery. [Live Monitor implementation and bounds](https://github.com/DrCum/Foldurora/blob/520782f2998150a26c74c51e18eccc7c4994c737/docs/FOLDURORA_LIVE_MONITORS_20261109.md)

Preservation checks matched metadata for 3,460 prior artifacts and hashes for 197 retained cached outputs. Worktree and branch inventories matched and the radio checkout is retained by ref. The radio, 98b and a903 ZIP hashes independently matched after the run. Raw logs, private coverage inputs, launch context and screenshots remain local.
