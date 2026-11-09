# Expanded Inventory folders in workspaces

Reference: Folderstorm abbe3cb0 `fsworkspacelayout.h::InventoryFolder`,
`MAX_EXPANDED_INVENTORY_FOLDERS`256 and FSWorkspaceController capture/apply.
This extends the existing opt-in main Inventory folder group; it does not add
custom tabs or copy inventory assets.

A new optional expanded-folder set stores at most256 unique folder UUIDs in the
existing private account/grid catalogue. Values are canonical and contain no
names, paths or item/asset payload. The aggregate catalogue also retains its
existing256KiB limit. Absent preserves older records and leaves
expansion alone; an explicit empty set records no expanded ordinary folders.
Preview stays reversible without navigating or changing the tree. Switch and
Previous apply the final scoped set only after layout placement commits.

Capture/restore supports the visible ordinary main Inventory plain tree, in
Legacy and Aurora tree layouts. Search/filter/other tabs, explorer lists,
partial/additional owners, inline rename, dialogs, progress and a pending
expand-all gesture are incompatible. Unobserved owners are not opened. A fresh
Ready ordinary owner mask with a non-exhausted revision must qualify the actual
Inventory. Every requested folder and ancestor must already be Fetched, under
the current own root, outside Library/Trash and pass the native loaded-path
checks. Restoring expansion therefore does not cause the normal tree's lazy
loader to fetch new branches. Overflow/incomplete visible ancestry refuses save
instead of silently turning a partial snapshot into an empty success.

The restore closes only current qualified ordinary expansions, opens qualified
saved ones, and preserves unrelated/hidden owner state. It does not change
search text, selection, item data, editor buffers or keyboard focus. A pending
reveal cannot reopen explicitly excluded branches. New restrictions, source,
profile and owner state are rechecked on the App thread at consumption. Failed
expanded restoration is reported separately from selected-folder navigation;
other successfully committed workspace groups are not presented as rolled back.

The new expanded context and HUD projection also require the workspace grid to
match the archive model's original submitted-login namespace, independent of IO
completion and mutable form state. The new read-only native_grid helper confers
no visibility/store authority. A regression rejects a mismatched grid. The HUD
native fixtures now use their actual native model namespace, with an explicit
negative case; no production permission or existing test assertion was relaxed.
The older geometry-source selection behavior was not redesigned in this slice.

Last-arrangement optional capture and the open manager's advisory modified mark
are sampled at most four times a second. Exact source/revision changes invalidate
the advisory cache; explicit saves and forced logout captures still qualify
current facts immediately. Closed managers do not calculate the modified mark.
Hidden/incompatible expansion is absent, not empty, and preserves an earlier
same-source last-arrangement expansion. These changes avoid an every-frame
folder walk or a second archive projection/budget.

Focused evidence: two actual DTO tests; the unchanged full-tree appearance scan;
formatting, qualified viewer all-target compilation and13 localization tests as
recorded in the current handoff. Six additional genuine native archive/UI/model/
controller/source cases are required on the next combined candidate. Component
passes are not World/App/native execution. Full Linux linking remains resource
blocked. No new Windows build/package, native or real-grid acceptance is implied.

Human checks: expand several loaded ordinary folders, save opposite records,
Preview/Cancel without tree changes, then Switch/Previous. Verify explicit empty
and old records, search/explorer/other windows, new archive/restriction changes,
closed/minimized Inventory, unloaded ancestry, over256 expansions, last-arrangement
restart and a character change. Existing hidden branches/editor state must stay
intact. Only Sodie performs grid/restriction checks.

Still unfinished: additional Inventory window lifecycle/geometry/folder state,
Received Items details and image/per-display backgrounds. This bounded main-tree
slice does not establish full advanced-workspace parity. The disabled Monitor
foundation and all native/GPU/host/XR acceptance gaps remain separate.
