# Functional MCP integration with current Advanced Workspaces

Input PR33: head674a1a73989b5319f955b7b2cf4b26e9e4e0c3a6,
reviewed author implementation16b3f9895b2843d85dbf7d1573e73306e707f2bd.
Base integration76913ee8 (queued PR18/19/26/32) on frozen Advanced Workspace
da8ee12fd. Earlier descriptor tests and Windows OVERLAPPED fixes remain.
This is local source integration, not a GitHub merge, installation or release.

The independent controller/character tools, explicit opt-in, per-profile policy,
session-only character enablement, native frozen-argument Ask, bounded official
SDK stdio/private IPC, source/incarnation/revision fences and native adapters are
integrated. Current permissions, RLVa, credentials, creator protections and
upload spending/verification safeguards remain authoritative. SDK discovery is
not proof of support by the exact viewer/platform/grid. See FOLDURORA_MCP.md.

## Deliberately bounded workspace bridge

The PR's older workspace context model overlaps the existing source and must
not replace the current strict catalogue, private background/Inventory refs,
scoped graphics/camera or native one-shot HUD review. This integration retains
the native model/controller/UI unchanged and adapts read-only MCP list/status/
preview to an explicit geometry/chrome projection. Every read reacquires the
actual source/profile/grid and privacy gate. No stored HUD names/item/asset IDs,
folder refs, local image IDs/account/grid/pixels/paths enter the projection.
Only native named workspace records are supported; the separate older layout
catalogue is not synthesized.

MCP workspace apply/previous remain unavailable in native capabilities and fail
explicitly; no alternative path can exercise them. The SDK may discover those
tools, so clients must use the exact native capabilities. Existing user-driven
Advanced Workspaces remain intact. Completing MCP mutation requires an adapter
that fences asynchronous placement/final effects to the original MCP capture and
preserves current archive/RLVa/source checks and explicit HUD consent. Do not
substitute another queue or permission bypass while this dependency is open.

The author's two alternate-context and one alternate-HUD-queue test definitions
belong to that unintegrated backend. They are retained in the exact reviewed
source, not relabeled passed or silently waived from a previously integrated
gate. A comparison found ZERO removed baseline test definitions. New real
projection and unsupported-capability regressions cover the chosen boundary.

Preferences use the current workspace busy/held/HUD/Inventory/import state;
native workspace admission refuses a curated MCP preview in progress. Retained
descriptor fixtures adopt the new discovery domain without changing assertions.
Native profile/storage/host/workspace security source remains intact.

## Measured checks

- Formatting, strict MCP all-target ci Clippy and qualified native net/session/
  viewer all-target ci Clippy pass (existing Linux dead-code allowance only).
- Actual MCP library70, session library56, session core30 and real SDK stdio4
  pass:160 distinct crate/target cases,0failed/0ignored. These are focused groups,
  not an unfiltered full workspace suite. All four stdio cases use isolated
  synthetic native authority/private IPC; they are not grid acceptance.
- Localization13/0/0 and unchanged full-tree appearance scan1/0/0 pass.
- Python LSL harness syntax passes; lummao/runtime behavior remains unrun.
- Native viewer/net cases compile but cannot execute via full viewer linking in
  this32GB environment. New read-projection/capability tests are compiled/unrun.
  No full Linux/Windows/package/native acceptance for this new candidate.

Initial integration Clippy failures (missing descriptor domain, obsolete
Controller.assistant reference) were corrected; both complete negative logs
remain. No security assertion/source guard was weakened. Windows must retain
strict lint and unfiltered full suite, source-bound coverage and packaging gates.
Five new Unix/Linux-only source cases are recorded separately from Windows
requirements; their guards are unchanged. Three upload-handle cases remain
Linux native-viewer execution requirements, not passes or Windows waivers.

The earlier a903 Windows8970/0/58 and verified package remain a distinct frozen
checkpoint. Da8ee12f's separate prepared request must not be duplicated or
superseded blindly. Check existing Builder history through Slick before choosing
the next combined request. Raw logs/coverage inputs stay private. No production
live inactive rendering, hosted responsiveness, GPU-reset, XR or human grid
acceptance is claimed.
