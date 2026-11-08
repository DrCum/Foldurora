# Foldurora PR #14 — synthetic CPU font-search evidence

Source: `899d75c4bd2e840699a95eaea3b00909499512e4`.
PR head: `5dc8f3e620aa4d31fca47d0d155fa918d3a6f426` (TASKS-only follow-up).
Upstream: `bd9509fedc203387df7b5b83167ae9abb0270b98`.
Base: `388b299c429d776b3fc7a9890162cd3d940b9dc3`.

These PNGs are unmodified outputs of the actual production Preferences Appearance
panel in the `the_system_font_search_` regression tests, painted by the existing
CPU painter. They use four mock fonts, 1x DPI and the supported opaque skin
(`panel_alpha=255`). The production font-picker implementation matches upstream.
Every tessellated primitive was required to be a mesh; no GPU paint callback was
silently dropped. Ordinary regressions separately retain the default skin.

For each of 1600x1000 (popup below its button) and 1600x420 (popup above), the
sequence is opened, filtered by `Be` to Beta, cleared/restored to all four rows,
and reopened with an empty query. Matching JSON records contain painted text
and its bounds. `checks.json` and `runner.json` retain commands/counts and the
actual source checkout. These synthetic images do not establish native font
loading, default-glass, Vulkan, Windows/macOS/Linux native demo or human acceptance.
No native before/after comparison was run. Keep the PR draft.

Evidence run: https://github.com/DrCum/Foldurora/actions/runs/42459800071
Validation workflow: `236afc376df4bb929a391e399746fb6de4719a58`, on the isolated
validation-only branch, excluded from the integration PR.

All eight images were inspected. Opened, filtered and reopened frames include
transient popup fading; the short filtered popup changes side as expected. The
restored frames show all four rows within the popup above/below its button.
These are raw regression-frame outputs, not settled native visual acceptance.

Artifact: `13609753734`, SHA-256
`b5347b603844ff2ad8dd2ce05db11bbe09098e2daf238dbe84441fd67e05b970`.
