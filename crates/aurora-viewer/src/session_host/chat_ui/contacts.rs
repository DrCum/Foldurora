//! Per-surface chooser selection; contact data and immutable Open receipts are
//! source-owned by the controller. No native activation, Send or Read here.
use super::super::chat_broker::OpenStatus;
use super::*;
use aurora_session::ConversationId;
use aurora_session::chat::{ContactKey, ContactText, OpenDirectOutcome, OpenDirectRequest};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OpenDraft {
    pub request: OpenDirectRequest,
    pub instance: u64,
    pub surface: ChatSurface,
    pub generation: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ContactAction {
    Refresh(ChatSource),
    Open(OpenDraft),
    Search {
        source: ChatSource,
        revision: u64,
        query: ContactText<127>,
    },
}
impl ContactAction {
    pub fn source(&self) -> ChatSource {
        match self {
            Self::Refresh(source) => *source,
            Self::Open(draft) => draft.request.selection.source,
            Self::Search { source, .. } => *source,
        }
    }
}
#[derive(Default)]
pub(super) struct ContactsUi {
    visible: bool,
    filter: String,
    query: String,
    search_mode: bool,
    search_key: Option<ContactKey>,
    search_requested: Option<(Option<ContactKey>, Instant)>,
    search_error: bool,
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
        self.search_requested = None;
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
        let next_key = snapshot.and_then(|snapshot| snapshot.contacts.search_key);
        if let Some((previous, since)) = self.search_requested {
            let changed = next_key != previous
                || snapshot.is_some_and(|snapshot| snapshot.contacts.search_loading || snapshot.contacts.search_failed);
            if changed || since.elapsed() > Duration::from_secs(5) {
                self.search_requested = None;
                self.search_error = !changed;
            }
        }
        self.search_key = next_key;
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
        let mut clicked = false;
        ui.horizontal_wrapped(|ui| {
            let friends_button = ui.add_enabled(
                available && matches!(mode, InputMode::Live | InputMode::Editing),
                egui::Button::selectable(self.visible && !self.search_mode, aurora_i18n::tr!("common-friends")),
            );
            #[cfg(test)]
            ui.ctx()
                .data_mut(|data| data.insert_temp(Id::unique("host_contacts_friends_rect"), friends_button.rect));
            let new_button = ui.add_enabled(
                available && matches!(mode, InputMode::Live | InputMode::Editing),
                egui::Button::selectable(self.visible && self.search_mode, aurora_i18n::tr!("host-contacts-new")),
            );
            #[cfg(test)]
            ui.ctx()
                .data_mut(|data| data.insert_temp(Id::unique("host_contacts_new_rect"), new_button.rect));
            if friends_button.clicked() {
                self.visible = !(self.visible && !self.search_mode);
                self.search_mode = false;
                clicked = true;
            }
            if new_button.clicked() {
                self.visible = !(self.visible && self.search_mode);
                self.search_mode = true;
                clicked = true;
            }
        });
        if clicked {
            self.cancel();
            self.selection = None;
        }
        if !self.visible || !available {
            return clicked;
        }
        let snapshot = snapshot.expect("checked snapshot");
        let now = Instant::now();
        if !self.search_mode
            && !snapshot.contacts.loading
            && self
                .refreshed
                .is_none_or(|when| now.saturating_duration_since(when) >= Duration::from_secs(5))
            && mode == InputMode::Live
        {
            self.action = Some(ContactAction::Refresh(source));
        }
        if self.search_mode {
            ui.horizontal_wrapped(|ui| {
                let input = ui.add(
                    TextEdit::singleline(&mut self.query)
                        .hint_text(aurora_i18n::tr!("host-contacts-query"))
                        .char_limit(127)
                        .desired_width(180.0),
                );
                #[cfg(test)]
                ui.ctx()
                    .data_mut(|data| data.insert_temp(Id::unique("host_contacts_query_rect"), input.rect));
                let query = ContactText::new(&self.query).ok().filter(|query| query.as_str().len() >= 2);
                let enabled = mode == InputMode::Live
                    && ui.input(|input| input.focused)
                    && !egui::Popup::is_any_open(ui.ctx())
                    && !snapshot.contacts.search_loading
                    && self.search_requested.is_none()
                    && query.is_some();
                let button = ui.add_enabled(enabled, egui::Button::new(aurora_i18n::tr!("host-contacts-search")));
                #[cfg(test)]
                ui.ctx()
                    .data_mut(|data| data.insert_temp(Id::unique("host_contacts_search_rect"), button.rect));
                if button.clicked()
                    && let (Some(query), Some(policy)) = (query, snapshot.contacts.policy)
                {
                    self.cancel();
                    self.selection = None;
                    self.search_error = false;
                    self.action = Some(ContactAction::Search {
                        source,
                        revision: policy.revision,
                        query,
                    });
                }
            });
            if let Some(query) = &snapshot.contacts.search_query {
                ui.label(aurora_i18n::tr!("host-contacts-results", query = query.as_str()));
            }
            if snapshot.contacts.search_loading || self.search_requested.is_some() {
                ui.label(aurora_i18n::tr!("host-contacts-searching"));
            }
            if snapshot.contacts.search_failed || self.search_error {
                ui.label(aurora_i18n::tr!("host-contacts-search-failed"));
            }
        } else {
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
        }
        let mut double = false;
        if let Some(friends) = if self.search_mode {
            &snapshot.contacts.search
        } else {
            &snapshot.contacts.friends
        } {
            if friends.limited {
                ui.label(aurora_i18n::tr!("host-contacts-limited"));
            }
            let filter = if self.search_mode {
                String::new()
            } else {
                self.filter.to_ascii_lowercase()
            };
            let rows: Vec<_> = friends
                .rows
                .iter()
                .filter(|row| {
                    (self.search_mode || !self.online || row.online)
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
        let enabled = (!self.search_mode || (self.search_requested.is_none() && !snapshot.contacts.search_loading))
            && !snapshot.contacts.open_busy
            && self.pending.is_none()
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
                OpenStatus::Completed(OpenDirectOutcome::Opened { .. }) if operation.presented.is_some() => "host-contacts-opened",
                OpenStatus::Completed(OpenDirectOutcome::Opened { .. }) if snapshot.contacts.open_busy => "host-contacts-opening",
                OpenStatus::Completed(_) => "host-contacts-open-failed",
            };
            ui.label(aurora_i18n::text(message));
        }
        if self.error {
            ui.label(aurora_i18n::tr!("host-contacts-open-failed"));
        }
        clicked
    }
}
impl ChatUi {
    pub fn cancel_contact_open(&mut self) {
        self.view.contacts.cancel();
    }
    pub fn discard_contact_action(&mut self) {
        self.view.contacts.action = None;
    }
    pub fn take_contact_action(&mut self) -> Option<ContactAction> {
        let action = self.view.contacts.action.take();
        match &action {
            Some(ContactAction::Refresh(_)) => self.view.contacts.refreshed = Some(Instant::now()),
            Some(ContactAction::Search { .. }) => {
                self.view.contacts.search_requested = Some((self.view.contacts.search_key, Instant::now()))
            }
            _ => {}
        }
        action
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
                && operation.presented.is_some()
                && operation.presented == native_policy(snapshot, *key)
                && !self.hidden.contains(&key.conversation)
                && native_policy(snapshot, *key).is_some_and(|policy| policy.ready && policy.can_read)
            {
                self.view.contacts.pending = None;
                self.select(key.conversation);
                self.view.contacts.visible = false;
            } else if !matches!(outcome, OpenDirectOutcome::Opened { .. }) || !snapshot.contacts.open_busy {
                self.view.contacts.pending = None;
            }
        } else if matches!(operation.status, OpenStatus::Uncertain) && !snapshot.contacts.open_busy {
            self.view.contacts.pending = None;
        }
    }
}
