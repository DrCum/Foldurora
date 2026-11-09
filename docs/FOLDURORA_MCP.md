# Host MCP foundation

This is a disabled-by-default foundation, separate from per-character Local
Assistant connections. The production broker owns the compiled metadata adapter,
but no service listener, settings UI, approval UI or worker action route is
enabled. The synthetic smoke runs the actual sidecar,
protocol and private transport without a viewer, grid or user credentials.

## Baseline and reference mapping

Starting Foldurora commit `f08c22e2b0875b7d9b22a1dfe8742c1884321ff5`, tree
`1847d3371bb2eb90b3ba6c03acf74f54212274db`. Foundation source
`0cbc3cf1d6184fc10ac609bbbe850b2a352a35e3` is integrated on the frozen host-repair
base `dd0860918dd52e2a70b6874a00e2a77ea1c18b3a`. Older published handoffs remain
historical evidence, including publication holds.

Folderstorm was inspected at
[`abbe3cb0ca66ee2484f277219f6a27706fab7fc6`](https://github.com/DrCum/Folderstorm/tree/abbe3cb0ca66ee2484f277219f6a27706fab7fc6):

| Reference | Responsibility | Foldurora |
| --- | --- | --- |
| `tools/fs-mcp/cmd/fs-mcp/main.go`, `tools/fs-mcp/internal/mcptools/controller_tools.go` | MCP stdio sidecar | `aurora-mcp::sdk`, `foldurora-mcp` |
| `tools/session-host/controller_pipe.h`, `controller_host.inc`, `controller_model.h` | Private controller IPC, discovery and host permissions; **not MCP** | `aurora-mcp::ipc`, `authority`, `wire` |
| `indra/newview/fseventapibridge.{h,cpp}`, `fssessionassistant.h`, `fssessionworker.cpp` (`AssistantInfo`, `AssistantConfigure`, `AssistantPause`, `AssistantReview`), `fsassistantpermissions.h` | Separately enabled character Event API and viewer-side native tools/permissions | Audited only; no character connection, configuration, discovery or tool implementation |

Folderstorm's `controllerLoginReference()` obtains a safe opaque login reference
without enabling Local Assistant. Foldurora instead reuses the existing complete
`aurora_session::Binding`; simulator session tokens never enter MCP.

Intentional differences: Folderstorm rejects metadata `Ask`, and its status read
principally checks stable profile/revision. Foldurora supports actual pending
metadata approvals and binds all reads/grants to the complete current login.
Foldurora uses its own closed private JSON framing, Windows named pipes and Unix
domain sockets; no FSC9 wire compatibility is claimed. Monitor and lifecycle
tools from the reference are absent.

## SDK and protocol

Official Rust SDK `rmcp = "=3.9.0"`, defaults disabled, features only `server` and
`transport-io`. Source inspected at
[`5596a8a0bba2a1ec5519cc855cdadf23e4dcecdc`](https://github.com/modelcontextprotocol/rust-sdk/tree/5596a8a0bba2a1ec5519cc855cdadf23e4dcecdc).
MSRV 1.88 fits the checked Rust 1.99.1. HTTP, OAuth, TLS, client, child-process,
elicitation and SDK macro features are not enabled. Eight registry packages are
added; existing locked versions remain unchanged. The initial dependency-only
probe added about 5.4 MiB of source/archives and 112 MiB of shared `ci` artifacts.
Those figures are not the final foundation's complete build footprint.

The server prefers handshake protocol `2025-11-25` and supports the SDK's earlier
known handshake revisions. The newer initialization-free lifecycle is excluded.
Only tools are advertised:

| Tool | Arguments and result |
| --- | --- |
| `controller_list` | No arguments; opaque controller instances and foundation version |
| `sessions_list` | Exact `controller_instance`; permitted profiles or a body-free pending receipt |
| `session_status` | Exact controller, `profile_id`, `profile_revision` and running `session_ref` |
| `controller_operation_status` | Exact controller and original connection-scoped `request_id`; never approves a request |

Results include permitted profile label, coarse phase, opaque reference and,
only with current native permission, displayed account/grid names. Counts and
rows omit unauthorized profiles. No raw snapshot, account catalogue, chat,
location, credential, private path or raw error is returned. Unknown tools/actions
fail explicitly; there is no generic command or Event API forwarding.

Client configuration uses the installed sidecar and the **viewer** installation
identity, never a hash of the sidecar executable:

```json
{"mcpServers":{"foldurora-host":{"command":"<installed foldurora-mcp>","args":["--installation","<64 lowercase hex viewer InstallationId>"]}}}
```

## Authority, transport and bounds

`HostPolicy::default()` disables host access and sets metadata/actions to `Never`.
Host enablement does not authorize a profile or enable a character assistant.
Global and per-profile metadata/action `Never/Ask/Allow` remain separate; even
action `Allow` has no dispatcher. Grants are in memory and require a current
typed ticket. Replacement worker/account/grid/session/generation, unavailable
state, detach or a provider gap retire the old source's authority.

`Ask` freezes typed source tickets, not data. Only trusted host code may call
`resolve(AllowOnce|Deny)` after a real local decision. Polling alone never grants
access. An approved poll rechecks current policy, source and freshness, reads
current permitted data once, then consumes the receipt. Another client cannot
reuse the operation or session reference. Display permissions are supplied only
by the native owner for the exact fresh source; missing permission omits names.

| Resource | Bound |
| --- | --- |
| Fresh host observation | 5 seconds |
| Profiles / simultaneous private clients | 5 / 4 |
| Pending and terminal read records | 128 total; approval 70 seconds, terminal retention at most 10 minutes |
| Discovery | 64 descriptors, 2 KiB each |
| Private frames including length prefix | 4 KiB request, 64 KiB response |
| MCP input lines / serialized output | 64 KiB; output newline is one additional byte |
| SDK request admissions | 16, before SDK handler task creation |
| IPC call / stdout write | 5 seconds; Windows kernel cancellation completion can outlast its requested timeout |
| Private connection idle / partial request | 120 seconds / 5 seconds |

Discovery lives beneath the existing `ProjectDirs("org","Aurora","AuroraViewer")`
local data root, `session-host/mcp/<InstallationId>`. Descriptors and endpoints
are private to the current user, with fresh IPC credentials/incarnations,
ordinary-ancestry checks, no-follow/reparse refusal, peer UID/SID checks and
replacement-aware cleanup. Windows pipe clients reject impersonation and remote
pipe access is disabled. The sidecar's independent security code follows the
existing profile security policy; the private viewer binary helpers are not a
linkable library. Existing data is refused rather than repaired.

Linux uses a verified directory descriptor to address long Unix socket paths.
Other Unix platforms fail explicitly if their native socket path limit is
exceeded. No TCP listener or externally supplied endpoint path exists.

The SDK alone does not cap its handler task population. The wrapper admits a
fixed number of request IDs, rejects duplicates/overload, and drops unadvertised
notifications. A bounded output pipe queues read descriptors/placeholders only;
the sole stdout writer performs the actual authorized read when ready to deliver.
No protected payload is retained behind another stdout response. Authorization
is checked at read evaluation; bytes already being delivered cannot be recalled.

A cancellation of an admitted request ends the sidecar connection, retiring all
its pending reads and references. This deliberately avoids ID-reuse and retained
slot races when the SDK drops cancelled responses. EOF, malformed frames and
transport loss close private connections without replay. Malformed MCP syntax
and oversized lines follow SDK discard/recovery behavior; no fabricated parse
error response is promised.

## Precise lead integration

`crates/aurora-viewer/src/session_host/mcp.rs` is registered and the viewer depends
on `aurora-mcp`. The broker constructs `Publication` only after registry ownership,
feeds it newly constructed snapshots, and revokes it on closing, invalid data or
drop. Account/grid display permission is absent, so production always supplies
an empty display whitelist. The authority stays disabled and no listener starts.
This source integration is not a usable host MCP feature yet. Remaining work:

1. Retain the broker-owned authority's existing installation and coordinator host
   IDs; do not replace them with sidecar or GUI-derived identities. Integrate
   service lifetime with that owner, preserving immediate fail-closed revocation.
2. Load explicit host policy through the normal trusted settings owner. Keep
   default policy disabled; call `Server::start` only while explicitly enabled.
   Own that service and authority together; disabling must stop both and remove
   discovery. Persisting source-bound grants across logins is not implemented.
3. Keep `mcp::publish_fresh` on the existing authoritative live snapshot
   publication path, not polling `HostController::snapshot()`. Supply only
   current native privacy-permitted displays with exact `SlotId` and `Binding`.
   That native display decision is currently missing from `HostSnapshot`; do
   not infer it from raw worker labels or chat text.
4. Native profile controls obtain `ticket(ProfileId::new(slot.number()))`, then
   explicitly `grant` metadata/actions independently. Enumerate
   `pending_approvals()` in trusted UI; only a real user decision calls `resolve`.
   Until this UI is wired, `Ask` stays pending until expiration.
5. Revoke before source replacement/detach; call `stop` and drop the service on
   shutdown/provider loss. Reopening/reconnecting requires current-source
   authorization. Leave chat, switching, Monitor rendering and native actions
   unchanged.

Host repair protocol v2 remains separate from the MCP private wire. No live
action or character assistant integration is included. Windows packaging accepts
an explicit `-McpExe <built sidecar>` argument and includes this document; it never
automatically selects a neighboring executable. Shipping the binary does not
enable the host service.

## Focused validation

The original foundation author could not run normal workspace commands because
its environment lacked the pinned wgpu Git source. That limitation does not
describe the lead's combined checkout: its existing cache supports normal Cargo
resolution, with no patch removal or package-version changes. Integration filled
the lockfile's missing feature dependency edges and viewer path dependency.

Combined source checks: `cargo test --profile ci --locked -p aurora-mcp
-p aurora-session -p aurora-i18n` passed 148 tests, zero failed/ignored, including
the 32 MCP cases. Scoped viewer/MCP all-target Clippy passed with the existing
Linux `-A dead_code` allowance alongside `-D warnings`; formatting passed. The
five new adapter tests also passed in an actual-source component harness with
source-extracted host DTOs and opaque unused fields. The ordinary Clippy check
compiles those tests against the full viewer's real types. The harness does not
run the native broker or substitute for the full Windows suite.

The focused helper copies the exact two small crates, original profiles/lints
and lock into a temporary source workspace, omitting unrelated workspace members
and their `wgpu` patch. It resolves offline, verifies no unlocked package/version
was introduced, and runs the real check with `--locked`. It prints origin commit,
dirty state and focused source hash. All Cargo artifacts reuse the common
checkout's target cache. This is component evidence, not full workspace evidence.

```sh
python3 scripts/check-host-mcp.py test
python3 scripts/check-host-mcp.py clippy --all-targets -- -D warnings
python3 scripts/check-host-mcp.py test --test synthetic_mcp_stdio -- --nocapture
cargo fmt --all -- --check
```

The Linux subprocess smoke uses the actual binary and normal XDG discovery under
a private synthetic temporary directory. It covers negotiation, tools, denied
and pending reads, real fixture approvals, native redaction, revocation, login
and profile isolation, malformed/oversized input and EOF. Unit tests cover
capacity, freshness, authority expiry, disconnects and transport ownership.

Final commands/results and exact local commit/tree belong to the delivery
handoff. Windows compilation, PowerShell package validation, native checks and
the full integrated suite remain pending. Cloud disk headroom is insufficient
for a full viewer test link; the separate terrain failure remains unresolved.
Do not claim these focused checks establish full acceptance.
