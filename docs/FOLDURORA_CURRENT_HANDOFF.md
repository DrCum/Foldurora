# Foldurora current handoff — Inventory-only package

The user requested an Inventory-only package while the host repair remains
unfinished. Source branch: `build/inventory-windows-package-20261110`.
The product code is exactly the reviewed Inventory integration checkpoint
`fa1581b607e95a1881feb57ae49996e31aacd3ba`, tree
`03048e5a30434fedc6367a1686797ef53a7012d3`, plus package workflow/documentation.
The frozen build commit and actual results are identified by the GitHub Actions
run and packaged `foldurora-build-info.json`; do not substitute branch tip later.

New full Inventory windows have fresh UUIDs on every main button/Ctrl+Shift+I
activation in both looks, independent state, shared clipboard/mutation/permission
paths and no fixed cap. Pending documents retain closed owners. Folder Jump and
the named-workspace single additional-window role are reconciled without
retargeting pending operations or serializing UI owner UUIDs. EN/FR/JA updated.
Source review, Linux scoped check/qualified Clippy and 13 i18n tests passed for
the Inventory implementation. New Windows full-suite and package gates remain
pending until this actual workflow completes. Prior inventory-only tests belong
to their older source; no native or optimized large-inventory acceptance inferred.

The existing Windows test-package workflow is reused for this branch through
manual workflow_dispatch, with Rust1.99.1, strict checks, one unfiltered full run,
an audit matching every current Windows-compiled test exactly once, and all eight
new ordinary Inventory regressions required passing. The selected new-window
behavior retires the old singleton-toggle case; it is not reported as passed.
Ignored profiling/hardware cases stay explicit. All security/ownership/ACL/long
path/creator/archive/RLVa checks remain. No fixture or safeguard is weakened.
The historical private 1,413-case declaration subset is not imported into CI;
the CI audit instead covers every test in this pin's compiled list, with source
comparison preserving the baseline and its single authorized retirement.

The package retains Monitor support, XR-static loader, MCP sidecar, manifests
and license/notices. Source stamps and built/staged/ZIP-entry hashes must match.
Tests or required checks failing block packaging. This is a downloadable Actions
test artifact, not a GitHub release, installation or merge. Keep raw local
Builder evidence private; the workflow creates its own synthetic CI evidence.
Windows/GPU reliability, native UI, real grid/RLVa, driver reset and XR human
acceptance remain pending even if compilation/tests/package succeed.

Human checks for this update: create several full windows in both looks;
confirm independent search/selection/expansion/scroll/minimize/close; exercise
clipboard/drag between nonadjacent windows; confirm Ctrl+Shift+I and @showinv;
close/reopen owners with pending documents; inspect named-workspace restore.
Extras remain temporary outside explicit workspace restoration. Large inventory
Windows performance is still unmeasured; no cap is invented from old CI numbers.

The experimental host-frame repair is excluded. DAB native preflight remains
inconclusive; correction `6451f7aeeda035b76ef9d06c8911573aa68d15e0` and its
review-first request stay preserved separately on
`fix/native-frame-foreground-preflight-20261110`. Popup preparation58fe4041 stays
on `feat/host-popup-surface-integration-20261110`. Neither is enabled by this ZIP.
The ordinary host's existing styling/behavior is unchanged from Monitor520.

Monitor520 (`520782f2998150a26c74c51e18eccc7c4994c737`) remains the prior verified
package: Windows9,198/0/63 and1,413 required cases, separate GPU export1 pass,
standalone Vulkan startup only. Hosted Monitor/movement/viewport/picking/mouselook
and privacy/teleport automatic recovery remain unaccepted. Uncertain preview
reservations may require worker exit. Keep radio/98b/a903 packages and all prior
failed/inconclusive evidence distinct. No repeated old build, credentials, grid
login, install, merge or release. The combined host/Inventory goal is unfinished.
