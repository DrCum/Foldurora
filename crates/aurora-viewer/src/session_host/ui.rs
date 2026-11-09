//! The controller shows acknowledged local sessions only. Chat is offered
//! only for its authenticated live native backend; logins remain manual.

#[cfg(test)]
mod visual;

use super::slots::{SlotId, SlotLabel, SlotSummary};
use super::transport::{HostError, HostSnapshot, WorkerSummary};
use crate::theme::Palette;
use crate::ui::widgets::{self, ActionKind};
use aurora_session::{AccountKey, MAX_WORKERS, Phase, WorkerId};
use egui::{Id, Rect, RichText, TextEdit, vec2};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Action {
    OpenLogin(Option<AccountKey>, Option<SlotId>),
    RenameSlot(SlotId, SlotLabel),
    CopyPreferences(super::profile_copy_ui::Action),
    Activate(WorkerId),
    Detach(WorkerId),
    OpenChat(aurora_session::chat::ChatSource),
    OpenAttention,
    #[cfg(windows)]
    OpenMonitor(aurora_session::preview_image::PreviewSource),
    #[cfg(windows)]
    OpenCyclingMonitor,
    #[cfg(windows)]
    ResumeCyclingMonitor(Box<super::monitor_manager::Recovery>),
    Close,
}

#[derive(Clone, Copy)]
pub(super) enum Notice {
    Queued,
    Error(HostError),
}

#[derive(Default)]
pub(super) struct HostUi {
    pub(super) copy_preferences: super::profile_copy_ui::CopyUi,
    selected: Option<WorkerId>,
    search: String,
    management_open: bool,
    selected_slot: Option<SlotId>,
    rename: Option<(SlotId, String)>,
    pub close_prompt: bool,
    pub notice: Option<Notice>,
    pub attention_count: usize,
    #[cfg(windows)]
    pub monitor_sources: Vec<(SlotId, aurora_session::preview_image::PreviewSource)>,
    #[cfg(windows)]
    pub monitor_cycle_ready: bool,
    #[cfg(windows)]
    pub monitor_recovery: Option<super::monitor_manager::Recovery>,
}

pub(super) struct Output {
    pub action: Option<Action>,
    pub world: Rect,
}

impl HostUi {
    pub fn modal_open(&self) -> bool {
        self.copy_blocked() || self.copy_preferences.is_open()
    }

    pub fn copy_blocked(&self) -> bool {
        self.close_prompt || self.rename.is_some()
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        snapshot: &HostSnapshot,
        native_ready: bool,
        hosted: Option<WorkerId>,
        closing: bool,
    ) -> Output {
        let mut action = None;
        let ctx = ui.ctx().clone();
        let frame = widgets::overlay_frame(&ctx, p);
        // Port of layoutCharacters (tools/session-host/main.cpp): keep the
        // character controls on one compact row, reducing names to slot numbers
        // before taking space from the world. Management remains an explicit view.
        egui::Panel::top("session_host_header")
            .exact_size(38.0)
            .frame(frame.inner_margin(egui::Margin::symmetric(4, 4)))
            .show(ui, |ui| {
                action = self.strip(ui, p, snapshot, native_ready, hosted, closing);
            });
        if self.management_open {
            let side_width = (ctx.content_rect().width() * 0.3)
                .clamp(260.0, 340.0)
                .min((ctx.content_rect().width() - 120.0).max(120.0));
            egui::Panel::left("session_host_roster")
                .exact_size(side_width)
                .frame(frame)
                .show(ui, |ui| {
                    ui.add(
                        TextEdit::singleline(&mut self.search)
                            .hint_text(aurora_i18n::tr!("host-search"))
                            .char_limit(128),
                    );
                    ui.add_space(8.0);
                    self.slot_management(ui, p, snapshot, closing);
                    if ui
                        .add_enabled(
                            !closing && !snapshot.stopped,
                            egui::Button::new(aurora_i18n::tr!("host-attention-show")),
                        )
                        .clicked()
                    {
                        action = Some(Action::OpenAttention);
                    }
                    let login_slot = self.login_slot(snapshot);
                    ui.add_space(10.0);
                    let query = self.search.trim().to_lowercase();
                    egui::ScrollArea::vertical().id_salt("session_host_rows").show(ui, |ui| {
                        #[cfg(windows)]
                        self.monitor_entries(ui, p, snapshot, closing, &mut action);
                        widgets::organization_header(ui, p, aurora_i18n::tr!("host-running-sessions"));
                        let mut count = 0;
                        for worker in snapshot.workers.iter().take(MAX_WORKERS).filter(|worker| {
                            matches(display_name(worker), display_grid(worker), &query)
                                || worker
                                    .slot
                                    .and_then(|id| snapshot.slots.iter().find(|slot| slot.id == id))
                                    .is_some_and(|slot| slot_name(slot).to_lowercase().contains(&query))
                        }) {
                            count += 1;
                            let id = worker.binding.worker.worker;
                            ui.push_id(binding_id(worker.binding), |ui| {
                                if let Some(slot) = worker.slot.and_then(|id| snapshot.slots.iter().find(|slot| slot.id == id)) {
                                    ui.label(RichText::new(slot_name(slot)).small().color(p.violet_pale));
                                }
                                let label = display_name(worker);
                                let selected = self.selected == Some(id);
                                if widgets::navigation(ui, p, "user-filled", label, selected).clicked() {
                                    self.selected = Some(id);
                                    if let Some(slot) = worker.slot {
                                        self.selected_slot = Some(slot);
                                    }
                                }
                                ui.label(RichText::new(display_grid(worker)).small().color(p.muted));
                                ui.label(
                                    RichText::new(status(worker))
                                        .small()
                                        .color(if worker.reachable { p.muted } else { p.warn }),
                                );
                                if hosted == Some(id) {
                                    ui.label(RichText::new(aurora_i18n::tr!("host-hosted")).small().color(p.success));
                                }
                                if snapshot.pending == Some(id) {
                                    ui.label(RichText::new(aurora_i18n::tr!("host-switching")).small().color(p.violet_pale));
                                }
                                if worker.status.as_ref().is_some_and(|status| status.keyboard_focused) {
                                    ui.label(RichText::new(aurora_i18n::tr!("host-keyboard-focus")).small().color(p.teal));
                                }
                                let source = chat_source(snapshot, worker);
                                // Scope the whole row by its current chat lease. Individual child
                                // scopes hide button widths from the parent's wrapping layout.
                                ui.push_id(("host_actions", source.map(|source| source.lease_epoch)), |ui| {
                                    ui.horizontal_wrapped(|ui| {
                                        let ready = can_activate(snapshot, worker, native_ready, hosted, closing);
                                        let activate = widgets::action_button_if(
                                            ui,
                                            p,
                                            "arrows-exchange",
                                            aurora_i18n::tr!("host-activate"),
                                            ActionKind::Flat,
                                            vec2(0.0, 28.0),
                                            ready,
                                        );
                                        #[cfg(test)]
                                        ctx.data_mut(|data| data.insert_temp(Id::unique(("host_activate", id)), activate.rect));
                                        if activate.clicked() {
                                            action = Some(Action::Activate(id));
                                        }
                                        if let Some(source) = source {
                                            let response = widgets::action_button_if(
                                                ui,
                                                p,
                                                "messages-filled",
                                                aurora_i18n::tr!("host-chat-open"),
                                                ActionKind::Flat,
                                                vec2(0.0, 28.0),
                                                !closing && !snapshot.stopped,
                                            );
                                            #[cfg(test)]
                                            ctx.data_mut(|data| data.insert_temp(Id::unique(("host_chat_open", id)), response.rect));
                                            if response.clicked() {
                                                action = Some(Action::OpenChat(source));
                                            }
                                        }
                                        let detach = widgets::action_button_if(
                                            ui,
                                            p,
                                            "external-link",
                                            aurora_i18n::tr!("host-detach"),
                                            ActionKind::Flat,
                                            vec2(0.0, 28.0),
                                            !closing && !snapshot.stopped,
                                        )
                                        .on_hover_text(aurora_i18n::tr!("host-detach-help"));
                                        #[cfg(test)]
                                        ctx.data_mut(|data| data.insert_temp(Id::unique(("host_detach", id)), detach.rect));
                                        if detach.clicked() {
                                            action = Some(Action::Detach(id));
                                        }
                                    });
                                });
                                ui.add_space(12.0);
                            });
                        }
                        if count == 0 {
                            ui.label(RichText::new(aurora_i18n::tr!("host-no-sessions")).color(p.muted));
                        }
                        ui.add_space(10.0);
                        widgets::organization_header(ui, p, aurora_i18n::tr!("host-saved-characters"));
                        for account in snapshot
                            .accounts
                            .iter()
                            .take(64)
                            .filter(|account| matches(&account.label, &account.grid, &query))
                        {
                            ui.push_id(account.key.to_string(), |ui| {
                                ui.label(RichText::new(&account.label).strong());
                                ui.label(RichText::new(&account.grid).small().color(p.muted));
                                let opening = snapshot.opening.contains(&account.key);
                                let response = ui
                                    .add_enabled_ui(!closing && !snapshot.stopped && !opening && login_slot.is_some(), |ui| {
                                        ui.push_id(("host_account_open", login_slot), |ui| {
                                            widgets::action_button(
                                                ui,
                                                p,
                                                "login-2",
                                                aurora_i18n::tr!("host-open-login"),
                                                ActionKind::Flat,
                                                vec2(0.0, 28.0),
                                            )
                                        })
                                        .inner
                                    })
                                    .inner;
                                #[cfg(test)]
                                ctx.data_mut(|data| data.insert_temp(Id::unique(("host_open", account.key)), response.rect));
                                if response.clicked() {
                                    action = Some(Action::OpenLogin(Some(account.key), login_slot));
                                }
                                if opening {
                                    ui.label(RichText::new(aurora_i18n::tr!("host-opening")).small().color(p.muted));
                                }
                                ui.add_space(12.0);
                            });
                        }
                        if snapshot.accounts.is_empty() {
                            ui.label(RichText::new(aurora_i18n::tr!("host-no-saved-characters")).color(p.muted));
                        }
                    });
                });
        }
        let world = ui.available_rect_before_wrap();
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space((ui.available_height() * 0.22).max(10.0));
                widgets::glyph(ui, "device-desktop-filled", p.violet_pale, 40.0);
                ui.add_space(12.0);
                let active = hosted.and_then(|id| snapshot.workers.iter().find(|worker| worker.binding.worker.worker == id));
                ui.label(
                    RichText::new(active.map_or(aurora_i18n::tr!("host-no-hosted-session"), display_name))
                        .strong()
                        .size(20.0),
                );
                ui.add_space(8.0);
                ui.label(if !cfg!(windows) {
                    aurora_i18n::tr!("host-platform-help")
                } else if !snapshot.can_activate {
                    aurora_i18n::tr!("host-unavailable-help")
                } else {
                    aurora_i18n::tr!("host-activate-help")
                });
                if closing {
                    ui.add_space(12.0);
                    ui.label(aurora_i18n::tr!("host-closing"));
                }
                if let Some(notice) = self.notice {
                    ui.add_space(10.0);
                    ui.label(RichText::new(notice_text(notice)).color(p.muted));
                }
                if let Some(error) = &snapshot.error {
                    ui.add_space(12.0);
                    ui.label(RichText::new(error).color(p.danger));
                }
            });
        });
        if self.close_prompt && !closing {
            let dialog = widgets::Modal::new(Id::unique("session_host_close")).show(&ctx, |ui| {
                ui.set_max_width(420.0);
                ui.label(RichText::new(aurora_i18n::tr!("host-close-question")).strong());
                ui.label(aurora_i18n::tr!("host-close-help"));
                ui.add_space(12.0);
                ui.horizontal_wrapped(|ui| {
                    let cancel = ui.button(aurora_i18n::tr!("common-cancel"));
                    #[cfg(test)]
                    ctx.data_mut(|data| data.insert_temp(Id::unique("host_cancel_close"), cancel.rect));
                    if cancel.clicked() {
                        self.close_prompt = false;
                    }
                    let confirm = widgets::action_button(
                        ui,
                        p,
                        "external-link",
                        aurora_i18n::tr!("host-close-confirm"),
                        ActionKind::Primary,
                        vec2(0.0, 30.0),
                    );
                    #[cfg(test)]
                    ctx.data_mut(|data| data.insert_temp(Id::unique("host_confirm_close"), confirm.rect));
                    if confirm.clicked() {
                        self.close_prompt = false;
                        action = Some(Action::Close);
                    }
                });
            });
            if dialog.should_close() {
                self.close_prompt = false;
            }
        }
        if !closing && !snapshot.stopped {
            action = self.rename_dialog(&ctx).or(action);
        } else {
            self.rename = None;
        }
        if closing || snapshot.stopped || self.copy_blocked() {
            self.copy_preferences.close();
        } else if let Some(copy) = self.copy_preferences.show(&ctx, p, snapshot, false) {
            action = Some(Action::CopyPreferences(copy));
        }
        Output { action, world }
    }

    fn strip(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        snapshot: &HostSnapshot,
        native_ready: bool,
        hosted: Option<WorkerId>,
        closing: bool,
    ) -> Option<Action> {
        let mut action = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let width = (ui.available_width() * 0.22).clamp(96.0, 220.0);
            // ComboBox::width is a minimum. Give its truncating face its own
            // maximum layout budget before it measures a long profile caption.
            let selection_changed = ui
                .allocate_ui_with_layout(vec2(width, 28.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.set_min_width(width);
                    ui.set_max_width(width);
                    self.slot_picker(ui, snapshot, width)
                })
                .inner;
            if selection_changed {
                if let Some(worker) = self.selected_worker(snapshot) {
                    if can_activate(snapshot, worker, native_ready, hosted, closing) {
                        action = Some(Action::Activate(worker.binding.worker.worker));
                    } else if hosted != Some(worker.binding.worker.worker) {
                        self.management_open = true;
                    }
                } else if self.login_slot(snapshot).is_none() {
                    self.management_open = true;
                }
            }
            let enabled = !closing && !snapshot.stopped;
            let login_slot = self.login_slot(snapshot);
            let open = ui
                .push_id(("host_strip_open", self.selected_slot), |ui| {
                    compact_button(
                        ui,
                        p,
                        "login-2",
                        aurora_i18n::tr!("host-open-login"),
                        ActionKind::Primary,
                        enabled && login_slot.is_some(),
                    )
                })
                .inner;
            #[cfg(test)]
            ui.ctx().data_mut(|data| data.insert_temp(Id::unique("host_slot_open"), open.rect));
            if open.clicked() {
                action = Some(Action::OpenLogin(None, login_slot));
            }
            let selected = self.selected_worker(snapshot);
            let activate = ui
                .push_id(("host_strip_activate", selected.map(|worker| binding_id(worker.binding))), |ui| {
                    compact_button(
                        ui,
                        p,
                        "arrows-exchange",
                        aurora_i18n::tr!("host-activate"),
                        ActionKind::Flat,
                        selected.is_some_and(|worker| can_activate(snapshot, worker, native_ready, hosted, closing)),
                    )
                })
                .inner;
            #[cfg(test)]
            ui.ctx()
                .data_mut(|data| data.insert_temp(Id::unique("host_strip_activate"), activate.rect));
            if activate.clicked()
                && let Some(worker) = selected
            {
                action = Some(Action::Activate(worker.binding.worker.worker));
            }
            let source = selected.and_then(|worker| chat_source(snapshot, worker));
            let chat = ui
                .push_id(
                    (
                        "host_strip_chat",
                        source.map(|source| (binding_id(source.binding()), source.lease_epoch)),
                    ),
                    |ui| {
                        compact_button(
                            ui,
                            p,
                            "messages-filled",
                            aurora_i18n::tr!("host-chat-open"),
                            ActionKind::Flat,
                            enabled && source.is_some(),
                        )
                    },
                )
                .inner;
            #[cfg(test)]
            ui.ctx().data_mut(|data| data.insert_temp(Id::unique("host_strip_chat"), chat.rect));
            if chat.clicked()
                && let Some(source) = source
            {
                action = Some(Action::OpenChat(source));
            }
            // Reserve Manage and Close before assigning the remaining width to
            // tabs. The bounded scroller keeps them reachable at enlarged UI zoom.
            let tabs_width = (ui.available_width() - 64.0).max(0.0);
            ui.allocate_ui_with_layout(vec2(tabs_width, 28.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.set_min_width(tabs_width);
                ui.set_min_height(28.0);
                let characters_width = (tabs_width - 32.0).max(0.0);
                ui.allocate_ui_with_layout(
                    vec2(characters_width, 28.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.set_min_width(characters_width);
                        if let Some(tab_action) = self.tabs(ui, p, snapshot, native_ready, hosted, closing) {
                            action = Some(tab_action);
                        }
                    },
                );
                let status = if closing {
                    Some((aurora_i18n::tr!("host-closing").to_owned(), false))
                } else if snapshot.stopped {
                    Some((aurora_i18n::tr!("host-stopped").to_owned(), true))
                } else if let Some(error) = snapshot.error.as_ref().or(snapshot.registry_error.as_ref()) {
                    Some((error.clone(), true))
                } else {
                    self.notice
                        .map(|notice| (notice_text(notice).to_owned(), matches!(notice, Notice::Error(_))))
                };
                if let Some((message, warning)) = status {
                    let response = compact_button(
                        ui,
                        p,
                        if warning { "alert-triangle-filled" } else { "info-circle-filled" },
                        &message,
                        if warning { ActionKind::Caution } else { ActionKind::Flat },
                        true,
                    );
                    #[cfg(test)]
                    ui.ctx().data_mut(|data| data.insert_temp(Id::unique("host_status"), response.rect));
                    if response.clicked() {
                        self.management_open = true;
                    }
                } else {
                    let preferences = &snapshot.attention_preferences;
                    let tooltip = if preferences.error.is_some() {
                        if preferences.writable {
                            aurora_i18n::tr!("host-attention-save-failed").to_owned()
                        } else {
                            aurora_i18n::tr!("host-attention-load-failed").to_owned()
                        }
                    } else if self.attention_count == 0 {
                        aurora_i18n::tr!("host-attention-show").to_owned()
                    } else {
                        aurora_i18n::tr!("host-attention-count", count = self.attention_count)
                    };
                    let inbox = compact_button(
                        ui,
                        p,
                        "bell-filled",
                        &tooltip,
                        if preferences.error.is_some() {
                            ActionKind::Caution
                        } else if self.attention_count > 0 {
                            ActionKind::Primary
                        } else {
                            ActionKind::Flat
                        },
                        enabled,
                    );
                    #[cfg(test)]
                    ui.ctx().data_mut(|data| data.insert_temp(Id::unique("host_attention"), inbox.rect));
                    if inbox.clicked() {
                        action = Some(Action::OpenAttention);
                    }
                }
            });
            let attention = snapshot.registry_error.is_some() || snapshot.error.is_some() || matches!(self.notice, Some(Notice::Error(_)));
            let manage = compact_button(
                ui,
                p,
                "settings-filled",
                if self.management_open {
                    aurora_i18n::tr!("host-manage-hide")
                } else {
                    aurora_i18n::tr!("host-manage")
                },
                if attention {
                    ActionKind::Caution
                } else if self.management_open {
                    ActionKind::Primary
                } else {
                    ActionKind::Flat
                },
                !closing,
            );
            #[cfg(test)]
            ui.ctx().data_mut(|data| data.insert_temp(Id::unique("host_manage"), manage.rect));
            if manage.clicked() {
                self.management_open = !self.management_open;
            }
            let close = compact_button(ui, p, "x", aurora_i18n::tr!("host-close"), ActionKind::FlatDanger, !closing);
            #[cfg(test)]
            ui.ctx()
                .data_mut(|data| data.insert_temp(Id::unique("host_request_close"), close.rect));
            if close.clicked() {
                self.close_prompt = true;
            }
        });
        action
    }

    fn selected_worker<'a>(&self, snapshot: &'a HostSnapshot) -> Option<&'a WorkerSummary> {
        self.selected
            .and_then(|id| snapshot.workers.iter().find(|worker| worker.binding.worker.worker == id))
            .or_else(|| {
                snapshot
                    .workers
                    .iter()
                    .find(|worker| worker.slot.is_some() && worker.slot == self.selected_slot)
            })
    }

    fn tabs(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        snapshot: &HostSnapshot,
        native_ready: bool,
        hosted: Option<WorkerId>,
        closing: bool,
    ) -> Option<Action> {
        let mut action = None;
        let count = snapshot.workers.len().min(MAX_WORKERS);
        let width = ((ui.available_width() - (count.saturating_sub(1) as f32 * 4.0)) / count.max(1) as f32).clamp(28.0, 150.0);
        egui::ScrollArea::horizontal()
            .id_salt("session_host_tabs")
            .max_height(28.0)
            .min_scrolled_height(28.0)
            .auto_shrink([false, false])
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                #[cfg(test)]
                ui.ctx()
                    .data_mut(|data| data.insert_temp(Id::unique("host_tabs_clip"), ui.clip_rect()));
                ui.horizontal(|ui| {
                    for (index, worker) in snapshot.workers.iter().take(MAX_WORKERS).enumerate() {
                        let id = worker.binding.worker.worker;
                        ui.push_id(binding_id(worker.binding), |ui| {
                            let slot = worker.slot.and_then(|id| snapshot.slots.iter().find(|slot| slot.id == id));
                            let number = slot.map_or(index + 1, |slot| usize::from(slot.id.number()));
                            let name = slot.map(slot_name).unwrap_or_else(|| display_name(worker).to_owned());
                            let label = if width < 86.0 {
                                number.to_string()
                            } else {
                                format!("{number}: {name}")
                            };
                            let ready = can_activate(snapshot, worker, native_ready, hosted, closing);
                            let active = hosted == Some(id);
                            let button = egui::Button::new(RichText::new(label).color(if active { p.on_accent } else { p.ink }))
                                .truncate()
                                .selected(active);
                            let response = ui
                                .add_enabled_ui(ready || (active && !closing && !snapshot.stopped), |ui| {
                                    ui.add_sized(vec2(width, 28.0), button)
                                })
                                .inner;
                            #[cfg(test)]
                            ui.ctx().data_mut(|data| {
                                data.insert_temp(Id::unique(("host_tab", id)), response.rect);
                                data.insert_temp(Id::unique(("host_tab_active", id)), active);
                            });
                            if response.has_focus() {
                                response.scroll_to_me(Some(egui::Align::Center));
                            }
                            let tip = format!(
                                "{name}\n{} · {}\n{}{}",
                                display_name(worker),
                                display_grid(worker),
                                status(worker),
                                if snapshot.pending == Some(id) {
                                    format!(" · {}", aurora_i18n::tr!("host-switching"))
                                } else {
                                    String::new()
                                }
                            );
                            let response = response.on_hover_text(tip.clone()).on_disabled_hover_text(tip);
                            if response.clicked() {
                                self.selected = Some(id);
                                if let Some(slot) = worker.slot {
                                    self.selected_slot = Some(slot);
                                }
                                if ready {
                                    action = Some(Action::Activate(id));
                                }
                            }
                        });
                    }
                });
            });
        action
    }

    fn login_slot(&self, snapshot: &HostSnapshot) -> Option<SlotId> {
        self.selected_slot
            .and_then(|id| snapshot.slots.iter().find(|slot| slot.id == id && slot.available))
            .map(|slot| slot.id)
    }

    fn slot_picker(&mut self, ui: &mut egui::Ui, snapshot: &HostSnapshot, width: f32) -> bool {
        if self.selected_slot.is_none() {
            self.selected_slot = snapshot
                .slots
                .iter()
                .find(|slot| slot.available)
                .or(snapshot.slots.first())
                .map(|slot| slot.id);
        }
        let previous = self.selected_slot;
        let selected = self.selected_slot.and_then(|id| snapshot.slots.iter().find(|slot| slot.id == id));
        let text = selected.map(|slot| slot_caption(slot, snapshot)).unwrap_or_default();
        let combo = egui::ComboBox::from_id_salt("host_profile_slot")
            .selected_text(text)
            .width(width)
            .truncate()
            .show_ui(ui, |ui| {
                // Preserve the popup's inner width budget; its frame must not
                // re-add the outer button margins around long profile names.
                let inner_width = (width - egui::Frame::popup(ui.style()).total_margin().sum().x).max(1.0);
                ui.set_max_width(ui.available_width().min(inner_width));
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                #[cfg(test)]
                ui.ctx()
                    .data_mut(|data| data.insert_temp(Id::unique("host_slot_popup_area"), ui.layer_id().id));
                for slot in snapshot.slots.iter().take(MAX_WORKERS) {
                    let text = if slot.available {
                        slot_caption(slot, snapshot)
                    } else {
                        format!("{} · {}", slot_caption(slot, snapshot), aurora_i18n::tr!("host-profile-occupied"))
                    };
                    let response = ui.selectable_value(&mut self.selected_slot, Some(slot.id), text);
                    #[cfg(test)]
                    ui.ctx()
                        .data_mut(|data| data.insert_temp(Id::unique(("host_slot_pick", slot.id)), response.rect));
                    #[cfg(not(test))]
                    let _ = response;
                }
            });
        #[cfg(test)]
        ui.ctx().data_mut(|data| {
            data.insert_temp(Id::unique("host_slot_combo"), combo.response.rect);
            if combo.inner.is_none() {
                data.remove::<Id>(Id::unique("host_slot_popup_area"));
            }
        });
        let selected = self.selected_slot.and_then(|id| snapshot.slots.iter().find(|slot| slot.id == id));
        let tip = selected
            .map(|slot| {
                let caption = slot_caption(slot, snapshot);
                if let Some(worker) = snapshot.workers.iter().find(|worker| worker.slot == Some(slot.id)) {
                    format!("{caption}\n{}\n{}", display_grid(worker), status(worker))
                } else {
                    caption
                }
            })
            .unwrap_or_else(|| aurora_i18n::tr!("host-profile-select").to_owned());
        combo.response.on_hover_text(tip);
        if self.selected_slot != previous {
            self.selected = snapshot
                .workers
                .iter()
                .find(|worker| worker.slot == self.selected_slot)
                .map(|worker| worker.binding.worker.worker);
        }
        self.selected_slot != previous
    }

    #[cfg(windows)]
    fn monitor_entries(&self, ui: &mut egui::Ui, p: &Palette, snapshot: &HostSnapshot, closing: bool, action: &mut Option<Action>) {
        if self.monitor_sources.is_empty() && self.monitor_recovery.is_none() {
            return;
        }
        widgets::organization_header(ui, p, aurora_i18n::tr!("host-monitor-title"));
        for (slot, source) in self.monitor_sources.iter().take(MAX_WORKERS) {
            let Some(profile) = snapshot.slots.iter().find(|profile| profile.id == *slot) else {
                continue;
            };
            let chat = aurora_session::chat::ChatSource {
                worker: source.worker(),
                session: source.session(),
                lease_epoch: source.lease_epoch(),
            };
            // Ordinary egui press ownership is scoped to the complete source,
            // including lease and session generation, never a reused slot row.
            if monitor_open_button(
                ui,
                chat,
                &aurora_i18n::tr!("host-monitor-open", profile = slot_name(profile)),
                !closing && !snapshot.stopped,
            )
            .clicked()
            {
                *action = Some(Action::OpenMonitor(*source));
            }
        }
        if let Some(recovery) = self.monitor_recovery {
            if monitor_recovery_button(ui, recovery, !closing && !snapshot.stopped && self.monitor_cycle_ready).clicked() {
                *action = Some(Action::ResumeCyclingMonitor(Box::new(recovery)));
            }
        } else if ui
            .add_enabled(
                !closing && !snapshot.stopped && self.monitor_cycle_ready,
                egui::Button::new(aurora_i18n::tr!("host-monitor-open-cycling")).wrap(),
            )
            .clicked()
        {
            *action = Some(Action::OpenCyclingMonitor);
        }
        ui.add_space(10.0);
    }

    fn slot_management(&mut self, ui: &mut egui::Ui, p: &Palette, snapshot: &HostSnapshot, closing: bool) {
        ui.label(RichText::new(aurora_i18n::tr!("host-profile-select")).small().color(p.muted));
        let selected = self.selected_slot.and_then(|id| snapshot.slots.iter().find(|slot| slot.id == id));
        ui.horizontal_wrapped(|ui| {
            let rename = widgets::action_button_if(
                ui,
                p,
                "pencil",
                aurora_i18n::tr!("common-rename"),
                ActionKind::Flat,
                vec2(0.0, 28.0),
                selected.is_some() && !closing && !snapshot.stopped && !self.copy_preferences.is_open(),
            );
            #[cfg(test)]
            ui.ctx()
                .data_mut(|data| data.insert_temp(Id::unique("host_slot_rename"), rename.rect));
            if rename.clicked()
                && let Some(slot) = selected
            {
                self.rename = Some((slot.id, slot.label.as_str().to_owned()));
            }
            let copy = ui.add_enabled(
                selected.is_some_and(|slot| slot.available) && !closing && !snapshot.stopped && !self.modal_open(),
                egui::Button::new(aurora_i18n::tr!("host-profile-copy-open")).wrap(),
            );
            #[cfg(test)]
            ui.ctx().data_mut(|data| data.insert_temp(Id::unique("host_copy_open"), copy.rect));
            if copy.clicked()
                && let Some(slot) = selected
            {
                self.copy_preferences.open(slot.id, snapshot);
            }
            if selected.is_some_and(|slot| !slot.available) {
                ui.label(RichText::new(aurora_i18n::tr!("host-profile-occupied")).small().color(p.muted));
            }
        });
        if selected.is_some_and(|slot| !slot.available) {
            ui.add(egui::Label::new(RichText::new(aurora_i18n::tr!("host-profile-occupied-help")).small().color(p.muted)).wrap());
        }
        if let Some(error) = &snapshot.registry_error {
            ui.label(RichText::new(error).small().color(p.danger));
        }
        ui.add_space(4.0);
    }

    fn rename_dialog(&mut self, ctx: &egui::Context) -> Option<Action> {
        let (slot, draft) = self.rename.as_mut()?;
        let mut close = false;
        let mut action = None;
        let dialog = widgets::Modal::new(Id::unique("host_profile_rename")).show(ctx, |ui| {
            ui.set_max_width(380.0);
            ui.label(RichText::new(aurora_i18n::tr!("host-profile-rename")).strong());
            let edit = ui.add(
                TextEdit::singleline(draft)
                    .hint_text(aurora_i18n::tr!("common-name"))
                    .char_limit(64),
            );
            #[cfg(test)]
            ctx.data_mut(|data| data.insert_temp(Id::unique("host_slot_name"), edit.rect));
            #[cfg(not(test))]
            let _ = edit;
            ui.label(aurora_i18n::tr!("host-profile-name-help"));
            let label = SlotLabel::new(draft);
            if label.is_err() {
                ui.label(aurora_i18n::tr!("host-profile-name-invalid"));
            }
            ui.horizontal_wrapped(|ui| {
                let cancel = ui.button(aurora_i18n::tr!("common-cancel"));
                #[cfg(test)]
                ctx.data_mut(|data| data.insert_temp(Id::unique("host_slot_cancel"), cancel.rect));
                close |= cancel.clicked();
                let save = ui.add_enabled(label.is_ok(), egui::Button::new(aurora_i18n::tr!("common-save")));
                #[cfg(test)]
                ctx.data_mut(|data| data.insert_temp(Id::unique("host_slot_save"), save.rect));
                if save.clicked()
                    && let Ok(label) = label
                {
                    action = Some(Action::RenameSlot(*slot, label));
                    close = true;
                }
            });
        });
        if close || dialog.should_close() {
            self.rename = None;
        }
        action
    }
}

fn binding_id(binding: aurora_session::Binding) -> Id {
    Id::unique((
        "host_worker",
        (binding.worker.installation, binding.worker.host, binding.worker.worker),
        binding
            .session
            .map(|session| (session.session, session.account, session.grid, session.generation)),
    ))
}

#[cfg(any(windows, test))]
fn monitor_open_button(ui: &mut egui::Ui, source: aurora_session::chat::ChatSource, label: &str, enabled: bool) -> egui::Response {
    ui.push_id(("host_monitor", binding_id(source.binding()), source.lease_epoch), |ui| {
        ui.add_enabled(enabled, egui::Button::new(label).wrap())
    })
    .inner
}

#[cfg(any(windows, test))]
pub(super) fn monitor_recovery_button(ui: &mut egui::Ui, recovery: super::monitor_manager::Recovery, enabled: bool) -> egui::Response {
    ui.push_id(("host_monitor_recovery", recovery.id), |ui| {
        ui.add_enabled(enabled, egui::Button::new(aurora_i18n::tr!("host-monitor-resume-cycling")).wrap())
    })
    .inner
}

fn compact_button(ui: &mut egui::Ui, p: &Palette, icon: &str, label: &str, kind: ActionKind, enabled: bool) -> egui::Response {
    let response = widgets::action_button_if(ui, p, icon, "", kind, vec2(28.0, 28.0), enabled).on_hover_text(label);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled && ui.is_enabled(), label));
    response
}

fn can_activate(snapshot: &HostSnapshot, worker: &WorkerSummary, native_ready: bool, hosted: Option<WorkerId>, closing: bool) -> bool {
    !closing
        && !snapshot.stopped
        && snapshot.can_activate
        && snapshot.pending.is_none()
        && hosted != Some(worker.binding.worker.worker)
        && native_ready
        && activation_ready(worker)
}

fn chat_source(snapshot: &HostSnapshot, worker: &WorkerSummary) -> Option<aurora_session::chat::ChatSource> {
    snapshot
        .chat
        .iter()
        .find(|chat| chat.source.binding() == worker.binding && super::chat_ui::eligible(chat))
        .map(|chat| chat.source)
}

fn slot_caption(slot: &SlotSummary, snapshot: &HostSnapshot) -> String {
    let profile = format!("{}: {}", slot.id.number(), slot_name(slot));
    if let Some(worker) = snapshot.workers.iter().find(|worker| worker.slot == Some(slot.id)) {
        format!("{profile} · {}", display_name(worker))
    } else {
        profile
    }
}

fn slot_name(slot: &SlotSummary) -> String {
    if slot.label.as_str().is_empty() {
        aurora_i18n::tr!("host-profile-default", number = i64::from(slot.id.number()))
    } else {
        slot.label.as_str().to_owned()
    }
}

fn matches(label: &str, grid: &str, query: &str) -> bool {
    query.is_empty() || label.to_lowercase().contains(query) || grid.to_lowercase().contains(query)
}

/// The IO thread may publish a fresh Arc for unchanged receipts. Only visible
/// changes should repaint the full software surface, including on a large host.
pub(super) fn same_snapshot(a: &HostSnapshot, b: &HostSnapshot) -> bool {
    a.copy_preferences == b.copy_preferences
        && a.attention_preferences == b.attention_preferences
        && a.slots == b.slots
        && a.registry_error == b.registry_error
        && a.stopped == b.stopped
        && a.error == b.error
        && a.can_activate == b.can_activate
        && a.active == b.active
        && a.active_viewport == b.active_viewport
        && a.completed_request == b.completed_request
        && a.pending == b.pending
        && a.opening == b.opening
        && a.chat.len() == b.chat.len()
        && a.chat.iter().zip(&b.chat).all(|(a, b)| std::sync::Arc::ptr_eq(a, b))
        && a.accounts.len() == b.accounts.len()
        && a.accounts
            .iter()
            .zip(&b.accounts)
            .all(|(a, b)| a.key == b.key && a.label == b.label && a.grid == b.grid)
        && a.workers.len() == b.workers.len()
        && a.workers.iter().zip(&b.workers).all(|(a, b)| {
            let same_status = match (&a.status, &b.status) {
                (Some(a), Some(b)) => {
                    a.phase == b.phase
                        && a.name == b.name
                        && a.grid_label == b.grid_label
                        && a.native_window == b.native_window
                        && a.keyboard_focused == b.keyboard_focused
                        && a.blocked == b.blocked
                        && a.last_frame.is_some_and(|frame| frame.validate().is_ok())
                            == b.last_frame.is_some_and(|frame| frame.validate().is_ok())
                }
                (None, None) => true,
                _ => false,
            };
            a.slot == b.slot
                && a.binding == b.binding
                && same_status
                && a.reachable == b.reachable
                && a.label == b.label
                && a.grid == b.grid
        })
}

fn display_name(worker: &WorkerSummary) -> &str {
    let label = worker
        .status
        .as_ref()
        .map(|status| status.name.as_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or(&worker.label);
    if label.trim().is_empty() {
        aurora_i18n::tr!("host-unnamed-session")
    } else {
        label
    }
}

fn display_grid(worker: &WorkerSummary) -> &str {
    worker
        .status
        .as_ref()
        .map(|status| status.grid_label.as_str())
        .filter(|grid| !grid.trim().is_empty())
        .unwrap_or(&worker.grid)
}

fn activation_ready(worker: &WorkerSummary) -> bool {
    worker.reachable
        && worker.binding.session.is_some()
        && worker
            .status
            .as_ref()
            .is_some_and(|status| status.phase == Phase::Online && !status.blocked && status.native_window != 0)
}

pub(super) fn presentation_ready(worker: &WorkerSummary) -> bool {
    activation_ready(worker)
        && worker
            .status
            .as_ref()
            .is_some_and(|status| status.last_frame.is_some_and(|frame| frame.validate().is_ok()))
}

fn status(worker: &WorkerSummary) -> &'static str {
    if !worker.reachable {
        return aurora_i18n::tr!("host-unreachable");
    }
    let Some(status) = &worker.status else {
        return aurora_i18n::tr!("host-starting");
    };
    match status.phase {
        Phase::Login => aurora_i18n::tr!("host-login"),
        Phase::Connecting => aurora_i18n::tr!("host-connecting"),
        Phase::Online if status.blocked => aurora_i18n::tr!("host-worker-blocked"),
        Phase::Online => aurora_i18n::tr!("host-online"),
        Phase::Disconnected => aurora_i18n::tr!("host-disconnected"),
    }
}

fn notice_text(notice: Notice) -> &'static str {
    match notice {
        Notice::Queued => aurora_i18n::tr!("host-request-queued"),
        Notice::Error(HostError::Busy) => aurora_i18n::tr!("host-busy"),
        Notice::Error(HostError::Unsupported) => aurora_i18n::tr!("host-unavailable-help"),
        Notice::Error(HostError::Stopped) => aurora_i18n::tr!("host-stopped"),
        Notice::Error(HostError::Unavailable) => aurora_i18n::tr!("host-chat-unavailable"),
    }
}

#[cfg(test)]
mod tests {
    use super::super::transport::SavedAccount;
    use super::*;
    use aurora_session::{
        Binding, FrameProof, GridKey, HostId, InstallationId, Label, SessionId, SessionIdentity, WorkerIdentity, WorkerStatus,
    };

    fn snapshot() -> HostSnapshot {
        let account = AccountKey::from_bytes([5; 32]).unwrap();
        HostSnapshot {
            slots: SlotId::all()
                .map(|id| SlotSummary {
                    id,
                    label: SlotLabel::default(),
                    available: true,
                })
                .collect(),
            accounts: vec![SavedAccount {
                key: account,
                label: "Demo One".into(),
                grid: "Example Grid".into(),
            }],
            workers: vec![WorkerSummary {
                slot: Some(SlotId::try_from(0).unwrap()),
                binding: Binding {
                    worker: WorkerIdentity {
                        installation: InstallationId::from_bytes([1; 32]).unwrap(),
                        host: HostId::from_bytes([2; 16]).unwrap(),
                        worker: WorkerId::from_bytes([3; 16]).unwrap(),
                    },
                    session: Some(SessionIdentity {
                        session: SessionId::from_bytes([4; 16]).unwrap(),
                        account,
                        grid: GridKey::from_bytes([6; 32]).unwrap(),
                        generation: 1,
                    }),
                },
                status: Some(WorkerStatus {
                    phase: Phase::Online,
                    name: Label::new("Demo One").unwrap(),
                    grid_label: Label::new("Example Grid").unwrap(),
                    native_window: 7,
                    last_frame: Some(FrameProof {
                        device_epoch: 1,
                        surface_epoch: 1,
                        frame: 1,
                    }),
                    ..Default::default()
                }),
                reachable: true,
                label: "Demo One".into(),
                grid: "Example Grid".into(),
            }],
            ..Default::default()
        }
    }

    #[test]
    fn controller_activation_requires_live_bound_unblocked_online_session_and_presentation_requires_completed_frame() {
        let base = snapshot();
        let mut worker = base.workers[0].clone();
        assert!(activation_ready(&worker));
        assert!(presentation_ready(&worker));
        worker.reachable = false;
        assert!(!activation_ready(&worker));
        worker = base.workers[0].clone();
        worker.binding.session = None;
        assert!(!activation_ready(&worker));
        for phase in [Phase::Login, Phase::Connecting, Phase::Disconnected] {
            worker = base.workers[0].clone();
            worker.status.as_mut().unwrap().phase = phase;
            assert!(!activation_ready(&worker));
        }
        worker = base.workers[0].clone();
        worker.status.as_mut().unwrap().blocked = true;
        assert!(!activation_ready(&worker));
        worker.status.as_mut().unwrap().blocked = false;
        worker.status.as_mut().unwrap().native_window = 0;
        assert!(!activation_ready(&worker));
        worker.status.as_mut().unwrap().native_window = 7;
        worker.status.as_mut().unwrap().last_frame = None;
        assert!(
            activation_ready(&worker),
            "first activation must be able to request its first prepared frame"
        );
        assert!(
            !presentation_ready(&worker),
            "a prepared/completed frame is still required before showing the world"
        );
    }

    #[test]
    fn host_monitor_open_entry_keeps_full_source_press_ownership_across_reorder_and_queued_replacement() {
        use aurora_session::chat::ChatSource;
        struct Buttons {
            ctx: egui::Context,
            frame: u64,
            rects: Vec<Rect>,
        }
        impl Buttons {
            fn show(&mut self, rows: &[ChatSource], events: Vec<egui::Event>) -> Option<ChatSource> {
                self.frame += 1;
                self.rects.clear();
                let mut action = None;
                let mut output = self.ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(380.0, 420.0))),
                        time: Some(self.frame as f64 / 60.0),
                        focused: true,
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        for source in rows {
                            let response = monitor_open_button(
                                ui,
                                *source,
                                &aurora_i18n::tr!("host-monitor-open", profile = "Synthetic profile"),
                                true,
                            );
                            self.rects.push(response.rect);
                            if response.clicked() {
                                action = Some(*source);
                            }
                        }
                    },
                );
                output.textures_delta.clear();
                action
            }
            fn edge(&mut self, rows: &[ChatSource], at: egui::Pos2, pressed: bool) -> Option<ChatSource> {
                self.show(
                    rows,
                    vec![
                        egui::Event::PointerMoved(at),
                        egui::Event::PointerButton {
                            pos: at,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                )
            }
        }
        let host = five_workers();
        let sources: Vec<_> = host
            .workers
            .iter()
            .take(2)
            .map(|worker| ChatSource {
                worker: worker.binding.worker,
                session: worker.binding.session.unwrap(),
                lease_epoch: 7,
            })
            .collect();
        let mut frames = Buttons {
            ctx: egui::Context::default(),
            frame: 0,
            rects: Vec::new(),
        };
        crate::theme::Theme::default().apply(&frames.ctx, 1.0);
        for _ in 0..4 {
            assert!(frames.show(&sources, Vec::new()).is_none());
        }
        let at = frames.rects[0].center();
        assert!(frames.edge(&sources, at, true).is_none());
        assert!(
            frames.edge(&[sources[1], sources[0]], at, false).is_none(),
            "held row cannot choose its replacement at the old position"
        );
        for _ in 0..4 {
            frames.show(&sources[..1], Vec::new());
        }
        let at = frames.rects[0].center();
        let mut replacement = sources[0];
        replacement.session.generation += 1;
        // Source replacement occurs BEFORE the first egui pass carrying this
        // native press; previous painted widget identity still owns that press.
        assert!(frames.edge(&[replacement], at, true).is_none());
        assert!(frames.edge(&[replacement], at, false).is_none());
        for _ in 0..4 {
            frames.show(&[replacement], Vec::new());
        }
        let at = frames.rects[0].center();
        assert!(frames.edge(&[replacement], at, true).is_none());
        assert_eq!(
            frames.edge(&[replacement], at, false),
            Some(replacement),
            "fresh explicit entry remains usable"
        );
    }

    struct Frames {
        ctx: egui::Context,
        ui: HostUi,
        frame: u64,
        size: egui::Vec2,
        hosted: Option<WorkerId>,
    }

    impl Frames {
        fn new() -> Self {
            let ctx = egui::Context::default();
            crate::theme::Theme {
                panel_alpha: 255,
                ..Default::default()
            }
            .apply(&ctx, 1.0);
            Self {
                ctx,
                ui: HostUi::default(),
                frame: 0,
                size: vec2(1240.0, 800.0),
                hosted: None,
            }
        }

        fn show(&mut self, snapshot: &HostSnapshot, events: Vec<egui::Event>, native_ready: bool) -> Option<Action> {
            self.frame += 1;
            let p = crate::theme::Theme {
                panel_alpha: 255,
                ..Default::default()
            }
            .palette();
            let mut action = None;
            let mut output = self.ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, self.size)),
                    time: Some(self.frame as f64 / 60.0),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let shown = self.ui.show(ui, &p, snapshot, native_ready, self.hosted, false);
                    ui.ctx()
                        .data_mut(|data| data.insert_temp(Id::unique("host_world_rect"), shown.world));
                    action = action.take().or(shown.action);
                    assert!(shown.world.is_positive(), "the compact shell must retain a usable world rectangle");
                    assert!(self.ctx.content_rect().expand(1.0).contains_rect(shown.world));
                },
            );
            assert!(
                output.shapes.iter().all(|shape| !matches!(&shape.shape, egui::Shape::Callback(_))),
                "software-only UI must use meshes"
            );
            output.textures_delta.clear();
            action
        }

        fn click(&mut self, snapshot: &HostSnapshot, id: Id, native_ready: bool) -> Option<Action> {
            for _ in 0..4 {
                self.show(snapshot, Vec::new(), native_ready);
            }
            let rect = self.ctx.data(|data| data.get_temp::<Rect>(id)).expect("action shown");
            self.show(snapshot, vec![egui::Event::PointerMoved(rect.center())], native_ready);
            let mut action = None;
            for pressed in [true, false] {
                action = action.or(self.show(
                    snapshot,
                    vec![egui::Event::PointerButton {
                        pos: rect.center(),
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    }],
                    native_ready,
                ));
            }
            action
        }
    }

    #[test]
    fn controller_buttons_issue_exact_typed_roster_actions_and_gate_unsupported_activation() {
        let mut snapshot = snapshot();
        let worker = snapshot.workers[0].binding.worker.worker;
        let account = snapshot.accounts[0].key;
        let mut frames = Frames::new();
        frames.ui.management_open = true;
        assert_eq!(frames.click(&snapshot, Id::unique(("host_activate", worker)), true), None);
        snapshot.can_activate = true;
        assert_eq!(frames.click(&snapshot, Id::unique(("host_activate", worker)), false), None);
        snapshot.workers[0].status.as_mut().unwrap().last_frame = None;
        assert_eq!(
            frames.click(&snapshot, Id::unique(("host_activate", worker)), true),
            Some(Action::Activate(worker))
        );
        assert_eq!(
            frames.click(&snapshot, Id::unique(("host_detach", worker)), true),
            Some(Action::Detach(worker))
        );
        assert_eq!(
            frames.click(&snapshot, Id::unique(("host_open", account)), true),
            Some(Action::OpenLogin(Some(account), Some(SlotId::try_from(0).unwrap())))
        );
        let at = frames
            .ctx
            .data(|data| data.get_temp::<Rect>(Id::unique(("host_open", account))))
            .unwrap()
            .center();
        frames.show(&snapshot, vec![egui::Event::PointerMoved(at)], true);
        assert_eq!(
            frames.show(
                &snapshot,
                vec![egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                }],
                true
            ),
            None
        );
        let other_slot = SlotId::try_from(4).unwrap();
        frames.ui.selected_slot = Some(other_slot);
        frames.ui.selected = None;
        frames.show(&snapshot, Vec::new(), true);
        assert_eq!(
            frames.show(
                &snapshot,
                vec![egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                true
            ),
            None,
            "a pending saved-account launch gesture must not select a replacement profile"
        );
        assert_eq!(
            frames.click(&snapshot, Id::unique(("host_open", account)), true),
            Some(Action::OpenLogin(Some(account), Some(other_slot)))
        );
        snapshot.opening.push(account);
        assert_eq!(frames.click(&snapshot, Id::unique(("host_open", account)), true), None);
    }

    #[test]
    fn persistent_slot_picker_opens_exact_free_profile_and_rename_is_only_metadata() {
        let mut snapshot = snapshot();
        let first = SlotId::try_from(0).unwrap();
        let last = SlotId::try_from(4).unwrap();
        snapshot.slots[0].label = SlotLabel::new("Custom profile label").unwrap();
        let actual = display_name(&snapshot.workers[0]).to_owned();
        let binding = snapshot.workers[0].binding;
        let mut frames = Frames::new();
        assert_eq!(
            frames.click(&snapshot, Id::unique("host_slot_open"), false),
            Some(Action::OpenLogin(None, Some(first)))
        );
        assert_eq!(frames.click(&snapshot, Id::unique("host_slot_combo"), false), None);
        assert_eq!(frames.click(&snapshot, Id::unique(("host_slot_pick", last)), false), None);
        assert_eq!(frames.ui.selected_slot, Some(last));
        assert_eq!(
            frames.click(&snapshot, Id::unique("host_slot_open"), false),
            Some(Action::OpenLogin(None, Some(last)))
        );
        snapshot.slots[4].available = false;
        assert_eq!(
            frames.click(&snapshot, Id::unique("host_slot_open"), false),
            None,
            "occupied profile cannot launch"
        );
        assert_eq!(frames.click(&snapshot, Id::unique("host_manage"), false), None);
        assert!(frames.ui.management_open);
        assert_eq!(frames.click(&snapshot, Id::unique("host_slot_rename"), false), None);
        assert!(frames.ui.modal_open(), "rename dialog must cover the fitted native world");
        frames.click(&snapshot, Id::unique("host_slot_name"), false);
        frames.show(&snapshot, vec![egui::Event::Text("A different label 猫".into())], false);
        assert_eq!(
            frames.click(&snapshot, Id::unique("host_slot_save"), false),
            Some(Action::RenameSlot(last, SlotLabel::new("A different label 猫").unwrap()))
        );
        assert!(!frames.ui.modal_open());
        assert_eq!(display_name(&snapshot.workers[0]), actual);
        assert_eq!(
            snapshot.workers[0].binding, binding,
            "labels never substitute native account identity"
        );
        assert_eq!(frames.click(&snapshot, Id::unique("host_slot_rename"), false), None);
        frames.click(&snapshot, Id::unique("host_slot_name"), false);
        frames.show(&snapshot, vec![egui::Event::Text("discard this".into())], false);
        assert_eq!(frames.click(&snapshot, Id::unique("host_slot_cancel"), false), None);
        assert!(!frames.ui.modal_open());
    }

    #[test]
    fn profile_rename_save_rejects_multibyte_byte_overflow_and_cancel_has_no_action() {
        let snapshot = snapshot();
        let mut frames = Frames::new();
        let slot = SlotId::try_from(0).unwrap();
        frames.ui.rename = Some((slot, "猫".repeat(22)));
        assert_eq!(frames.click(&snapshot, Id::unique("host_slot_save"), false), None);
        assert!(frames.ui.modal_open(), "invalid UTF-8 byte length cannot publish a label");
        assert_eq!(frames.click(&snapshot, Id::unique("host_slot_cancel"), false), None);
        assert!(!frames.ui.modal_open());
        assert_eq!(snapshot.slots[0].label, SlotLabel::default());
    }

    #[test]
    fn compact_profile_selector_outer_popup_stays_on_canvas_with_long_names_narrow_windows_and_fractional_zoom() {
        let mut snapshot = snapshot();
        for (index, slot) in snapshot.slots.iter_mut().enumerate() {
            slot.label = SlotLabel::new(&if index % 2 == 0 { "W".repeat(64) } else { "キャラ".repeat(7) }).unwrap();
        }
        for (size, zoom) in [
            (vec2(1240.0, 800.0), 1.0),
            (vec2(620.0, 680.0), 1.0),
            (vec2(990.0, 760.0), 1.25),
            (vec2(620.0, 680.0), 1.5),
        ] {
            let mut frames = Frames::new();
            frames.size = size;
            frames.ctx.set_zoom_factor(zoom);
            for smaller in [false, true] {
                if smaller {
                    frames.size.x = 540.0;
                }
                assert_eq!(frames.click(&snapshot, Id::unique("host_slot_combo"), false), None);
                for _ in 0..6 {
                    frames.show(&snapshot, Vec::new(), false);
                }
                let popup_id = frames
                    .ctx
                    .data(|data| data.get_temp::<Id>(Id::unique("host_slot_popup_area")))
                    .expect("live popup Area captured");
                assert!(egui::Popup::is_id_open(&frames.ctx, popup_id));
                // Area state includes the Frame's margins and stroke. Row or
                // label rectangles alone miss the part clipped by RGN_DIFF.
                let outer = frames
                    .ctx
                    .memory(|memory| memory.area_rect(popup_id))
                    .expect("actual popup outer Area");
                let world = frames
                    .ctx
                    .data(|data| data.get_temp::<Rect>(Id::unique("host_world_rect")))
                    .unwrap();
                let button = frames
                    .ctx
                    .data(|data| data.get_temp::<Rect>(Id::unique("host_slot_combo")))
                    .unwrap();
                let tolerance = 1.0 / frames.ctx.pixels_per_point();
                assert!(outer.is_positive(), "popup must be laid out");
                let canvas = frames.ctx.content_rect();
                assert!(
                    canvas.expand(tolerance).contains_rect(outer),
                    "outer popup {outer:?} must fit canvas {canvas:?}, size {size:?}, zoom {zoom}"
                );
                assert!(
                    outer.intersects(world),
                    "compact menus deliberately overlay the world; native presentation must preserve this popup"
                );
                assert!(
                    outer.width() <= button.width() + tolerance,
                    "popup frame must fit the button's outer budget: {outer:?} vs {button:?}"
                );
                assert_eq!(frames.click(&snapshot, Id::unique("host_slot_combo"), false), None);
                frames.show(&snapshot, Vec::new(), false);
                assert!(
                    frames
                        .ctx
                        .data(|data| data.get_temp::<Id>(Id::unique("host_slot_popup_area")))
                        .is_none(),
                    "closed popups cannot leave a stale geometry capture"
                );
            }
        }
    }

    #[test]
    fn host_attention_uses_existing_compact_island_without_activating_a_world() {
        let snapshot = snapshot();
        for width in [1240.0, 760.0] {
            let mut frames = Frames::new();
            frames.size.x = width;
            frames.show(&snapshot, vec![], false);
            let world = frames
                .ctx
                .data(|data| data.get_temp::<Rect>(Id::unique("host_world_rect")))
                .unwrap();
            let before = frames.ctx.data(|data| data.get_temp::<Rect>(Id::unique("host_attention"))).unwrap();
            frames.ui.attention_count = 7;
            frames.show(&snapshot, vec![], false);
            let after = frames.ctx.data(|data| data.get_temp::<Rect>(Id::unique("host_attention"))).unwrap();
            assert_eq!(before, after, "unread color/count must not move the strip or world");
            assert_eq!(
                world,
                frames
                    .ctx
                    .data(|data| data.get_temp::<Rect>(Id::unique("host_world_rect")))
                    .unwrap()
            );
            assert!(frames.ctx.content_rect().contains_rect(after));
            assert_eq!(
                frames.click(&snapshot, Id::unique("host_attention"), false),
                Some(Action::OpenAttention)
            );
            assert_eq!(snapshot.active, None);
            assert_eq!(snapshot.pending, None);
        }
    }

    #[test]
    fn host_chat_roster_button_requires_authenticated_live_exact_source_and_never_activates_world() {
        use super::super::transport::HostChatSnapshot;
        use aurora_session::chat::{ChatKey, ChatSource, Conversation, ConversationInfo, NativeChatPolicy};
        use std::sync::Arc;
        let mut snapshot = snapshot();
        let worker = snapshot.workers[0].binding.worker.worker;
        let source = ChatSource::try_from((snapshot.workers[0].binding, 1)).unwrap();
        let mut chat = HostChatSnapshot {
            contacts: Default::default(),
            source,
            authenticated: false,
            live: true,
            conversations: vec![ConversationInfo {
                policy: NativeChatPolicy {
                    key: ChatKey {
                        source,
                        conversation: Conversation::Nearby,
                    },
                    revision: 1,
                    lease_current: true,
                    ready: true,
                    can_read: true,
                    can_send: true,
                },
                title: Label::new("Synthetic nearby").unwrap(),
            }],
            lines: Vec::new(),
            receipts: Vec::new(),
            reads: Default::default(),
            read_operations: Vec::new(),
            closes: Default::default(),
            gap: false,
        };
        snapshot.chat.push(Arc::new(chat.clone()));
        let mut frames = Frames::new();
        frames.ui.management_open = true;
        frames.show(&snapshot, Vec::new(), true);
        assert!(
            frames
                .ctx
                .data(|data| data.get_temp::<Rect>(Id::unique(("host_chat_open", worker))))
                .is_none()
        );
        chat.authenticated = true;
        snapshot.chat[0] = Arc::new(chat.clone());
        assert_eq!(
            frames.click(&snapshot, Id::unique(("host_chat_open", worker)), true),
            Some(Action::OpenChat(source))
        );
        assert_eq!(
            frames.click(&snapshot, Id::unique("host_strip_chat"), true),
            Some(Action::OpenChat(source))
        );
        assert_eq!(snapshot.active, None);
        let at = frames
            .ctx
            .data(|data| data.get_temp::<Rect>(Id::unique("host_strip_chat")))
            .unwrap()
            .center();
        frames.show(&snapshot, vec![egui::Event::PointerMoved(at)], true);
        assert_eq!(
            frames.show(
                &snapshot,
                vec![egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                }],
                true
            ),
            None
        );
        chat.source.lease_epoch += 1;
        chat.conversations[0].policy.key.source = chat.source;
        snapshot.chat[0] = Arc::new(chat.clone());
        frames.show(&snapshot, Vec::new(), true);
        assert_eq!(
            frames.show(
                &snapshot,
                vec![egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                true
            ),
            None,
            "a pointer gesture must not retarget to a replacement chat lease"
        );
        assert_eq!(
            frames.click(&snapshot, Id::unique("host_strip_chat"), true),
            Some(Action::OpenChat(chat.source))
        );
        chat.source.session.generation += 1;
        chat.conversations[0].policy.key.source = chat.source;
        snapshot.chat[0] = Arc::new(chat);
        let mut frames = Frames::new();
        frames.ui.management_open = true;
        frames.show(&snapshot, Vec::new(), true);
        assert!(
            frames
                .ctx
                .data(|data| data.get_temp::<Rect>(Id::unique(("host_chat_open", worker))))
                .is_none(),
            "new session metadata cannot authorize the old worker binding"
        );
        assert_eq!(frames.click(&snapshot, Id::unique("host_strip_chat"), true), None);
    }

    #[test]
    fn controller_search_matches_display_names_and_grids_without_changing_identity() {
        assert!(matches("Demo One", "Example Grid", "example"));
        assert!(matches("Demo One", "Example Grid", "one"));
        assert!(!matches("Demo One", "Example Grid", "missing"));
    }

    fn five_workers() -> HostSnapshot {
        let mut snapshot = snapshot();
        let base = snapshot.workers[0].clone();
        snapshot.workers = SlotId::all()
            .enumerate()
            .map(|(index, slot)| {
                let mut worker = base.clone();
                worker.slot = Some(slot);
                worker.binding.worker.worker = WorkerId::from_bytes([index as u8 + 10; 16]).unwrap();
                worker.binding.session.as_mut().unwrap().session = SessionId::from_bytes([index as u8 + 20; 16]).unwrap();
                worker
            })
            .collect();
        for (index, slot) in snapshot.slots.iter_mut().enumerate() {
            slot.available = false;
            slot.label = SlotLabel::new(&if index % 2 == 0 { "W".repeat(64) } else { "キャラ".repeat(7) }).unwrap();
        }
        snapshot
    }

    #[test]
    fn compact_host_keeps_full_width_world_and_essential_controls_inside_narrow_zoomed_canvas() {
        let snapshot = five_workers();
        for (size, zoom) in [
            (vec2(1240.0, 800.0), 1.0),
            (vec2(760.0, 600.0), 1.0),
            (vec2(760.0, 600.0), 1.5),
            (vec2(540.0, 680.0), 1.5),
        ] {
            let mut frames = Frames::new();
            frames.size = size;
            frames.ctx.set_zoom_factor(zoom);
            for _ in 0..6 {
                frames.show(&snapshot, Vec::new(), true);
            }
            let canvas = frames.ctx.content_rect();
            let world = frames
                .ctx
                .data(|data| data.get_temp::<Rect>(Id::unique("host_world_rect")))
                .unwrap();
            let tolerance = 1.0 / frames.ctx.pixels_per_point();
            assert!(
                (world.left() - canvas.left()).abs() <= tolerance,
                "management must not occupy the default world: {world:?}"
            );
            assert!((world.right() - canvas.right()).abs() <= tolerance);
            assert!(
                world.top() - canvas.top() <= 40.0,
                "host controls should remain one compact row: {world:?}"
            );
            for key in [
                "host_slot_combo",
                "host_slot_open",
                "host_strip_activate",
                "host_strip_chat",
                "host_manage",
                "host_request_close",
            ] {
                let rect = frames.ctx.data(|data| data.get_temp::<Rect>(Id::unique(key))).unwrap();
                assert!(
                    canvas.expand(tolerance).contains_rect(rect),
                    "{key} {rect:?} escapes {canvas:?} at {size:?}/{zoom}"
                );
                assert!(
                    rect.bottom() <= world.top() + tolerance,
                    "{key} must not be behind the native world"
                );
            }
            let clip = frames.ctx.data(|data| data.get_temp::<Rect>(Id::unique("host_tabs_clip"))).unwrap();
            assert!(
                clip.right() <= canvas.right() + tolerance,
                "scrolled character tabs must retain a bounded clip"
            );
            assert_eq!(frames.click(&snapshot, Id::unique("host_manage"), true), None);
            assert!(frames.ui.management_open);
            let managed = frames
                .ctx
                .data(|data| data.get_temp::<Rect>(Id::unique("host_world_rect")))
                .unwrap();
            assert!(managed.left() > world.left(), "management is an explicit, reversible layout change");
            assert_eq!(frames.click(&snapshot, Id::unique("host_manage"), true), None);
            let restored = frames
                .ctx
                .data(|data| data.get_temp::<Rect>(Id::unique("host_world_rect")))
                .unwrap();
            assert_eq!(restored, world);
        }
    }

    #[test]
    fn compact_profile_commit_activates_exact_ready_worker_without_optimistic_active_highlight() {
        let mut snapshot = five_workers();
        snapshot.can_activate = true;
        let first = snapshot.workers[0].binding.worker.worker;
        let second = snapshot.workers[1].binding.worker.worker;
        let third = snapshot.workers[2].binding.worker.worker;
        let mut frames = Frames::new();
        frames.hosted = Some(first);
        assert_eq!(frames.click(&snapshot, Id::unique("host_slot_combo"), true), None);
        assert_eq!(
            frames.click(&snapshot, Id::unique(("host_slot_pick", snapshot.slots[1].id)), true),
            Some(Action::Activate(second))
        );
        assert_eq!(frames.ui.selected_slot, snapshot.workers[1].slot);
        frames.show(&snapshot, Vec::new(), true);
        assert_eq!(
            frames
                .ctx
                .data(|data| data.get_temp::<bool>(Id::unique(("host_tab_active", first)))),
            Some(true)
        );
        assert_eq!(
            frames
                .ctx
                .data(|data| data.get_temp::<bool>(Id::unique(("host_tab_active", second)))),
            Some(false)
        );
        snapshot.pending = Some(second);
        assert_eq!(frames.click(&snapshot, Id::unique("host_slot_combo"), true), None);
        assert_eq!(
            frames.click(&snapshot, Id::unique(("host_slot_pick", snapshot.slots[2].id)), true),
            None
        );
        assert_eq!(frames.ui.selected, Some(third));
        assert!(
            frames.ui.management_open,
            "a busy/unready selection must expose management without activating"
        );
        snapshot.pending = None;
        assert_eq!(frames.click(&snapshot, Id::unique("host_strip_activate"), false), None);
        assert_eq!(
            frames.click(&snapshot, Id::unique("host_strip_activate"), true),
            Some(Action::Activate(third))
        );
        assert_eq!(snapshot.active, None, "UI intent never changes acknowledged ownership");
    }

    #[test]
    fn compact_character_tabs_keep_duplicate_labels_bound_to_their_exact_worker() {
        let mut snapshot = five_workers();
        snapshot.can_activate = true;
        for slot in &mut snapshot.slots {
            slot.label = SlotLabel::new("Same label").unwrap();
        }
        let first = snapshot.workers[0].binding.worker.worker;
        let last = snapshot.workers[4].binding.worker.worker;
        let mut frames = Frames::new();
        assert_eq!(
            frames.click(&snapshot, Id::unique(("host_tab", last)), true),
            Some(Action::Activate(last))
        );
        assert_eq!(frames.ui.selected_slot, snapshot.workers[4].slot);
        snapshot.workers[0].status.as_mut().unwrap().blocked = true;
        assert_eq!(frames.click(&snapshot, Id::unique(("host_tab", first)), true), None);
        snapshot.workers[0].status.as_mut().unwrap().blocked = false;
        assert_eq!(frames.click(&snapshot, Id::unique(("host_tab", first)), false), None);
        assert_eq!(
            frames.click(&snapshot, Id::unique(("host_tab", first)), true),
            Some(Action::Activate(first))
        );
    }

    #[test]
    fn compact_status_keeps_tabs_interactive_and_preserves_world_and_essential_button_geometry() {
        let mut snapshot = five_workers();
        snapshot.can_activate = true;
        let first = snapshot.workers[0].binding.worker.worker;
        let second = snapshot.workers[1].binding.worker.worker;
        let mut frames = Frames::new();
        frames.hosted = Some(first);
        for _ in 0..4 {
            frames.show(&snapshot, Vec::new(), true);
        }
        let rect = |frames: &Frames, key: &str| frames.ctx.data(|data| data.get_temp::<Rect>(Id::unique(key))).unwrap();
        let world = rect(&frames, "host_world_rect");
        let manage = rect(&frames, "host_manage");
        let close = rect(&frames, "host_request_close");
        for notice in [Notice::Queued, Notice::Error(HostError::Busy)] {
            frames.ui.notice = Some(notice);
            for _ in 0..4 {
                frames.show(&snapshot, Vec::new(), true);
            }
            assert_eq!(
                frames.click(&snapshot, Id::unique(("host_tab", second)), true),
                Some(Action::Activate(second)),
                "a queued/error notice must not remove the primary character tabs"
            );
            let status = rect(&frames, "host_status");
            assert!(status.bottom() <= world.top(), "status must remain above the native cutout");
            assert_eq!(rect(&frames, "host_world_rect"), world);
            assert_eq!(rect(&frames, "host_manage"), manage);
            assert_eq!(rect(&frames, "host_request_close"), close);
        }
        snapshot.registry_error = Some("Synthetic registry failure".into());
        snapshot.pending = Some(second);
        frames.ctx.data_mut(|data| {
            data.remove::<Rect>(Id::unique(("host_tab", second)));
        });
        frames.show(&snapshot, Vec::new(), true);
        assert!(
            frames
                .ctx
                .data(|data| data.get_temp::<Rect>(Id::unique(("host_tab", second))))
                .is_some(),
            "pending gates must leave live tab widgets mounted"
        );
        assert_eq!(frames.click(&snapshot, Id::unique(("host_tab", second)), true), None);
        assert_eq!(rect(&frames, "host_world_rect"), world);
        assert_eq!(rect(&frames, "host_manage"), manage);
        assert_eq!(rect(&frames, "host_request_close"), close);
    }

    #[test]
    fn compact_tabs_never_retarget_a_press_when_workers_reorder_before_release() {
        let mut snapshot = five_workers();
        snapshot.can_activate = true;
        let first = snapshot.workers[0].binding.worker.worker;
        let mut frames = Frames::new();
        for _ in 0..4 {
            frames.show(&snapshot, Vec::new(), true);
        }
        let at = frames
            .ctx
            .data(|data| data.get_temp::<Rect>(Id::unique(("host_tab", first))))
            .unwrap()
            .center();
        frames.show(&snapshot, vec![egui::Event::PointerMoved(at)], true);
        assert_eq!(
            frames.show(
                &snapshot,
                vec![egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                }],
                true
            ),
            None
        );
        snapshot.workers.swap(0, 1);
        frames.show(&snapshot, Vec::new(), true);
        assert_eq!(
            frames.show(
                &snapshot,
                vec![egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                true
            ),
            None,
            "reordered display positions must not reinterpret a pending character click"
        );
        let target = snapshot.workers[0].binding.worker.worker;
        assert_eq!(
            frames.click(&snapshot, Id::unique(("host_tab", target)), true),
            Some(Action::Activate(target)),
            "a fresh explicit click may activate the new occupant"
        );
    }

    #[test]
    fn controller_generic_login_adopts_acknowledged_character_and_grid_metadata() {
        let mut snapshot = snapshot();
        let worker = &mut snapshot.workers[0];
        worker.label.clear();
        worker.grid.clear();
        let identity = worker.binding;
        assert_eq!(display_name(worker), "Demo One");
        assert_eq!(display_grid(worker), "Example Grid");
        assert!(matches(display_name(worker), display_grid(worker), "demo"));
        assert!(matches(display_name(worker), display_grid(worker), "example"));
        assert_eq!(worker.binding, identity, "labels never substitute the protocol binding");
        worker.status.as_mut().unwrap().phase = Phase::Login;
        worker.status.as_mut().unwrap().blocked = true;
        assert_eq!(status(worker), aurora_i18n::tr!("host-login"));
        worker.status.as_mut().unwrap().name = Label::new("").unwrap();
        worker.status.as_mut().unwrap().grid_label = Label::new("").unwrap();
        worker.label = "Saved Character".into();
        worker.grid = "Saved Grid".into();
        assert_eq!(display_name(worker), "Saved Character");
        assert_eq!(display_grid(worker), "Saved Grid");
    }

    #[test]
    fn host_presentation_requires_exact_native_ack_beyond_activation_eligibility() {
        use aurora_session::{HostedViewport, PhysicalRect};
        let mut snapshot = snapshot();
        let worker = snapshot.workers[0].binding.worker.worker;
        let viewport = HostedViewport {
            host_window: 7,
            rect: PhysicalRect {
                x: -1600,
                y: 80,
                width: 1000,
                height: 700,
            },
        };
        snapshot.can_activate = true;
        snapshot.active = Some(worker);
        snapshot.active_viewport = Some(viewport);
        snapshot.registry_error = Some("Synthetic profile-label write failure".into());
        let presented = super::super::presented_worker;
        assert_eq!(presented(&snapshot, Some(viewport), false), Some(worker));
        assert_eq!(
            presented(&snapshot, Some(viewport), true),
            None,
            "modal/local intent covers the world"
        );
        assert_eq!(presented(&snapshot, None, false), None);
        let moved = HostedViewport {
            rect: PhysicalRect {
                x: viewport.rect.x + 10,
                ..viewport.rect
            },
            ..viewport
        };
        assert_eq!(
            presented(&snapshot, Some(moved), false),
            None,
            "a live move needs a new exact acknowledgment"
        );
        snapshot.pending = Some(worker);
        assert_eq!(presented(&snapshot, Some(viewport), false), None);
        snapshot.pending = None;
        snapshot.active_viewport = Some(moved);
        assert_eq!(presented(&snapshot, Some(moved), false), Some(worker));
        snapshot.workers[0].status.as_mut().unwrap().last_frame = None;
        assert!(activation_ready(&snapshot.workers[0]));
        assert_eq!(
            presented(&snapshot, Some(moved), false),
            None,
            "no completed frame means no world cutout"
        );
    }

    #[test]
    fn controller_header_close_click_keeps_its_modal_open_across_following_passes() {
        let snapshot = snapshot();
        let mut frames = Frames::new();
        assert_eq!(frames.click(&snapshot, Id::unique("host_request_close"), true), None);
        assert!(frames.ui.close_prompt, "the opening pointer release must not dismiss the new modal");
        for _ in 0..4 {
            assert_eq!(frames.show(&snapshot, Vec::new(), true), None);
            assert!(
                frames.ui.close_prompt,
                "the close confirmation must survive discarded/following UI passes"
            );
            assert!(
                frames
                    .ctx
                    .data(|data| data.get_temp::<Rect>(Id::unique("host_confirm_close")))
                    .is_some()
            );
        }
    }

    #[test]
    fn controller_close_requires_confirmation_and_cancel_keeps_sessions_untouched() {
        let snapshot = snapshot();
        let mut frames = Frames::new();
        frames.ui.close_prompt = true;
        assert_eq!(frames.show(&snapshot, Vec::new(), true), None);
        assert_eq!(frames.click(&snapshot, Id::unique("host_cancel_close"), true), None);
        assert!(!frames.ui.close_prompt);
        frames.ui.close_prompt = true;
        assert_eq!(frames.click(&snapshot, Id::unique("host_confirm_close"), true), Some(Action::Close));
        assert!(!frames.ui.close_prompt);
    }

    #[test]
    fn profile_copy_manage_entry_opens_real_modal_without_opening_or_retargeting_profile() {
        let mut snapshot = snapshot();
        let mut frames = Frames::new();
        assert_eq!(frames.click(&snapshot, Id::unique("host_manage"), false), None);
        assert_eq!(frames.click(&snapshot, Id::unique("host_copy_open"), false), None);
        assert!(frames.ui.copy_preferences.is_open() && frames.ui.modal_open());
        for _ in 0..3 {
            assert_eq!(frames.show(&snapshot, vec![], false), None);
            assert!(
                frames.ui.copy_preferences.is_open(),
                "opening release cannot dismiss the new copy modal"
            );
        }
        frames.ui.copy_preferences.close();
        snapshot.slots[0].available = false;
        assert_eq!(frames.click(&snapshot, Id::unique("host_copy_open"), false), None);
        assert!(
            !frames.ui.copy_preferences.is_open(),
            "occupied target has no presentation entry authority"
        );
    }

    #[test]
    fn controller_unchanged_receipts_sleep_and_visible_status_changes_repaint() {
        let a = snapshot();
        assert!(same_snapshot(&a, &a.clone()));
        let mut b = a.clone();
        b.workers[0].status.as_mut().unwrap().last_frame.as_mut().unwrap().frame += 1;
        assert!(same_snapshot(&a, &b), "progress receipts do not animate the roster");
        b = a.clone();
        b.workers[0].reachable = false;
        assert!(!same_snapshot(&a, &b));
        b = a.clone();
        b.workers[0].status.as_mut().unwrap().keyboard_focused = true;
        assert!(!same_snapshot(&a, &b));
        b = a.clone();
        b.pending = Some(b.workers[0].binding.worker.worker);
        assert!(!same_snapshot(&a, &b));
        b = a.clone();
        b.accounts[0].grid = "Different Grid".into();
        assert!(!same_snapshot(&a, &b));
    }
}
