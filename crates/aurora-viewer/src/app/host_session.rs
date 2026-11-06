//! Apply host commands only to this process's world and native window.

use super::{App, Screen};
use aurora_session::{AccountKey, Label, Phase, WorkerStatus};

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
            grid_label: label(&uri),
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
            self.down.clear();
            self.input = Default::default();
            self.right_drag = false;
            self.mic_button_held = false;
            self.drop_clicks();
            self.on_left_release();
            self.release_cursor(true);
        }
        if effects.restore_window
            && let Some(gfx) = &self.gfx
        {
            bridge.restore(&gfx.window);
        }
        self.host_session = Some(bridge);
    }
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
    fn local_status_labels_are_bounded_without_splitting_unicode() {
        let label = super::label(&format!("\n{}", "界".repeat(60)));
        assert!(label.as_str().len() <= 128);
        assert!(!label.as_str().chars().any(char::is_control));
    }
}
