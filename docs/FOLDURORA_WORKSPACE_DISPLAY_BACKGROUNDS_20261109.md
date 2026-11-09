# Workspace colors per display — 9 November 2026

A saved workspace can now include a common padding color and optional colors
for up to16 displays. Only normalized display rectangles are stored in the
existing private account/grid catalogue. No monitor names, hardware identifiers,
image/asset references or file paths are stored. The existing Black and Solid
records remain readable.

Display indices use canonical left/top geometry. Moving the entire desktop
origin preserves the arrangement. Missing, reordered, invalid or unsupported
geometry falls back to the common color; overrides are never assigned to a
different arrangement. The manager explains the fallback and clears its editing
target when the topology changes. Editing the common color remains available.
Selecting Black clears the overrides. Editing one display in a new arrangement
creates new overrides while preserving the common color.

Native queries are read-only, capped at17 entries to detect overflow and sampled
at most four times per second while needed. A window or UI-scale change refreshes
the projection. A display-specific edit performs a fresh native topology check
after the click, under the existing source-bound action ticket and private CAS
lane. This code never moves, focuses, resizes or docks the window. The projected
display clips may lag a native move by at most250ms; every paint still clips
against the current world rectangle. Common color covers gaps/unavailable
projection. XR, photo and source-isolation guards remain unchanged.

Preview/Cancel/Previous retain the complete original background configuration.
Painting uses the same non-world padding strips as the solid-color port. There
is no renderer owner, texture allocation, network request or inventory fetch.
All new visible strings are localized in EN/FR/JA and use shared themed controls.

Reference behavior was inspected in DrCum/Folderstorm at
`abbe3cb0ca66ee2484f277219f6a27706fab7fc6`,
`indra/newview/fsworkspacebackgroundmodel.h` (`arrangement`, `orderedDisplays`,
`forDisplay`), its bounded codec and `fsworkspacebackground.cpp` sampling/paint.
Excess or collapsed geometry fails to common color instead of truncating the
topology. Image import, texture backgrounds, brightness and placement modes
remain unfinished; this slice establishes their display-matching dependency.

Validation: formatting, qualified Linux viewer all-target ci Clippy,13 i18n
cases and the unchanged full-tree appearance scan pass. Eight focused actual-
source codec/geometry/paint cases pass (four new and four reused). The small
model harness changes only module-path qualification and uses the actual emath
types reexported by egui. Two further genuine controller/UI regressions compile
but were not executed locally. The initial harness linkage error and a test
borrow-check error are preserved. No full Linux viewer link, Windows package,
native display/DPI/host acceptance or human pass is claimed.

The existing a903 Windows candidate/request stays frozen. This slice adds six
source regressions for the next combined checkpoint; it is not a competing
build request. Native acceptance should cover independent display colors,
negative origins, mixed DPI, moving a hosted/spanning window, display removal,
Preview/Cancel/Previous, restart, and account switching. The world, camera,
input and floating windows must remain unaffected by padding.
