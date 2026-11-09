# Standard additional Inventory window in workspaces

Reference: Folderstorm abbe3cb0 `fsworkspacelayout.h::extra_inventory` and
FSWorkspaceController capture/apply/placement. Aurora natively supports ONE
standard additional full Inventory window; this port reuses that role instead
of creating a second manager, arbitrary folder/document windows or custom tabs.
That count is an intentional difference from Folderstorm's multiple instances.

Capture defaults off. An optional private workspace group stores open/minimized
state, normalized geometry and optional folder/expanded references. Old records
remain absent. The existing remember-folders choice controls durable metadata
capture. No UI ID, title, search, clipboard, editor text or asset payload is stored.
A closed/minimized target cannot contain navigation references. Geometry uses
the existing limits; native four-role arrays and serialized identities stay
unchanged. The shared placement helper is extracted without changing those
roles' behavior. No new IPC protocol or store/permission policy is introduced.

Preview does not create, close or move the extra window. Committed Switch,
Previous or startup changes its lifecycle/geometry. This commit-only behavior is
another conservative difference; the manager's ordinary reversible geometry
Preview remains unchanged. Previous follows the final target's touched scopes;
absent folder groups do not become default navigation or full editor rollback.
Native closing normally drops this window's search/folder state; unsaved editor/
dialog/dependent state is refused rather than destroyed. Other partial Inventory
windows and the main owner's selection/expansion remain untouched.

All effects require actual native presentation/source/activation, original native
grid namespace, no showinv, current owner, no renderer failure/recovery or capture/
settings/Preview conflict. A fixed native role supplies the ID, never catalogue
input. Ambiguous/wrong owners refuse rather than select a different window.
Existing native open/close/minimize mechanisms and shared anchors are reused;
a saved minimized state supersedes queued Expand, and closing does not leave a
latent close request that can cancel a later explicit open. No OS reparenting or
repeated host hide/show/fit loop is added.

A newly opened visible window must paint its own owner before metadata is used.
One pending item allows at most three UI passes and one absolute two-second
deadline. Each pass checks exact source/binding, rules, archive publication,
actual permission and new input. Focus/occlusion/native geometry/recovery/source
loss or another workspace action retires pending work. A new window with a fresh
user selection is not retargeted. Already accepted visibility is not pretended
undone by cancelling later metadata. There is no worker, unbounded replay or
later other-source navigation.

Folder/expansion consumption uses this exact window's ordinary Ready mask and
non-exhausted policy revision, native own-root path checks, and Fetched ancestry.
Unknown data waits only within that bounded slot; denied/missing/restricted data
refuses. No hidden owner, exemption, extra tree scan/budget or fetch to make
captured metadata available. Opening a saved visible Inventory is explicit UI
restoration; its normal browser loading does not grant metadata restoration.
Baseline capture may preserve known lifecycle when optional metadata is unknown;
explicit Save/Update remains strict about requested available metadata.

Focused evidence: two new actual InventoryWindow/Geometry cases plus two reused
expanded-folder DTO cases (four component passes, with overlap). Qualified real
viewer all-target Clippy compiles all eight new regressions. Six genuine UI/
shared-floater/controller cases remain unrun locally. Formatting/localization and
unchanged appearance-scan results are recorded in the current handoff/receipts.
No full Linux link, Windows package, native/Vulkan, grid or overall parity claim.
Development lint/type/script failures remain preserved; no lint/security exception
was added. A903 remains a separate frozen Windows request, not this source slice.

Human checks: save open, closed and minimized extra-window records with opposite
positions/folders. Preview/Cancel changes no extra lifecycle; Switch/Previous and
startup restore only included groups. Dirty document/rename/dialog/operation
must prevent destructive changes; unrelated partial windows and main state stay.
Immediately reopen after close; minimized state must survive the next native
paint without flicker. Type/select/resize/switch source during delayed restoration;
no stale navigation, newly revealed restricted rows or hidden fetch. Only Sodie
performs real grid/restriction acceptance.

Additional arbitrary Inventory instances/Received Items detail and image/per-
display backgrounds remain separate work. Functional MCP stays with its owner;
production live inactive Monitor and original host/driver-reset/XR acceptance
are not established by this bounded utility slice.
