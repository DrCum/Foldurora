//! Character switching keeps existing session owners intact. A click either
//! focuses an exact live viewer or opens a manual, preselected login screen.
use super::{App, Screen};
use crate::characters::{Notice, Phase};
use crate::theme::Palette;
use crate::ui;

impl App {
    pub(super) fn tick_characters(&mut self) {
        // The child must finish exact-account selection before it publishes:
        // its inherited last-used profile belongs to another window.
        let ready = crate::cli::get().saved_account.is_none() || self.characters.startup_applied;
        let phase = match self.screen {
            Screen::World => Phase::Online,
            Screen::Loading { .. } => Phase::Connecting,
            Screen::Login if self.login_form.busy => Phase::Connecting,
            Screen::Login => Phase::Login,
        };
        self.characters
            .follow(!self.demo && ready, self.settings.login_uri(), &self.settings.username, phase);
        self.characters.watch(!self.demo && self.characters_ui.open);
        if self.characters.take_focus()
            && let Some(gfx) = &self.gfx
        {
            gfx.window.set_minimized(false);
            gfx.window.focus_window();
            // Window managers that forbid programmatic foreground activation
            // still show that this user-requested character wants attention.
            gfx.window
                .request_user_attention(Some(winit::window::UserAttentionType::Informational));
        }
        if let Some(notice) = self.characters.take_notice() {
            self.characters_ui.notice = Some(notice);
        }
        if self.demo && self.frame_count >= 550 && !self.characters_ui.demo_opened && std::env::var_os("AURORA_DEMO_CHARACTERS").is_some() {
            self.characters_ui.demo_opened = true;
            self.characters_ui.open = true;
        }
    }

    pub(super) fn show_characters(&mut self, ctx: &egui::Context, p: &Palette) {
        if !self.characters_ui.open {
            return;
        }
        let demo_accounts;
        let snapshot = if self.demo {
            crate::characters::demo_snapshot(&synthetic_accounts(), self.characters.own_instance())
        } else {
            self.characters.snapshot()
        };
        let accounts = if self.demo {
            demo_accounts = synthetic_accounts();
            &demo_accounts
        } else {
            snapshot.catalogue.as_deref().unwrap_or(&self.login_form.accounts)
        };
        let own = self.characters.own_instance();
        let action = self
            .characters_ui
            .show(ctx, p, accounts, &self.settings.grids, &snapshot, own, self.demo);
        if let Some(action) = action {
            if self.demo {
                self.characters_ui.notice = Some(Notice::Demo);
                return;
            }
            match action {
                ui::characters::Action::Open(id) => self.characters.open(id),
                ui::characters::Action::Focus(peer) => {
                    // Stop held movement, camera drags and push-to-talk even
                    // if the OS takes a moment to transfer native focus. The
                    // ordinary Focused(false) handler completes that cleanup.
                    self.down.clear();
                    self.input = Default::default();
                    self.right_drag = false;
                    self.mic_button_held = false;
                    self.drop_clicks();
                    self.on_left_release();
                    self.release_cursor(true);
                    self.characters.focus(peer);
                }
            }
        }
    }

    pub(super) fn apply_startup_character(&mut self) {
        if self.characters.startup_applied || !self.login_form.accounts_loaded {
            return;
        }
        self.characters.startup_applied = true;
        if let Some(id) = crate::cli::get().saved_account.as_deref() {
            if !ui::login::select_saved_character(&mut self.settings, &mut self.login_form, id) {
                self.login_form.error = Some(aurora_i18n::tr!("character-switch-missing").into());
            }
        }
    }
}

fn synthetic_accounts() -> Vec<crate::login_accounts::Account> {
    ["Synthetic Aurora", "Synthetic Builder", "Synthetic Explorer"]
        .into_iter()
        .map(|name| crate::login_accounts::Account {
            username: name.into(),
            uri: aurora_net::login::SL_MAIN_GRID.into(),
            remember_password: false,
            remember_mfa: false,
            start_location: "last".into(),
            start_region: String::new(),
            agent: None,
            last_place: None,
        })
        .collect()
}
