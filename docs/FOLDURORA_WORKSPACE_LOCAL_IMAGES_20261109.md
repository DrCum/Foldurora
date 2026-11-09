# Managed local workspace images

Advanced Workspace continuation after Inventory image backgrounds (PR34).
Folderstorm reference: DrCum/Folderstorm `abbe3cb0ca66ee2484f277219f6a27706fab7fc6`,
`fsworkspacebackgroundmodel.{h,cpp}` and `fsworkspacebackground.cpp`.

The workspace manager now imports PNG, JPEG, BMP and TGA files into the current
account/grid's private managed image library. The catalogue stores an immutable
image ID plus account/grid identity, placement and brightness; it never stores a
live external path. Existing Inventory picture serialization stays compatible.
The six placement modes, display-specific targets, common spanning image and
world-viewport clipping are shared with Inventory backgrounds. The original
image dimensions determine layout; managed pixel copies are at most 256 pixels
per side. This intentionally bounds quality and residency rather than claiming
full-resolution Folderstorm parity. No image is uploaded to a grid.

The native dialog and bounded decode run on one reusable worker with one pending
request/reply. Selection only normalizes pixels. Before private publication and
again before changing the catalogue, App requires the original explicit intent,
controller instance, source/login, binding, window, target topology and background,
plus fresh native input authority. Expected dialog focus changes do not preserve
ordinary UI tickets or bypass authority. There is one two-second focus-return
window at each completed stage, measured from worker completion including queue
time. Another workspace action, source/host change, closed manager or GPU reset
retires the intent; a cancelled dialog keeps its original worker slot until its
reply drains. Nothing joins a blocked dialog on the frame thread.

Input files are bounded to 32 MiB, 8192 pixels per side and 32 megapixels, with a
128 MiB decoder allocation limit. Normalization removes source metadata. Each
managed PNG is bounded to 512 KiB; the versioned library holds at most 128 immutable
IDs and 32 MiB. Decode validates PNG contents before load/publication; structural
header validation alone is insufficient. There is no automatic garbage collector.
A cancelled post-publication action can leave an unreferenced image in its
original profile. Full or corrupt libraries refuse new imports without repair or
overwrite. No original filename/path is retained or logged.

The closed native store kind reuses the existing ownership, ACL, no-reparse,
single-link, lock, compare-and-swap and cancellable atomic-publication safeguards.
The earlier three store kinds and their limits are unchanged. Passive loads never
create a directory, lock or library. Separate managed display handles require a
fresh actual profile/grid/login and privacy/restriction proof; Inventory images
retain their independent creator/archive policy. Hidden/minimized, photo and XR
presentation gates remain in force. Missing/failed passive reads are bounded and
cached, not retried every frame. At most 18 visible reference images are requested.

## Validation

Actual-source background/managed-reference/placement component: 15 passed,
including two new managed cases. Structural library component: 2 passed.
Two genuine in-memory codec cases, two import-intent cases, three native-store
cases, one store-kind/path case and one native UI import/busy case are in the real
viewer test target. Isolated codec execution is blocked by existing image/moxcms
rlib incompatibility (E0460); no new dependency cache or replacement decoder was
introduced. Formatting, qualified Linux viewer all-target ci Clippy (existing dead-code
allowance), all 13 localization cases and the unchanged appearance scan pass.
Those nine compiled-but-unrun cases still require the combined Windows full-suite
gate. No security, source or platform guard was weakened.

No full Linux viewer link or new Windows/package/native acceptance is claimed.
The a903 package predates these changes. Targeted human acceptance: import/cancel
in the hosted and standalone manager, change source while the dialog is open,
reopen the original profile, check display targets and all placement/brightness
controls, then hide/minimize/enter photo mode and verify no world overlay leak.
No grid login, installation, merge or release is authorized by this document.
