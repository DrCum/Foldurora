# Workspace themed-control correction

The a6119664 Windows run passed parser/format/strict Clippy but its full suite
failed: 8,920 passed, 1 failed, 58 ignored (37 summaries, zero filtered). All
1,134 required names passed once; that audit does not waive the suite failure.
The only failure, `ui::theme_guard::windows_take_their_appearance_from_the_theme`,
correctly rejected a direct egui checkbox in the startup/favorites manager.
Builds, versions, packages and native/CPU smoke were not run. Exact evidence:
https://github.com/DrCum/Foldurora/blob/f901066de67e40a59caf518a8e411405bf796708/docs/FOLDURORA_WINDOWS_VALIDATION_A6119664.md

The combined workspace source now uses `ui::widgets::checkbox` for all four
capture/favorites controls, within `add_enabled_ui` and the existing source
and Preview gates. The shared response retains click/accessibility/change
semantics. No theme rule, allowlist, test, source security check or required
failure was removed or relaxed. The upstream MCP owner still owns its separate
functional feature; the two prior Windows OVERLAPPED lint fixes remain intact.

A focused component run executes the unchanged full-tree appearance guard from
the verbatim std-only prefix of theme_guard.rs against the real source tree:
1 passed, 0 failed/ignored/filtered. This is source-guard execution; it excludes
the module's separate real-egui scrollbar test and does not establish a full
viewer pass. Real viewer all-target Clippy compiles the shared widget integration.
The full Windows suite, expanded coverage gate and package must run on the new
frozen cumulative candidate. Preserve a611 and d0d as failed candidates.

New group context cases remain in the default Windows suite. The nondefault
Monitor prototype requires its separate feature-enabled checks; it remains
disabled in the production viewer. Neither a passing audit nor a successful
compile is native acceptance, and no previously failed candidate is rerun here.
