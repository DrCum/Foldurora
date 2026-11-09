# Queued upstream integration after Advanced Workspaces

Separate source group on frozen workspace da8ee12fd; not part of its Windows
request/package. No GitHub PR is merged by this local integration.

| PR | Reviewed port head | Upstream change |
| --- | --- | --- |
| 18 | 475df44eee0525696151840ecf063540555a8bf1 | 3413d4058a037d955aca5306c9524d86ff2f0259: bounded plugin backlog/FIFO on disconnect/error |
| 19 | 65da1dfbcecfda273c5f1cb0650746374499732a | 479f15c2dda6e9335395316de0b64c43a1aaae1f: decoded-clip edge fade |
| 26 | 16071c7d9f53d65214b34a8f18e6f0453e0313e3 | c10cd55456d4b0fadd78f833012c14588cccbf6e: confirmed Preferences reset |
| 32 | 0d2d1ae9f09d1cfe0f090f06eebf4f832f693d6c | Mac Delete, fixed font-popup width, layout-independent shortcut tests, independent toolbar style IDs |

Heads/bases and already-present source were rechecked. Full non-TASKS patches,
including source tests/fixture, attribution, architecture and testing notes,
applied cleanly with three-way checks. Existing TASKS progress is retained with
an additive integration entry. PR32's base is the already integrated font-search
head5dc8f3e; the other three use388b299c. Prior PR11–16 are not repeated.

Media reads at most256KiB per idle, frames forward once and preserves complete
messages before close/error. Heartbeat grace only for a Running plugin while its
backlog consumes budget. The deferred browser picker stage is not imported.
Decoded sounds receive a shared stereo edge envelope after resampling; generated
PCM/stream packets and existing parcel-music buffering remain unchanged. The
sound fixture is documented synthetic data. Preferences use the shared danger/
Cancel modal and existing single-application/Undo/account exclusions. Inventory
Delete remains focused-list-only and uses existing confirmation, worn/selected
item guards. Font search retains the earlier port and fixed width; toolbar style
controls receive independent IDs in both editor looks.

Actual combined source checks: formatting0; audio/media/viewer all-target ci
Clippy0 with the existing Linux dead-code allowance. Complete audio library suite
106passed/0failed/4ignored; media library25/0/0; localization13/0/0; unchanged full-
tree appearance scan1/0/0. Twenty-two new test definitions:12 media/audio executed,
10 genuine viewer cases compiled but not run locally. No full Linux viewer link,
new Windows/package, native/GPU/grid acceptance. The four audio ignores are
inherited, not new waivers. No security/restriction tests or fixtures are weakened.

Functional MCP PR33 remains a separate reviewed-input boundary. Its alternate
workspace context/HUD model must be adapted to the lead model; read-only saved
HUD detail requires a fresh privacy projection. Review notice:
https://github.com/DrCum/Foldurora/pull/33#issuecomment-6743332165
Keep frozen reviewed source, lead OVERLAPPED fixes and a903 descriptor regressions.
No competing owner changes are requested. Inactive Monitor author head remains
42734a3d, already integrated as a disabled prototype; no live caller is claimed.

Source/check evidence is retained in queued-upstream-integration scratch. Full
combined validation follows the next coherent integration checkpoint; do not
rerun a903 or dispatch a second competing workspace build. Merge/install/release
and grid login remain unauthorized. Shelved feature groups stay shelved.
