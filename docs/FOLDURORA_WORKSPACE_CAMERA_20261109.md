# Optional avatar-relative workspace camera context

Reference: Folderstorm abbe3cb0 `fsworkspacecontext.h::controls(true)`,
`FSWorkspaceController::applyContext` and its guarded camera rollback. This
implementation uses Aurora's own CameraSettings ranges and existing camera/RLVa
helpers. It saves only preset type, avatar camera/focus offsets, scale, FOV,
transition/smoothing, tracking preference and avatar zoom fraction.

Capture defaults off. Save/Update and last-arrangement records opt into the
current values; disabling capture clears only last-arrangement camera capture,
not named records. Old catalogues default to no camera group. Malformed,
nonfinite/out-of-range values, zero camera offsets, unknown fields and future
versions are refused. This is a typed preference snapshot, not a saved camera
library, free/world/object anchor or photo/environment state.

Save and target switching refuse camera work unless the source is current and
in ordinary avatar-focused third person, with no free camera, camera owner or
native preset lock. Other groups remain usable in records without a camera.
Startup camera admission failure is reported and is not silently replayed later.

Every live settings application, Preview replacement and Cancel goes through one
App helper. Rollback preferences require the original logical account/grid/login;
activation/focus changes cannot promote a worker or grant input. The existing
RLVa `preserve_camera_settings` helper gives new script locks priority over old
preview values. Zoom application additionally rechecks exact hosted source,
ordinary mode and current restrictions. It uses native `set_zoom_fraction`;
orbit, pitch, camera mode and avatar controls are not reset. If mode/authority
changes before a pending placement commits, the Preview cancels. Last-arrangement
capture skips an unavailable camera instead of claiming to save its transient
state. A prior successful arrangement remains intact. The native frame's RLVa
view constraints continue to apply.

Preview retains a union of touched groups for Cancel and durable Settings save
projection; Previous uses only the final committed target's groups. Unrelated
mouselook, XR, movement and constraint preferences are preserved. Zoom comparisons
tolerate only the small rounding error of native fraction conversion.

Focused component evidence executes the three new snapshot cases against the
actual CameraSettings, preset/range helpers and snapshot source. Unused photo
CameraShot data types are fixture declarations; no World, App, store, renderer,
full Settings save or RLVa execution is claimed by this harness. The two new
real Settings/controller cases compile in ordinary all-target viewer Clippy and
remain Windows execution gates, together with existing RLVa camera regressions.
13 localization tests cover EN/FR/JA. Full suite, package and native/grid acceptance
remain pending for the combined group; no full suite per implementation slice.

Targeted human checks: save opposite camera contexts; switch, Preview/Cancel,
Previous and replacement by a geometry-only target; change unrelated camera
options during Preview and save settings; enter free/alt focus/mouselook before
capture and during Preview; acquire a script camera lock during Preview; switch
characters and restart. Only the user performs grid/restriction acceptance.

Workspace HUD wear/confirmation and padding backgrounds are still unfinished.
The separately gated disabled Monitor renderer is not a live inactive producer.
