//! Per-surface chooser selection; contact data and immutable Open receipts are
//! source-owned by the controller. No native activation, Send or Read here.
use super::super::chat_broker::OpenStatus;
use super::*;
use aurora_session::ConversationId;
use aurora_session::chat::{ContactKey, OpenDirectOutcome, OpenDirectRequest};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OpenDraft {
    pub request: OpenDirectRequest,
    pub instance: u64,
    pub surface: ChatSurface,
    pub generation: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ContactAction {
    Refresh(ChatSource),
    Open(OpenDraft),
}
impl ContactAction {
    pub fn source(self) -> ChatSource {
        match self {
            Self::Refresh(source) => source,
            Self::Open(draft) => draft.request.selection.source,
        }
    }
}
#[derive(Default)]
pub(super) struct ContactsUi {
    visible: bool,
    filter: String,
    online: bool,
    selection: Option<(ContactKey, ConversationId, u64)>,
    policy: Option<u64>,
    refreshed: Option<Instant>,
    pub action: Option<ContactAction>,
    pub pending: Option<OpenDraft>,
    pub(super) generation: u64,
    error: bool,
}
impl ContactsUi {
    pub fn cancel(&mut self) {
        self.pending = None;
        self.action = None;
        self.generation = self.generation.saturating_add(1);
    }
    pub fn sync(&mut self, snapshot: Option<&HostChatSnapshot>) {
        let policy = snapshot
            .and_then(|snapshot| snapshot.contacts.policy)
            .filter(|policy| policy.can_browse)
            .map(|policy| policy.revision);
        if policy != self.policy {
            self.cancel();
            self.selection = None;
            self.refreshed = None;
        }
        self.policy = policy;
        if policy.is_none() {
            self.visible = false;
            self.selection = None;
        }
        if self
            .selection
            .is_some_and(|(key, resident, revision)| !snapshot.is_some_and(|snapshot| snapshot.contacts.selected(key, resident, revision)))
        {
            self.selection = None;
        }
    }
    pub fn queued(&mut self, draft: OpenDraft, ticket: u64) {
        if self.generation == draft.generation && ticket > 0 {
            self.error = false;
            self.pending = Some(OpenDraft {
                request: OpenDirectRequest {
                    request: ticket,
                    ..draft.request
                },
                ..draft
            });
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        snapshot: Option<&HostChatSnapshot>,
        source: ChatSource,
        instance: u64,
        surface: ChatSurface,
        mode: InputMode,
    ) -> bool {
        self.action = None;
        let available = snapshot.is_some_and(|snapshot| snapshot.contacts.policy.is_some_and(|policy| policy.can_browse));
        let friends_button = ui.add_enabled(
            available && matches!(mode, InputMode::Live | InputMode::Editing),
            egui::Button::selectable(self.visible, aurora_i18n::tr!("common-friends")),
        );
        #[cfg(test)]
        ui.ctx()
            .data_mut(|data| data.insert_temp(Id::unique("host_contacts_friends_rect"), friends_button.rect));
        let clicked = friends_button.clicked();
        if clicked {
            self.visible = !self.visible;
            self.cancel();
        }
        if !self.visible || !available {
            return clicked;
        }
        let snapshot = snapshot.expect("checked snapshot");
        let now = Instant::now();
        if !snapshot.contacts.loading
            && self
                .refreshed
                .is_none_or(|when| now.saturating_duration_since(when) >= Duration::from_secs(5))
            && mode == InputMode::Live
        {
            self.action = Some(ContactAction::Refresh(source));
            self.refreshed = Some(now);
        }
        ui.horizontal_wrapped(|ui| {
            ui.add(
                TextEdit::singleline(&mut self.filter)
                    .hint_text(aurora_i18n::tr!("host-contacts-filter"))
                    .char_limit(127)
                    .desired_width(180.0),
            );
            ui.checkbox(&mut self.online, aurora_i18n::tr!("host-contacts-online"));
        });
        if snapshot.contacts.loading {
            ui.label(aurora_i18n::tr!("host-contacts-loading"));
        }
        if snapshot.contacts.failed {
            ui.label(aurora_i18n::tr!("host-contacts-failed"));
        }
        let mut double = false;
        if let Some(friends) = &snapshot.contacts.friends {
            if friends.limited {
                ui.label(aurora_i18n::tr!("host-contacts-limited"));
            }
            let filter = self.filter.to_ascii_lowercase();
            let rows: Vec<_> = friends
                .rows
                .iter()
                .filter(|row| {
                    (!self.online || row.online)
                        && (filter.is_empty()
                            || format!("{} {}", row.display.as_str(), row.username.as_str())
                                .to_ascii_lowercase()
                                .contains(&filter))
                })
                .collect();
            egui::ScrollArea::vertical()
                .id_salt((source_id(source), surface.index(), "contacts"))
                .max_height(190.0)
                .show_rows(ui, 22.0, rows.len(), |ui, range| {
                    for index in range {
                        let row = rows[index];
                        let revision = friends.policy_revision;
                        let target = (friends.key, row.resident, revision);
                        let caption = format!(
                            "{} {}{}",
                            if row.online { "●" } else { "○" },
                            row.display.as_str(),
                            if row.username.as_str().is_empty() {
                                String::new()
                            } else {
                                format!(" ({})", row.username.as_str())
                            }
                        );
                        let response = ui.add_enabled(
                            matches!(mode, InputMode::Live | InputMode::Editing),
                            egui::Button::selectable(self.selection == Some(target), caption).truncate(),
                        );
                        #[cfg(test)]
                        ui.ctx()
                            .data_mut(|data| data.insert_temp(Id::unique(("host_contact_row_rect", row.resident)), response.rect));
                        if response.clicked() || response.double_clicked() {
                            self.selection = Some(target);
                        }
                        double |= response.double_clicked();
                    }
                });
        }
        let enabled = self.pending.is_none()
            && self.generation != u64::MAX
            && mode == InputMode::Live
            && ui.input(|input| input.focused)
            && !egui::Popup::is_any_open(ui.ctx())
            && self
                .selection
                .is_some_and(|(key, resident, revision)| snapshot.contacts.selected(key, resident, revision));
        let open = ui.add_enabled(enabled, egui::Button::new(aurora_i18n::tr!("host-contacts-open")));
        #[cfg(test)]
        ui.ctx()
            .data_mut(|data| data.insert_temp(Id::unique("host_contacts_open_rect"), open.rect));
        if enabled
            && (open.clicked() || double)
            && let Some((selection, resident, policy_revision)) = self.selection
        {
            self.action = Some(ContactAction::Open(OpenDraft {
                request: OpenDirectRequest {
                    selection,
                    resident,
                    policy_revision,
                    request: 0,
                },
                instance,
                surface,
                generation: self.generation,
            }));
        }
        if let Some(operation) = snapshot.contacts.opens.last() {
            let message = match &operation.status {
                OpenStatus::Pending => "host-contacts-opening",
                OpenStatus::Uncertain => "host-contacts-uncertain",
                OpenStatus::Completed(OpenDirectOutcome::Opened { .. }) => "host-contacts-opened",
                OpenStatus::Completed(_) => "host-contacts-open-failed",
            };
            ui.label(aurora_i18n::text(message));
        }
        if self.error {
            ui.label(aurora_i18n::tr!("host-contacts-open-failed"));
        }
        clicked || self.action.is_some_and(|action| matches!(action, ContactAction::Open(_)))
    }
}
impl ChatUi {
    pub fn cancel_contact_open(&mut self) {
        self.view.contacts.cancel();
    }
    pub fn take_contact_action(&mut self) -> Option<ContactAction> {
        self.view.contacts.action.take()
    }
    pub fn open_queue_failed(&mut self, draft: OpenDraft) {
        if draft.instance == self.instance && draft.request.selection.source == self.source {
            self.with_surface(draft.surface, |model| {
                if model.view.contacts.generation == draft.generation {
                    model.view.contacts.error = true;
                }
            });
        }
    }
    pub fn queued_open(&mut self, draft: OpenDraft, ticket: u64) {
        if draft.instance == self.instance && draft.request.selection.source == self.source {
            self.with_surface(draft.surface, |model| model.view.contacts.queued(draft, ticket));
        }
    }
    pub(super) fn sync_contact_open(&mut self, current: Option<&HostChatSnapshot>) {
        self.view.contacts.sync(current);
        let Some(pending) = self.view.contacts.pending else {
            return;
        };
        let Some(snapshot) = current else {
            self.view.contacts.cancel();
            return;
        };
        let Some(operation) = snapshot
            .contacts
            .opens
            .iter()
            .find(|operation| operation.request == pending.request)
        else {
            return;
        };
        if let OpenStatus::Completed(outcome) = &operation.status {
            if let OpenDirectOutcome::Opened { key, .. } = outcome
                && !self.hidden.contains(&key.conversation)
                && native_policy(snapshot, *key).is_some_and(|policy| policy.ready && policy.can_read)
            {
                self.view.contacts.pending = None;
                self.select(key.conversation);
                self.view.contacts.visible = false;
            } else {
                self.view.contacts.pending = None;
            }
        }
    }
}
