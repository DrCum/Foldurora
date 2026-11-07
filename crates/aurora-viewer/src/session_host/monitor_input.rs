//! Native activation-key fences without a second editor input implementation.
use super::chat_window::enter_release::EnterRelease;
use winit::event::ElementState;
use winit::keyboard::{Key, ModifiersState, NamedKey};

#[derive(Default)]
pub(super) struct MonitorKeys {
    enter: EnterRelease,
    space: EnterRelease,
    modifiers: ModifiersState,
}
impl MonitorKeys {
    pub fn focused(&mut self, focused: bool) {
        self.enter.focused(focused);
        self.space.focused(focused);
        if !focused {
            self.modifiers = ModifiersState::empty();
        }
    }
    pub fn modifiers(&mut self, modifiers: ModifiersState) {
        self.modifiers = modifiers;
    }
    /// Both fences observe every edge, including composer Space. Suppression of
    /// Space belongs only to owned command focus; Enter retains Shift+Enter.
    pub fn suppress(&mut self, key: &Key, state: ElementState, repeat: bool, synthetic: bool, command_focused: bool) -> bool {
        match key {
            Key::Named(NamedKey::Enter) => {
                self.enter.suppress(
                    key,
                    state,
                    repeat,
                    synthetic,
                    if command_focused { ModifiersState::empty() } else { self.modifiers },
                ) || (command_focused && synthetic)
            }
            // egui-winit maps both logical spellings to the same Space key.
            key if matches!(key, Key::Named(NamedKey::Space)) || matches!(key, Key::Character(value) if value.as_str() == " ") => {
                let held = self
                    .space
                    .suppress(&Key::Named(NamedKey::Enter), state, repeat, synthetic, ModifiersState::empty());
                command_focused && (held || synthetic)
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn edge(keys: &mut MonitorKeys, key: NamedKey, down: bool, repeat: bool, synthetic: bool, command: bool) -> bool {
        edge_key(keys, &Key::Named(key), down, repeat, synthetic, command)
    }
    fn edge_key(keys: &mut MonitorKeys, key: &Key, down: bool, repeat: bool, synthetic: bool, command: bool) -> bool {
        keys.suppress(
            key,
            if down { ElementState::Pressed } else { ElementState::Released },
            repeat,
            synthetic,
            command,
        )
    }
    #[test]
    fn monitor_native_space_tracks_composer_hold_before_toolbar_refocus_without_eating_text() {
        // egui-winit accepts both representations as egui::Key::Space. Match
        // the actual Windows (before Focused) and X11 (after Focused) snapshots;
        // a missing held-key snapshot correctly means released outside focus.
        for space in [Key::Named(NamedKey::Space), Key::Character(" ".into())] {
            for snapshot_before_focus in [true, false] {
                let mut keys = MonitorKeys::default();
                keys.focused(true);
                assert!(!edge_key(&mut keys, &space, true, false, false, false));
                assert!(
                    !edge_key(&mut keys, &space, true, true, false, false),
                    "composer repeat remains text"
                );
                assert!(!edge_key(&mut keys, &space, false, false, true, false));
                keys.focused(false);
                if snapshot_before_focus {
                    assert!(!edge_key(&mut keys, &space, true, false, true, false));
                }
                keys.focused(true);
                if !snapshot_before_focus {
                    assert!(!edge_key(&mut keys, &space, true, false, true, false));
                }
                assert!(
                    edge_key(&mut keys, &space, true, false, false, true),
                    "first real refocus event can have repeat=false"
                );
                assert!(edge_key(&mut keys, &space, true, true, false, true));
                assert!(edge_key(&mut keys, &space, true, true, false, true));
                assert!(
                    edge_key(&mut keys, &space, false, false, true, true),
                    "synthetic release cannot activate a command"
                );
                assert!(
                    edge_key(&mut keys, &space, true, false, false, true),
                    "synthetic release cannot rearm a held command"
                );
                assert!(!edge_key(&mut keys, &space, false, false, false, true));
                assert!(!edge_key(&mut keys, &space, true, false, false, true));
            }
        }
    }
    #[test]
    fn monitor_native_enter_keeps_multiline_and_independent_space_fence() {
        let mut keys = MonitorKeys::default();
        keys.focused(true);
        keys.modifiers(ModifiersState::SHIFT);
        assert!(!edge(&mut keys, NamedKey::Enter, true, false, false, false));
        assert!(!edge(&mut keys, NamedKey::Enter, true, true, false, false));
        assert!(
            edge(&mut keys, NamedKey::Enter, true, false, false, true),
            "toolbar has no multiline exception"
        );
        assert!(
            !edge(&mut keys, NamedKey::Space, true, false, false, true),
            "Space is independently fresh"
        );
        assert!(!edge(&mut keys, NamedKey::Enter, false, false, false, true));
        assert!(!edge(&mut keys, NamedKey::Enter, true, false, false, true));
    }
}
