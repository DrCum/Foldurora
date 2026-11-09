# Foldurora MCP integration

The controller and each character are separate authority domains. Both start
with their MCP listener disabled. Enabling the controller never enables a
character connection. Native permission controls are the only way to grant or
approve access; MCP callers cannot configure permissions or approve themselves.

This document describes the implementation contract and lead integration. Check
`viewer_status` for the exact running character's native capabilities. A tool's
presence in stdio discovery does not establish that an older viewer, platform or
grid implements its native prerequisite. Unsupported capabilities fail explicitly.
Windows, GPU and interactive grid acceptance are separate from Linux compilation
and synthetic tests.

## Source and behavior mapping

The foundation began at Foldurora
`f08c22e2b0875b7d9b22a1dfe8742c1884321ff5`, tree
`1847d3371bb2eb90b3ba6c03acf74f54212274db`. Foundation commit
`0cbc3cf1d6184fc10ac609bbbe850b2a352a35e3` remains frozen on
`feat/host-mcp-foundation`. Completion starts from the lead's integrated
`d0d8f8f5ad73f177160882a4543252e6941d661d`; its existing Windows candidate is
unchanged. Earlier documentation branches are historical references.

Folderstorm source inspected:
[`abbe3cb0ca66ee2484f277219f6a27706fab7fc6`](https://github.com/DrCum/Folderstorm/tree/abbe3cb0ca66ee2484f277219f6a27706fab7fc6).

| Layer | Exact Folderstorm paths | Foldurora |
| --- | --- | --- |
| MCP protocol | `tools/fs-mcp/cmd/fs-mcp/main.go`; `tools/fs-mcp/internal/mcptools/{server,controller_tools}.go` and feature-specific Go files | Official Rust SDK stdio sidecar, `aurora-mcp::sdk` |
| Private controller IPC | `tools/session-host/{controller_pipe.h,controller_host.inc,controller_model.h}` | Current-user private IPC, host authority and narrow native controller adapters |
| Character native API | `indra/newview/{fseventapibridge.cpp,fssessionassistant.h,fsassistantpermissions.h,fsassistantfeatures.h,fssessionworker.cpp}` and its `Assistant*` handlers | Separate character IPC/runtime, 45 permission classes, owning-thread preparation and native adapters |
| Native tools | `indra/newview/llinventorylistener.cpp`, `fsobjecttools.cpp`; feature listeners listed below | Existing typed inventory, scene, chat, camera, navigation and settings boundaries; no generic Event API forwarding |
| Shared posing | `fssharedpose{manager.cpp,model.h,policy.h}`; `fsposepolicy.h` | Native FSP1 packet/model and bridge relay; owner-granted peer authority is separate from MCP permission and photo posing consent |

Native feature paths under `indra/newview/` include `fscontentlistener.cpp`,
`fsbuildlistener.cpp`, `fsworldlistener.cpp`, `fsnavigationlistener.cpp`,
`fsgesturelistener.cpp`, `fspreferenceslistener.cpp`, `fsworkspacelistener.cpp`,
`fsposelistener.cpp` and `fsfileuploadlistener.cpp`.

The reference character bridge is an authenticated loopback HTTP/Event API,
**not** MCP. The reference controller pipe is private host IPC, **not** MCP.
Foldurora uses stdio MCP in front of two independently authenticated local IPC
namespaces. It does not claim Folderstorm private-wire compatibility.

The stdio catalogue has 173 tool names: 19 controller tools, 153 character
entries and the protected camera artifact reader. The character catalogue is
closed over these dispositions:

| Disposition | Count |
| --- | ---: |
| Native inventory, appearance, contents, uploads, interactions and testing | 74 |
| Native world, social, camera, navigation and gestures | 36 |
| Native build, posing and shared posing | 25 |
| Native curated preferences | 6 |
| Workspace discovery / integrated native reads | 5 / 3 |
| SDK/core discovery and status | 4 |
| Explicit refusal: remote `confirm_action`, permanent purge and empty-trash | 3 |

Native prerequisites and per-call authority still determine availability.

## SDK, discovery and client configuration

`rmcp = "=3.9.0"`, `default-features = false`, features only `server` and
`transport-io`. SDK source inspected at
[`5596a8a0bba2a1ec5519cc855cdadf23e4dcecdc`](https://github.com/modelcontextprotocol/rust-sdk/tree/5596a8a0bba2a1ec5519cc855cdadf23e4dcecdc).
Its Rust 1.88 requirement fits the existing Rust 1.99.1 toolchain. No compiler
upgrade is required. HTTP, OAuth, TLS, SDK client, child-process, elicitation and
macro features stay disabled. The initial dependency probe reused 77 locked
versions and added eight registry packages, approximately 5.4 MiB of sources and
archives and 112 MiB of dependency-check artifacts. Those are SDK probe figures,
not the complete viewer's build footprint.

The sidecar prefers protocol `2025-11-25`, supports SDK-known earlier handshake
versions and excludes the newer initialization-free lifecycle. Discovery
advertises tools only and paginates schemas within the output bound.

An MCP `notifications/cancelled` for a known in-flight request closes that stdio
session because the pinned SDK drops the cancelled response. Connection cleanup
retires its pending and unsent work. Unknown request IDs are harmless. The
source-owned controller cancel, upload cancel and script-stop tools provide
logical operation cancellation separately. A client must not automatically
reconnect and replay an uncertain mutation.

Use the installation identity copied from native MCP controls. It identifies the
**viewer installation**, not the sidecar executable. There is no PID or active
character fallback. Every character call supplies its exact `viewer_session`
from `viewer_list`.

```json
{"mcpServers":{"foldurora":{"command":"<foldurora-mcp executable>","args":["--installation","<64 lowercase hex viewer InstallationId>"]}}}
```

Production discovery uses the existing local data convention
`ProjectDirs("org", "Aurora", "AuroraViewer")` and separate
`session-host/mcp/<InstallationId>` and
`session-host/assistant/<InstallationId>` namespaces. Descriptors are published
only for enabled services. They contain private credentials and must never be
copied into client configuration, logs, PRs or tool output. Restart/reenablement
rotates endpoint identity and credentials. Shutdown removes only owned discovery.

## Controller permissions and native actions

The native management pane has eight independent global and per-current-profile
policies: metadata, activation, background mode, monitors, manual login, restart,
logout and detach. Each supports Never/Ask/Allow. Every profile grant is in memory
and bound to the full current `aurora_session::Binding`, or to the exact stopped
profile revision for a manual open. A stable slot never grants authority to a
replacement worker or login.

Metadata reads expose only authorized slot labels, coarse phase and opaque
references. Displayed account/grid names additionally require fresh exact-binding
native redaction/permission observations. Missing or stale observations withhold
both fields. Unauthorized profiles are omitted from rows and counts. Raw host
snapshots, saved-account catalogues, credentials, tokens, locations, chat,
filesystem paths and raw diagnostics are not generic metadata.

Metadata Ask freezes source tickets and remains body-free until actual native
AllowOnce. It revalidates and consumes the approved read once. Action approvals
freeze their affected profiles and operation arguments. Activation/manual-open/
restart also capture and authorize the owner displaced by their presentation
change. Final dispatch rechecks ownership and policy. MCP detach restores the
exact target while leaving unrelated managed workers alone.

Host tools cover controller/session discovery and status, activation, warm/
Economy background mode, manual login, graceful logout/restart/detach, monitor
open/close/list/arrange and cycling. Native capability absence is explicit.
Monitor calls require the current monitor revision and use a frozen opted-in
source scope; cycling does not silently adopt replacement logins.

Worker native observations use their own schema **2** over the existing repair
Envelope protocol **2**. Old workers without positive fresh native capabilities
remain unavailable for these actions. NativeAccepted is admission, not evidence
of a completed effect.

`controller_operation_cancel` retires an operation owned by the current
authenticated client and endpoint and disarms its unsent restart continuation.
It retains the operation tombstone. An already committed native exit prompt can
still finish through the ordinary viewer flow; cancellation does not claim to
reverse committed work.

Graceful exit uses prepare → actual fenced ordinary-window restoration and
ReadyForExit → one-shot final commit under the host authority lock. Preparation
expires in ten seconds; delayed commit consumption expires in two seconds.
Revoke, disconnect, source replacement, timeout and ordinary detach cannot revive
an uncommitted preparation. The existing native quit confirmation and editor
cancellation remain authoritative after committed admission. Restart additionally
requires QuitChosen, actual process exit, profile-lock release and final source/
policy/client/owner checks. It times out after 60 seconds without force-kill or
automatic retry, then opens a fresh **manual** login form; no credential or
login-submit route is added.

Economy performs an ordered GPU fence and positively acknowledged trim of lazy
blur/glass/selection/DoF buffers. It preserves scene resources, caches and saved
quality settings. Its disposable target set is intentionally narrower than the
reference's screen/shadow buffer trimming. Neither background mode grants input
or voice.

## Character authority, preparation and delivery

The native Local Assistant pane is independent of host settings and profile
grants. Enablement is session-only. ReadOnly and AskBeforeChanges presets never
opt in private reads. The 45 reference classes remain separate; read classes
with no reference Ask support offer Never/Allow. Invalid persisted combinations
fail closed instead of retaining a previous Allow.

The owning App thread observes the actual login and full binding. The IPC thread
can enqueue bounded immutable intents; it cannot touch viewer state. Native
preparation validates typed arguments, source, creator permissions, RLVa and
required additional classes, then freezes native evidence before Ask. Cold
inventory reads use separately authorized, correlated preparation before freezing
the final mutation scope. Permission changes never silently approve preparation.

An approval is an actual native click on the frozen request. Full arguments and
native evidence remain reviewable; long scripts and batch targets are not
silently truncated in the approval pane. Remote legacy confirmation cannot
replace native approval. Permanent inventory purge/empty-trash remain denied.

Private transactions and receipts bind authenticated client, endpoint incarnation,
full login binding and policy revision. Typed status aliases cannot cross
permission domains or extend their original receipt's expiry. Native update
barriers close delivery while state changes;
protected result proofs are checked before reopening delivery. Cached source and
asset disclosures cannot outlive current creator rights, asset revisions, object
ownership or native redaction. Body-free factual acknowledgments retain their
honest status without renewing action authority.

Native continuations commit under `Runtime::continue_action` after exact source
and policy validation. An active action retains its original native scope; a
switch away/back does not revive it. Disconnect/revoke/source retirement stops
unsent work and removes owned transient effects. Already committed network/native
work cannot be recalled; submitted/uncertain never means confirmed success and
must not be automatically replayed.

Take confirmation follows Folderstorm's inventory transaction observer in
`indra/newview/fsobjecttools.cpp:1988`, called from
`indra/newview/llinventorymodel.cpp:4540` during `BulkUpdateInventory`. The private
native observation requires the authenticated sender and circuit, the raw server
transaction, positive `DeRezAck`, and one exact fresh owned inventory item.
Original Take also requires observed object removal. Ambiguous or transactionless
responses stay uncertain. Ordinary inventory delivery and RLVa offer handling
retain their existing semantics.

Script-test recipe success describes completion of the recipe and its declared
assertions against observed native evidence. As in Folderstorm's
`tools/fs-mcp/internal/mcptools/objects_testing.go`, a passed recipe does not prove
an unacknowledged UDP effect or that a target script ran. Effect steps retain
their explicit simulator-confirmation status; reports identify their assertion
scope.

File-upload preview freezes encoded bytes, hashes and safe relative names.
Execute uses those bytes and checks the destination, permissions, balance and
spending ceiling before starting. Unsent native phases recheck current authority
and their destination and price conditions. A positive ceiling requires actual
native review even when Upload is set to Allow. The staged server offer rejects
a reported price increase before the paid POST; approval covers the frozen
ceiling. Reports distinguish that ceiling from a known actual charge and leave
an omitted completion charge unknown. Submission and private inventory
verification remain separate phases.

Snapshot uploads use native capture and separate free-thumbnail and paid-texture
flows. Paid texture uploads require the same frozen-ceiling native review. Thumbnail upload, assignment and readback each retain the original source
and current target proof. A confirmed assignment requires the expected native
readback. `inventory_upload_mesh_review` opens an owned pending native review;
an explicit user handoff opens the idle native DAE importer at the requested
inventory destination. The MCP tool makes no quote, spending or mesh-upload
request. Retiring its source removes its pending review; the user owns review
after the manual handoff.

Camera snapshots use native rendering and bounded JPEG encoding. Full JPEG bytes
stay in protected opaque artifacts, never exported paths. `camera_artifact_read`
retrieves at most 8192 bytes per hex chunk, using the original request and artifact
IDs on the same authenticated connection. Assemble by byte offset and verify the
returned SHA256. Artifact reads revalidate the original authority.

Polling a completed camera snapshot can also return a real MCP JPEG image
preview, at most 256 × 256 pixels and 12 KiB before base64 encoding. The private
typed image read uses the original completed capture, artifact, client and
current authority. Only native camera snapshot captures are eligible.

MCP requests for `show_ui: true` require a standalone character UI. All hosted
characters with a managed worker bridge are denied because a hosted frame can
contain Monitors or another profile's panels. This conservative scope applies to
both camera snapshots and snapshot uploads and is rechecked at capture and
continuation. World-only captures and ordinary manual snapshots keep their
existing paths.

Curated preference previews are temporary native transactions. They report
effective and saved values separately, require the observed revision, and expire
after at most 300 seconds. Accept persists through the ordinary settings
lifecycle; cancel, disconnect and revoke restore only values still owned by that
preview. A manual change is preserved and reported as a cleanup conflict.

The lead integration retains the current native workspace catalogue and its
existing HUD review, graphics/camera, folders and background schema. Read-only
MCP list/status/preview use a fresh actual login/profile/grid and privacy gate;
preview exports an explicit geometry/chrome allowlist, never saved HUD,
Inventory or managed image references or names. Native records are reported as
`workspace`; the author's separate `layout` catalogue is unavailable.

**Bounded integration limitation:** MCP workspace apply/previous are not advertised
as native capabilities and fail explicitly. The alternate context model and
second native HUD continuation from the PR are not integrated. A later adapter
must fence every asynchronous native placement/effect against the original MCP
capture and preserve current source/archive/RLVa qualification and HUD consent.
This is a deferred bridge, not a security exemption or a claim that all discovered
tools are native-supported. Ordinary user-driven Advanced Workspaces remain.
The PR's two alternate-context unit cases and alternate-HUD-queue case belong to
that unintegrated backend; all baseline lead regressions are retained. New native
projection/capability-refusal tests cover this explicit integration boundary.

## Bounds

| Resource | Limit |
| --- | --- |
| Fresh owning-thread/host observations | 5 seconds |
| Host profiles / cached character connections | 5 / 5 |
| Private clients per enabled endpoint | 4 |
| Discovery | 64 descriptors, 2 KiB each |
| Private frame request / response | Host 4 KiB; character 64 KiB / 64 KiB |
| MCP input and serialized output | 64 KiB per message; output newline adds one byte |
| SDK in-flight admission | 16, checked before SDK handler creation |
| Character queued operations / total records | 16 / 128 |
| Character arguments / native result body | 48 KiB / 24 KiB |
| Native Ask / retained terminal records | 70 seconds / at most 10 minutes |
| Approved bounded script recipe | At most 600 seconds plus five seconds; never extends Ask |
| Approved frozen upload execution | At most 600 seconds from actual native Execute; never extends preparation or Ask |
| Protected JPEG artifacts | 2, each at most 8 MiB |
| MCP camera capture/encode admission | 2 per process; retired descriptors retain their worker/output lease until payload release |
| Upload file read/encode admission | 1 per process, including queued work and a retained reader result |
| Frozen upload plans | 4 and 256 MiB of retained encoded bytes per character |
| Upload inputs / encoded batch | At most 32 files, 64 MiB per file and 128 MiB per batch |
| Upload directory depth / full-report preflight | 8 levels / 16 KiB, reserving 8 KiB before payment |
| Snapshot thumbnail / texture | 256-pixel, 2 MiB thumbnail; 2048-pixel source, 64 MiB encoded texture |
| IPC call / stdout write | 5 seconds; Windows kernel cancellation completion may outlast its requested timeout |
| Idle private connection / partial frame | 120 seconds / 5 seconds |

Each native feature also enforces its own file, row, step, snapshot and queue
bounds. For example, editable inline content snapshots are currently limited to
16 KiB; larger snapshots are marked incomplete and cannot be used as truncated
edits. Native authoritative ranges and unavailable native features take precedence
over the reference's broader platform ranges.

## Verification and lead handoff

Use the existing toolchain and shared compatible `ci` cache. Check free space and
confirm no other heavy build is running before a check. Do not clean frozen build
artifacts or dispatch a second Windows build for this branch.

```sh
. /workspace/.setup-tools/activate.sh
export CARGO_TARGET_DIR=/workspace/Foldurora/target
cargo fmt --all -- --check
cargo check --profile ci -p aurora-mcp -p aurora-net -p aurora-viewer --locked --offline
cargo test --profile ci -p aurora-mcp -p aurora-session --locked --offline
cargo test --profile ci -p aurora-mcp --locked --offline \
  --test synthetic_mcp_stdio -- --nocapture
cargo test --profile ci -p aurora-viewer --locked --offline app::mcp:: -- --nocapture
cargo clippy --profile ci --workspace \
  --all-targets --locked --offline -- -D warnings -A dead-code
```

The non-Windows dead-code allowance is the repository's existing CI convention.
The small `scripts/check-host-mcp.py` source-copy probe is still useful when only
core source is needed; it does not compile viewer adapters or establish full
workspace acceptance.

The user deferred the full integrated workspace checkpoint to the lead. Local
verification covers the affected MCP/session/native groups, Linux compilation
and aggregate lint. It does not establish full-workspace or native acceptance.
The lead should run the unfiltered suite on its combined integration source:

```sh
cargo test --profile ci --workspace --locked --offline --no-fail-fast
```

Do not describe an earlier focused pass as validation of a later tree.

For the isolated native-control CPU fixtures, use a fresh output directory:

```sh
export AURORA_MCP_VISUAL_DIR=/tmp/foldurora-mcp-ui-en
export AURORA_MCP_VISUAL_LANGUAGE=en
cargo test --profile ci -p aurora-viewer --locked --offline \
  mcp_cpu_visual_fixture -- --ignored --test-threads=1 --nocapture
```

These fixtures exercise actual pointer Deny/AllowOnce against in-memory
authority, complete frozen-argument review, pending host discovery and both
notification styles. The character IO owner is inert. Inspect the emitted PNGs;
their geometry and action receipts describe synthetic CPU controls only. See
[the fixture catalogue](TESTING.md) for the capture variables and scope.

The synthetic stdio test launches the real SDK binary and secured current-user
IPC with synthetic identities. It covers negotiation, paginated discovery, typed
calls, denied/pending/actual approval, revocation, isolation, replacement, malformed
and oversized input, overload, cancellation and disconnect cleanup. It asserts
that private sentinels never enter output. Malformed JSON syntax may be discarded
by the SDK; tests check bounded recovery rather than inventing a protocol error.

The completed implementation is commit
`16b3f9895b2843d85dbf7d1573e73306e707f2bd`, tree
`139ed1e4c69a15150b92a6ac0a07486965327cce`. The following Linux receipts use
that exact immutable source, Rust 1.99.1, the `ci` profile and the shared compatible
cache. A later documentation-only receipt commit does not change the checked
code. Independent GPT-6.1 Sol source review found no open concrete source blocker
on this commit after fixes for five isolation/proof issues, two resource-lifetime
issues and missing native Take observation paths.

| Check on the implementation commit | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Pass |
| Focused `cargo check` command above | Pass |
| `aurora-mcp` and `aurora-session` tests | 180 passed, including four real SDK stdio integration tests |
| Viewer `app::mcp::` | 40 passed; one CPU fixture ignored here and run explicitly below |
| Additional affected viewer groups below | 45 passed |
| Five affected network filters below | 36 passing executions; filters overlap |
| Aggregate workspace/all-target Clippy command above | Pass |
| Three explicit production-control CPU fixtures | 3 passed, 0 failed, 0 ignored; 92.58 seconds |

No failure occurred in these final focused runs. These are selected affected
groups; their totals do not represent a full workspace suite. The viewer filters,
each passed separately with `cargo test --profile ci -p aurora-viewer --locked
--offline <filter> -- --nocapture`, were:

```text
imports::assistant::tests
delegated_ui_capture_never_runs_in_composite_host_context
world::assistant_observations::tests
world::inventory::mcp_change_journal_tests
session_host::mcp::tests
session_host::worker::native_control::tests
session_host::monitor_model::tests
settings::curated::tests
workspace::context::tests
workspace::model::tests
owned_teleport_marker_cannot_cancel_a_new_manual_or_same_destination_request
```

The network filters, each passed separately with `cargo test --profile ci -p
aurora-net --locked --offline <filter> -- --nocapture`, were:

```text
assistant_
private_create_take_evidence_
inventory::upload::http_tests
bridge::shared_pose
session::teleport_cancel::tests
```

The focused regressions cover the closed 153-entry native disposition catalogue,
protected original-receipt expiry, visibility and owner/circuit revocation,
client/endpoint event isolation, same-caller-UUID callback isolation, staged
thumbnail/payment/readback, hosted-context capture refusal, upload/camera leases
through worker and output lifetime, native Take evidence and owned cancellation.
Final CPU artifacts are in `/tmp/foldurora-mcp-ui-11-en` (14 PNGs and 18 JSON
receipts). The host pending-review, character full frozen-tail and both Legacy /
Aurora selected-review captures were visually inspected. Actual pointer Deny
returned `permission_denied`; AllowOnce returned `queued`, with native dispatch
false. Both notification styles preserved the unrelated unread offer. These
captures validate synthetic production controls, without a native window or GPU.
Final local logs include `/tmp/mcp-completion-native-check11.log`,
`/tmp/mcp-completion-core-session11-tests.log`,
`/tmp/mcp-completion-native11-tests.log`,
`/tmp/mcp-completion-native11-ui-tests.log` and
`/tmp/mcp-completion-final11-clippy.log`. The additional exact commands and
outcomes are retained in `/tmp/mcp-completion-native11-viewer-extra-results.json`
and `/tmp/mcp-completion-native11-net-results.json`.

Native Windows, GPU and interactive grid acceptance remain pending. Python syntax
checks do not establish LSL behavior: this cloud environment lacks the harness's
Python 3.10/lummao runtime. No grid login or user credentials were used. The lead's
full integrated workspace checkpoint remains deferred as requested.

Earlier checkpoints exposed failures that were corrected before the final
receipts. Preserved local evidence includes:

| Earlier failure | Resolution / retained local log |
| --- | --- |
| Initial offline probe could not resolve the pinned wgpu source | The existing pin was subsequently fetched; no dependency version change was needed |
| Compiler and test-fixture defects: UUID serialization, missing Session defaults, native dispatch placement, digest formatting and hosted-context field name | Corrected in the implementation; representative logs: `/tmp/mcp-completion-core-session7-tests.log`, `/tmp/mcp-completion-native7-upload-tests.log`, `/tmp/mcp-completion-native-check8.log`, `/tmp/mcp-completion-native-check9.log`, `/tmp/mcp-completion-native-check10.log` |
| Formatting and aggregate lint defects | Corrected without additional lint allowances; representative failure: `/tmp/mcp-completion-native8b-clippy.log`; final pass: `/tmp/mcp-completion-final11-clippy.log` |
| CPU fixture texture-delta acknowledgement, synthetic observation freshness and an incorrect horizontal-scroll requirement for wrapped text | Actual painting, production 5-second freshness/70-second Ask guards and full-tail reviewability assertions retained; failed runs: `/tmp/mcp-completion-native9c-ui-tests.log` (0 passed, 3 failed), `/tmp/mcp-completion-native9d-ui-tests.log` (1 passed, 2 failed); corrected checkpoint: `/tmp/mcp-completion-native9e-ui-tests.log` (3 passed) |

A snapshot identity bookkeeping error also affected earlier reported Git trees.
The temporary index began empty and omitted the already tracked, ignored baseline
file `crates/aurora-viewer/src/ik_edit/project/target/tests.rs` from the tree
metadata. Every physical source copy and build included its unchanged baseline
blob `8a39feb99ce94050e69b8584b2090fde292a35d2`. Seeding the temporary index from
the baseline corrected source11's reported tree from
`8c1a34535d05cb557e1a198b6dfd5dbe45d8409b` to
`139ed1e4c69a15150b92a6ac0a07486965327cce`, matching the implementation
commit. Source bytes, the physical source digest and executed tests were
unchanged. The local errata ledger is
`/tmp/mcp-completion-source-tree-errata.json`.

For lead integration:

1. Build the viewer and stdio sidecar from the same reviewed source. Retain their
   installation identity relationship and current-user security helpers.
2. Review host opt-in/current-login grants and character enablement separately.
   Keep defaults off. Verify actual native AllowOnce/Deny and revocation through
   replacement, detach and shutdown.
3. Run the focused protocol command above before the lead's combined workspace
   checkpoint. Keep native Windows, GPU, UI and grid acceptance explicitly pending
   until performed on that combined source.
4. Inspect the shared permission and approval controls, long approval contents
   and both notification styles. Verify source replacement during Ask, current
   creator/redaction changes before delivery, graceful exit cancellation,
   detached late-finalization rejection and exact restore/fence ownership.
5. Shared posing requires the compatible native bridge source (version 1.42).
   Wearing/updating it and interactive grid tests belong to the user/lead; this
   implementation task does not install it or log in.

Likely integration conflicts are App module/state and per-frame/event hooks,
settings persistence, host worker protocol/transport, scene pose/effect overlays,
renderer control jobs and EN/FR/JA strings. Keep the narrow adapters and existing
native permission/ownership gates when resolving them. Do not merge, install or
release this draft on the strength of focused synthetic checks.
