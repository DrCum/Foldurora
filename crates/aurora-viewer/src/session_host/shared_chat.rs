//! One canonical composer per exact source, independent of disposable CPU
//! surfaces and world activation. Selecting a row emits no worker command.
//! The Legacy host selects a character/conversation without acknowledging read
//! (tools/session-host/main.cpp); Aurora retains its own native policy fences.

use super::chat_ui::{self, ChatUi, InputMode, SendDraft};
use super::slots::SlotId;
use super::transport::{HostChatSnapshot, HostError, HostSnapshot};
use crate::theme::Palette;
use aurora_session::MAX_WORKERS;
use aurora_session::chat::ChatSource;
use egui::{Id, RichText, vec2};
use std::sync::Arc;

struct SourceUi {
    ui: ChatUi,
    snapshot: Arc<HostChatSnapshot>,
    slot: Option<SlotId>,
    label: String,
    account: String,
    grid: String,
}

impl SourceUi {
    fn caption(&self) -> String {
        aurora_i18n::tr!(
            "host-chat-source-caption",
            label = self.label.as_str(),
            account = self.account.as_str(),
            grid = self.grid.as_str()
        )
    }
}

/// Shell owns this even while the tool is closed. A future alternate surface
/// must borrow these same composers, never copy their drafts/undo/receipts.
#[derive(Default)]
pub(super) struct SharedChat {
    states: Vec<SourceUi>,
    selected: Option<ChatSource>,
    changed: bool,
    next_instance: u64,
}

impl SharedChat {
    /// Every retained source observes policy loss before any native event is
    /// handled, including when no chat surface exists or it is minimized.
    pub fn sync(&mut self, snapshot: &HostSnapshot, closing: bool) -> bool {
        let before: Vec<_> = self.states.iter().map(|state| state.ui.source()).collect();
        let live: Vec<_> = snapshot
            .chat
            .iter()
            .filter(|chat| !closing && !snapshot.stopped && chat_ui::eligible(chat))
            .filter_map(|chat| {
                let worker = snapshot.workers.iter().find(|worker| worker.binding == chat.source.binding())?;
                let account = worker
                    .status
                    .as_ref()
                    .map(|status| status.name.as_str())
                    .filter(|name| !name.is_empty())
                    .unwrap_or(&worker.label)
                    .to_owned();
                let grid = worker
                    .status
                    .as_ref()
                    .map(|status| status.grid_label.as_str())
                    .filter(|grid| !grid.is_empty())
                    .unwrap_or(&worker.grid)
                    .to_owned();
                let custom = worker
                    .slot
                    .and_then(|id| snapshot.slots.iter().find(|slot| slot.id == id))
                    .map(|slot| slot.label.as_str())
                    .unwrap_or(&worker.label)
                    .to_owned();
                let label = match worker.slot {
                    Some(slot) => {
                        let numbered = aurora_i18n::tr!("host-profile-default", number = i64::from(slot.number()));
                        if custom.is_empty() {
                            numbered
                        } else {
                            format!("{numbered} · {custom}")
                        }
                    }
                    None => custom,
                };
                Some((chat.clone(), worker.slot, label, account, grid))
            })
            .take(MAX_WORKERS)
            .collect();
        let mut changed = false;
        self.states.retain_mut(|state| {
            let current = live.iter().find(|(chat, ..)| chat.source == state.ui.source());
            state.ui.sync(current.map(|(chat, ..)| chat.as_ref()));
            changed |= current.is_none();
            current.is_some()
        });
        for (chat, slot, label, account, grid) in live {
            if let Some(state) = self.states.iter_mut().find(|state| state.ui.source() == chat.source) {
                let identity_changed = state.slot != slot || state.label != label || state.account != account || state.grid != grid;
                self.changed |= identity_changed;
                changed |= !Arc::ptr_eq(&state.snapshot, &chat) || identity_changed;
                state.snapshot = chat;
                state.slot = slot;
                state.label = label;
                state.account = account;
                state.grid = grid;
            } else if self.states.len() < MAX_WORKERS {
                let Some(instance) = self.next_instance.checked_add(1) else {
                    continue;
                };
                self.next_instance = instance;
                let mut ui = ChatUi::for_instance(chat.source, instance);
                ui.sync(Some(&chat));
                self.states.push(SourceUi {
                    ui,
                    snapshot: chat,
                    slot,
                    label,
                    account,
                    grid,
                });
                changed = true;
            }
        }
        self.states.sort_by_key(|state| state.slot.map(u8::from).unwrap_or(u8::MAX));
        let after: Vec<_> = self.states.iter().map(|state| state.ui.source()).collect();
        if before != after {
            self.changed = true;
        }
        changed
    }

    /// An unavailable explicit selection stays unavailable. Discovery and an
    /// unrelated world switch never choose another sender as a fallback.
    pub fn select(&mut self, source: ChatSource) {
        if source.validate().is_err() || self.selected == Some(source) {
            return;
        }
        self.selected = Some(source);
        self.cancel_gestures();
    }

    pub fn cancel_gestures(&mut self) {
        self.changed = true;
        for state in &mut self.states {
            state.ui.cancel_gesture();
        }
    }

    pub fn clear(&mut self) {
        self.states.clear();
        self.selected = None;
        self.changed = true;
    }

    pub fn composer_rect(&self, ctx: &egui::Context) -> Option<egui::Rect> {
        self.states
            .iter()
            .find(|state| Some(state.ui.source()) == self.selected)
            .and_then(|state| state.ui.composer_rect(ctx))
    }

    pub fn queued(&mut self, draft: &SendDraft, request: u64) {
        if let Some(state) = self.states.iter_mut().find(|state| state.ui.source() == draft.key.source) {
            state.ui.queued(draft, request);
        }
    }

    pub fn queue_failed(&mut self, draft: &SendDraft, error: HostError) {
        if let Some(state) = self.states.iter_mut().find(|state| state.ui.source() == draft.key.source) {
            state.ui.queue_failed(error);
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui, p: &Palette, mode: InputMode) -> Option<SendDraft> {
        let changed = std::mem::take(&mut self.changed);
        if changed || mode == InputMode::Navigating {
            chat_ui::cancel_input(ui.ctx());
        }
        let entries: Vec<_> = self.states.iter().map(|state| (state.ui.source(), state.caption())).collect();
        let selected = self.selected;
        let mut picked = selected;
        let mut selector = |ui: &mut egui::Ui| {
            if mode != InputMode::Closing {
                source_selector(ui, p, &entries, selected, &mut picked, mode);
            }
            mode == InputMode::Live && picked != selected
        };
        let action = if let Some(state) = self.states.iter_mut().find(|state| Some(state.ui.source()) == selected) {
            let body_mode = if changed && mode == InputMode::Live {
                InputMode::Navigating
            } else {
                mode
            };
            state
                .ui
                .show_with_selector(ui, p, Some(&state.snapshot), &state.account, &state.grid, body_mode, selector)
        } else {
            selector(ui);
            egui::CentralPanel::default()
                .frame(crate::ui::widgets::overlay_frame(ui.ctx(), p))
                .show(ui, |ui| {
                    ui.label(aurora_i18n::tr!("host-chat-source-unavailable"));
                });
            None
        };
        if mode == InputMode::Live && picked != selected {
            if let Some(source) = picked {
                self.select(source);
                ui.ctx().request_repaint();
            }
            return None;
        }
        action
    }
}

fn source_selector(
    ui: &mut egui::Ui,
    p: &Palette,
    entries: &[(ChatSource, String)],
    selected: Option<ChatSource>,
    picked: &mut Option<ChatSource>,
    mode: InputMode,
) {
    let caption = entries
        .iter()
        .find(|(source, _)| Some(*source) == selected)
        .map(|(_, caption)| caption.clone())
        .unwrap_or_else(|| aurora_i18n::tr!("host-chat-source-unavailable").to_owned());
    egui::Panel::top(Id::unique("shared_chat_source"))
        .frame(crate::ui::widgets::overlay_frame(ui.ctx(), p))
        .show(ui, |ui| {
            if !matches!(mode, InputMode::Live | InputMode::Editing) {
                ui.disable();
            }
            ui.label(RichText::new(aurora_i18n::tr!("host-chat-send-as")).strong());
            let width = ui.available_width();
            let combo = ui
                .allocate_ui_with_layout(vec2(width, 28.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_min_width(0.0);
                    ui.set_max_width(width);
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                    egui::ComboBox::from_id_salt("shared_chat_source")
                        .selected_text(caption)
                        .width(width)
                        .wrap_mode(egui::TextWrapMode::Truncate)
                        .show_ui(ui, |ui| {
                            ui.set_width((width - ui.spacing().menu_margin.sum().x).max(1.0));
                            for (source, caption) in entries {
                                let response = ui
                                    .push_id(chat_ui::source_id(*source), |ui| {
                                        ui.selectable_value(picked, Some(*source), caption)
                                    })
                                    .inner;
                                #[cfg(test)]
                                ui.ctx().data_mut(|data| {
                                    data.insert_temp(Id::unique(("shared_chat_pick", chat_ui::source_id(*source))), response.rect)
                                });
                                #[cfg(not(test))]
                                let _ = response;
                            }
                        })
                })
                .inner;
            #[cfg(test)]
            ui.ctx().data_mut(|data| {
                data.insert_temp(Id::unique("shared_chat_combo_rect"), combo.response.rect);
                data.insert_temp(Id::unique("shared_chat_popup"), combo.response.id.with("popup"));
            });
            #[cfg(not(test))]
            let _ = combo;
        });
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod visual;
