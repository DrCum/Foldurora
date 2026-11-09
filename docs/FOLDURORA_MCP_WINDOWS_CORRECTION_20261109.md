# MCP Windows correction and remaining integration dependencies

Frozen input1ba972d1419b22ee918515b12ecd5e6d42c4ca45 remains FAILED at strict
Windows workspace Clippy. Parser/format/strict MCP lint and68 early MCP cases
passed, including retained descriptor/error32 checks. Full tests,1356-name audit,
prototype checks, builds/package/CPU/native smoke were NOT RUN. No new binary.
Report:
https://github.com/DrCum/Foldurora/blob/6a4eb8498a2e90847f0447331af2ed6af4f050b3/docs/FOLDURORA_WINDOWS_VALIDATION_1BA972D1.md

This separate correction keeps the unsupported MCP workspace mutation boundary,
all native workspace schemas and source/privacy/HUD/archive/creator/RLVa guards.
No Windows security/ACL/policy changes, warning attributes or security exceptions.

## Eight diagnostics

| Diagnostic | Correction and retained proof |
| --- | --- |
| Windows-unused opened metadata | Validate opened regular-file shape and original length on all platforms; retain Unix device/inode and final metadata/no-links checks. |
| Unused inventory_continuation wrapper | Remove the uncalled wrapper; final effects still use existing commit_continuation/Runtime guards. |
| ContentSave asset/script fields | Remove redundant old-asset copy (native save still receives the validated snapshot asset). Retain script and bind replies to original save kind/item/object before cache or receipt updates. |
| Running object/item copies | Remove duplicate fields from the transient receipt marker. Actual query correlation remains keyed by object/item, execution generation and original capture. |
| PutObserve operation copy | Remove duplicate request-id string. Original full Capture remains the operation/receipt correlation. |
| Unconstructed TaskMutation | Remove the unconstructed/unhandled enum variant; no native route or permission check used it. |
| Uncalled continuation helper | Remove its uncalled definition; retain the separate active commit_continuation implementation and call sites. |
| OwnerEdit typed copy | Remove always-true unused local flag. Typed packet flags, joint bounds, generation/session/token/lease/revision checks and native acceptance are unchanged. |

Two genuine reply-correlation regressions use actual NetEvent/ScriptTarget types.
A matching request ID cannot confirm another target or save kind; such a reply
is consumed without clearing the original pending operation, which still requires
its proper reply or expires as unconfirmed. Baseline tests/assertions remain.

## Additional source review

Workspace metadata preparation previously checked a no-op under
Runtime::continue_preparation then queued its read outside that guard. The actual
bounded demand now enters inside the same guard, linearized against cancellation,
disconnect and permission change. The original source/profile/grid/privacy check
and later execute/delivery checks remain. No result disclosure was established.
The callback checks/queues work; the existing worker performs filesystem IO.
General Read deliberately allows only Never/Allow. No Read=Ask mode is introduced.

A discarded uncommitted regression experiment incorrectly assumed Read=Ask was
supported: both tests failed at the existing permission setter as intended.
The production policy is unchanged; the mistaken experiment/source/logs and an
initial isolated ABI-mismatch error are retained privately as negative evidence.
They are not product test failures, ignores or passes for this candidate.

## Verification and outstanding gates

Formatting and qualified Linux viewer all-target ci Clippy pass. A strict Linux
probe without the existing dead-code convention failed in platform-dead paths
outside the correction:29 diagnostic source files were byte-checked unchanged
from1ba. Full output is retained; this is not a strict Linux pass. No suppression
was added to source. Both new genuine cases compile but are unrun locally; full
viewer linking remains blocked by the fixed32GB environment. Windows strict lint,
unfiltered full suite and1358-name audit remain mandatory on corrected source.
No full Linux rerun, new package, native acceptance or permission weakening.

## Remaining dependencies, checked against current author heads

MCP33 head674a1a73 and Monitor21 head42734a3d are unchanged; comments have no
owner response. The following source joins remain unimplemented, not enabled:

- MCP mutation must bind original client/endpoint/login/policy and immutable native
  workspace definition through each late placement/Inventory/HUD effect. A current
  native UI Ticket is not that authority. Existing Runtime::continue_action holds
  its mutex and prohibits waiting/IO/reentry, so wrapping an entire multi-pass
  workspace routine is unsuitable. Native HUD consent, final COF/rez guards,
  conservative cleanup and factual observed outcomes must stay. Folderstorm
  fsworkspacelistener.cpp explicitly rejects general Event API mutations and uses
  prepared validation plus acceptAssistantSwitch(context.allowed). Only its
  prepared execution route is the relevant reference.
- Monitor App caller must prepare the inactive scene at small dimensions without
  driving the presentation path or consuming an unsubmitted journal. Existing
  host preview_projection/demand.rs sends RememberActive only; its reservations
  cannot be relabeled live authority. Admission must include acknowledged/pending/
  uncertain/draining peers and fresh native resource headroom.
- Renderer output is a same-device GPU lease, not a cross-process CPU frame. A
  bounded asynchronous export must retain source/role/epoch/claim/deadline and
  drain before promotion. Current output textures intentionally lack COPY_SRC;
  photo snapshot/readback is not a suitable fallback. Current privacy/HUD/photo/
  teleport/XR and loss gates must refuse unknown state.
- Cold allocation is materially larger than output pixels. Backend::new creates
  a4096x4096 Depth32Float shadow atlas (2048 cascade, count2) and32 radiance probe
  cubes at128px/Rgba16Float/seven mips before Monitor packet settings disable them.
  Those two requested texture payloads alone are100661248 bytes (~96MiB), excluding
  irradiance/scratch/targets/buffers/pipelines/driver overhead. This is source
  descriptor arithmetic, NOT measured VRAM or total peak. Cold constructor,
  allocation inventory and actual hardware admission remain prerequisites.

References: DrCum/Folderstorm abbe3cb0ca66ee2484f277219f6a27706fab7fc6,
fsworkspacelistener.cpp and fssessionworker.cpp:1641–1675; Foldurora
monitor/{main,backend}.rs, renderer.rs::Backend::new, projector_shadows::atlas_size,
probes.rs and session_host/preview_projection/demand.rs. No OpenGL path, generic
Event API or competing preview owner is imported. The disabled prototype stays.

Smallest safe next step after correction: validate the corrected combined source
through the existing Builder, then coordinate/review the prepared mutation adapter
against the current native controller. Production live Monitor acceptance also
needs actual scene/transport/headroom joins and Windows/GPU/human evidence. Do not
enable either boundary by configuration or inference from unit/package passes.
