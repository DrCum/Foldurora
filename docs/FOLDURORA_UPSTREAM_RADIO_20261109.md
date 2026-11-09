# Independent Aurora radio and team metadata ports — 9 November 2026

Reviewed Aurora main 19d9948482d35c01e8525b930d587fa63655e9d6 after the
previous d174ae17e895bbb399e28165757c3b99e8168952 checkpoint: seven new
commits. Refreshed all published non-capture branches and open PRs and checked
provenance and source signatures before editing.

Current policy is on main f78d28147773ae9b2873bc64a0da212853f5c085.
The published current handoff was read at docs/workspace-context-build-20261109
251ae5ec50d89602d2f8883c2c08b19044f77dac, with the integration plan on
frozen lead 98b30be2ff08da897b625a7014c00fa5d5bbbb03, PR41.
That corrected Windows candidate is actively being tested. It and all earlier
sources/builds are preserved; no duplicate or superseding Builder request.
PR40 already integrates PR38; PR11/26/32/38 are not repeated.

Owned branch/worktree: fix/upstream-radio-playlists-20261109,
/workspace/work/upstream-radio-playlists. Independent safe base:
388b299c429d776b3fc7a9890162cd3d940b9dc3,
feat/foldurora-upstream-integration.

## Changes

| Aurora source | Port commit | Behavior |
| --- | --- | --- |
| 19d9948482d35c01e8525b930d587fa63655e9d6 (#3076) | 239130b6b10c789a9b6b411a0d810ffb794cad6c | Resolve AzuraCast player, PLS and M3U documents before audio decoding. |
| 39fd2e79dfb9678939c63a0cdbb300f855090459 (#3065) | 4f94cf3243a8b832fcd07dbe6fa8df478f0a05c4 | One new key in the existing explicitly labeled Aurora Team decoration list. |

Radio resolution stays on the existing audio thread after ordinary native
media-filter consent. It changes no parcel intent, account/source ownership,
App routing, host parking, background audio or saved station state. It is
independent of deferred cc604479's persistent parcel-radio lifecycle changes.
Documents are limited to128KiB each and four discovery steps. Discovered links
and their redirects must retain the same HTTP(S) origin and have no credentials.
Ordinary initial stream redirects retain their existing policy. Loops, malformed
documents, oversized input and HLS are rejected; JSON is data, never executed.
The resolved final path supplies the decoder hint. Existing stream buffering,
stop/reconnect and ICY handling remain on their existing paths.

Folderstorm reference was read at abbe3cb0ca66ee2484f277219f6a27706fab7fc6:
LLAudioStreamManagerFMODSTUDIO in indra/llaudio/llstreamingaudio_fmodstudio.cpp
delegates URLs to FMOD::createStream. Bounded player/playlist discovery is an
Aurora addition. Existing Rust TLS/certificate verification remains unchanged.
serde_json is already locked in the workspace; this adds it to aurora-audio,
without a version upgrade.

The badge remains Aurora Team, not Foldurora membership or a privilege grant.
Existing keyed lookup, nil/duplicate checks, grid/name rules and both display
styles remain shared. No new label, control, layout, asset-copy/wear operation,
selection or confirmation path is introduced. No new localization keys.

## Deferred dependencies and overlap

| Source | Reason |
| --- | --- |
| 450dc8bf0f8fcf85b19d91d96871d80ded1b21c4 | Raw inventory totals and shared Floater title-note interactions need archive-aware adaptation. Lead UI uses archive::Browse and the raw model item_count is test-only; current archive/image-policy work is protected. |
| 87201c21e186d7f8b9b92985116738d586a8a7e2 | Depends on collect_os_events_once from unported GPU-loss diagnostics 9eabd93990f2aabea9a2173722dab661157d34c9. It is absent in the lead source. The tentative patch was withdrawn completely; crash code is unchanged. |
| 03675c3ffad940cf2b2cd24acb33d5066d4cd129 | Media consent Modal→banner changes focus, placement and host overlay/blocking integration. |
| f1922d8e844efeb5f8550fc35f3e0fb444608061 | Favorite emoji UI/persistence depends on unported recent-row3afebccf and Shared Chat source ownership. |
| 3b76dc88dc1f3c73c83662e2949ffd232d3ea098 | Protected renderer blend-pipeline depth comparisons and scene/demo work. |

Earlier PR38 deferrals, including canonical-name privacy in avatar-only tips,
remain with their owners; this port does not repair them.

## Validation and integration

Focused checks: complete audio library tests, including offline loopback
playlist/redirect/MP3/security cases, and actual viewer staff tests. Added
regressions exercise the accepted four-document boundary and unchanged direct
audio bytes above the document limit, even at a .pls path. Public live probes
stay explicitly ignored. Run formatting and strict lint/parser CI; exact frozen
SHA, commands/totals/logs belong to the PR. No heavyweight local viewer build or
cleanup in the disk-constrained shared workspace.

Coordinate Cargo.lock/Cargo.toml, the audio module declaration, ARCHITECTURE.md,
TESTING.md and this note/TASKS. Architecture reconciliation retains only the new
audio row; unrelated upstream renderer documentation is not imported.
No shared App, UI, localization or host/Monitor/Chat/Attention wiring changes.
Slick is not callable here; no delivery or native build start is claimed.
Queue integration after the active98b30be2 checkpoint, then freeze the combined
source and run its affected guards/full validation at the agreed checkpoint.

No new screenshots: the resolver adds no UI and badge appearance is unchanged.
Native Windows/macOS, final fonts, actual station playback and human/grid
acceptance remain separate. Human follow-up: with normal media consent, compare
a player URL and its direct stream, stop/restart/switch while resolving and
confirm denied media stays stopped; verify the ordinary Aurora Team mark with
privacy rules intact. Only the human logs into a grid or handles credentials.
No merge, install or release.
