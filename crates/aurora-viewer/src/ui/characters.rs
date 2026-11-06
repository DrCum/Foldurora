//! Saved characters and their exact live viewer windows.
use super::widgets::{self, ActionKind, Floater};
use crate::characters::{Notice, Phase, Presence, Snapshot, account_id};
use crate::login_accounts::Account;
use crate::theme::Palette;
use egui::{RichText, TextEdit};

pub enum Action {
    Open(String),
    Focus(Presence),
}

#[derive(Default)]
pub struct CharactersUi {
    pub open: bool,
    pub demo_opened: bool,
    search: String,
    pub notice: Option<Notice>,
}

impl CharactersUi {
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        accounts: &[Account],
        grids: &[crate::grids::Grid],
        snapshot: &Snapshot,
        own: uuid::Uuid,
        demo: bool,
    ) -> Option<Action> {
        let mut action = None;
        Floater::new(
            "characters",
            aurora_i18n::tr!("character-switch-title"),
            egui::pos2(100.0, 130.0),
            egui::vec2(570.0, 470.0),
        )
        .show(ctx, p, &mut self.open, |ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
            ui.label(aurora_i18n::tr!("character-switch-description"));
            ui.label(
                RichText::new(aurora_i18n::tr!("character-switch-shared-settings"))
                    .size(11.0)
                    .color(p.muted),
            );
            if demo {
                ui.label(RichText::new(aurora_i18n::tr!("character-switch-demo")).color(p.violet_pale));
            }
            ui.add_space(8.0);
            ui.add(
                TextEdit::singleline(&mut self.search)
                    .hint_text(aurora_i18n::tr!("character-switch-search"))
                    .desired_width(f32::INFINITY),
            );
            ui.add_space(6.0);
            let search = self.search.trim().to_lowercase();
            let mut shown = false;
            egui::ScrollArea::vertical()
                .id_salt("character_list")
                .max_height(270.0)
                .show(ui, |ui| {
                    let switch = aurora_i18n::tr!("character-switch-switch");
                    let open_login = aurora_i18n::tr!("character-switch-open-login");
                    let button_width = widgets::action_button_fit(ui, "switch-horizontal", switch)
                        .max(widgets::action_button_fit(ui, "external-link", open_login))
                        .ceil()
                        .clamp(128.0, 220.0)
                        .min(ui.available_width());
                    for account in accounts.iter().take(64).filter(|account| matches(account, grids, &search)) {
                        shown = true;
                        let id = account_id(account);
                        let peer = snapshot.peers.iter().find(|peer| peer.account == id);
                        let current = peer.is_some_and(|peer| peer.instance == own);
                        let opening = peer.is_none() && snapshot.opening.contains(&id);
                        let other_grid = peer.is_none() && crate::characters::other_grid_open(account, accounts, snapshot);
                        ui.push_id(&id, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                widgets::glyph(ui, "user-filled", p.violet_pale, 21.0);
                                // A wrapping row's available_width includes a fresh row;
                                // budget only the space remaining after this glyph.
                                let details_width =
                                    (ui.available_size_before_wrap().x - button_width - ui.spacing().item_spacing.x - 1.0).max(100.0);
                                ui.vertical(|ui| {
                                    // Reserve the action column before laying out long names.
                                    // A narrow floater wraps the action onto the next line.
                                    ui.set_width(details_width);
                                    ui.add(egui::Label::new(RichText::new(&account.username).strong().color(p.ink)).truncate())
                                        .on_hover_text(&account.username);
                                    let grid = grid_label(account, grids);
                                    ui.add(egui::Label::new(RichText::new(&grid).size(11.0).color(p.muted)).truncate())
                                        .on_hover_text(grid);
                                    let status = if current {
                                        aurora_i18n::tr!("character-switch-current")
                                    } else if opening {
                                        aurora_i18n::tr!("character-switch-opening")
                                    } else if other_grid {
                                        aurora_i18n::tr!("character-switch-other-grid")
                                    } else {
                                        match peer.map(|peer| peer.phase) {
                                            Some(Phase::Online) => aurora_i18n::tr!("character-switch-online"),
                                            Some(Phase::Connecting) => aurora_i18n::tr!("character-switch-connecting"),
                                            Some(Phase::Login) => aurora_i18n::tr!("character-switch-login-open"),
                                            None => aurora_i18n::tr!("character-switch-offline"),
                                        }
                                    };
                                    ui.label(
                                        RichText::new(status)
                                            .size(11.0)
                                            .color(if current { p.violet_pale } else { p.muted }),
                                    );
                                });
                                ui.allocate_ui_with_layout(
                                    egui::vec2(button_width, 30.0),
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        let response = ui
                                            .add_enabled_ui(!current && !opening && !other_grid, |ui| {
                                                widgets::action_button(
                                                    ui,
                                                    p,
                                                    if peer.is_some() { "switch-horizontal" } else { "external-link" },
                                                    if peer.is_some() { switch } else { open_login },
                                                    ActionKind::Flat,
                                                    egui::vec2(button_width, 30.0),
                                                )
                                            })
                                            .inner;
                                        #[cfg(test)]
                                        ctx.data_mut(|data| data.insert_temp(egui::Id::unique(("character_action", &id)), response.rect));
                                        if response.clicked() {
                                            action = Some(match peer {
                                                Some(peer) => Action::Focus(peer.clone()),
                                                None => Action::Open(id.clone()),
                                            });
                                        }
                                    },
                                );
                            });
                            ui.add_space(10.0);
                        });
                    }
                    if !shown {
                        ui.label(if accounts.is_empty() {
                            aurora_i18n::tr!("character-switch-empty")
                        } else {
                            aurora_i18n::tr!("character-switch-no-results")
                        });
                    }
                });
            if accounts.len() > 64 || snapshot.catalogue_count > 64 {
                ui.label(RichText::new(aurora_i18n::tr!("character-switch-limit")).size(11.0).color(p.muted));
            }
            if let Some(notice) = &self.notice {
                ui.add_space(8.0);
                ui.label(
                    RichText::new(match notice {
                        Notice::Opening => aurora_i18n::tr!("character-switch-launch-requested"),
                        Notice::FocusRequested => aurora_i18n::tr!("character-switch-focus-requested"),
                        Notice::Busy => aurora_i18n::tr!("character-switch-busy"),
                        Notice::Failed => aurora_i18n::tr!("character-switch-failed"),
                        Notice::Demo => aurora_i18n::tr!("character-switch-demo-action"),
                        Notice::OtherGrid => aurora_i18n::tr!("character-switch-other-grid"),
                    })
                    .color(if matches!(notice, Notice::Failed | Notice::OtherGrid) {
                        p.danger
                    } else {
                        p.muted
                    }),
                );
            }
        });
        action
    }
}

fn grid_label(account: &Account, grids: &[crate::grids::Grid]) -> String {
    match crate::login_accounts::grid(&account.uri) {
        crate::settings::GridChoice::SecondLife => "Second Life".into(),
        crate::settings::GridChoice::SecondLifeBeta => aurora_i18n::tr!("character-switch-beta-grid").into(),
        crate::settings::GridChoice::Custom => {
            if let Some(grid) = grids.iter().find(|grid| grid.login.trim() == account.uri.trim())
                && !grid.name.trim().is_empty()
            {
                return grid.name.clone();
            }
            // Never display URL userinfo, queries, paths or fragments. A short
            // catalogue ID distinguishes two unmapped endpoints on one host.
            let host = reqwest::Url::parse(&account.uri).ok().and_then(|url| {
                url.host_str().map(|host| match url.port() {
                    Some(port) => format!("{host}:{port}"),
                    None => host.to_owned(),
                })
            });
            let id = account_id(account);
            format!(
                "{} · {}",
                host.as_deref().unwrap_or(aurora_i18n::tr!("character-switch-custom-grid")),
                &id[..8]
            )
        }
    }
}

fn matches(account: &Account, grids: &[crate::grids::Grid], search: &str) -> bool {
    search.is_empty() || account.username.to_lowercase().contains(search) || grid_label(account, grids).to_lowercase().contains(search)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn accounts() -> Vec<Account> {
        (0..3)
            .map(|index| Account {
                username: format!("synthetic-{index}"),
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

    fn frame(
        ctx: &egui::Context,
        panel: &mut CharactersUi,
        accounts: &[Account],
        snapshot: &Snapshot,
        own: uuid::Uuid,
        events: Vec<egui::Event>,
    ) -> Option<Action> {
        let mut action = None;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 800.0))),
                events,
                ..Default::default()
            },
            |ui| {
                action = panel.show(
                    ui.ctx(),
                    &crate::theme::Theme::default().palette(),
                    accounts,
                    &[],
                    snapshot,
                    own,
                    true,
                )
            },
        );
        output.textures_delta.clear();
        action
    }

    fn click(
        ctx: &egui::Context,
        panel: &mut CharactersUi,
        accounts: &[Account],
        snapshot: &Snapshot,
        own: uuid::Uuid,
        id: &str,
    ) -> Option<Action> {
        for _ in 0..4 {
            frame(ctx, panel, accounts, snapshot, own, vec![]);
        }
        let rect = ctx
            .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("character_action", id))))
            .unwrap();
        let pos = rect.center();
        for pressed in [true, false] {
            let result = frame(
                ctx,
                panel,
                accounts,
                snapshot,
                own,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            if result.is_some() {
                return result;
            }
        }
        None
    }

    #[test]
    fn row_actions_bind_to_exact_identity_and_current_or_opening_rows_are_disabled() {
        let accounts = accounts();
        let own = uuid::Uuid::new_v4();
        let mut snapshot = crate::characters::demo_snapshot(&accounts, own);
        let ctx = egui::Context::default();
        let mut panel = CharactersUi {
            open: true,
            ..Default::default()
        };
        assert!(click(&ctx, &mut panel, &accounts, &snapshot, own, &account_id(&accounts[0])).is_none());
        let action = click(&ctx, &mut panel, &accounts, &snapshot, own, &account_id(&accounts[1]));
        assert!(matches!(action, Some(Action::Focus(peer)) if peer.account == account_id(&accounts[1]) && peer.instance != own));
        let action = click(&ctx, &mut panel, &accounts, &snapshot, own, &account_id(&accounts[2]));
        assert!(matches!(action, Some(Action::Open(id)) if id == account_id(&accounts[2])));
        snapshot.opening.insert(account_id(&accounts[2]));
        assert!(click(&ctx, &mut panel, &accounts, &snapshot, own, &account_id(&accounts[2])).is_none());
    }

    #[test]
    fn custom_grid_labels_are_distinct_searchable_and_do_not_show_url_secrets() {
        let mut account = accounts().remove(0);
        account.uri = "https://synthetic-user:synthetic-secret@grid.invalid:8002/private/path?token=synthetic-token".into();
        let label = grid_label(&account, &[]);
        assert!(label.starts_with("grid.invalid:8002"));
        for private in ["synthetic-user", "synthetic-secret", "private", "synthetic-token"] {
            assert!(!label.contains(private));
        }
        let mut other = account.clone();
        other.uri = "https://grid.invalid:8002/another".into();
        assert_ne!(label, grid_label(&other, &[]));
        let grids = vec![crate::grids::Grid {
            name: "Synthetic Grid".into(),
            login: other.uri.clone(),
            ..Default::default()
        }];
        assert_eq!(grid_label(&other, &grids), "Synthetic Grid");
        assert!(matches(&other, &grids, "synthetic grid"));
    }

    #[test]
    fn same_avatar_cross_grid_launch_is_disabled_to_protect_shared_account_storage() {
        let mut accounts = accounts();
        let avatar = uuid::Uuid::from_u128(42);
        accounts[0].agent = Some(avatar);
        accounts[2].agent = Some(avatar);
        accounts[2].uri = aurora_net::login::SL_BETA_GRID.into();
        let own = uuid::Uuid::new_v4();
        let snapshot = crate::characters::demo_snapshot(&accounts, own);
        assert!(crate::characters::other_grid_open(&accounts[2], &accounts, &snapshot));
        let ctx = egui::Context::default();
        let mut panel = CharactersUi {
            open: true,
            ..Default::default()
        };
        assert!(click(&ctx, &mut panel, &accounts, &snapshot, own, &account_id(&accounts[2])).is_none());
        accounts[2].agent = Some(uuid::Uuid::from_u128(43));
        assert!(!crate::characters::other_grid_open(&accounts[2], &accounts, &snapshot));
    }
}
