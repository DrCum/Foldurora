# Explicit workspace update

Folderstorm abbe3cb0 `FSWorkspaceController::saveCurrent(...,overwrite,...)`
allows an explicit overwrite and keeps its active workspace identity. The
Foldurora manager previously only supported new Save/Rename/Delete. It now
includes Update selected, capturing the current authorized arrangement and
current capture choices into that existing record.

The record's ID and name, favorites and named startup reference remain intact.
No physical switch or asset action occurs. Preview/pending/busy/native-source
and selected-record ticket guards remain in force in UI and App dispatch.
Invalid capture or missing target changes nothing. Definitions publish through
the existing private CAS lane; active identity updates only after its own exact
receipt. A failed write preserves the original file and never reports success.
Updating does not create another Previous entry because it does not move/apply
a layout. Existing Previous continues to describe the prior physical switch.

Two new real model/UI cases cover stable references and atomic refusal, and a
captured-source UI request without mutating the painted catalogue. They compile
with the viewer and remain runtime gates for the next related-group Windows
checkpoint. Focused theme/localization/scoped lint results are in the handoff;
no full test/package/native pass is claimed for this slice. Keep5f990's existing
build request unchanged. This authoring dependency precedes selected HUD capture.
