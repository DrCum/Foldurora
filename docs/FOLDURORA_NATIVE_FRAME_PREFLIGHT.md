# Rectangular host: native prerequisite

This is an intermediate checkpoint in the combined Inventory-window and host
styling group. **The production host repair is not implemented or enabled.**
The ordinary viewer still uses its existing HRGN presentation. Do not describe
this source, its example, or an Inventory-only package as a repaired host.

The [reviewed plan](SESSION_HOST_NATIVE_FRAME_PLAN.md) requires a real native
placement proof before production enablement. The example below is that first
experiment. It does not run the viewer, log in, select profiles or use credentials.
It creates a decorated host, an independent borderless worker in another process,
a same-process owned popup, and a third synthetic foreground application. It
changes only its own windows. It uses normal foreground requests, respecting OS
refusal; no injected keys, thread attachment, cross-process parenting or topmost.

```powershell
cargo clippy --locked -p aurora-viewer --example host_native_frame -- -D warnings
cargo build --locked -p aurora-viewer --example host_native_frame
.\target\debug\examples\host_native_frame.exe
```

Use an available owned local Windows desktop with an innocuous background. Do
not run alongside another desktop automation or while interacting with the proof.
A locked/remote/inaccessible desktop or OS foreground refusal is inconclusive,
not permission to force focus. Preserve the log and actual exit code privately.
The parent has a 20-second native-loop deadline and checked child shutdown;
emergency termination is confined to synthetic children it spawned and cannot
produce a successful prerequisite result.

The helper uses one persistent hidden native insertion anchor. Worker show and
activation constrain only the worker's own WINDOWPOS to that anchor. Covers can
hide without changing worker placement policy. The controller raises only its
own rectangular host/masks for cover and lowers its host after the exact
synthetic placement acknowledgement. The production source/lease/GPU grant
remains independent and is deliberately absent from this experiment.

The example checks exact child PID/registration and target geometry; duplicate
worker-hook refusal; current-focus input-open positive/negative baselines;
pending-target coverage; native hit targets and flat desktop pixels; ordinary
host activation while the worker UI sleeps; movement/resize and minimize/restore
with old-location black coverage; a separate foreground application's order;
stale PID/registration refusal; and checked own-worker hidden detach/exit.
Native cover calls must return within 200 ms. Pixel samples occur later, at
least 150 ms apart, with completion checked before the worker can resume its
1.8-second pause. **This is sampled opacity and call latency, not an instantaneous
compositor/GPU fence or production responsiveness measurement.** All processes
set and verify per-monitor-v2 DPI awareness before creating windows.

The original DAB prototype received independent source review. The foreground
correction described below is statically checked with a small MSVC target
harness; its independent diff review is still required before native execution.
These are separate gates, not Windows viewer compilation or native acceptance.
A successful example is partial prerequisite evidence only. It does not prove
normal frame pixels, snap/caption behavior, egui menus, tooltips, dismissal,
pointer/wheel/keyboard/IME routing, native dialog/tool families, mixed DPI and
negative origins, source lease expiry, failure injection, production recovery,
real GPU ownership, privacy or hosted Monitor acceptance. Keep these gates open.

## DAB result and foreground correction

[The immutable DAB result](https://github.com/DrCum/Foldurora/blob/91e1900b9fa08eee65bb5bcb99db9e08c006de3c/docs/FOLDURORA_HOST_FRAME_PREFLIGHT_DAB73B2B.md)
is **INCONCLUSIVE**, not passed. Windows parser/C# compilation, format, strict
example Clippy and example build passed. One native invocation exited 1 at the
actual host-foreground check while the worker paused. Initial placement/popup,
third-app order, worker focus and cover-only input refusal passed; the cover call
was 3.720 ms. Later activation, minimize/restore, second pending target, stale
PID/registration and detach were not reached. Both children exited normally.
No product viewer version/package or full suite was produced/run.

The proof had returned foreground eligibility from the third app only to the
worker. It never handed eligibility back to the controller, and discarded
SetForegroundWindow's BOOL. This is a source-backed setup gap, **not an identified
cause of the measured failure**. See Microsoft's
[foreground rules](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setforegroundwindow)
and [AllowSetForegroundWindow](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-allowsetforegroundwindow).

The correction checks actual foreground and exact host registration, then has
that foreground worker permit its exact controller PID before pausing. This is
ordinary process-specific activation eligibility, not a global policy/ACL change.
The controller makes exactly one normal activation request and records its BOOL.
It observes its own message loop for at most 250 ms, and requires both actual
host foreground and guarded exposure before pause+900 ms, well inside the same
1.8-second pause. An expired request is not issued. Refusal, late confirmation or
timeout remains inconclusive; no focus retry or simulated input is introduced.
The original coverage/input/geometry assertions and 200-ms cover bound remain.

The target host belongs to the requesting controller thread. The general
[cross-input-queue target explanation](https://devblogs.microsoft.com/oldnewthing/20161118-00/?p=94745)
does not by itself explain DAB. A bounded in-memory trace observes request BOOL,
foreground identity/thread, GUI active/focus query availability, own host order,
activation/position callbacks and exposure entry/exit. It prints after the timed
operations and cleanup, with no window text/profile content. Dropped records
block final PASS. SetForegroundWindow's LastError is not a cause diagnostic.
Raw observations stay private. A new Windows run is still needed; no correction
pass or production-enable authority is inferred from the added observations.

After measured native results, the lead still must join bounded popup surfaces
from one existing CPU egui paint/context, input/focus/platform-output routing,
controller cover authority and exact worker placement/input checks; replace both
legacy placement paths together; and fence incompatible presentation versions.
No new protocol mode is advertised by this intermediate source. The normal
Monitor/XR dependency and all account/creator/archive/RLVa/consent boundaries
remain unchanged. Follow the full plan before enabling production.

Keep one combined product-validation checkpoint for Inventory and host styling.
This small native prerequisite is not a separate Inventory product build or a
substitute for the eventual unfiltered suite, inherited requirement audit and
verified combined package. No completed Monitor test/build is repeated here.
