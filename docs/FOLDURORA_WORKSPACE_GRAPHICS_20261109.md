# Optional workspace graphics snapshots

Reference: Folderstorm abbe3cb0 `FSWorkspaceController::capture` and
`applyContext`, which capture/apply explicitly selected graphics context controls.
Foldurora reuses Aurora's native `graphics_presets::saved::Saved` allowlist.

The default-off **Remember graphics quality** option includes the current native
quality in subsequent Save/Update and last-arrangement records. Named records
retain their own optional snapshot when capture is later disabled. Existing
catalogues default to no graphics capture. An available identical named preset
keeps its selection; a removed/changed preset loses its stale label while its
captured quality still restores. This slice captures current values, without
Folderstorm's separate preset-by-name mode.

The account/grid-private catalogue contains a versioned snapshot and bounded
canonical native quality data. Unknown nested fields, oversized input, invalid
profile labels and future snapshot versions are refused. Application uses the
existing native normalization and explicit quality fields. Hardware selection,
frame pacing, background audio, environment, account and credential preferences
are excluded. No full Settings object is persisted here.

Preview retains a union of touched groups for Cancel and Settings save projection.
Replacing a Preview restores its prior baseline before applying the next target.
Previous includes only groups changed by the committed target. The modified
indicator compares typed native quality without parsing JSON or constructing
Settings each frame. The manager body scrolls so additional controls remain
reachable in a short window. All actions retain the native source/ticket gate.

Measured evidence is focused: three actual codec cases and 13 localization tests
passed. The real viewer's all-target ci Clippy compiles the four additional native
Settings/Preview cases; they have not been executed in the cloud. Full Windows
suite, named coverage, package and UI/native acceptance remain pending. There is
no new full cloud suite or new package for this implementation slice. The earlier
startup/favorites a611 candidate and request remain frozen and distinct.

Human checks at the next combined checkpoint: save two opposite qualities,
switch/Preview/Cancel/Previous, replace a graphics Preview with a geometry-only
record, change an unrelated FPS setting during Preview and save preferences,
remove/change the named quality profile, restart the isolated account and use a
short/narrow manager. Existing host/Monitor, renderer and native GPU gates remain.
