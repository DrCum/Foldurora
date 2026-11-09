# Workspace favorites and startup restoration checkpoint

Extends the four-role native workspace implementation on d0d8f8f5. Reference:
Folderstorm abbe3cb0, FSWorkspaceController startup restore, favorites and context
capture. This checkpoint adds favorites and startup policy; HUDs, inventory
folders, graphics/camera and background restoration remain subsequent slices.

## Behavior

- Favorite/unfavorite the selected named workspace. Stable IDs survive rename;
  deletion retires favorites and startup references in the same atomic write.
- The optional favorites strip switches through the same captured-source/native
  layout gate as the manager. It follows the toolbar viewport and remains hidden
  during an operation or preview. Its default is off.
- Per-account/per-grid login policy defaults to Do not restore. Choose the named
  selected workspace or the last arrangement. Startup waits for an authorized,
  focused, ready native World; it never activates a worker or logs in.
- Run once per logical login. Source gaps before admission defer the operation;
  focus/activation changes after it do not repeat it. Opening native settings or
  the workspace manager gives explicit manual choices precedence.
- Last arrangement records the existing geometry/viewport/bar groups after two
  stable seconds. It does not persist an uncommitted Preview. Logout/exit attempts
  a final write and waits at most 1.5 seconds per outstanding write (at most two).
  Timeout preserves the original asynchronous job/receipt; it does not claim
  success, replace a different profile or relax store security. A forced crash or
  storage stall may retain the earlier successfully saved arrangement.
- Existing version-1 files decode with disabled/empty optional settings. Unknown
  fields, invalid/duplicate references and malformed stored layouts still refuse
  the file. Older viewers refuse the extended file rather than rewriting it.

## Validation and limits

The five new pure model/controller cases run in an actual-source component
harness with minimal Settings/store fixtures and the real emath Rect type. The
filter excludes the full Settings projection and native IO/UI cases: this is
focused component evidence, never a full workspace/runtime pass. Full source
and UI/IO tests compile in ordinary viewer all-target Clippy. Windows must run
all seven new cases plus existing workspace, source isolation and store gates.

EN/FR/JA controls are included. Native layout, both viewer styles, narrow-window
usability, exit durability, favorites switching, hosted viewport and cross-login
acceptance remain pending. The cloud cannot link a full viewer test binary with
its current disk headroom. Reuse the existing Windows Builder and full checkpoint
cadence; do not initiate separate full runs for every subsequent small slice.
