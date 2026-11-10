# Foldurora current handoff — Inventory/host group

## Current group: Inventory windows and host native frame

Repository **DrCum/Foldurora**, branch
`fix/native-frame-foreground-preflight-20261110`.
Corrected prerequisite source: `6451f7aeeda035b76ef9d06c8911573aa68d15e0`,
tree `5c30f3bb47ad92a2948e3f0cabcd1967e8b7e57c`.
[One corrected, review-first Windows request](FOLDURORA_HOST_FRAME_PREFLIGHT_REQUEST_6451F7AE.md).
Prepared/published is not delivery, acknowledgment or execution.
Canonical Git-LF Cargo.lock SHA-256:
`e04f6814529802ba6469b9455294aabfcfbec989d0e271698b3723a04d682e32`.

**This group is unfinished. No repaired host or new Windows package is claimed.**
Keep one eventual combined product-validation/build handoff. Do not create
separate Inventory and host product builds, or repeat the completed Monitor run.

| Slice | Implementation | Actual validation |
| --- | --- | --- |
| Unlimited full Inventory windows | Integrated, including reconciliation with Folder Jump and the existing single additional-window workspace role | Linux all-target viewer check and qualified Clippy passed; 13 i18n tests passed. Viewer regressions compiled, not run on this source. Windows/native acceptance pending. |
| Rectangular-host prerequisite | Native helper/example remains separate from production; corrected foreground setup and bounded diagnostic trace | DAB native result inconclusive. Corrected-source formatting and small MSVC-target harness strict Clippy passed; independent diff review and new Windows execution pending. |
| Production host styling | Still pending: popup surfaces/input, exact authority join, both worker placement paths and mixed-version fence | No production repair or acceptance. Ordinary host retains its existing HRGN behavior. |

Inventory source provenance: implementation commits `c09a063a2949b31a005e351fa7fe223762c5d885`,
`250eee4d1ea6d78bafcbf2c80e641c1ce3ca75c2`,
`36f9576a366f417abd42f536ee46d086a446989e`, reconciled on the Monitor base;
local integrated Inventory checkpoint `fa1581b607e95a1881feb57ae49996e31aacd3ba`.
The separate original documentation-only commit is not executable validation.

Every main button/Ctrl+Shift+I action creates a fresh full owner, in both looks,
with independent state and shared mutation/clipboard/permission/RLVa paths.
Closed owners with pending documents remain retained. Extras are temporary;
explicit named-workspace restore still controls one runtime role without
serializing UI UUIDs or retargeting pending operations. EN/FR/JA text is updated.
The former singleton-toggle test is retired for the user's selected behavior,
not passed. Eight new ordinary regressions and one ignored large profiling case
replace/extend coverage; unrelated permission/storage cases remain intact.
Large-inventory optimized Windows performance and both-look native behavior are
unaccepted; there is no invented window cap.

The [native prerequisite runbook](FOLDURORA_NATIVE_FRAME_PREFLIGHT.md) and
[reviewed PR44 plan](SESSION_HOST_NATIVE_FRAME_PLAN.md) define the next gate.
The prototype uses one persistent insertion anchor, independent own-worker
ordering, owned popup and unowned opaque guards, plus exact native identity,
current focus and geometry observations. Its synthetic acknowledgement never
stands in for a production lease/GPU/input grant. The ordinary host/worker
protocol is unchanged; no unfinished v3 presentation is advertised.

## Recorded DAB result and correction

Frozen `dab73b2b70c1a2e35ffc331e5c4cc7b2c9b96415`, tree
`7e034cd93da34686608bfded83b8e27def7bfaeb`, remains **INCONCLUSIVE**.
[Verified immutable result](https://github.com/DrCum/Foldurora/blob/91e1900b9fa08eee65bb5bcb99db9e08c006de3c/docs/FOLDURORA_HOST_FRAME_PREFLIGHT_DAB73B2B.md):
parser/C# compile/format/strict example Clippy/build passed; ONE native invocation
exited 1 at host foreground while worker paused. Initial placement/popup/third-app
and worker focus/cover-only refusal passed; first native cover call 3.720 ms.
Paused-host activation, later cover/minimize/restore, second pending target,
stale PID/registration and detach were NOT RUN. Both children exited 0. No final
PASS, product version/package or full suite. Preserve its 397,312-byte example,
SHA-256 `64abcb4f6b05e03354ec2e51bc3798256ad4ba42fb013b088b9ec89c460f16fb`.

6451 corrects the missing worker-to-controller activation-eligibility handoff,
checks SetForegroundWindow's BOOL, and observes actual host foreground plus
existing guarded order through bounded own-controller message-pump turns, before
the same worker pause ends. It captures bounded native focus/order metadata in
memory and prints after timed operations. No retry/forced input/global setting,
assertion removal, production enablement or new protocol mode. The DAB cause is
still unidentified; the setup gap is a source-backed hypothesis, not a diagnosis.
Independent review of this concrete correction is required as the first Builder
gate; a passing static harness does not replace it or establish native behavior.
Do not rerun DAB, repeat Monitor520 or dispatch a competing product build.

Separate local popup preparation `58fe40417d1e3d69d322c1fb34f5f58ada75d9f4`,
tree `85c756a38e7267205238dfab87c25d93569aa973`, is preserved on
`feat/host-popup-surface-integration-20261110`. Its bounded crop/coordinate model
has six focused component passes, strict small-harness lint and source review;
it is unwired, not part of 6451, and not native UI evidence. No source rollback
or new package is implied by keeping the preflight correction separate.

Next: obtain real Windows prerequisite observations, then join bounded popup
surfaces from the existing single CPU egui context/paint, input/IME/platform
output, immediate cover authority and both native worker placement paths.
Independent review and one frozen combined full-validation/package checkpoint
must follow. A small native preflight is not the final combined build.
No full Linux linking rerun was attempted. Historical terrain/filesystem and
resource limitations remain unresolved; Windows results are not Linux passes.

## Preserved verified Monitor base

- Base: `520782f2998150a26c74c51e18eccc7c4994c737`,
  tree `bcded0890dbdb359d44db28c399956fa2fb1ccd5`,
  branch `feat/live-inactive-monitors-20261109`, draft PR46.
- [Immutable Windows result](https://github.com/DrCum/Foldurora/blob/f5352283395e7c6f04e3765582435d5590aa5130/docs/FOLDURORA_WINDOWS_VALIDATION_520782F2.md):
  9,198 passed / 0 failed / 63 ignored, zero filtered; all 1,413 requirements
  passed exactly once. Separate ignored GPU export: one pass. Parser, format,
  strict Clippy, Monitor/XR viewer and sidecar package verification passed.
- Verified ZIP `Foldurora-520782f29981-windows-x64.zip`, 74,819,659 bytes,
  SHA-256 `6868063c75d7d0928c0468f5f89697e7b8965f01f4d28375f4611be010b919ce`.
  Viewer version `0.9.3-beta.20261106.1+525.g520782f29`.
- Standalone offline Vulkan startup passed on RTX3060/NVIDIA610.88. This is
  startup only. Adapter identity was unobserved in the separate GPU fixtures.
- Hosted live Monitor sampling/movement/switching, viewport/picking/mouselook,
  responsiveness, native workspaces/MCP, real-grid/RLVa, driver-reset, XR and
  Japanese glyph acceptance remain open. Uncertain privacy/teleport reservations
  can still require worker exit; no automatic recovery is inferred.

Keep Monitor, radio47955237, 98b and a903 packages and earlier failed candidates
separate. [The prior current record](https://github.com/DrCum/Foldurora/blob/9ac30677b6ab32c97b0efffa39c94a40486894f6/docs/FOLDURORA_CURRENT_HANDOFF.md)
preserves exact history, failed/unrun stages, upstream mappings and remaining
MCP boundaries. No old failure is relabeled by this integration. Profile Copy
was removed with approval; shared store/ACL/no-reparse/hardlink/CAS/long-path
regressions remain. MCP workspace Apply/Previous remain unsupported.

Quick-action extensions, aliases/accents/alerts, session sets, restart/recovery,
background audio/voice, conference extensions and automatic-read/notification
Attention remain shelved. Do not resume them for this group. No merge, install,
release, credentials or grid login. The overall porting goal is not complete.
