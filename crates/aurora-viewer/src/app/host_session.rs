//! Apply host commands only to this process's world and native window.

use super::{App, Screen, TpOverlay};
use aurora_session::{AccountKey, Label, Phase, WorkerStatus};
use std::time::{Duration, Instant};

impl App {
    pub(super) fn tick_host_session(&mut self) {
        let Some(mut bridge) = self.host_session.take() else {
            return;
        };
        let phase = match self.screen {
            Screen::World => Phase::Online,
            Screen::Loading { .. } => Phase::Connecting,
            Screen::Login if self.login_form.busy => Phase::Connecting,
            Screen::Login => Phase::Login,
        };
        let uri = self.settings.login_uri();
        let account = (phase == Phase::Online)
            .then(|| {
                let account = AccountKey::parse_hex(&crate::characters::identity(&uri, &self.settings.username)).ok()?;
                let grid = crate::session_host::transport::grid_key(&uri).ok()?;
                Some((account, grid))
            })
            .flatten();
        let status = WorkerStatus {
            phase,
            name: label(&self.settings.username),
            grid_label: label(&grid_label(&uri, &self.settings.grids)),
            native_window: self
                .gfx
                .as_ref()
                .map_or(0, |gfx| crate::session_host::worker::native_window(&gfx.window)),
            keyboard_focused: self.focused,
            blocked: phase != Phase::Online
                || self.exit_prompt.open
                || self.login_form.busy
                || self.options_ui.capture.is_some()
                || self.gfx.is_none(),
            last_frame: None,
        };
        let effects = bridge.tick(status, account);
        if effects.release_input {
            self.release_host_input();
        }
        self.pump_host_display(&mut bridge, effects);
        self.host_session = Some(bridge);
        // The bridge is restored before policy observation: take() above is
        // a temporary borrow, never a source disappearance.
        self.tick_host_preview();
    }

    fn pump_host_display(&mut self, bridge: &mut crate::session_host::worker::WorkerBridge, effects: aurora_session::WorkerEffects) {
        let release = self
            .gfx
            .as_mut()
            .is_some_and(|gfx| bridge.pump_display(effects, &gfx.window, &mut gfx.renderer));
        if release {
            self.release_host_input();
            if let Some(gfx) = &mut self.gfx {
                bridge.pump_display(Default::default(), &gfx.window, &mut gfx.renderer);
            }
        }
    }

    pub(super) fn release_host_input(&mut self) {
        self.host_preview.capture_barrier();
        self.down.clear();
        self.input = Default::default();
        self.right_drag = false;
        self.mic_button_held = false;
        self.drop_clicks();
        self.on_left_release();
        self.release_cursor(true);
        self.alt_cursor.left_button(false);
        self.numeric_drag.cancel();
        self.chat_ui.cancel_gesture_key();
        self.gesture_state.manager.release(None);
        self.alt_text.clear();
        // Native events queued before revocation must not become a delayed
        // Enter/click on this account when its first prepared UI pass resumes.
        if let Some(gfx) = &mut self.gfx {
            let _ = gfx.egui_state.take_egui_input(&gfx.window);
        }
        self.egui_ctx.input_mut(|input| {
            input.keys_down.clear();
            input.events.clear();
            input.pointer = Default::default();
            input.modifiers = Default::default();
        });
        self.egui_ctx.stop_dragging();
        self.mic_on = false;
        self.voice.set_transmit(false);
        self.controls_held = Default::default();
        self.controls_free = Default::default();
        self.joystick.motion = Default::default();
        self.joystick.running = false;
        self.options_ui.cancel_joystick_capture();
        self.alt_codes.reset();
        self.alt = false;
        self.ctrl = false;
        self.shift = false;
        self.command = false;
        self.alt_graph = false;
        if self.in_world() {
            let mut controls = *self.net.controls.lock();
            controls.control_flags = release_held_controls(controls.control_flags);
            self.net.set_controls(controls);
        }
        if self.temp_run.take().is_some() && !self.always_run {
            self.send(aurora_net::NetCommand::SetAlwaysRun(false));
        }
    }

    /// CPU readiness advances even when the managed renderer is parked.
    /// This grants no prepared-frame or input authority.
    pub(super) fn advance_loading_screen(&mut self) {
        if let Screen::Loading { since } = self.screen {
            let moved = self.move_complete_at.map(|t| t.elapsed().as_secs_f32()).unwrap_or(0.0);
            let terrain = self.world.terrain_progress();
            if loading_ready(self.world.movement_complete, since.elapsed(), terrain, moved) {
                self.screen = Screen::World;
                self.world.notifications.started = true;
                self.camera.snap();
                // the loading screen fades out over the world
                let mut o = TpOverlay::new(String::new(), true);
                o.start = since;
                o.shown = 1.0;
                o.end = Some(Instant::now());
                self.tp_overlay = Some(o);
                self.was_teleporting = self.world.teleporting;
            }
        }
    }

    pub(super) fn host_input_allowed(&self) -> bool {
        self.host_session.as_ref().is_none_or(|bridge| bridge.input_allowed())
    }

    pub(super) fn host_world_allowed(&self) -> bool {
        self.host_session.as_ref().is_none_or(|bridge| bridge.world_allowed())
    }

    pub(super) fn host_submission_allowed(&self) -> bool {
        self.host_session.as_ref().is_none_or(|bridge| bridge.submission_allowed())
    }

    pub(super) fn host_render_priority(&self) -> bool {
        self.host_session
            .as_ref()
            .map_or(self.focused, |bridge| bridge.render_priority(self.focused))
    }

    pub(super) fn host_fitted(&self) -> bool {
        self.host_session.as_ref().is_some_and(|bridge| bridge.hosted())
    }

    pub(super) fn invalidate_host_display(&mut self) {
        self.host_preview.retire();
        let Some(mut bridge) = self.host_session.take() else {
            return;
        };
        let effects = bridge.invalidate_display();
        self.release_host_input();
        self.pump_host_display(&mut bridge, effects);
        self.host_session = Some(bridge);
        // The bridge is restored before policy observation: take() above is
        // a temporary borrow, never a source disappearance.
        self.tick_host_preview();
    }

    pub(super) fn host_resized(&mut self, width: u32, height: u32) {
        if self
            .host_session
            .as_ref()
            .is_some_and(|bridge| bridge.hosted() && !bridge.expected_resize(width, height))
            && self.gfx.as_ref().is_some_and(|gfx| gfx.renderer.size() != (width, height))
        {
            self.invalidate_host_display();
        }
    }
}

fn release_held_controls(flags: u32) -> u32 {
    use aurora_net::control::*;
    flags
        & !(AT_POS
            | AT_NEG
            | LEFT_POS
            | LEFT_NEG
            | UP_POS
            | UP_NEG
            | PITCH_POS
            | PITCH_NEG
            | YAW_POS
            | YAW_NEG
            | FAST_AT
            | FAST_LEFT
            | FAST_UP
            | NUDGE_AT_POS
            | NUDGE_AT_NEG
            | NUDGE_LEFT_POS
            | NUDGE_LEFT_NEG
            | NUDGE_UP_POS
            | NUDGE_UP_NEG
            | TURN_LEFT
            | TURN_RIGHT
            | LBUTTON_DOWN
            | ML_LBUTTON_DOWN)
}

fn loading_ready(movement_complete: bool, loading_age: Duration, terrain: f32, movement_age: f32) -> bool {
    movement_complete && loading_age > Duration::from_millis(1500) && (terrain > 0.97 || movement_age > 6.0)
}

pub(crate) fn grid_label(uri: &str, grids: &[crate::grids::Grid]) -> String {
    super::attention::caption_grid(uri, grids)
}

fn label(text: &str) -> Label {
    let mut value = String::new();
    for character in text.chars().filter(|character| !character.is_control()) {
        if value.len() + character.len_utf8() > 128 {
            break;
        }
        value.push(character);
    }
    Label::new(&value).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    #[test]
    fn revocation_releases_held_network_controls_without_changing_persistent_flight_or_away() {
        use aurora_net::control::*;
        let persistent = FLY | AWAY | LBUTTON_UP;
        assert_eq!(
            super::release_held_controls(persistent | AT_POS | FAST_AT | YAW_NEG | NUDGE_UP_POS | LBUTTON_DOWN),
            persistent
        );
    }

    #[test]
    fn loading_can_advance_from_cpu_network_state_without_a_rendered_world() {
        let elapsed = std::time::Duration::from_secs(8);
        assert!(super::loading_ready(true, elapsed, 0.0, 7.0));
        assert!(super::loading_ready(true, elapsed, 1.0, 0.0));
        assert!(!super::loading_ready(false, elapsed, 1.0, 7.0));
        assert!(!super::loading_ready(true, std::time::Duration::from_secs(1), 1.0, 7.0));
    }

    #[test]
    fn local_status_labels_are_bounded_without_splitting_unicode() {
        let label = super::label(&format!("\n{}", "界".repeat(60)));
        assert!(label.as_str().len() <= 128);
        assert!(!label.as_str().chars().any(char::is_control));
    }
}
