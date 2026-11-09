# Foldurora current handoff — 9 November 2026 UTC

## Goal, policy and current source

Continue Sodie's ordered AFK roadmap, make routine decisions and document them.
When blocked, retain source/evidence and take the next independent feature.
Check the independently owned functional MCP PR between feature groups; no
competing implementation. No Astra. At most two implementation tasks and one
heavy cloud check. Standing permission covers source/build handoff publication;
no merge, installation, release, credentials or grid login. Human acceptance is
Sodie's. Overall Folderstorm/Aurora work remains unfinished.

Current corrected frozen candidate:
- `fix/mcp-descriptor-regression-20261109`
- Commit `a903e16c01db412b0a7fd77886851be3145e1852`
- Tree `a338a4b88a8a4e660efa59ad71d18878dc603e91`
- PR29 https://github.com/DrCum/Foldurora/pull/29, stacked after expanded-folder
  PR28, HUD PR27, Update PR25, background PR24 and themed contexts PR23.
- Source/remote tree verified; no new Windows full/package/native result.
- Source worktree `/workspace/work/workspace-folders-20261109` now continues on
  separate `feat/workspace-display-backgrounds-20261109`; all frozen heads remain.

Sanitized pinned Windows request (prepared only; delivery/ack/run UNCONFIRMED):
https://github.com/DrCum/Foldurora/blob/f305efdc55dc0d6d8f71ea9849b1f9e748e8df7a/docs/FOLDURORA_ADVANCED_CONTEXT_WINDOWS_REQUEST_A903E16C.md
15475bytes; SHA256 `85e45ec4ae4798294f3c6665cd34814fd2d59ef7c36d7bc743d863add8464f64`.
Source target is a903, not the documentation commit. Default full-run gate1184;
early small MCP package gate then ONE full workspace run and separate nondefault
renderer gates, ordinary xr-static-loader viewer/sidecar package. Reuses Builder's
existing dev cache. Source-visible additions are public; private coverage inputs
and identifying metadata are not in this request. Slick must check history and
relay ONCE to the existing Cumputer Builder01a1af77-a465-71c3-be08-cd9d1b927f83.
Do not duplicate a delivered/running request or rerun an unchanged failed source.

Automatic review rejected the first request upload because it contained private
coverage-input metadata; known not executed. Those references were removed,
destination absence checked, and the sanitized payload was published via the same
supported tool and byte-verified. No private evidence/alternate-route fallback. Local progress-docs history retains
the unsanitized draft as evidence: publish only reviewed sanitized file bytes,
never push that branch/history wholesale.
Direct app connection/PR attachments remain closed; no owner acknowledgment or
new build start is claimed. The latest completed Builder was idle.

Slick delivered the pinned5f990 request ONCE. Builder has completed and is
IDLE. The candidate FAILED its full suite:8935passed/1failed/58ignored, zero
filtered,37 summaries.1149/1150 required names pass once; the theme guard passes.
The sole failed requirement `ipc::tests::wrong_secret_and_stale_incarnation_are_refused`
failed at synthetic descriptor std::fs::write, Windows sharing violation32,
ipc/tests.rs122. Wrong-secret assertion and pipe connection precede it; stale-
incarnation and replacement-preservation assertions did NOT run. Reader sharing
(read/delete, no write) is consistent with this race, but the conflicting original
handle is unidentified. Do not claim a proven cause or weaken production sharing.
Monitor feature gates, viewer/sidecar builds, versions, packages, CPU/native smoke
were NOT RUN; no new binary/package identity. Source clean;2001 artifact records
and37 saved cache hashes matched. Original logs/private checklist remain local.
Sanitized report:
https://github.com/DrCum/Foldurora/blob/2cbeca99372f6ab1c8127b21399cd8479e8e0584/docs/FOLDURORA_WINDOWS_VALIDATION_5F99030.md

No duplicate/unchanged build or diagnostic. Next combined request must include
a reviewed MCP regression correction without weakening any security assertion;
coordinate with the independent owner before functional overlap. Direct app
connection/PR attachments are closed; no owner acknowledgment or new dispatch is
claimed. Continue source-side advanced workspaces in the meantime.

## Measured evidence and failures

- a6119664d9f576925c66e4bac20bbf049d0fdca9/tree70c1a3d0 remains FAILED.
  Windows parser/format/strict Clippy0; full suite101:8920passed/1failed/58ignored,
  zero filtered,37 summaries. All1134 required cases passed once, including all
  seven startup/UI/IO cases and33 Windows-enabled MCP/adapters. The sole failure
  was `ui::theme_guard::windows_take_their_appearance_from_the_theme` (direct
  checkbox). Builds/versions/package/CPU/native smoke NOT RUN. Preservation
  matched1945 artifact records and19 cache-output hashes. Exact sanitized report:
  https://github.com/DrCum/Foldurora/blob/f901066de67e40a59caf518a8e411405bf796708/docs/FOLDURORA_WINDOWS_VALIDATION_A6119664.md
- 5f990 replaces all four context/favorites checkboxes with existing shared
  themed controls. No guard, exception, cfg, test or permission rule changes.
  Its unchanged full-tree appearance scan passes as a std-only exact-source
  component (1/0/0; no filtering). Separate egui scrollbar test remains in full
  suite. Real viewer all-target ci Clippy0 with existingLinux `-A dead_code`.
- Source slices: folder8ea8e412 (treeccd8b157), graphics e9830d5e (tree5ef9c321),
  camera17b6337b (treeccc6aa4d). Focused actual-source component cases passed3 each
  and localization13 in each slice. Camera uses real CameraSettings helpers,
  with unused photo DTO fixtures. Graphics codec has no Settings fixtures.
  These do not establish App/RLVa/store/native acceptance; additional real
  Settings/controller/UI/IO cases compile and remain Windows gates. The UI
  missing-field development error was fixed and its negative log retained.
- Disabled renderer integration d08b2438/treeb8817f68 contains exact author
  commits504ef73c/42734a3d; feature-on renderer all-target Clippy0. Author's10
  Monitor,19 rendezvous,4 fence and30 preview scoped passes are inherited
  software-Vulkan evidence, with overlaps; not new lead GPU execution or a full
  suite. PR21 review https://github.com/DrCum/Foldurora/pull/21#issuecomment-6735782050
- d0d8f8f5 remains failed at old Windows MCP Clippy; later stages were NOT RUN.
  a611 cleared those two OVERLAPPED diagnostics without authority/ACL changes.
  The wrongly classified Unix-only connection-capacity case remains mandatory
  Linux coverage; its source guard is unchanged, not waived on Windows.
- Previous packaged f08c22e2 passed8875/0/58 and1087-name audit, but failed human
  host usability. Its package and every old failed candidate/evidence remain.
  Profile Copy's exclusive tests were retired with the approved removal, not
  passed. The old Linux terrain/software-GPU failure remains unresolved.

Cloud has32GB fixed disk, about184MiB free. Full viewer test linking is resource-
blocked; no new full Linux run is claimed. Only idle reproducible caches have
been reclaimed; source, binaries, profiles and needed evidence remain. Current
receipts/logs are under `/workspace/scratch/workspace-{folders,graphics,camera,
monitor-integration,theme-correction}-20261109/` and earlier advanced-workspaces.
Do not rerun full suites merely to chase deferred/historical cases.

## Subsequent source slice, outside the frozen build

Solid-color padding and catalogue reload are published at
`975250266db7eaf93f8f9af9cd4e9d8ffd4126c6`, tree
`316805c23e6b333c5b340ca00edcb0afd232144b`, branch
`feat/workspace-background-20261109`, draft PR24. Qualified real-viewer all-target
Clippy0; actual DTO/geometry4/0/0, unchanged appearance scan1/0/0, localization13/0/0.
Four additional real controller/source cases compile, not run locally. No new
full suite/package/Windows request for this slice. Future default requirements
are1158, separately retained in its scratch directory; current5f990 request stays1150.

Background choices are private account/grid metadata, opt-in per record and
paint only outside the actual world. Captured Preview target avoids borrowing
an old record's excluded context during Previous. Source gaps reload private
presentation without replaying consumed startup or polling failed IO. The first
DTO run3/1 failed because a unit variant accepted an extra field; the unchanged
refusal test passes with an empty struct variant. Negative evidence is retained.

An additional authoring dependency is published at
`bd6e631d4f78591286abf9e181c24c9de8873e96`, tree
`b5f984bbd6a4c78ada24d55875e74c6720419afa`, branch
`feat/workspace-update-20261109`, PR25. Explicit Update selected preserves record
ID/name/favorite/startup, uses current capture choices and the existing native
selected-record/CAS/receipt gates, changes no physical layout or Previous.
Qualified viewer Clippy0, actual theme scan1/0/0 and localization13/0/0; two real
model/UI cases compile, not run locally. Future requirements1160, no new full
build request. Earlier Save/Update wording did not establish an Update action;
the missing action was found and implemented in this slice.

Reviewed additive HUD restoration is published/frozen at
`48d6b4099aa143956ae62321525dcd16a26dccb6`, tree
`9a84502cc549cf047e697746b5dacb7248db401a`, branch
`feat/workspace-hud-restore-v2-20261109`, draft PR27
https://github.com/DrCum/Foldurora/pull/27. Actual source/remote tree verified.
Formatting, qualified real-viewer all-target Clippy0, six exact-source DTO/lane
cases,13 localization and unchanged full-tree appearance scan1/0/0 pass. Eight
additional genuine World/model/controller/UI cases compile but are UNRUN.
The first5/1 lane run, three development type/borrow errors and format diff remain
preserved. No full Linux link, new Windows package or native/human pass is claimed.
Future combined Windows requirements1174 (14 new cases), private scratch only.
Historical5f990 request stays1150; its failed result above is preserved. No competing request.

HUDs are opt-in private item/point references, freshly ordinary/archive/RLVa-
qualified, no assets/names persisted. Preview never wears. Committed switches,
Previous and startup require explicit one-shot review; native additive batches
serialize by point and finish only on observed wear under one30-second deadline.
Source/focus/resize/recovery/policy loss or manager close retires future work.
Existing worn items stay; unavailable references conservatively refuse the whole
request. Native COF/capacity/final send checks remain. Documentation:
`docs/FOLDURORA_WORKSPACE_HUDS_20261109.md`. Receipts and development negatives:
`/workspace/scratch/workspace-hud-restore-v2-20261109/`.

Expanded Inventory-folder state is published/frozen at
`e2fc23ea2c38f93840a16bbb8b3e96b40bbb9998`, tree
`b15062df5f7c408c576d7ac6666ea86db42f3704`, branch
`feat/workspace-expanded-folders-20261109`, PR28
https://github.com/DrCum/Foldurora/pull/28. Formatting, qualified viewer Clippy0,
2 actual DTO cases,13 localization and unchanged appearance scan1/0/0 pass.
Six further genuine native/archive/UI/controller/model/source cases compile
UNRUN. Future default gate1182. Fetched ordinary visible main-tree capture is
bounded256 references; commit-only restore initiates no hidden opens/fetches.
Advisory modified/last-arrangement capture is sampled at fourHz. New expanded and
HUD contexts also compare the original native login grid namespace. Original
geometry-source selection behavior is unchanged. Source doc and scratch receipts:
`docs/FOLDURORA_WORKSPACE_EXPANDED_FOLDERS_20261109.md`,
`/workspace/scratch/workspace-expanded-folders-20261109/`.

MCP correction a903 is published/frozen as above; only tests and its explanation
change from e2fc. Live replacement now uses a fully written private atomic
successor, retaining all original wrong-secret/stale/ref-preservation/no-dispatch
assertions. Separate deterministic cases retain SAME-identity changed-content
cleanup and original-reader/atomic-successor coverage; Windows also checks
in-place write refusal32. No production sharing/ACL/transport changes, retry,
assertion removal or security waiver. Linux MCP package34/0/0, zero filtered
(32unit+2 Linux-only stdio); strict MCP all-target Clippy0 and format0. This is a
package-selected graph, not full workspace/Windows/native proof. Windows held-
reader/rename checks remain UNRUN. Parent real viewer code/compile is unchanged.
Receipts: `/workspace/scratch/mcp-descriptor-regression-20261109/`.
Coordination notice before edits:
https://github.com/DrCum/Foldurora/pull/28#issuecomment-6739054196
No functional MCP PR visible; direct app closed and owner acknowledgment pending.
The functional owner must retain this correction and earlier OVERLAPPED lint fixes.
No competing enablement/privacy/tools implementation was started.

Additional native Inventory state is published/frozen at
`8b211528a6db6516cf83d588b106201a5bce434d`, tree
`7c9ebd64ba799b35159f24de78a14c0e1a46fc82`, branch
`feat/workspace-additional-inventory-20261109`, draft PR30
https://github.com/DrCum/Foldurora/pull/30. One existing standard additional
window; opt-in life/minimize/normalized geometry, bounded loaded folder refs.
Commit-only lifecycle, Preview never opens/closes/moves it. Native dependent
editors/operations are preserved. One continuation, at most3 fresh passes and
absolute2sec deadline; exact original source/grid/owner/privacy/RLVa/archive
fences. No hidden fetch or new arbitrary window owner. Qualified viewer Clippy0,
format0,13 localization,1 appearance scan and4 component cases pass (2 new,
2 reused expanded-folder cases). Six further genuine cases compile UNRUN;
8 new cases total, future gate1192. No new build/request/native claim.
Source doc FOLDURORA_WORKSPACE_ADDITIONAL_INVENTORY_20261109.md and scratch
/workspace/scratch/workspace-additional-inventory-20261109/ retain receipts.

Per-display padding colors are published/frozen at
`b8ab61831e104942f706f401f822cca3258871de`, tree
`c984fd9163f51e1c8a03ec6d6c11d452a19ff59e`, branch
`feat/workspace-display-backgrounds-20261109`, draft PR31
https://github.com/DrCum/Foldurora/pull/31, stacked on PR30. Source and remote
tree verified. Normalized geometry only, up to16 displays, common fallback
on changed/unavailable geometry. Bounded4Hz read-only native projection, fresh
post-click topology check and current-world paint clipping; no native move/
focus/resize. Existing Black/Solid wire forms remain. Images/import/placement/
brightness are unfinished and are the next dependency slice.

Qualified viewer all-target ci Clippy0, format0,13 localization,1 unchanged
appearance scan and8 focused actual-source codec/geometry/paint cases pass
(4 new/4 reused). Two further genuine controller/UI cases compile UNRUN;
6 new cases total, future combined gate1198. No full Linux link, new Windows
package or native acceptance. Development harness linkage and test E0499
negative logs remain; fixes retain assertions. Source doc
FOLDURORA_WORKSPACE_DISPLAY_BACKGROUNDS_20261109.md and scratch
/workspace/scratch/workspace-display-backgrounds-20261109/ retain receipts.

A903 request/source stay frozen; delivery/ack/run remain unconfirmed. Direct
Builder read and PR attachment both returned connection closed; no new dispatch
or execution claimed. Slick must check/relay the already prepared request once.
The feature-boundary open-PR collection still has no functional MCP PR.

## Roadmap and next actions

1. **Live inactive Monitor:** PR21 small same-device view foundation integrated
   but nondefault/disabled and no App caller. Actual source/privacy/RLVa/mandatory
   HUD admission, aggregate reservations, inactive scene preparation, bounded
   async native transport/freshness/retirement, cold allocation headroom and
   promotion/drain joins remain. Existing images still say LastActive. Reuse
   the preserved graph/old prototype as references, not a second renderer owner.
2. **Advanced workspaces:** startup/favorites implemented; optional graphics and
   avatar-relative camera contexts implemented; folder restoration covers only
   loaded visible ordinary main Inventory. Expanded main-tree and one native additional-window state are implemented as
   source slices above. Image backgrounds remain incomplete; per-display colors
   are implemented as the source slice above.
   HUD restore/review is implemented in the subsequent source slice, not yet native accepted. Solid padding colors are implemented in the
   subsequent source slice above; bounded expanded-folder and additional native
   Inventory restoration are published as above. Named-preset lookup beyond current
   snapshots and free/world/object/photo camera anchors remain intentional
   differences, not silently full Folderstorm parity.
3. Fixed-slot actions, aliases/accents/alerts, session sets, restart/recovery,
   background audio/voice: audit actual source/backlog then port in dependencies.
4. Conference identification gaps and Aurora catch-up; PR18media475df44e and
   PR19audio65da1dfb were read, not integrated. Verify current heads/source before
   applying. PR11–16 already integrated; do not duplicate. Refresh upstream head.
5. Automatic read policies and offer/permission/notification Attention remain.
Newly observed upstream PR26 (Preferences reset confirmation) head
16071c7d9f53d65214b34a8f18e6f0453e0313e3 is queued for phase4 review, not
integrated. No functional MCP PR was in the latest actual open-PR collection.

Latest feature-boundary PR search found no functional MCP PR; PR20/21 references
MCP but does not implement that independently assigned work. Retain the two
Windows OVERLAPPED style corrections when integrating it. Do not change frozen5f990 or rerun it unchanged. Continue independent source work meanwhile.

User exclusions: custom Inventory tabs, Previous Look, extra wear confirmations,
Fade/Birdseye, Folderstorm assistant/controller integration and Squirty notices.
The independent functional MCP assignment remains expected.

## Acceptance and targeted human checklist

Native default-glass/Vulkan, host movement/resize/responsiveness, switching,
first Open Monitor, world viewport/picking/mouselook/camera, driver-reset recovery,
XR, grid chat/restrictions and Japanese glyph acceptance remain open. A build or
CPU image does not prove them. No GPU loss should be induced by automation.

At the next verified package: move/resize/snap host and use chat/chooser; open/
reopen Monitor; test left/right/asymmetric world margins; save opposite workspace
contexts and exercise Preview/Cancel/Previous/restart and source changes. Verify
closed/restricted Inventory is not revealed and new camera locks win during
Preview. Native scripted restrictions/grid login remain Sodie's tests. Detailed
offline expectations and required NOT RUN reporting are in the pinned request.
