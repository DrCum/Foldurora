//! Enter admission before egui-winit discards native focus-snapshot presses.

use winit::event::ElementState;
use winit::keyboard::{Key, ModifiersState};

#[derive(Default)]
pub(in crate::session_host) struct EnterRelease {
    held: bool,
    focused: bool,
    before_focus: bool,
}

impl EnterRelease {
    pub fn focused(&mut self, focused: bool) {
        if self.focused == focused {
            return;
        }
        self.focused = focused;
        if focused {
            // Windows sends held-key snapshot presses before Focused(true),
            // X11 afterward. No Enter in the snapshot lets a key released
            // outside the window send on its first subsequent fresh press.
            self.held = self.before_focus;
        }
        self.before_focus = false;
    }

    pub fn suppress(&mut self, key: &Key, state: ElementState, repeat: bool, synthetic: bool, modifiers: ModifiersState) -> bool {
        if *key != Key::Named(winit::keyboard::NamedKey::Enter) {
            return false;
        }
        if state == ElementState::Released {
            if !synthetic {
                self.held = false;
                self.before_focus = false;
            }
            // A focus-loss snapshot release clears egui's key set, not the
            // physical Enter. Only real release or the next focus snapshot
            // may rearm this logical Enter-family fence.
            return false;
        }
        let held = std::mem::replace(&mut self.held, true);
        if !self.focused {
            self.before_focus = true;
        }
        if synthetic {
            return false; // egui-winit ignores snapshot presses after we observe them.
        }
        // X11 explicitly resets its repeat tracking on focus loss, so the
        // first real held-key event after refocus has repeat=false (winit
        // xinput_key_input). Shift+Enter retains multiline repeat behavior.
        !self.focused || (!modifiers.shift_key() && held) || super::suppress_enter_repeat(key, state, repeat, modifiers)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::NamedKey;

    fn enter(guard: &mut EnterRelease, pressed: bool, synthetic: bool, repeat: bool, shift: bool) -> bool {
        guard.suppress(
            &Key::Named(NamedKey::Enter),
            if pressed { ElementState::Pressed } else { ElementState::Released },
            repeat,
            synthetic,
            if shift { ModifiersState::SHIFT } else { ModifiersState::empty() },
        )
    }

    #[test]
    fn native_enter_x11_observed_focus_trace_blocks_the_first_real_false_repeat_until_release() {
        let mut guard = EnterRelease::default();
        guard.focused(true);
        assert!(!enter(&mut guard, true, false, false, false)); // observed event 40: modal rejects this fresh press
        assert!(!enter(&mut guard, false, true, false, false)); // 42: synthetic release before focus loss
        guard.focused(false); // 43
        guard.focused(true); // 50
        assert!(!enter(&mut guard, true, true, false, false)); // 51: egui-winit discards this snapshot press
        assert!(
            enter(&mut guard, true, false, false, false),
            "59 is still the original held key, despite repeat=false"
        );
        assert!(enter(&mut guard, true, false, true, false)); // 62: later OS repeats
        assert!(!enter(&mut guard, false, false, false, false));
        assert!(
            !enter(&mut guard, true, false, false, false),
            "a physical up/fresh down sends normally"
        );
    }

    #[test]
    fn native_enter_windows_snapshot_before_focus_and_duplicate_focus_keep_the_release_fence() {
        let mut guard = EnterRelease::default();
        guard.focused(true);
        assert!(!enter(&mut guard, true, false, false, false));
        enter(&mut guard, false, true, false, false);
        guard.focused(false);
        enter(&mut guard, true, true, false, false); // WM_SETFOCUS keyboard callback precedes Focused(true)
        guard.focused(true);
        guard.focused(true);
        assert!(enter(&mut guard, true, false, false, false));
        enter(&mut guard, false, true, false, false);
        assert!(
            enter(&mut guard, true, false, false, false),
            "synthetic up is never physical release proof"
        );
    }

    #[test]
    fn native_enter_released_outside_focus_allows_the_first_fresh_press_without_double_enter() {
        let mut guard = EnterRelease::default();
        guard.focused(true);
        enter(&mut guard, true, false, false, false);
        enter(&mut guard, false, true, false, false);
        guard.focused(false);
        // The physical release occurs elsewhere: focus returns without an Enter snapshot press.
        guard.focused(true);
        assert!(!enter(&mut guard, true, false, false, false));
        assert!(enter(&mut guard, true, false, false, false), "another down without up remains held");
    }

    #[test]
    fn native_enter_real_release_while_unfocused_clears_a_pre_focus_snapshot() {
        let mut guard = EnterRelease::default();
        enter(&mut guard, true, true, false, false);
        enter(&mut guard, false, false, false, false);
        guard.focused(true);
        assert!(!enter(&mut guard, true, false, false, false));
        assert!(enter(&mut guard, true, false, false, false));
    }

    #[test]
    fn native_enter_new_window_held_snapshot_and_unfocused_real_press_are_fenced() {
        let mut before = EnterRelease::default();
        enter(&mut before, true, true, false, false); // Windows: held before the new window receives focus
        before.focused(true);
        assert!(enter(&mut before, true, false, false, false));
        let mut after = EnterRelease::default();
        after.focused(true); // X11: focus then held-key snapshot
        enter(&mut after, true, true, false, false);
        assert!(enter(&mut after, true, false, false, false));
        let mut unfocused = EnterRelease::default();
        assert!(enter(&mut unfocused, true, false, false, false));
        unfocused.focused(true);
        assert!(enter(&mut unfocused, true, false, false, false));
    }

    #[test]
    fn native_enter_shift_newlines_continue_but_releasing_shift_cannot_send_a_held_enter() {
        let mut guard = EnterRelease::default();
        guard.focused(true);
        assert!(!enter(&mut guard, true, false, false, true));
        assert!(!enter(&mut guard, true, false, true, true));
        assert!(enter(&mut guard, true, false, false, false));
        enter(&mut guard, false, false, false, false);
        assert!(!guard.suppress(
            &Key::Named(NamedKey::Enter),
            ElementState::Pressed,
            false,
            false,
            ModifiersState::CONTROL
        ));
        assert!(guard.suppress(
            &Key::Named(NamedKey::Enter),
            ElementState::Pressed,
            true,
            false,
            ModifiersState::CONTROL
        ));
    }

    #[test]
    fn native_enter_navigation_ime_completion_and_other_keys_do_not_rearm_a_held_send() {
        let mut guard = EnterRelease::default();
        guard.focused(true);
        enter(&mut guard, true, false, false, false); // IME/navigation may reject it after native admission
        for key in [Key::Named(NamedKey::Tab), Key::Named(NamedKey::Escape), Key::Character("a".into())] {
            for state in [ElementState::Pressed, ElementState::Released] {
                assert!(!guard.suppress(&key, state, false, false, ModifiersState::empty()));
            }
        }
        // IME/target changes do not call focused() or supply a physical Enter release.
        assert!(enter(&mut guard, true, false, false, false));
        enter(&mut guard, false, false, false, false);
        assert!(!enter(&mut guard, true, false, false, false));
    }

    #[test]
    fn native_enter_both_focus_snapshot_orders_keep_shift_release_from_sending() {
        for snapshot_before_focus in [false, true] {
            let mut guard = EnterRelease::default();
            guard.focused(true);
            assert!(!enter(&mut guard, true, false, false, true));
            enter(&mut guard, false, true, false, true);
            guard.focused(false);
            if snapshot_before_focus {
                enter(&mut guard, true, true, false, false);
            }
            guard.focused(true);
            if !snapshot_before_focus {
                enter(&mut guard, true, true, false, false);
            }
            assert!(!enter(&mut guard, true, false, false, true), "held Shift+Enter remains multiline");
            assert!(
                enter(&mut guard, true, false, false, false),
                "dropping Shift must not turn the held key into Send"
            );
            enter(&mut guard, false, false, false, false);
            assert!(!enter(&mut guard, true, false, false, false));
        }
    }
}
