//! Owned CPU-window body for conversation reminders. Port of the Attention
//! controls in tools/session-host/main.cpp (abbe3cb0ca66ee2484f277219f6a27706fab7fc6).
//! Passive opening only navigates SharedChat. Explicit native Review captures
//! one target for the broker's exact-grant transaction; no transcript or draft
//! belongs here, and this UI cannot itself activate, read or send.

use super::attention::{Attention, Direction, Ticket};
use super::attention_prefs::{PreferenceState, Preferences};
use super::chat_ui::{self, InputMode};
use super::review::{self, Status};
use super::slots::SlotId;
use super::transport::HostSnapshot;
use crate::theme::Palette;
use crate::ui::widgets;
use aurora_session::chat::{ReviewCapture, ReviewOutcome};
use egui::{Id, Key, RichText, vec2};

#[derive(Clone, Copy, Debug)]
pub(super) enum Action {
    OpenChat(Ticket),
    Select(Ticket),
    Review(Ticket),
    CancelReview(u64),
    Dismiss(Ticket),
    Preferences(Preferences),
}

#[derive(Clone, Copy)]
enum Intent {
    Select(Ticket),
    Open(Ticket),
    Review(Ticket),
    CancelReview(u64),
    Dismiss(Ticket),
    Preferences { base: Preferences, desired: Preferences },
}

impl Intent {
    fn valid(self, model: &Attention, state: &PreferenceState) -> bool {
        match self {
            Self::Select(ticket) | Self::Open(ticket) | Self::Review(ticket) | Self::Dismiss(ticket) => model.validate(ticket),
            Self::CancelReview(request) => request != 0,
            Self::Preferences { base, .. } => state.writable && state.preferences == base,
        }
    }
}

#[derive(Clone, Copy)]
enum Trigger {
    Pointer,
    Key(Key),
    Accessibility,
}

struct Gesture {
    widget: Id,
    trigger: Trigger,
    intent: Intent,
    frame: u64,
    press_index: usize,
}

struct PaintedControl {
    id: Id,
    widget: Id,
    rect: egui::Rect,
    layer: egui::LayerId,
    intent: Intent,
    #[cfg(all(test, target_os = "linux"))]
    frame: u64,
}

#[cfg(all(test, target_os = "linux"))]
struct NativeControlProbe {
    id: Id,
    widget: Id,
    rect: egui::Rect,
    interact_rect: egui::Rect,
    clip_rect: egui::Rect,
    layer: egui::LayerId,
    enabled: bool,
    live: bool,
    frame: u64,
    intent: Option<Intent>,
    presentation: bool,
}

#[derive(Clone, Copy)]
enum Save {
    Admitting,
    Queued { ticket: u64, preferences: Preferences },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Cancellation {
    Queued,
    Failed,
}

struct Reviewing {
    // Purged on source/policy/incarnation loss even while closed; a late
    // receipt cannot restore revoked target authority or private identity.
    target: Option<ReviewCapture>,
    request: u64,
    status: Status,
    cancellation: Option<Cancellation>,
}

#[derive(Default)]
pub(super) struct AttentionUi {
    gesture: Option<Gesture>,
    save: Option<Save>,
    save_failed: bool,
    ime_composing: bool,
    controls: Vec<PaintedControl>,
    painting: Vec<PaintedControl>,
    input_frame: Option<u64>,
    filters_open: bool,
    reviewing: Option<Reviewing>,
    review_failed: bool,
    #[cfg(all(test, target_os = "linux"))]
    native_probes: Vec<NativeControlProbe>,
    #[cfg(all(test, target_os = "linux"))]
    native_painting: Vec<NativeControlProbe>,
}

impl AttentionUi {
    pub fn review_queued(&mut self, request: u64, target: ReviewCapture) {
        self.reviewing = Some(Reviewing {
            target: Some(target),
            request,
            status: Status::WaitingForGrant,
            cancellation: None,
        });
        self.review_failed = false;
        self.cancel_gestures();
    }

    pub fn review_failed(&mut self) {
        self.review_failed = true;
        self.cancel_gestures();
    }

    /// Neither a newer host intent nor a matching unread row acknowledges this
    /// operation. Counts can advance or disappear during normal native paint.
    pub fn sync_review(&mut self, snapshot: &HostSnapshot) -> bool {
        let Some(current) = &mut self.reviewing else { return false };
        let before = (current.target, current.status);
        if let Some(target) = current.target {
            let chat = snapshot.chat.iter().find(|chat| chat.source == target.key.source);
            let valid = !snapshot.stopped
                && snapshot
                    .workers
                    .iter()
                    .any(|worker| worker.reachable && worker.binding == target.key.source.binding())
                && chat.is_some_and(|chat| {
                    review::current_target(
                        target,
                        Some(chat.source),
                        chat.live,
                        chat.authenticated,
                        chat.conversations
                            .iter()
                            .take(aurora_session::chat::MAX_CONVERSATIONS)
                            .find(|info| info.policy.key == target.key)
                            .map(|info| info.policy),
                        chat.reads.get(&target.key.conversation).copied(),
                    )
                });
            if valid {
                if let Some(operation) = snapshot
                    .review_operations
                    .iter()
                    .take(review::MAX_OPERATIONS)
                    .find(|operation| operation.request == current.request && operation.target == target)
                {
                    current.status = operation.status;
                    if !matches!(current.status, Status::WaitingForGrant | Status::Dispatched | Status::Uncertain) {
                        current.cancellation = None;
                    }
                }
            } else {
                current.target = None;
                current.status = match current.status {
                    Status::WaitingForGrant => Status::Unavailable,
                    Status::Dispatched | Status::Uncertain => Status::Uncertain,
                    status => status,
                };
                self.cancel_gestures();
            }
        }
        self.reviewing
            .as_ref()
            .is_some_and(|current| before != (current.target, current.status))
    }

    pub fn review_busy(&self) -> bool {
        self.reviewing
            .as_ref()
            .is_some_and(|current| matches!(current.status, Status::WaitingForGrant | Status::Dispatched))
    }

    pub fn review_to_cancel(&self) -> Option<u64> {
        self.reviewing
            .as_ref()
            .filter(|current| matches!(current.status, Status::WaitingForGrant | Status::Dispatched | Status::Uncertain))
            .map(|current| current.request)
    }

    pub fn review_cancelled(&mut self, request: u64, queued: bool) {
        if let Some(current) = self.reviewing.as_mut().filter(|current| current.request == request) {
            // FIFO admission cannot prove that cancellation beat native dispatch.
            current.cancellation = Some(if queued { Cancellation::Queued } else { Cancellation::Failed });
        }
        self.cancel_gestures();
    }

    fn review_status(&self) -> Option<&'static str> {
        let Some(current) = self.reviewing.as_ref() else {
            return self.review_failed.then_some("host-attention-review-failed");
        };
        Some(match current.status {
            Status::WaitingForGrant if current.cancellation == Some(Cancellation::Queued) => "host-attention-review-cancelling",
            Status::WaitingForGrant => "host-attention-review-waiting",
            Status::Dispatched => "host-attention-review-dispatched",
            Status::Uncertain => "host-attention-review-uncertain",
            Status::Completed(ReviewOutcome::OpenedLocally) => "host-attention-review-opened",
            Status::Completed(ReviewOutcome::NotForeground) => "host-attention-review-foreground",
            Status::Completed(ReviewOutcome::Expired) | Status::Expired => "host-attention-review-expired",
            Status::Completed(ReviewOutcome::RejectedNative | ReviewOutcome::StaleTarget) | Status::Unavailable => {
                "host-attention-review-unavailable"
            }
            Status::Cancelled => "host-attention-review-cancelled",
            Status::Busy => "host-attention-review-busy",
        })
    }

    /// Shell calls this on native focus loss, modality and surface disposal,
    /// including when no egui pass will run before reopening.
    pub fn cancel_gestures(&mut self) {
        self.gesture = None;
        self.ime_composing = false;
        self.controls.clear();
        self.painting.clear();
        #[cfg(all(test, target_os = "linux"))]
        {
            self.native_probes.clear();
            self.native_painting.clear();
        }
    }

    pub fn queued(&mut self, ticket: u64, preferences: Preferences) {
        self.save = Some(Save::Queued { ticket, preferences });
        self.save_failed = false;
        self.cancel_gestures();
    }

    pub fn queue_failed(&mut self) {
        self.save = None;
        self.save_failed = true;
        self.cancel_gestures();
    }

    /// A surface can fail after producing an intent but before Shell admits
    /// it. Recover that local intent without forgetting an actual IO ticket.
    pub fn surface_failed(&mut self) {
        if matches!(self.save, Some(Save::Admitting)) {
            self.queue_failed();
        }
        self.cancel_gestures();
    }

    /// Only the exact persisted operation completes a local request. Unrelated
    /// host operations and an older save receipt cannot acknowledge this one.
    pub fn sync_preferences(&mut self, state: &PreferenceState) {
        if let Some(Save::Queued { ticket, preferences }) = self.save
            && let Some(operation) = &state.operation
            && operation.ticket == ticket
            && operation.preferences == preferences
        {
            self.save = None;
            self.save_failed = operation.error.is_some();
            self.cancel_gestures();
        }
    }

    /// Shell calls this after every model sync, including while minimized or
    /// closed, so revoked policies/observations cannot linger in hit mappings.
    pub fn sync(&mut self, model: &Attention, state: &PreferenceState) {
        self.sync_preferences(state);
        if !state.writable || self.save.is_some() {
            // Preference IO cannot strand an already admitted Review. Cancel
            // carries its original request and needs no row/filter authority.
            if self
                .gesture
                .as_ref()
                .is_some_and(|gesture| !matches!(gesture.intent, Intent::CancelReview(_)))
            {
                self.gesture = None;
            }
            self.controls.retain(|control| matches!(control.intent, Intent::CancelReview(_)));
        } else {
            if self.gesture.as_ref().is_some_and(|gesture| !gesture.intent.valid(model, state)) {
                self.gesture = None;
            }
            self.controls.retain(|control| control.intent.valid(model, state));
        }
        self.painting.clear();
        #[cfg(all(test, target_os = "linux"))]
        {
            self.native_probes
                .retain(|probe| probe.intent.is_none_or(|intent| intent.valid(model, state)));
            self.native_painting.clear();
        }
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        model: &mut Attention,
        snapshot: &HostSnapshot,
        mode: InputMode,
    ) -> Option<Action> {
        let state = &snapshot.attention_preferences;
        self.sync(model, state);
        self.sync_review(snapshot);
        let available = state.writable && !snapshot.stopped;
        let focused = ui.input(|input| input.focused);
        let focus_changed = ui.input(|input| input.events.iter().any(|event| matches!(event, egui::Event::WindowFocused(_))));
        let was_composing = self.ime_composing;
        let ime_event = ui.input(|input| {
            let mut seen = false;
            for event in &input.events {
                if let egui::Event::Ime(event) = event {
                    seen = true;
                    match event {
                        egui::ImeEvent::Preedit { text, .. } => self.ime_composing = !text.is_empty(),
                        egui::ImeEvent::Commit(_) => self.ime_composing = false,
                        #[allow(deprecated)]
                        egui::ImeEvent::Disabled => self.ime_composing = false,
                        _ => {}
                    }
                }
            }
            seen
        });
        if !focused || focus_changed || mode == InputMode::Closing {
            self.cancel_gestures();
        }
        let input_live = mode == InputMode::Live
            && focused
            && !focus_changed
            && !was_composing
            && !self.ime_composing
            && !ime_event
            && !egui::Popup::is_any_open(ui.ctx());
        let live = available && self.save.is_none() && input_live;
        let cancel_live = !snapshot.stopped && input_live;
        if self.gesture.as_ref().is_some_and(|gesture| {
            !gesture.intent.valid(model, state) || !(live || (cancel_live && matches!(gesture.intent, Intent::CancelReview(_))))
        }) {
            self.gesture = None;
        }
        self.capture_press(ui, model, state, live, cancel_live);
        self.painting.clear();
        #[cfg(all(test, target_os = "linux"))]
        self.native_painting.clear();
        let mut activated = None;
        egui::Panel::top(Id::unique("attention_filters"))
            .frame(widgets::overlay_frame(ui.ctx(), p))
            .show(ui, |ui| {
                ui.label(RichText::new(aurora_i18n::tr!("host-attention-help")).color(p.muted));
                let filters = ui
                    .push_id("attention_filter_toggle", |ui| {
                        ui.add_enabled(
                            mode == InputMode::Live && focused && !focus_changed && !was_composing && !self.ime_composing && !ime_event,
                            egui::Button::new(aurora_i18n::tr!("host-attention-filters")).selected(self.filters_open),
                        )
                    })
                    .inner;
                #[cfg(test)]
                ui.ctx()
                    .data_mut(|data| data.insert_temp(Id::unique("attention_filter_toggle").with("rect"), filters.rect));
                if filters.clicked() {
                    self.filters_open = !self.filters_open;
                    self.cancel_gestures();
                    activated = None;
                }
                #[cfg(all(test, target_os = "linux"))]
                self.native_probe(
                    ui,
                    &filters,
                    Id::unique("attention_filter_toggle"),
                    None,
                    mode == InputMode::Live && focused && !focus_changed && !was_composing && !self.ime_composing && !ime_event,
                    true,
                );
                if self.filters_open {
                    // Fit the heading and a wrapped first profile row even at
                    // native minimum size; longer contents remain scrollable.
                    // This still reserves the reminder list and bottom actions.
                    let height = (ui.ctx().content_rect().height() - 340.0).clamp(64.0, 160.0);
                    let scroll = egui::ScrollArea::vertical()
                        .id_salt("attention_filter_rows")
                        .max_height(height)
                        // The top panel initially offers its last collapsed
                        // height. Reserve this bounded viewport so it can grow;
                        // accepting zero here permanently clips the filters.
                        .min_scrolled_height(height)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            ui.add_enabled_ui(live, |ui| {
                                ui.label(RichText::new(aurora_i18n::tr!("host-attention-profiles")).strong());
                                // Give each caption a full, stable cell width before
                                // shaping it. A wrapping horizontal row can offer
                                // only a few trailing pixels to the next checkbox.
                                let cell_width = ((ui.available_width() - ui.spacing().item_spacing.x) / 2.0).max(1.0);
                                egui::Grid::new("attention_filter_grid")
                                    .num_columns(2)
                                    .min_col_width(cell_width)
                                    .max_col_width(cell_width)
                                    .show(ui, |ui| {
                                        for (index, slot) in SlotId::all().enumerate() {
                                            let name = aurora_i18n::tr!("host-profile-default", number = i64::from(slot.number()));
                                            let custom = snapshot.slots.iter().find(|item| item.id == slot).map(|item| item.label.as_str());
                                            let caption = custom
                                                .filter(|label| !label.is_empty())
                                                .map_or(name.clone(), |label| format!("{name} · {label}"));
                                            let response = ui
                                                .allocate_ui_with_layout(
                                                    vec2(cell_width, 0.0),
                                                    egui::Layout::top_down(egui::Align::Min),
                                                    |ui| {
                                                        ui.set_max_width(cell_width);
                                                        published_checkbox(
                                                            ui,
                                                            p,
                                                            state.preferences.include_slots[index],
                                                            caption,
                                                            Id::unique(("attention_slot", slot)),
                                                        )
                                                    },
                                                )
                                                .inner;
                                            let mut desired = state.preferences;
                                            desired.include_slots[index] = !desired.include_slots[index];
                                            self.control(
                                                ui,
                                                response,
                                                Id::unique(("attention_slot", slot)),
                                                Some(Intent::Preferences {
                                                    base: state.preferences,
                                                    desired,
                                                }),
                                                live,
                                                &mut activated,
                                            );
                                            if index % 2 == 1 {
                                                ui.end_row();
                                            }
                                        }
                                        ui.end_row();
                                        for (kind, label, value) in [
                                            ("nearby", aurora_i18n::tr!("host-attention-nearby"), state.preferences.nearby),
                                            ("direct", aurora_i18n::tr!("host-attention-direct"), state.preferences.direct),
                                        ] {
                                            let response = ui
                                                .allocate_ui_with_layout(
                                                    vec2(cell_width, 0.0),
                                                    egui::Layout::top_down(egui::Align::Min),
                                                    |ui| {
                                                        ui.set_max_width(cell_width);
                                                        published_checkbox(ui, p, value, label, Id::unique(("attention_kind", kind)))
                                                    },
                                                )
                                                .inner;
                                            let mut desired = state.preferences;
                                            if kind == "nearby" {
                                                desired.nearby = !value;
                                            } else {
                                                desired.direct = !value;
                                            }
                                            self.control(
                                                ui,
                                                response,
                                                Id::unique(("attention_kind", kind)),
                                                Some(Intent::Preferences {
                                                    base: state.preferences,
                                                    desired,
                                                }),
                                                live,
                                                &mut activated,
                                            );
                                        }
                                        ui.end_row();
                                    });
                            });
                        });
                    #[cfg(test)]
                    ui.ctx().data_mut(|data| {
                        data.insert_temp(Id::unique("attention_filter_scroll"), scroll.inner_rect);
                        data.insert_temp(Id::unique("attention_filter_content"), scroll.content_size);
                    });
                    #[cfg(not(test))]
                    let _ = scroll;
                }
                if !state.writable && state.error.is_some() {
                    ui.label(RichText::new(aurora_i18n::tr!("host-attention-load-failed")).color(p.warn));
                } else if !state.writable {
                    ui.label(aurora_i18n::tr!("host-attention-loading"));
                } else if self.save.is_some() {
                    ui.label(aurora_i18n::tr!("host-attention-saving"));
                } else if self.save_failed || state.error.is_some() {
                    ui.label(RichText::new(aurora_i18n::tr!("host-attention-save-failed")).color(p.warn));
                }
            });

        egui::Panel::bottom(Id::unique("attention_actions"))
            .frame(widgets::overlay_frame(ui.ctx(), p))
            .show(ui, |ui| {
                ui.add_enabled_ui(live, |ui| {
                    let ticket = model.selected().and_then(|key| model.capture(key));
                    let width = ui.available_width();
                    let response = ui
                        .push_id("attention_open", |ui| {
                            ui.add_enabled_ui(ticket.is_some(), |ui| {
                                widgets::primary_button_sized(ui, p, aurora_i18n::tr!("host-attention-open"), vec2(width, 30.0))
                            })
                            .inner
                        })
                        .inner;
                    self.control(
                        ui,
                        response,
                        Id::unique("attention_open"),
                        ticket.map(Intent::Open),
                        live,
                        &mut activated,
                    );
                    // can_activate is published only by the supported hosted
                    // platform. Unsupported hosts expose no inert Review feature.
                    if snapshot.can_activate {
                        let enabled = ticket.is_some() && snapshot.pending.is_none() && !self.review_busy();
                        let response = ui
                            .push_id("attention_review", |ui| {
                                ui.add_enabled(enabled, egui::Button::new(aurora_i18n::tr!("host-attention-review")))
                                    .on_hover_text(aurora_i18n::tr!("host-attention-review-help"))
                            })
                            .inner;
                        self.control(
                            ui,
                            response,
                            Id::unique("attention_review"),
                            ticket.map(Intent::Review),
                            live,
                            &mut activated,
                        );
                    }
                    ui.horizontal_wrapped(|ui| {
                        for (name, direction, label) in [
                            ("previous", Direction::Previous, aurora_i18n::tr!("host-attention-previous")),
                            ("next", Direction::Next, aurora_i18n::tr!("host-attention-next")),
                        ] {
                            let intent = model.capture_navigation(direction).map(Intent::Open);
                            let response = ui
                                .push_id(("attention_action", name), |ui| {
                                    ui.add_enabled(intent.is_some(), egui::Button::new(label))
                                })
                                .inner;
                            self.control(ui, response, Id::unique(("attention_action", name)), intent, live, &mut activated);
                        }
                        let response = ui
                            .push_id("attention_dismiss", |ui| {
                                ui.add_enabled(ticket.is_some(), egui::Button::new(aurora_i18n::tr!("host-attention-dismiss")))
                            })
                            .inner;
                        self.control(
                            ui,
                            response,
                            Id::unique("attention_dismiss"),
                            ticket.map(Intent::Dismiss),
                            live,
                            &mut activated,
                        );
                    });
                });
                if let Some(key) = self.review_status() {
                    ui.label(RichText::new(aurora_i18n::text(key)).small().color(p.muted));
                }
                if self.review_failed && self.reviewing.is_some() {
                    ui.label(
                        RichText::new(aurora_i18n::tr!("host-attention-review-failed"))
                            .small()
                            .color(p.warn),
                    );
                }
                if self
                    .reviewing
                    .as_ref()
                    .is_some_and(|current| current.cancellation == Some(Cancellation::Failed))
                {
                    ui.label(
                        RichText::new(aurora_i18n::tr!("host-attention-review-cancel-failed"))
                            .small()
                            .color(p.warn),
                    );
                }
                if let Some(request) = self.review_to_cancel() {
                    let enabled = cancel_live
                        && self
                            .reviewing
                            .as_ref()
                            .is_some_and(|current| current.cancellation != Some(Cancellation::Queued));
                    let response = ui
                        .push_id(("attention_review_cancel", request), |ui| {
                            ui.add_enabled(enabled, egui::Button::new(aurora_i18n::tr!("host-attention-review-cancel")))
                                .on_hover_text(aurora_i18n::tr!("host-attention-review-cancel-help"))
                        })
                        .inner;
                    self.control(
                        ui,
                        response,
                        Id::unique(("attention_review_cancel", request)),
                        Some(Intent::CancelReview(request)),
                        enabled,
                        &mut activated,
                    );
                }
                ui.label(
                    RichText::new(aurora_i18n::tr!("host-attention-passive-help"))
                        .small()
                        .color(p.muted),
                );
            });

        egui::CentralPanel::default()
            .frame(widgets::overlay_frame(ui.ctx(), p))
            .show(ui, |ui| {
                // Loading and malformed preferences never reveal stale rows.
                if !available {
                    return;
                }
                if model.selected().is_some_and(|key| !model.rows().iter().any(|row| row.key == key)) {
                    ui.label(RichText::new(aurora_i18n::tr!("host-attention-unavailable")).color(p.warn));
                }
                if model.rows().is_empty() {
                    ui.label(aurora_i18n::tr!("host-attention-empty"));
                    return;
                }
                egui::ScrollArea::vertical()
                    .id_salt("attention_rows")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        #[cfg(test)]
                        ui.ctx()
                            .data_mut(|data| data.insert_temp(Id::unique("attention_list_clip"), ui.clip_rect()));
                        for row in model.rows() {
                            let identity = aurora_i18n::tr!("host-chat-identity", account = row.account.as_str(), grid = row.grid.as_str());
                            let unread = aurora_i18n::tr!("host-attention-unread", count = row.read.unread);
                            let numbered = aurora_i18n::tr!("host-profile-default", number = i64::from(row.slot.number()));
                            let profile = if row.profile.is_empty() {
                                numbered
                            } else {
                                format!("{numbered} · {}", row.profile)
                            };
                            let caption = format!("{profile}\n{identity}\n{} · {unread}", row.title.as_str());
                            let id = chat_ui::source_id(row.key.source).with(("attention_row", row.key.conversation, row.id()));
                            let response = ui
                                .push_id(id, |ui| {
                                    ui.add_enabled(
                                        live,
                                        egui::Button::new(caption)
                                            .selected(model.selected() == Some(row.key))
                                            .wrap()
                                            .min_size(vec2(ui.available_width(), 0.0)),
                                    )
                                })
                                .inner;
                            self.control(ui, response, id, model.capture(row.key).map(Intent::Select), live, &mut activated);
                        }
                    });
            });

        // A release has authority only from a matching press on this widget.
        // The current selection/list index is never consulted to retarget it.
        if ui.ctx().will_discard() {
            return None;
        }
        self.controls = std::mem::take(&mut self.painting);
        #[cfg(all(test, target_os = "linux"))]
        {
            self.native_probes = std::mem::take(&mut self.native_painting);
        }
        if let Some(intent) = activated.filter(|intent| intent.valid(model, state)) {
            self.gesture = None;
            return match intent {
                Intent::Select(ticket) if self.review_to_cancel().is_some() => Some(Action::Select(ticket)),
                Intent::Select(ticket) => {
                    model.select(ticket.key());
                    ui.ctx().request_repaint();
                    None
                }
                Intent::Open(ticket) => Some(Action::OpenChat(ticket)),
                Intent::Review(ticket) => Some(Action::Review(ticket)),
                Intent::CancelReview(request) => Some(Action::CancelReview(request)),
                Intent::Dismiss(ticket) => Some(Action::Dismiss(ticket)),
                Intent::Preferences { desired, .. } => {
                    self.save = Some(Save::Admitting);
                    self.save_failed = false;
                    Some(Action::Preferences(desired))
                }
            };
        }
        let frame = ui.ctx().cumulative_frame_nr();
        if self.gesture.as_ref().is_some_and(|gesture| {
            ui.input(|input| {
                input.events.iter().enumerate().any(|(index, event)| {
                    (gesture.frame != frame || index > gesture.press_index)
                        && match (gesture.trigger, event) {
                            (
                                Trigger::Pointer,
                                egui::Event::PointerButton {
                                    button: egui::PointerButton::Primary,
                                    pressed: false,
                                    ..
                                },
                            ) => true,
                            (
                                Trigger::Key(key),
                                egui::Event::Key {
                                    key: released,
                                    pressed: false,
                                    ..
                                },
                            ) => key == *released,
                            _ => false,
                        }
                })
            })
        }) || self
            .gesture
            .as_ref()
            .is_some_and(|gesture| matches!(gesture.trigger, Trigger::Accessibility))
        {
            self.gesture = None;
        }
        None
    }

    fn control(
        &mut self,
        ui: &egui::Ui,
        response: egui::Response,
        id: Id,
        intent: Option<Intent>,
        live: bool,
        activated: &mut Option<Intent>,
    ) {
        #[cfg(test)]
        ui.ctx().data_mut(|data| {
            data.insert_temp(id.with("rect"), response.rect);
            data.insert_temp(id.with("widget"), response.id);
        });
        #[cfg(all(test, target_os = "linux"))]
        self.native_probe(ui, &response, id, intent, live, false);
        let Some(intent) = intent.filter(|_| live && response.enabled()) else {
            return;
        };
        self.painting.push(PaintedControl {
            id,
            widget: response.id,
            rect: response.interact_rect.intersect(ui.clip_rect()),
            layer: response.layer_id,
            intent,
            #[cfg(all(test, target_os = "linux"))]
            frame: ui.ctx().cumulative_frame_nr(),
        });
        let owns =
            |pos| response.interact_rect.intersect(ui.clip_rect()).contains(pos) && ui.ctx().layer_id_at(pos) == Some(response.layer_id);
        if let Some(gesture) = &self.gesture
            && gesture.widget == id
        {
            let frame = ui.ctx().cumulative_frame_nr();
            let after_press = |index| gesture.frame != frame || index > gesture.press_index;
            let released = match gesture.trigger {
                Trigger::Pointer => ui
                    .input(|input| {
                        input.events.iter().enumerate().find_map(|(index, event)| match event {
                            egui::Event::PointerButton {
                                pos,
                                button: egui::PointerButton::Primary,
                                pressed: false,
                                ..
                            } if after_press(index) => Some(*pos),
                            _ => None,
                        })
                    })
                    .is_some_and(owns),
                Trigger::Key(key) => {
                    response.has_focus()
                        && ui.input(|input| {
                            input.events.iter().enumerate().any(|(index, event)| {
                                matches!(event,
                                    egui::Event::Key { key: released, pressed: false, .. } if *released == key && after_press(index)
                                )
                            })
                        })
                }
                Trigger::Accessibility => {
                    response.has_focus()
                        && ui.input(|input| input.has_accesskit_action_request(response.id, egui::accesskit::Action::Click))
                }
            };
            if released && activated.is_none() {
                *activated = Some(gesture.intent);
            }
        }
    }

    fn capture_press(&mut self, ui: &egui::Ui, model: &Attention, state: &PreferenceState, live: bool, cancel_live: bool) {
        // A native press can arrive before a new snapshot replaces what was
        // painted. Resolve it against those previous pixels and their exact
        // ticket, then validate against current authority. Never lend an old
        // row's pixels the replacement row's ticket. Repeated layout passes
        // cannot reconsider the same press against newly painted controls.
        let previous = &self.controls;
        let frame = ui.ctx().cumulative_frame_nr();
        if self.input_frame == Some(frame) {
            return;
        }
        self.input_frame = Some(frame);
        if !(live || cancel_live) || self.gesture.is_some() {
            return;
        }
        let press = ui.input(|input| {
            input.events.iter().enumerate().find_map(|(index, event)| match event {
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    ..
                } => Some((index, *pos)),
                _ => None,
            })
        });
        let captured = if let Some((index, pos)) = press {
            previous
                .iter()
                .rev()
                .find(|control| control.rect.contains(pos) && ui.ctx().layer_id_at(pos) == Some(control.layer))
                .map(|control| (index, Trigger::Pointer, control))
        } else {
            let ambiguous = ui.input(|input| {
                input
                    .events
                    .iter()
                    .any(|event| matches!(event, egui::Event::Key { key: Key::Tab, .. }))
            });
            let focus = ui.ctx().memory(|memory| memory.focused());
            let key = (!ambiguous)
                .then(|| {
                    ui.input(|input| {
                        input.events.iter().enumerate().find_map(|(index, event)| match event {
                            egui::Event::Key {
                                key,
                                pressed: true,
                                repeat: false,
                                modifiers,
                                ..
                            } if matches!(key, Key::Enter | Key::Space) && modifiers.is_none() => Some((index, *key)),
                            _ => None,
                        })
                    })
                })
                .flatten();
            key.and_then(|(index, key)| {
                previous
                    .iter()
                    .find(|control| Some(control.widget) == focus)
                    .map(|control| (index, Trigger::Key(key), control))
            })
            .or_else(|| {
                previous
                    .iter()
                    .find(|control| {
                        Some(control.widget) == focus
                            && ui.input(|input| input.has_accesskit_action_request(control.widget, egui::accesskit::Action::Click))
                    })
                    .map(|control| (0, Trigger::Accessibility, control))
            })
        };
        if let Some((press_index, trigger, control)) = captured.filter(|(_, _, control)| {
            control.intent.valid(model, state) && (live || (cancel_live && matches!(control.intent, Intent::CancelReview(_))))
        }) {
            self.gesture = Some(Gesture {
                widget: control.id,
                trigger,
                intent: control.intent,
                frame,
                press_index,
            });
        }
    }
}

fn published_checkbox(ui: &mut egui::Ui, p: &Palette, on: bool, label: impl Into<egui::WidgetText>, id: Id) -> egui::Response {
    // A save intent leaves the shared widget's published check mark unchanged.
    ui.push_id(id, |ui| widgets::checkbox_value(ui, p, on, label)).inner
}

#[cfg(all(test, target_os = "linux"))]
pub(super) fn native_fixture_host() -> HostSnapshot {
    tests::host()
}

#[cfg(all(test, target_os = "linux"))]
impl AttentionUi {
    fn native_probe(&mut self, ui: &egui::Ui, response: &egui::Response, id: Id, intent: Option<Intent>, live: bool, presentation: bool) {
        assert!(self.native_painting.len() < super::attention::MAX_ROWS + 16);
        self.native_painting.push(NativeControlProbe {
            id,
            widget: response.id,
            rect: response.rect,
            interact_rect: response.interact_rect,
            clip_rect: ui.clip_rect().intersect(ui.ctx().content_rect()),
            layer: response.layer_id,
            enabled: response.enabled(),
            live,
            frame: ui.ctx().cumulative_frame_nr(),
            intent,
            presentation,
        });
    }

    pub(super) fn native_fixture_control(
        &self,
        id: Id,
        model: &Attention,
        state: &PreferenceState,
        context: &egui::Context,
    ) -> serde_json::Value {
        let Some(probe) = self.native_probes.iter().find(|probe| probe.id == id) else {
            return serde_json::json!({"rect": null, "hit_rect": null, "focused": false,
                "committed": false, "enabled": false, "live": false, "valid": false, "actionable": false, "frame": null});
        };
        let coordinates = |rect: egui::Rect| [rect.min.x, rect.min.y, rect.max.x, rect.max.y];
        let hit = probe.interact_rect.intersect(probe.clip_rect);
        let visible = hit.is_positive();
        let layer_at_hit = visible && context.layer_id_at(hit.center()) == Some(probe.layer);
        let valid = probe.presentation || probe.intent.is_some_and(|intent| intent.valid(model, state));
        let admitted = probe.presentation || (state.writable && self.save.is_none() && !self.ime_composing);
        serde_json::json!({"rect": coordinates(probe.rect), "interact_rect": coordinates(probe.interact_rect),
            "clip_rect": coordinates(probe.clip_rect), "hit_rect": visible.then(|| coordinates(hit)),
            "layer": format!("{:?}", probe.layer), "layer_at_hit": layer_at_hit,
            "focused": context.memory(|memory| memory.focused()) == Some(probe.widget),
            "committed": true, "enabled": probe.enabled, "live": probe.live, "valid": valid,
            "actionable": probe.enabled && probe.live && valid && admitted && layer_at_hit,
            "frame": probe.frame})
    }

    pub(super) fn native_fixture_status(&self, model: &Attention, state: &PreferenceState) -> serde_json::Value {
        let save = self.save.map(|save| match save {
            Save::Admitting => serde_json::json!({"state": "admitting"}),
            Save::Queued { ticket, preferences } => serde_json::json!({"state": "queued", "ticket": ticket, "preferences": preferences}),
        });
        // controls contains only the committed previous paint's live, enabled
        // responses. Focus/modality/surface cancellation clears it before any
        // new frame, so context focus or stale rects alone cannot imply ready.
        let open = self.controls.iter().find_map(|control| match control.intent {
            Intent::Open(ticket) if control.id == Id::unique("attention_open") => Some((control, ticket)),
            _ => None,
        });
        let open_live_valid =
            state.writable && open.is_some_and(|(_, ticket)| model.selected() == Some(ticket.key()) && model.validate(ticket));
        let open_painted = open.map(|(control, ticket)| {
            serde_json::json!({"frame": control.frame,
            "ticket": ticket.native_fixture_status(), "valid": model.validate(ticket),
            "selected": model.selected() == Some(ticket.key()), "live_enabled": true})
        });
        serde_json::json!({"gesture": self.gesture.is_some(), "save": save, "save_failed": self.save_failed,
            "open_painted": open_painted, "open_live_valid": open_live_valid,
            "open_ready": open_live_valid && self.gesture.is_none() && self.save.is_none() && !self.ime_composing,
            "filters_open": self.filters_open, "controls": self.controls.len(), "ime_composing": self.ime_composing})
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod visual;
