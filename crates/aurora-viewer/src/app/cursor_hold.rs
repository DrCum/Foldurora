//! The system cursor while the camera holds it: hidden and locked during
//! mouselook (LLToolGun) and during the camera drags (LLToolCamera), given
//! back whatever ends the hold. Alt held in mouselook gives it back for as
//! long as it is held (LLToolCompGun::handleHover, indra/newview/lltoolcomp.cpp,
//! originally LGPL 2.1).
//!
//! While held the cursor is the tool's (the mouse capture of LLToolGun and
//! LLToolCamera): the windows neither see it hover, nor get its clicks or its
//! wheel. Windows and X11 cannot lock it, only confine it to the window, so
//! mouselook brings it back to the world viewport's centre whenever it moves
//! (LLToolGun::handleHover → LLViewerWindow::moveCursorToCenter,
//! indra/newview/lltoolgun.cpp): it never drifts over a window (#1343).

use super::App;
use winit::event::{ElementState, WindowEvent};
use winit::keyboard::KeyCode;
use winit::window::CursorGrabMode;

/// A desktop mouselook hold belongs to the configured world view even when
/// a photo temporarily renders a full-frame output. Ordinary camera drags
/// and VR keep their existing effective-render placement.
fn capture_center(
    configured: aurora_render::WorldViewport,
    rendered: aurora_render::WorldViewport,
    mouselook: bool,
    vr: bool,
) -> (f32, f32) {
    let view = if mouselook && !vr { configured } else { rendered };
    let [x, y] = view.project([0.0, 0.0]);
    (x, y)
}

/// Recentring is a cursor placement operation, never a clamp or rescale of
/// raw device motion. A drag can enter mouselook without replacing its first
/// hold, so the camera mode, rather than HoldKind alone, selects the anchor.
fn recenter_target(hold: &CursorHold, mouselook: bool, alt_free: bool, cursor: (f32, f32), center: (f32, f32)) -> Option<(f32, f32)> {
    if !hold.held() || !mouselook || alt_free {
        return None;
    }
    (!cursor.0.is_finite() || !cursor.1.is_finite() || (cursor.0 - center.0).abs() > 0.5 || (cursor.1 - center.1).abs() > 0.5)
        .then_some(center)
}

/// A pointer event the windows must not see while the camera holds the
/// cursor. A release still reaches them: a press they saw before the hold
/// ends there, and a release alone does nothing.
pub(super) fn held_from_ui(event: &WindowEvent) -> bool {
    match event {
        WindowEvent::CursorMoved { .. } | WindowEvent::MouseWheel { .. } => true,
        WindowEvent::MouseInput { state, .. } => *state == ElementState::Pressed,
        _ => false,
    }
}

/// Why the camera took the cursor: it decides where the cursor reappears.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum HoldKind {
    /// LLToolGun::handleSelect: the cursor goes to the centre of the world view
    /// and comes back there (handleDeselect).
    Mouselook,
    /// LLToolCamera::handleMouseUp on a valid click point (Alt+click, focus
    /// tool): the cursor comes back at the centre, where the camera turned
    /// the clicked point, so it is still over what was clicked.
    Focus,
    /// LLToolCamera::handleMouseUp otherwise (avatar steering, a click with
    /// no point): the cursor comes back where the drag started.
    Drag { origin: (f32, f32) },
}

/// Where the cursor is put when the camera gives it back.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Return {
    /// Left where it is (focus lost: LLViewerWindow::handleFocusLost only
    /// shows it again).
    Stay,
    Center,
    At((f32, f32)),
}

/// What the camera asked of the cursor. `kind` is set as soon as the cursor
/// is hidden, even when the system refused to lock it: what was hidden must
/// always be shown again (#271).
#[derive(Debug, Default)]
pub(super) struct CursorHold {
    kind: Option<HoldKind>,
    /// A give-back asked while the window was out of reach (the frame holds
    /// it while the interface is built): done as soon as the frame gives the
    /// window back, so the cursor is never left hidden and locked.
    pending: Option<bool>,
}

impl CursorHold {
    pub(super) fn held(&self) -> bool {
        self.kind.is_some()
    }

    /// Take the cursor; None when it is already held (a drag that turns into
    /// mouselook keeps its first hold). `focus`: the camera drag began on a
    /// valid focus point.
    pub(super) fn take(&mut self, mouselook: bool, focus: bool, cursor: (f32, f32)) -> Option<HoldKind> {
        // a hold asked after a postponed give-back keeps the cursor
        self.pending = None;
        if self.kind.is_some() {
            return None;
        }
        let kind = if mouselook {
            HoldKind::Mouselook
        } else if focus {
            HoldKind::Focus
        } else {
            HoldKind::Drag { origin: cursor }
        };
        self.kind = Some(kind);
        Some(kind)
    }

    /// Give the cursor back; None when it was not held. `focus_lost`: the
    /// window lost the focus, the cursor stays where the system left it.
    pub(super) fn give_back(&mut self, focus_lost: bool) -> Option<Return> {
        self.pending = None;
        let kind = self.kind.take()?;
        Some(match kind {
            _ if focus_lost => Return::Stay,
            HoldKind::Mouselook | HoldKind::Focus => Return::Center,
            HoldKind::Drag { origin } => Return::At(origin),
        })
    }
}

/// Alt in mouselook (LLToolCompGun::handleHover): held, the gun gives way to
/// the grab tool, the cursor comes back at the centre and moves freely over
/// the windows, the HUDs and the world while the view stays still; let go,
/// the gun takes it again. Like the Legacy Viewer, neither switch happens
/// while the left button is down: a click or a drag ends with the tool that
/// began it.
#[derive(Debug, Default)]
pub(super) struct AltCursor {
    free: bool,
    left: bool,
}

/// What the cursor must do after `AltCursor::update`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum AltSwitch {
    /// LLToolGun::handleDeselect: shown and released at the centre.
    Free,
    /// LLToolGun::handleSelect: hidden and locked at the centre again.
    Take,
}

impl AltCursor {
    /// Alt frees the cursor: the view no longer follows the mouse, a click
    /// lands under the cursor and not at the crosshair.
    pub(super) fn free(&self) -> bool {
        self.free
    }

    /// The left button of the window, pressed or released.
    pub(super) fn left_button(&mut self, pressed: bool) {
        self.left = pressed;
    }

    /// Once a frame. `mouselook`: in mouselook with the window focused (the
    /// cursor is given back otherwise); `alt`: Alt held.
    pub(super) fn update(&mut self, mouselook: bool, alt: bool) -> Option<AltSwitch> {
        if !mouselook {
            // out of mouselook or of the focus, the cursor is already shown;
            // mouselook takes it again with the focus, Alt then frees it
            self.free = false;
            return None;
        }
        if self.left || self.free == alt {
            return None;
        }
        self.free = alt;
        Some(if alt { AltSwitch::Free } else { AltSwitch::Take })
    }
}

impl App {
    /// Once a frame, before mouselook takes a cursor it does not hold: Alt
    /// frees it or gives it back to the gun.
    pub(super) fn mouselook_alt_cursor(&mut self) {
        // (AURORA_DEMO_CAMERA scripts the keys: the window needs no focus)
        let focused = self.focused || self.demo && std::env::var_os("AURORA_DEMO_CAMERA").is_some();
        let mouselook = self.camera.mouselook() && focused && !self.exit_prompt.is_open();
        match self.alt_cursor.update(mouselook, self.alt && !self.alt_gr_alone()) {
            Some(AltSwitch::Free) => self.release_cursor(false),
            Some(AltSwitch::Take) => self.set_mouselook_grab(true),
            None => {}
        }
        // A viewport edit or native resize can move the anchor without a
        // CursorMoved event. Nothing is warped while Alt or another window
        // has the cursor, and an unchanged anchor makes no native request.
        self.recenter_mouselook_cursor();
    }

    /// AltGr alone (right Alt, typed as Ctrl + Alt) is a character key, not
    /// the camera's Alt: typing `@` in mouselook must not free the cursor.
    pub(super) fn alt_gr_alone(&self) -> bool {
        self.down.contains(&super::Input::key(KeyCode::AltRight)) && !self.down.contains(&super::Input::key(KeyCode::AltLeft))
    }
    /// Hide and lock the cursor for mouselook or a camera drag (`on`), or
    /// give it back.
    pub(super) fn set_mouselook_grab(&mut self, on: bool) {
        if on {
            self.hold_cursor(false);
        } else {
            self.release_cursor(false);
        }
    }

    /// Hide and lock the cursor for a camera drag that focused on the clicked
    /// point: it comes back at the centre, over that point.
    pub(super) fn hold_focus_cursor(&mut self) {
        self.hold_cursor(true);
    }

    fn hold_cursor(&mut self, focus: bool) {
        if self.gfx.is_none() || self.clicks.scripted() {
            // AURORA_DEMO_CLICKS: several camera drags a second must not
            // take the real cursor of someone using the PC
            return;
        }
        let mouselook = self.camera.mouselook();
        let Some(kind) = self.cursor_hold.take(mouselook, focus, self.cursor_pos) else {
            return;
        };
        if kind == HoldKind::Mouselook {
            // LLToolGun::handleSelect: the hidden cursor waits at the centre,
            // not over the button that started mouselook
            self.move_cursor_to_center();
        }
        let Some(g) = &self.gfx else {
            return;
        };
        if let Err(e) = g
            .window
            .set_cursor_grab(CursorGrabMode::Locked)
            .or_else(|_| g.window.set_cursor_grab(CursorGrabMode::Confined))
        {
            log::warn!("camera: the cursor could not be locked ({e}), it is only hidden");
        }
        g.window.set_cursor_visible(false);
        // the windows stop seeing it now: a window it was over loses its hover
        if let Some(g) = &mut self.gfx {
            g.egui_state.egui_input_mut().events.push(egui::Event::PointerGone);
        }
    }

    /// The hidden mouselook cursor moved (it can only be confined): back to
    /// the centre, like LLToolGun::handleHover. The view turns from the raw
    /// motion, which the move back does not produce.
    pub(super) fn recenter_mouselook_cursor(&mut self) {
        if !self.focused
            || self.exit_prompt.is_open()
            || !self.camera.mouselook()
            || !self.cursor_hold.held()
            || self.alt_cursor.free()
            || self.gfx.as_ref().is_some_and(|gfx| gfx.renderer.vr_engaged())
        {
            return;
        }
        let Some(center) = self.cursor_capture_center() else {
            return;
        };
        if let Some(target) = recenter_target(
            &self.cursor_hold,
            self.camera.mouselook(),
            self.alt_cursor.free(),
            self.cursor_pos,
            center,
        ) {
            self.move_cursor(target);
        }
    }

    /// Give the cursor back. `focus_lost`: like LLViewerWindow::handleFocusLost,
    /// shown and released where it is; mouselook takes it again when the
    /// window has the focus back.
    pub(super) fn release_cursor(&mut self, focus_lost: bool) {
        if self.gfx.is_none() {
            // the window is the frame's for now: forgetting the hold here
            // left the cursor hidden and locked for good
            if self.cursor_hold.held() {
                self.cursor_hold.pending = Some(focus_lost);
            }
            return;
        }
        let Some(back) = self.cursor_hold.give_back(focus_lost) else {
            return;
        };
        let Some(g) = &self.gfx else {
            return;
        };
        let _ = g.window.set_cursor_grab(CursorGrabMode::None);
        match back {
            Return::Stay => {}
            Return::Center => self.move_cursor_to_center(),
            Return::At((x, y)) => self.move_cursor((x, y)),
        }
        if let Some(g) = &self.gfx {
            g.window.set_cursor_visible(true);
        }
    }

    /// The give-back the frame postponed, once it has the window back.
    pub(super) fn release_postponed_cursor(&mut self) {
        if let Some(focus_lost) = self.cursor_hold.pending {
            self.release_cursor(focus_lost);
        }
    }

    fn move_cursor_to_center(&mut self) {
        if let Some(center) = self.cursor_capture_center() {
            self.move_cursor(center);
        }
    }

    fn cursor_capture_center(&self) -> Option<(f32, f32)> {
        let g = self.gfx.as_ref()?;
        let size = g.window.inner_size();
        if size.width == 0 || size.height == 0 {
            return None;
        }
        let native = (size.width, size.height);
        let vr = g.renderer.vr_engaged();
        let configured = self.effective_viewport_settings().viewport(native);
        let rendered = self.world_viewport_for_size(native, vr);
        Some(capture_center(configured, rendered, self.camera.mouselook(), vr))
    }

    fn move_cursor(&mut self, (x, y): (f32, f32)) {
        let Some(g) = &self.gfx else {
            return;
        };
        let _ = g
            .window
            .set_cursor_position(winit::dpi::PhysicalPosition::new(f64::from(x), f64::from(y)));
        self.cursor_pos = (x, y);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosted_mouselook_center_matches_the_fitted_world_and_restores_after_detach() {
        let settings = crate::world_viewport::ViewportSettings {
            enabled: true,
            restrict_ui: true,
            left: 25.0,
            right: 10.0,
            top: 15.0,
            bottom: 20.0,
        };
        let native = (3200, 2000);
        for (fitted, center) in [(false, (1840.0, 950.0)), (true, (1600.0, 1000.0)), (false, (1840.0, 950.0))] {
            let effective = settings.for_hosted_worker(fitted);
            let configured = effective.viewport(native);
            let rendered = effective.render_viewport(native, true, true, false, false);
            assert_eq!(capture_center(configured, rendered, true, false), center);
        }
    }

    #[test]
    fn desktop_mouselook_anchors_in_the_configured_physical_view_not_a_full_photo_output() {
        let full = aurora_render::WorldViewport::full((3200, 2000));
        let mut settings = crate::world_viewport::ViewportSettings::default();
        settings.half(true);
        for restrict_ui in [false, true] {
            settings.restrict_ui = restrict_ui;
            let view = settings.viewport((3200, 2000));
            assert_eq!(capture_center(view, full, true, false), (2400.0, 1000.0));
            assert_eq!(capture_center(view, full, false, false), (1600.0, 1000.0));
            assert_eq!(capture_center(view, full, true, true), (1600.0, 1000.0));
        }
        // CursorMoved and set_cursor_position both use physical client pixels;
        // neither native DPI nor interface zoom rescales this inset anchor.
        let inset = aurora_render::WorldViewport {
            x: 1000,
            y: 250,
            width: 2000,
            height: 1250,
        };
        assert_eq!(capture_center(inset, full, true, false), (2000.0, 875.0));
    }

    #[test]
    fn captured_cursor_motion_returns_to_the_viewport_without_a_warp_feedback_loop() {
        let full = aurora_render::WorldViewport::full((1600, 1000));
        let view = aurora_render::WorldViewport::from_margins((1600, 1000), [0.5, 0.0, 0.0, 0.0]);
        let center = capture_center(view, full, true, false);
        let mut hold = CursorHold::default();
        hold.take(true, false, (1500.0, 700.0));
        for at in [(800.0, 500.0), (50.0, 100.0), (1599.0, 999.0)] {
            assert_eq!(recenter_target(&hold, true, false, at, center), Some((1200.0, 500.0)));
        }
        assert_eq!(recenter_target(&hold, true, false, center, center), None);
        assert_eq!(recenter_target(&hold, true, false, (1200.4, 500.4), center), None);
        assert_eq!(recenter_target(&hold, true, false, (f32::NAN, 500.0), center), Some(center));
    }

    #[test]
    fn entering_mouselook_during_a_drag_and_changing_the_viewport_updates_the_anchor() {
        let mut hold = CursorHold::default();
        let origin = (1300.0, 600.0);
        hold.take(false, false, origin);
        assert_eq!(hold.take(true, false, origin), None, "retain the original drag's release behavior");
        let right = (1200.0, 500.0);
        assert_eq!(
            recenter_target(&hold, false, false, origin, right),
            None,
            "ordinary camera drags are unchanged"
        );
        assert_eq!(recenter_target(&hold, true, false, origin, right), Some(right));
        let inset = (900.0, 450.0);
        assert_eq!(recenter_target(&hold, true, false, right, inset), Some(inset));
        assert_eq!(hold.give_back(false), Some(Return::At(origin)));
    }

    #[test]
    fn alt_and_focus_release_leave_the_cursor_free_for_workspace_windows() {
        let mut hold = CursorHold::default();
        hold.take(true, false, (1200.0, 500.0));
        let outside = (50.0, 100.0);
        let center = (1200.0, 500.0);
        assert_eq!(recenter_target(&hold, true, true, outside, center), None);
        assert_eq!(hold.give_back(true), Some(Return::Stay));
        assert_eq!(recenter_target(&hold, true, false, outside, center), None);
        hold.take(true, false, outside);
        assert_eq!(recenter_target(&hold, true, false, outside, center), Some(center));
    }

    #[test]
    fn viewport_recentring_does_not_change_relative_or_absolute_camera_sensitivity() {
        let size = (3840, 2160);
        let full = aurora_render::WorldViewport::full(size);
        let right = aurora_render::WorldViewport::from_margins(size, [0.5, 0.0, 0.0, 0.0]);
        let inset = aurora_render::WorldViewport::from_margins(size, [0.25, 0.125, 0.1, 0.2]);
        let mut expected = None;
        for view in [full, right, inset] {
            let mut camera = crate::camera::Camera::default();
            let mut agent = crate::agent::AgentState::default();
            camera.change_to_mouselook(&mut agent, &crate::camera::CameraSettings::default(), false);
            let mut hold = CursorHold::default();
            hold.take(true, false, (0.0, 0.0));
            let center = capture_center(view, full, camera.mouselook(), false);
            let relative = crate::raw_mouse::motion((27.0, -13.0)).expect("relative input");
            assert_eq!(relative, (27.0, -13.0));
            camera.look(relative.0 as f32, relative.1 as f32, 1.25, &mut agent);
            assert_eq!(recenter_target(&hold, true, false, (0.0, 0.0), center), Some(center));
            assert_eq!(recenter_target(&hold, true, false, center, center), None);
            // Remote/absolute devices still measure the complete virtual
            // desktop, never the smaller rendered viewport.
            let mut absolute = crate::raw_mouse::Absolute::default();
            let packet = |x, y| crate::raw_mouse::Packet {
                device: 7,
                flags: 0x03,
                x,
                y,
            };
            absolute.packet(packet(32768, 32768), |_| (3840.0, 2160.0));
            let delta = absolute
                .packet(packet(32896, 32704), |_| (3840.0, 2160.0))
                .expect("absolute device input");
            camera.look(delta.0 as f32, delta.1 as f32, 1.25, &mut agent);
            let aim = (agent.yaw, agent.pitch);
            assert_ne!(aim, (0.0, 0.0));
            if let Some(expected) = expected {
                assert_eq!(aim, expected, "viewport geometry must not clamp or scale device motion");
            } else {
                expected = Some(aim);
            }
        }
    }

    #[test]
    fn mouselook_gives_the_cursor_back_at_the_centre() {
        let mut hold = CursorHold::default();
        assert_eq!(hold.take(true, false, (10.0, 900.0)), Some(HoldKind::Mouselook));
        assert!(hold.held());
        assert_eq!(hold.give_back(false), Some(Return::Center));
        assert!(!hold.held());
        assert_eq!(hold.give_back(false), None);
    }

    #[test]
    fn a_drag_gives_the_cursor_back_where_it_started() {
        let mut hold = CursorHold::default();
        assert_eq!(hold.take(false, false, (40.0, 50.0)), Some(HoldKind::Drag { origin: (40.0, 50.0) }));
        assert_eq!(hold.give_back(false), Some(Return::At((40.0, 50.0))));
    }

    #[test]
    fn a_focus_drag_gives_the_cursor_back_at_the_centre() {
        // the camera turned the clicked point to the centre: the cursor
        // reappears over it, not over what is now at the press position
        let mut hold = CursorHold::default();
        assert_eq!(hold.take(false, true, (40.0, 50.0)), Some(HoldKind::Focus));
        assert_eq!(hold.give_back(false), Some(Return::Center));
        // losing the focus still leaves it where it is
        hold.take(false, true, (40.0, 50.0));
        assert_eq!(hold.give_back(true), Some(Return::Stay));
    }

    #[test]
    fn a_second_take_keeps_the_first_hold() {
        let mut hold = CursorHold::default();
        hold.take(false, false, (40.0, 50.0));
        assert_eq!(hold.take(true, false, (0.0, 0.0)), None);
        assert_eq!(hold.give_back(false), Some(Return::At((40.0, 50.0))));
    }

    #[test]
    fn losing_the_focus_shows_the_cursor_where_it_is() {
        let mut hold = CursorHold::default();
        hold.take(true, false, (0.0, 0.0));
        assert_eq!(hold.give_back(true), Some(Return::Stay));
        // mouselook takes it again once the window has the focus back
        assert_eq!(hold.take(true, false, (0.0, 0.0)), Some(HoldKind::Mouselook));
    }

    #[test]
    fn alt_frees_the_mouselook_cursor_while_held() {
        let mut alt = AltCursor::default();
        assert_eq!(alt.update(true, false), None);
        assert_eq!(alt.update(true, true), Some(AltSwitch::Free));
        assert!(alt.free());
        // once a frame: nothing more while Alt stays held
        assert_eq!(alt.update(true, true), None);
        assert_eq!(alt.update(true, false), Some(AltSwitch::Take));
        assert!(!alt.free());
        // Alt outside mouselook is the camera's, not this
        assert_eq!(alt.update(false, true), None);
        assert!(!alt.free());
    }

    #[test]
    fn the_left_button_down_keeps_the_tool_it_began_with() {
        let mut alt = AltCursor::default();
        // a mouselook click held, then Alt: the gun keeps the cursor
        alt.left_button(true);
        assert_eq!(alt.update(true, true), None);
        alt.left_button(false);
        assert_eq!(alt.update(true, true), Some(AltSwitch::Free));
        // a drag begun with the free cursor, then Alt let go: free until released
        alt.left_button(true);
        assert_eq!(alt.update(true, false), None);
        assert!(alt.free());
        alt.left_button(false);
        assert_eq!(alt.update(true, false), Some(AltSwitch::Take));
    }

    #[test]
    fn leaving_mouselook_or_the_focus_ends_the_free_cursor_without_taking_it() {
        let mut alt = AltCursor::default();
        assert_eq!(alt.update(true, true), Some(AltSwitch::Free));
        // (Alt+Tab: the focus goes with Alt still down)
        assert_eq!(alt.update(false, true), None);
        assert!(!alt.free());
        // back with Alt still held: freed again once mouselook took it
        assert_eq!(alt.update(true, true), Some(AltSwitch::Free));
    }

    #[test]
    fn a_postponed_give_back_is_kept_until_done_and_cancelled_by_a_new_hold() {
        let mut hold = CursorHold::default();
        assert!(hold.take(false, false, (10.0, 20.0)).is_some());
        // asked while the window was busy: still held, give-back pending
        hold.pending = Some(false);
        assert!(hold.held());
        assert_eq!(hold.give_back(false), Some(Return::At((10.0, 20.0))));
        assert_eq!(hold.pending, None);
        assert!(!hold.held());
        // a new hold before the frame gave the window back drops the pending give-back
        hold.pending = Some(true);
        assert!(hold.take(true, false, (0.0, 0.0)).is_some());
        assert_eq!(hold.pending, None);
    }
}
