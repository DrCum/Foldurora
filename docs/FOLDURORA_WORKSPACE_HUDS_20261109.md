# Additive workspace HUD restoration

Reference: Folderstorm abbe3cb0ca66ee2484f277219f6a27706fab7fc6,
`fsworkspacecontext.h`, `fsworkspacecontextadapter.cpp::attachedHUDs/hudAvailable/
hudWorn/attachHUD`, and `FSWorkspaceController::acceptHUDs/pollHUDs`. Aurora's
native appearance planner, COF transaction and final RLVa send guards remain the
mutation authority. No direct rez, detach, replacement or asset-copy path was added.

Capture is opt-in in the native workspace manager's HUD section. Choose eligible
currently worn HUDs, then Save or Update. Only original item UUID and HUD point
are stored in the existing private account/grid catalogue (64 unique references,
points31–38); names, assets and paths are never persisted there. Old catalogues
have no HUD group. Clear selection can discard stale references without exposing
their names. Save refuses an unavailable selected reference rather than silently
saving a partial/empty result. The current worn baseline and selected save set
are intentionally different. Previous restores its scoped set additively; it
cannot remove items added by the intervening switch.

Qualification requires the actual source/login/avatar, own inventory/object item,
native HUD point, original item, no Library/Trash, ordinary native archive Ready
mask and current RLVa visibility/add rights. Names are ephemeral and bounded.
Unknown, partial, offline and unavailable visibility never mean exemption. A
workspace owns its own ordinary policy and borrows the existing projection/CPU
turn; there is no separate tree scan, budget reset, hidden fetch or asset lookup.

Preview never wears anything. Successful Switch/Previous/startup placement opens
the manager with an explicit review of missing items; startup never auto-accepts.
A manual Review selected workspace HUDs action can request a fresh review.
Already worn items stay on, and an item worn at a different point is refused.
The conservative port refuses the whole request when any captured reference is
unavailable, rather than displaying or restoring a partially qualified selection.
A review is one-shot, has a unique nonce and native source/ticket checks; button
IDs include the nonce so a held click cannot accept a replacement review.

Approval starts at most one native appearance transaction per HUD point. It
waits for actual item+point wear and native request retirement before advancing.
One30-second deadline covers the entire run, including native busy time; no
resend is sent. Every batch requalifies references and preflights the existing
COF/capacity planner before the normal native action, preventing its error
recovery from initiating hidden metadata fetches. Native final restriction checks
remain in place. Completion is observed wear, not a successful dispatch.

Source/binding, native focus/occlusion/resize, renderer failure/recovery, archive publication/profile and
restriction changes retire reviews and future work. Closing the manager also
retires future work even if its final paint could not emit a ticket. An accepted
native transaction may still finish; cancellation does not pretend to undo it.
Ordinary outfit inventory-generation changes alone do not replay or cancel the
current transaction; every next batch still needs a fresh ordinary mask. The
archive helper exposes only native publication identity, never visibility or
mutation authority. Closed idle managers do not repeatedly build HUD views.

Validation scope: six actual-source DTO/lane tests and the unchanged full-tree
appearance guard are component checks; they do not execute World/App/native IO.
Eight additional genuine World/model/controller/UI regressions are required on
the combined candidate, including the real native archive projection and held
review click. Qualified Linux viewer all-target compilation and localization
results are recorded in the current handoff/receipts. No full Linux viewer link,
Windows package, native/GPU or human grid acceptance is inferred from this slice.
The first5/1 lane run and development compile errors remain preserved.

Targeted human acceptance: capture two eligible worn HUDs on different points;
Preview/Cancel without wear; remove one manually, Switch and decline, then review
and accept; verify extra worn items stay and both groups serialize. Replace a
review during a held press, close/minimize/resize the manager's native window or
switch characters before approval/during the run; no future batch may resume.
Unavailable/archive/locked items must not reveal names or be fetched. Check full
COF/attachment limits, timeout/reporting, named/last startup and Previous. Only
Sodie performs real grid/restriction acceptance. Generic extra wear confirmations
and Previous Look remain excluded; this review belongs to the workspace feature.
