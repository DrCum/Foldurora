# MCP Windows correction and owner coordination

Original failed source remains frozen at d0d8f8f5ad73f177160882a4543252e6941d661d,
tree af882184e494d378b9523c9f90c3686dd003a2dc. Builder handoff:
https://github.com/DrCum/Foldurora/blob/7491bdee09fac83c80566416086f9c1e4f3a6fe5/docs/FOLDURORA_WINDOWS_VALIDATION_D0D8F8F5.md

Parser/format passed. Strict Windows Clippy exited101 on field_reassign_with_default
at ipc/windows.rs:346 and replace_box at :472. Tests, name audit, viewer/sidecar
builds, packaging and native smoke were NOT RUN. The previous Linux focused pass
is not Windows evidence. Original raw logs remain with the existing Builder.

The lead correction changes only these two initialization expressions:
- Initialize OVERLAPPED with its hEvent in the struct literal.
- Reset the existing boxed OVERLAPPED through its dereference, keeping the same
  allocation and event. No pending IO, ACL, identity, path or timeout rule changes.

MCP owner: retain these two corrections when preparing the functional MCP PR.
The lead is not implementing service enablement, permissions UI or action tools
on top of your work. Your feature PR will be reviewed after this feature group;
resolve these small overlapping Windows lines in favor of the corrected form.
No MCP PR was visible at the most recent feature-boundary check. Direct chat
relay is unavailable; this public document is prepared for Slick to relay.
Delivery/owner acknowledgment is not claimed.

Coverage correction: only_four_connections_are_admitted_until_a_slot_closes is
#[cfg(unix)] at ipc/tests.rs:176. Remove this impossible Windows requirement;
keep it as a Linux requirement. The other Unix-only unit case is
links_foreign_modes_and_descriptor_overflow_fail_closed. Both original tests and
their guards stay unchanged. All Windows-enabled MCP tests (28) remain mandatory,
along with all unrelated native-store/security gates. d0d's corrected Windows
requirement count is1127; workspace startup/favorites add seven cases, giving1134.
A new source freeze and actual Windows rerun are needed; this patch is not proof
of a strict Windows lint or runtime pass. Do not rerun unchanged d0d.
