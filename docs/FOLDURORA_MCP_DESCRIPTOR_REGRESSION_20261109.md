# MCP descriptor regression after the5f990 Windows run

Frozen5f99030a7daabb2879563c998828a2beb2f7099d remains FAILED:
8935passed/1failed/58ignored,37 summaries, zero filtered. Parser/format/strict
workspace Clippy passed; theme guard passed.1149/1150 requirements passed once.
`ipc::tests::wrong_secret_and_stale_incarnation_are_refused` failed at the
synthetic descriptor std::fs::write with Windows sharing violation32. Its prior
wrong-secret assertion and pipe connection succeeded; the subsequent stale-
incarnation and replacement-preservation assertions did not execute. Monitor
prototype gates, builds, versions, packaging, CPU/native smoke were NOT RUN.
Source/artifacts/logs remain preserved. Sanitized measured handoff:
https://github.com/DrCum/Foldurora/blob/2cbeca99372f6ab1c8127b21399cd8479e8e0584/docs/FOLDURORA_WINDOWS_VALIDATION_5F99030.md

The listener rereads discovery while serving. Windows readers intentionally
share read/delete, not write. This is a source-based explanation consistent with
the write failure; the original conflicting handle was not identified. No
production sharing, ACL, ownership, link/reparse, path, authentication, cleanup
or transport behavior is changed by this correction.

The live test now stages complete descriptor bytes with the existing production
private creator, syncs/closes the file, and atomically replaces the synthetic
path. It validates the successor before the original stale-client call. All
original wrong-secret/refusal/preservation/no-dispatch assertions remain.
No retry, sleep, permission repair, exception or ignored test hides an error.

Atomic replacement changes file identity, so it alone would lose coverage of
the cleanup guard for changed contents at the SAME identity. A separate
thread-free test explicitly preserves that branch: publish a private descriptor,
rewrite its contents at the same identity, drop its original Publication, and
require the valid replacement to survive. Another test holds the actual original
reader across atomic replacement, validates the private successor, and verifies
the old handle still reads the old complete descriptor. Its Windows branch also
requires an in-place write to fail with32. That deliberately held reader proves
OUR test conflict, not the identity of the old failing process's handle.

Coordination was published before edits:
https://github.com/DrCum/Foldurora/pull/28#issuecomment-6739054196
The lead owns only this narrow test correction. The independent functional MCP
owner must retain it and the two earlier OVERLAPPED lint fixes. Direct app
connection is closed; owner acknowledgment is pending, not claimed delivered.
No functional MCP PR was visible at the feature boundary. No competing service
UI/tools implementation, merge, installation or release is authorized here.

Focused Linux package evidence:34passed/0failed/0ignored, zero filtered (32unit,
2stdio integration), including the formerly failing named case, both additions
and both Unix-only security/capacity cases. Strict ci-profile aurora-mcp all-target
Clippy and formatting pass. This package-selected graph is not the full workspace
or Windows graph. Windows reader sharing/rename/assertions remain unrun locally.
Full Linux viewer linking remains resource blocked; terrain/native/GPU/host/XR
acceptance is unchanged. The next pinned combined Windows run must keep the
original failed requirement and both new cases, plus all workspace/security/
renderer gates, before any viewer/sidecar/package/native claim.
