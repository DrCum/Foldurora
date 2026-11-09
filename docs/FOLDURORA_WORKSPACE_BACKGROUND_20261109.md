# Workspace padding colors — partial background port

Reference: Folderstorm abbe3cb0 `fsworkspacebackground.cpp` (Color source,
profile/grid configuration and drawing outside the actual world rectangle).
This slice implements Black and an opaque chosen sRGB color. Local image imports,
texture permissions, image fit/per-display backgrounds and their managed asset
store remain unimplemented. No background file path or asset identifier is read,
serialized, decoded or fetched here.

Current color is metadata in the existing account/grid-private workspace
catalogue. The shared native color picker edits a local draft; explicit Use solid
color / Black background requests publish through the existing source ticket,
private store, CAS and owned receipt. A matching pending write can show its choice;
foreign/stale pending metadata cannot. Failure does not report success and source
loss retires the view. No global Settings field or credential/account copying.

Remember background defaults off. Save/Update and last arrangement optionally
capture the current color. A target with no background leaves it alone; explicit
Black means restore the original black margins. Preview chooses its captured
value without publishing catalogue metadata. Cancel reverts naturally to the
committed color. Switch/Previous persist their target color through the existing
private lane after the native placement completes. Previous never re-reads a
named record to import a group excluded from its captured target.

The UI paints at most four non-overlapping padding rectangles below windows,
clipped to the actual native canvas and around this frame's rendered world
rectangle. It never fills the world, resizes a target, consumes input, grants
focus or changes environment. Full/unknown geometry draws no custom padding.
Photo/capture/freeze/interface-hidden and XR paths omit the custom painting.

Persistent workspace presentation now reloads its catalogue after an admitted
source gap even if startup was already consumed. It does not replay startup or
poll failed IO every frame. This also restores favorites-strip availability
without reopening the manager. Reads remain asynchronous, one pending lane,
current native permission and captured profile, with no ACL/security repair.

Four exact-source DTO/geometry component cases pass using real serde/emath;
no fake World or renderer. The unchanged full-tree theme guard also passes.
The first DTO run failed3/1 because serde's internally tagged unit variant ignored
an extra field despite deny_unknown_fields. Black is now an empty struct variant;
the unchanged refusal test passes, and its failure log is preserved. Additional
controller/source-gap cases compile against the real viewer but await the full
Windows group. Localization, focused lint and final source receipts are recorded
in the current handoff; no full runtime/package/native pass is claimed here.

Native checks: both styles, normal/short/narrow manager and color picker, full /
left / right / asymmetric / resized / negative-origin / DPI layouts, no covered
world pixels or click changes; Preview/Cancel/Previous and a geometry-only target;
private publication failure and account/source changes; favorites and background
after activation without startup replay; no background in photo/XR outputs.
Keep frozen5f990's current Windows request unchanged; this is the next source slice.
