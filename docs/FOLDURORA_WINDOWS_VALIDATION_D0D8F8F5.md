# Foldurora Windows validation for d0d8f8f5

**Failed at strict Clippy on November 9, 2026.** The Windows handoff stops at this gate.

- Repository `DrCum/Foldurora`
- Branch `feat/host-mcp-integration-20261109`
- Frozen source `d0d8f8f5ad73f177160882a4543252e6941d661d`
- Tree `af882184e494d378b9523c9f90c3686dd003a2dc`
- Base `f08c22e2b0875b7d9b22a1dfe8742c1884321ff5`
- Toolchain `x86_64-pc-windows-msvc`, Rust and Cargo `1.99.1`, default dev profile

[Original pinned Windows request](https://github.com/DrCum/Foldurora/blob/98a33e1746089ca264abe43cd22b24edc564949b/docs/FOLDURORA_MCP_WINDOWS_REQUEST_20261109.md)

| Gate | Exit | Result |
| --- | ---: | --- |
| `powershell.exe -NoProfile -ExecutionPolicy Bypass -File ./scripts/check-scripts.ps1` | 0 | Passed |
| `cargo fmt --all -- --check` | 0 | Passed |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | 101 | Failed with two diagnostics in `aurora-mcp` |
| Unfiltered workspace tests and required-name audit | — | Not run |
| Viewer and MCP sidecar build, version/help, packaging and smoke | — | Not run |

The same two diagnostics stopped the library and library-test targets.

| Diagnostic | Expression | Frozen source |
| --- | --- | --- |
| `clippy::field_reassign_with_default` | `overlap.hEvent = event.0` after `OVERLAPPED::default()` | [windows.rs:346–347](https://github.com/DrCum/Foldurora/blob/d0d8f8f5ad73f177160882a4543252e6941d661d/crates/aurora-mcp/src/ipc/windows.rs#L346-L347) |
| `clippy::replace_box` | `self.overlap = Box::default()` | [windows.rs:472](https://github.com/DrCum/Foldurora/blob/d0d8f8f5ad73f177160882a4543252e6941d661d/crates/aurora-mcp/src/ipc/windows.rs#L472) |

The prepared checklist applies the documented deltas to the verified f08 manifest and contains **1,128** requirements. It includes `ipc::tests::only_four_connections_are_admitted_until_a_slot_closes`, whose test definition is guarded by `#[cfg(unix)]` in this commit. The list still contains the entry. This is a source/checklist conflict observed before runtime validation. [Test definition](https://github.com/DrCum/Foldurora/blob/d0d8f8f5ad73f177160882a4543252e6941d661d/crates/aurora-mcp/src/ipc/tests.rs#L176-L178)

The builder retained the clean frozen checkout, the f08 source ref and prior packages. Preservation checks matched the metadata for 1,899 prior artifact files and the hashes of 17 saved cache outputs. No executable or package identity was measured for this candidate. Native host/Monitor, driver reset, XR, Linux terrain and Japanese glyph acceptance remain open.

### Local evidence identity

| Local file under the build's `logs` directory | SHA-256 |
| --- | --- |
| `clippy.stderr.log` (203,252 bytes) | `de00bb48abafdb189a2a4781247b7c57b5309c421b40a58b420a8306546d4549` |
| `clippy.stdout.log` (0 bytes) | `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` |
| `final-report.json` | `d926a16f6da6b50514b293abbed9769e187094cf0def59f83366db5069431b9a` |

This handoff contains the shareable diagnostics. The raw command logs, test checklist and machine metadata are retained locally.
