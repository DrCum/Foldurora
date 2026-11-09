# Windows request — combined radio/upstream and visual evidence checkpoint

Prepared for ONE relay through Slick to EXISTING Foldurora Windows Builder on
Cumputer, thread01a1af77-a465-71c3-be08-cd9d1b927f83. FIRST check current history:
last verified idle after completed successful98b. No duplicate/replacement Builder
or cancellation. The standalone d836 fixture request was prepared only, never
published/delivered; supersede it with this combined group. No old98b rerun.
Publication/preparation does not establish acknowledgment, execution or acceptance.

Repository: https://github.com/DrCum/Foldurora.git
Branch: `feat/upstream-radio-integration-20261109`
Commit: `47955237fd40dd61e4fab8ca9a8a19078ef74e7d`
Tree: `0a16570f3944a4f1e1e9e9391cb0ae009321c1fa`
Cargo.lock canonical Git-LF SHA256:
`e04f6814529802ba6469b9455294aabfcfbec989d0e271698b3723a04d682e32`
Use this exact source, not the documentation commit. Reuse owned
camera-6a3009f-windows and compatible DEV target/tooling/caches; no clean/new cache,
dependency/tool upgrade, ACL/security/long-path-policy change, source repair,
credentials, grid login, install, merge or release. Preserve source/heads/worktrees,
profiles, logs/screenshots,98b/a903 packages and earlier failures/open reproductions.
Builder source fixes must return to lead for integration/revalidation.

## Source group and prior evidence

Stacks frozen fixture PR43/d83654141777d4323182340aa862ef325f7a3c69 on verified
product98b30be2ff08da897b625a7014c00fa5d5bbbb03, then integrates reviewed PR42
head513795069b58ffaa852ef1ceb047b5a371591ccc (base388b299c).
- Resolve bounded same-origin player/PLS/M3U documents before audio decoding.
  Existing native media consent, buffering/starvation, cancellation, TLS and ICY
  handling remain. Discovered URLs/redirects cannot change origin/use credentials.
  Four128KiB document steps, loops/HLS/oversized/malformed input fail closed.
  One existing locked serde_json dependency edge added, no package version change.
- One Aurora Team decoration key; existing keyed Team/Second-Life-only Linden
  name rules and no privilege grant. No new control, UI string or source owner.
- Real icons in the hover CPU capture and test-only actual review action clip
  metadata. No product layout/permission change; preserve partial visibility.

HTTP/playlist/audio manifest/staff match author input; the streamer keeps the
lead's previous PR13 buffering/starvation alongside exactly the new patch lines.
Format and qualified workspace all-target ci Clippy pass (existing Linux platform
dead-code allowance). Scoped Linux audio test attempt exited101 ENOSPC during a
different dependency feature graph, before tests. Negative log preserved; only
idle cache outputs reclaimed to restore normal execution; no retry or local audio
pass. Author CI129 distinct passes/5 ignores are qualified input evidence, not
combined results. Full Linux viewer linking/terrain limitations remain unresolved.
Source doc FOLDURORA_UPSTREAM_RADIO_20261109.md records mappings through Aurora
19d9948482d35c01e8525b930d587fa63655e9d6 and still-deferred overlaps.

Verified98b Windows9164/0/61, all1366 requirements,62-distinct prototype checks,
ordinary viewer/sidecar package and standalone Vulkan startup remain separate:
https://github.com/DrCum/Foldurora/blob/039313a7d421147dab46113791162e103ab89a35/docs/FOLDURORA_WINDOWS_VALIDATION_98B30BE2.md
Its exact ZIP SHA256348fad299ccce543869f66130c7faedf3d1c24ac6729cbf9c08d87c8c67b5e68
and package/binaries/evidence must be preserved. This changed functional group
needs a new full run/package; do not relabel98b's results as this source.

## Checks and full-source coverage

Run sequentially with exact commands/exits/stdout/stderr. Verify clean pinned
source and canonical lock (account for existing CRLF without changing settings).
A failure stops downstream stages; preserve it without source fixes/waivers or
unchanged reruns. One unfiltered workspace suite is required for this group:

```powershell
if ((git rev-parse HEAD).Trim() -ne '47955237fd40dd61e4fab8ca9a8a19078ef74e7d') { throw 'Wrong source commit' }
if ((git rev-parse 'HEAD^{tree}').Trim() -ne '0a16570f3944a4f1e1e9e9391cb0ae009321c1fa') { throw 'Wrong source tree' }
git status --short
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked --no-fail-fast
```

No separate early MCP/focused audio suite or repeated fixed catalogue/image gate
is requested. All full cases must pass, not only the named audit. Keep actual
summaries/ignored/failing names/zero-filtered evidence. Use only this full run's
rows for1390 requirements: retain98b's1366 exact audited baseline and add the24
public source identifiers below. These comprise11 new default offline playlist
cases plus13 existing consent/staff guards newly included in the audit (not new
test definitions). Exact full-name or unique leaf matches, exactly one PASS each,
zero missing/duplicate/nonpassing. Never publish private checklist metadata/raw
rows/logs. Existing SDK Unix/Linux and native upload guards stay platform-excluded;
unsupported alternate MCP workspace backend's3 tests remain unintegrated/unpassed.

- `playlist::tests::player_playlist_decodes_json_escapes_without_executing_scripts`
- `playlist::tests::pls_uses_the_first_numbered_stream_and_preserves_query_strings`
- `playlist::tests::m3u_supports_relative_streams_but_rejects_hls_segments`
- `playlist::tests::playlist_discovery_never_expands_the_allowed_origin_or_scheme`
- `playlist::tests::azuracast_player_and_pls_resolve_to_decodable_audio`
- `playlist::tests::redirected_playlists_resolve_relative_to_the_final_response_url`
- `playlist::tests::discovered_redirects_cannot_contact_a_second_origin`
- `playlist::tests::playlist_loops_oversized_documents_and_cancellation_are_bounded`
- `playlist::tests::playlist_nesting_stops_after_the_document_budget`
- `playlist::tests::four_playlist_documents_can_still_end_in_decodable_audio`
- `playlist::tests::direct_audio_keeps_its_bytes_even_at_a_playlist_path_and_above_the_document_limit`
- `media::filter::tests::domain_is_the_host_alone`
- `media::filter::tests::a_domain_rule_covers_its_subdomains_at_a_dot`
- `media::filter::tests::the_most_precise_rule_wins`
- `media::filter::tests::an_unknown_site_is_asked_about_once`
- `media::filter::tests::a_data_page_is_asked_about_as_the_domain_data`
- `media::filter::tests::scripts_files_and_browser_pages_never_load`
- `media::filter::tests::answers_are_kept_as_asked`
- `media::filter::tests::trusted_media_is_never_asked_about_but_refusals_hold`
- `media::filter::tests::questions_nothing_asks_for_go_away`
- `media::filter::tests::typed_entries_become_domains_unless_they_have_a_path`
- `media::filter::tests::the_list_reads_and_writes_the_legacy_file`
- `world::staff::tests::nobody_is_staff_by_default`
- `world::staff::tests::the_team_is_marked_by_key`

The added lightman_live_probe and existing real-station/hardware probes remain
explicitly ignored, not passing. Do not run public/live network probes. Playlist
unit tests use only owned loopback servers, including forbidden-origin contact
checks, cancellation, document caps/loops/HLS and real MP3 decode. No security
assertion changes. Native media consent/denial remains authoritative.

Renderer, session and MCP core source/versions are unchanged from98b; independently
verify those trees and unchanged renderer dependency versions. Do not repeat its
nondefault Monitor/prototype gates: retain its62 distinct passes as qualified
prior-source evidence, not new execution. Default renderer/session cases still
execute in this new full suite. Production live inactive rendering stays disabled.
MCP workspace Apply/Previous remain unsupported; no capability/consent bypass.

## Build and package after the full gate

```powershell
cargo build --locked -p aurora-viewer -p aurora-mcp --features aurora-viewer/xr-static-loader
.\target\debug\aurora-viewer.exe --version
.\target\debug\foldurora-mcp.exe --version
.\target\debug\foldurora-mcp.exe --help
.\scripts\package.ps1 -Exe .\target\debug\aurora-viewer.exe -McpExe .\target\debug\foldurora-mcp.exe -Out 'C:\Users\Marty\..Projects\Aurora Viewer\Builds\Foldurora-47955237fd40\Foldurora-47955237fd40-windows-x64'
```

Ordinary package excludes monitor-prototype. Viewer must identify g47955237f;
return actual full version, sidecar0.1.0, package/ZIP paths, bytes/SHA256 and built/
staged/ZIP-entry hashes for both executables. Verify manifests/install info/MCP
and licensing documents included. Preserve any version mismatch; only normal
Cargo regeneration, no source/version override/clean. Keep prior packages intact.

## Changed CPU captures, standalone smoke and scope

After full/build/package pass, run only the two changed CPU fixtures with their
existing inert dispatch and English/single-thread setup. The following scoped
environment is restored; preserve any existing output rather than overwrite it.
```powershell
$folduroraVisualRoot = 'C:\Users\Marty\..Projects\Aurora Viewer\Builds\Foldurora-47955237fd40\cpu-captures'
if (Test-Path -LiteralPath $folduroraVisualRoot) { throw 'Preserve existing capture root; report duplicate/uncertain request' }
New-Item -ItemType Directory -Path $folduroraVisualRoot | Out-Null
$folduroraOldVisualDir = [Environment]::GetEnvironmentVariable('AURORA_MCP_VISUAL_DIR', 'Process')
$folduroraOldVisualLanguage = [Environment]::GetEnvironmentVariable('AURORA_MCP_VISUAL_LANGUAGE', 'Process')
$folduroraOldCapture = [Environment]::GetEnvironmentVariable('AURORA_CAPTURE', 'Process')
try {
    $env:AURORA_MCP_VISUAL_DIR = Join-Path $folduroraVisualRoot 'mcp'
    $env:AURORA_MCP_VISUAL_LANGUAGE = 'en'
    cargo test --workspace --locked character_mcp_cpu_visual_fixture -- --ignored --test-threads=1 --nocapture
    if ($LASTEXITCODE -ne 0) { throw 'Character fixture failed; preserve evidence and stop' }
    $folduroraHoverRoot = Join-Path $folduroraVisualRoot 'hover'
    New-Item -ItemType Directory -Path $folduroraHoverRoot | Out-Null
    $env:AURORA_CAPTURE = Join-Path $folduroraHoverRoot 'capture.png'
    cargo test --workspace --locked inventory_hover_picture_only_capture_both_presentations -- --test-threads=1 --nocapture
    if ($LASTEXITCODE -ne 0) { throw 'Hover fixture failed; preserve evidence and stop' }
}
finally {
    [Environment]::SetEnvironmentVariable('AURORA_MCP_VISUAL_DIR', $folduroraOldVisualDir, 'Process')
    [Environment]::SetEnvironmentVariable('AURORA_MCP_VISUAL_LANGUAGE', $folduroraOldVisualLanguage, 'Process')
    [Environment]::SetEnvironmentVariable('AURORA_CAPTURE', $folduroraOldCapture, 'Process')
}
```

Inspect actual resulting images: character default plus both before/after/full-tail
review flows (normally7 PNGs), both hover layouts (2 PNGs); report actual counts.
Real embedded/global icons should replace Classic fixed-box fallback labels;
report remaining overlap, never infer native footer behavior from CPU paint.
Review actions metadata must report actual clip, bounds/intersection, full/center
visibility before/after scrolling and at the tail. Partial cards may remain before
scrolling; preserve observation. Original frozen-tail/no-Enter/pointer Deny/Allow
once and no-native-IO assertions remain. Do not repeat host/notification/language/
DPI matrices or add these filtered rows to the full1390 audit.

Use the existing isolated packaged offline demo for default-theme Vulkan startup
and passive MCP pipe/descriptor checks. Synthetic owned profile/PIDs/title/border
only; preserve overrides and earlier fixtures. No remote audio station or grid
login. Record actual hardware/driver/style/DPI, startup/exit and discovery facts;
this does not prove hosted, login-bound native authority, real playback or GPU
recovery. Existing auth/native-acceptance gaps remain NOT RUN when no real source
is available; never manufacture one. No deliberate GPU loss.

Human checklist: with ordinary consent, compare a player URL to its direct stream,
stop/restart/switch while resolving and confirm denial stays stopped; verify Team
decoration/privacy. Native host movement/resize/switch, viewport/picking/mouselook,
workspace/HUD/extra Inventory/images/startup, MCP Ask/source retirement, LastActive
Monitor, GPU-reset/XR and Japanese glyphs remain open until actually tested.
Only Sodie uses grid/accounts. Shelved features remain shelved.

Return one sanitized GitHub-accessible report via Slick: exact source, all commands/
exits, actual full/focused totals/ignored/failure names,1390 source-bound audit,
version/package/bytes/hashes/ZIP verification, images/metadata observations,
startup/native/unrun cases and source/artifact/cache preservation. Raw logs/audits/
context/images remain private/local. A build/package pass is not human acceptance.
