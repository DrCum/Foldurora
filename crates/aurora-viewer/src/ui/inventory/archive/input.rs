//! Main-window input history owned by App, independent of UI/source resets.
//! Logical gesture cancellation never clears physical Enter/Space fences.
use winit::{
    event::{ElementState, WindowEvent},
    keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey},
};

#[derive(Clone, Copy)]
pub(crate) struct InputRead {
    epoch: u64,
    plain: bool,
    rejected: [bool; 2],
}
impl Default for InputRead {
    fn default() -> Self {
        NativeInput::default().read()
    }
}
impl InputRead {
    pub(super) fn epoch(self) -> u64 {
        self.epoch
    }
    pub(super) fn plain(self) -> bool {
        self.plain
    }
    pub(super) fn key_allowed(self, key: egui::Key) -> bool {
        self.plain
            && match key {
                egui::Key::Enter => !self.rejected[0],
                egui::Key::Space => !self.rejected[1],
                _ => false,
            }
    }
}

pub(crate) struct NativeInput {
    enter: crate::session_host::EnterRelease,
    space: crate::session_host::EnterRelease,
    focused: bool,
    modifiers: ModifiersState,
    alt_graph: bool,
    alt_graph_before_focus: bool,
    transition: bool,
    rejected: [bool; 2],
    epoch: Option<u64>,
}
impl Default for NativeInput {
    fn default() -> Self {
        // Same initial native-main-window convention as the established EFB
        // consumer. Real focus snapshots still enter both physical guards.
        let mut enter = crate::session_host::EnterRelease::default();
        enter.focused(true);
        let mut space = crate::session_host::EnterRelease::default();
        space.focused(true);
        Self {
            enter,
            space,
            focused: true,
            modifiers: ModifiersState::empty(),
            alt_graph: false,
            alt_graph_before_focus: false,
            transition: false,
            rejected: [false; 2],
            epoch: Some(1),
        }
    }
}
impl NativeInput {
    #[cfg(test)]
    pub(super) fn exhaust_epoch(&mut self) {
        self.epoch = Some(u64::MAX);
    }
    fn boundary(&mut self) {
        self.epoch = self.epoch.and_then(|epoch| epoch.checked_add(1));
        self.transition = true;
    }
    pub(crate) fn read(&self) -> InputRead {
        InputRead {
            epoch: self.epoch.unwrap_or(0),
            plain: self.epoch.is_some() && self.focused && self.modifiers.is_empty() && !self.alt_graph && !self.transition,
            rejected: self.rejected,
        }
    }
    /// Only after the whole UI batch and final effect check. Neither begin,
    /// source/owner reset, modifier cancellation nor layout retries calls this.
    pub(crate) fn finish(&mut self) {
        self.rejected = [false; 2];
        self.transition = false;
    }
    pub(crate) fn event(&mut self, event: &WindowEvent) -> bool {
        match event {
            WindowEvent::Focused(focused) => {
                let changed = self.focused != *focused;
                if changed {
                    if *focused {
                        self.alt_graph = self.alt_graph_before_focus;
                    }
                    self.alt_graph_before_focus = false;
                    self.focused = *focused;
                    self.boundary();
                }
                self.enter.focused(*focused);
                self.space.focused(*focused);
                changed || !*focused
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers.state();
                // Even a returning-empty observation retires all earlier proof;
                // final plain flags alone cannot erase an intervening Alt ABA.
                self.boundary();
                true
            }
            WindowEvent::KeyboardInput { event, is_synthetic, .. } => {
                self.key(&event.logical_key, event.physical_key, event.state, event.repeat, *is_synthetic)
            }
            _ => false,
        }
    }
    pub(crate) fn key(&mut self, logical: &Key, physical: PhysicalKey, state: ElementState, repeat: bool, synthetic: bool) -> bool {
        let alt_right = physical == PhysicalKey::Code(KeyCode::AltRight);
        let alt_graph = *logical == Key::Named(NamedKey::AltGraph);
        let modifier = alt_graph
            || alt_right
            || matches!(
                logical,
                Key::Named(NamedKey::Alt | NamedKey::Control | NamedKey::Shift | NamedKey::Super | NamedKey::Meta)
            );
        let mut invalid = modifier || repeat || synthetic;
        if alt_graph || alt_right {
            if state == ElementState::Pressed && alt_graph {
                self.alt_graph = true;
                if !self.focused {
                    self.alt_graph_before_focus = true;
                }
            } else if state == ElementState::Released && !synthetic {
                self.alt_graph = false;
                self.alt_graph_before_focus = false;
            }
        }
        if invalid {
            self.boundary();
        }
        let (guard, index) = match logical {
            Key::Named(NamedKey::Enter) => (&mut self.enter, 0),
            Key::Named(NamedKey::Space) => (&mut self.space, 1),
            Key::Character(value) if value.as_str() == " " => (&mut self.space, 1),
            _ => return invalid,
        };
        if guard.suppress(&Key::Named(NamedKey::Enter), state, repeat, synthetic, self.modifiers) {
            self.rejected[index] = true;
            if !invalid {
                self.boundary();
            }
            invalid = true;
        }
        invalid
    }
}
