//! Bounded, body-free conversation reminders owned by Shell, including while
//! the Attention window is closed. Call `sync` before handling any gesture.
//!
//! Port of collectAttention, navigateAttention and attentionAction
//! (tools/session-host/main.cpp), AttentionNavigator::update/step
//! (indra/newview/fssessionattentionnav.h) and AttentionBook::hiddenUnread/dismiss
//! (indra/newview/fssessionusability.h), pinned to
//! abbe3cb0ca66ee2484f277219f6a27706fab7fc6. Navigation and dismissal are local:
//! neither emits Read, Activate, Send, native Review nor an offer response.

use super::attention_prefs::Preferences;
use super::slots::SlotId;
use super::transport::HostSnapshot;
use aurora_session::chat::{ChatKey, Conversation, MAX_CONVERSATIONS, NativeChatPolicy, ReadMetadata, ReviewCapture};
use aurora_session::{Label, MAX_WORKERS};
use std::collections::VecDeque;

pub(super) const MAX_ROWS: usize = MAX_WORKERS * MAX_CONVERSATIONS;
const MAX_DISMISSALS: usize = 128;

#[derive(Clone, Debug)]
pub(super) struct Row {
    pub key: ChatKey,
    pub policy: NativeChatPolicy,
    pub title: Label,
    pub slot: SlotId,
    /// Presentation only; none of these labels authorize an action.
    pub profile: String,
    pub account: Label,
    pub grid: Label,
    pub read: ReadMetadata,
    /// Monotonic first local observation of this eligible unread reminder.
    /// This is not a native arrival timestamp or cross-worker event clock.
    pub first_observed: u64,
    observation: u64,
}

impl Row {
    /// Stable widget identity for this visible reminder incarnation. Filtered
    /// reappearance and clear allocate a new ID; metadata changes do not.
    pub fn id(&self) -> u64 {
        self.first_observed
    }
}

/// An opaque gesture capture, valid only for this exact row incarnation,
/// policy, native observation and local selection context.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Ticket {
    key: ChatKey,
    policy: NativeChatPolicy,
    read: ReadMetadata,
    observation: u64,
    interaction: u64,
}

impl Ticket {
    pub fn key(self) -> ChatKey {
        self.key
    }

    #[cfg(all(test, target_os = "linux"))]
    pub(super) fn native_fixture_status(self) -> serde_json::Value {
        serde_json::json!({"key": self.key, "policy": self.policy, "read": self.read,
            "observation": self.observation, "interaction": self.interaction})
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Direction {
    Previous,
    Next,
}

/// Only conversation items exist in this increment. ChatKey includes the full
/// source and typed conversation class, so Nearby and Direct never alias.
struct Dismissal {
    key: ChatKey,
    policy: NativeChatPolicy,
    read: ReadMetadata,
}

impl Dismissal {
    fn hides(&self, row: &Row) -> bool {
        self.key == row.key
            && self.policy == row.policy
            && self.read.observed.epoch == row.read.observed.epoch
            && row.read.observed.through <= self.read.observed.through
            && self.read.unread == row.read.unread
    }
}

#[derive(Default)]
pub(super) struct Attention {
    rows: Vec<Row>,
    dismissed: VecDeque<Dismissal>,
    selected: Option<ChatKey>,
    serial: u64,
    interaction: u64,
}

impl Attention {
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// A missing explicit selection remains missing; sync never substitutes a
    /// different sender. Only a fresh explicit navigation/select changes it.
    pub fn selected(&self) -> Option<ChatKey> {
        self.selected
    }

    pub fn select(&mut self, key: ChatKey) -> bool {
        if key.validate().is_err() {
            return false;
        }
        if self.selected != Some(key) {
            let Some(interaction) = self.interaction.checked_add(1) else {
                self.clear();
                return false;
            };
            self.selected = Some(key);
            self.interaction = interaction;
        }
        self.rows.iter().any(|row| row.key == key)
    }

    pub fn clear(&mut self) {
        self.rows.clear();
        self.dismissed.clear();
        self.selected = None;
        // Never recycle row incarnations after a source disappears/reappears.
        self.interaction = self.interaction.saturating_add(1);
    }

    pub fn sync(&mut self, snapshot: &HostSnapshot, preferences: &Preferences) {
        // Build authority before user filters: hiding a slot/category does not
        // discard its reminder book, while policy/source loss always does.
        let mut current = Vec::with_capacity(MAX_ROWS);
        if !snapshot.stopped {
            let mut sources: Vec<_> = snapshot
                .chat
                .iter()
                .filter(|chat| chat.authenticated && chat.live && chat.source.validate().is_ok())
                .filter_map(|chat| {
                    let worker = snapshot
                        .workers
                        .iter()
                        .find(|worker| worker.binding == chat.source.binding() && worker.reachable)?;
                    // Stable saved slots own filters; legacy unslotted workers
                    // cannot borrow another profile's persisted preference.
                    let slot = worker.slot?;
                    Some((slot, worker, chat))
                })
                .collect();
            sources.sort_by_key(|(slot, worker, _)| (u8::from(*slot), worker.binding.worker.worker));
            sources.dedup_by_key(|(_, _, chat)| chat.source);
            for (slot, worker, chat) in sources.into_iter().take(MAX_WORKERS) {
                let mut catalogue: Vec<_> = chat.conversations.iter().take(MAX_CONVERSATIONS).collect();
                // Simultaneous discoveries have no provable native order.
                // Use stable typed identity, never label or roster position.
                catalogue.sort_by_key(|info| info.policy.key.conversation);
                catalogue.dedup_by_key(|info| info.policy.key);
                for info in catalogue {
                    let policy = info.policy;
                    let key = policy.key;
                    if key.source != chat.source
                        || policy.validate().is_err()
                        || !policy.lease_current
                        || !policy.ready
                        || !policy.can_read
                        || !matches!(key.conversation, Conversation::Nearby | Conversation::Direct(_))
                    {
                        continue;
                    }
                    let Some(read) = chat.reads.get(&key.conversation).copied().filter(|read| read.validate().is_ok()) else {
                        continue;
                    };
                    let profile = snapshot
                        .slots
                        .iter()
                        .find(|saved| saved.id == slot)
                        .map(|saved| saved.label.as_str())
                        .unwrap_or_default()
                        .to_owned();
                    let (account, grid) = worker
                        .status
                        .as_ref()
                        .map(|status| (status.name.clone(), status.grid_label.clone()))
                        .unwrap_or_default();
                    current.push(Row {
                        key,
                        policy,
                        title: info.title.clone(),
                        slot,
                        profile,
                        account,
                        grid,
                        read,
                        first_observed: 0,
                        observation: 0,
                    });
                }
            }
        }
        // Keep the bounded tombstone while its native identity/epoch/policy
        // exists, even when its count changes. A count can round-trip without
        // an incoming advance (e.g. native local/own counting effects), making
        // the original comparator hide it again. Reappearance is not erasure.
        self.dismissed.retain(|dismissal| {
            current.iter().any(|row| {
                row.key == dismissal.key && row.policy == dismissal.policy && row.read.observed.epoch == dismissal.read.observed.epoch
            })
        });
        current.retain(|row| {
            // can_mark_read is native mutation capacity, not permission to
            // display unread or navigate. can_send is equally independent.
            row.read.unread > 0
                && preferences.include_slots[usize::from(u8::from(row.slot))]
                && match row.key.conversation {
                    Conversation::Nearby => preferences.nearby,
                    Conversation::Direct(_) => preferences.direct,
                    _ => false,
                }
                && !self.dismissed.iter().any(|dismissal| dismissal.hides(row))
        });
        for row in &mut current {
            let previous = self.rows.iter().find(|old| old.key == row.key && old.policy == row.policy);
            if let Some(previous) = previous.filter(|old| old.read == row.read) {
                row.first_observed = previous.first_observed;
                row.observation = previous.observation;
            } else {
                let Some(serial) = self.serial.checked_add(1) else {
                    self.clear();
                    return;
                };
                self.serial = serial;
                row.first_observed = previous.map(|old| old.first_observed).unwrap_or(serial);
                row.observation = serial;
            }
        }
        // AttentionNavigator default is offers first, ascending event order;
        // oldest_first removes offer priority. All present rows are the same
        // conversation class, so both preferences use ascending observation.
        current.sort_by_key(|row| row.first_observed);
        self.rows = current;
    }

    pub fn capture(&self, key: ChatKey) -> Option<Ticket> {
        let row = self.rows.iter().find(|row| row.key == key)?;
        Some(Ticket {
            key,
            policy: row.policy,
            read: row.read,
            observation: row.observation,
            interaction: self.interaction,
        })
    }

    pub fn validate(&self, ticket: Ticket) -> bool {
        ticket.interaction == self.interaction
            && self.rows.iter().any(|row| {
                row.key == ticket.key && row.policy == ticket.policy && row.read == ticket.read && row.observation == ticket.observation
            })
    }

    /// The click retains the original policy/observation, never a refreshed
    /// selected row. Across handoff, the broker independently validates the
    /// catalogue incarnation rather than Attention's transient unread filter.
    /// Port of reviewChat (tools/session-host/main.cpp, pinned above).
    pub fn review_capture(&self, ticket: Ticket) -> Option<ReviewCapture> {
        self.validate(ticket)
            .then_some(ReviewCapture {
                key: ticket.key,
                policy_revision: ticket.policy.revision,
                observed: ticket.read.observed,
            })
            .filter(|target| target.validate().is_ok())
    }

    /// Returns only an exact canonical Shared Chat target for Shell to open.
    /// Shell retains its existing edit-prefix/navigation fences and composer.
    pub fn open_chat(&mut self, ticket: Ticket) -> Option<ChatKey> {
        if !self.validate(ticket) {
            return None;
        }
        self.select(ticket.key).then_some(ticket.key)
    }

    pub fn dismiss(&mut self, ticket: Ticket) -> bool {
        if !self.validate(ticket) {
            return false;
        }
        if let Some(existing) = self.dismissed.iter_mut().find(|entry| entry.key == ticket.key) {
            existing.policy = ticket.policy;
            existing.read = ticket.read;
        } else {
            if self.dismissed.len() == MAX_DISMISSALS {
                self.dismissed.pop_front();
            }
            self.dismissed.push_back(Dismissal {
                key: ticket.key,
                policy: ticket.policy,
                read: ticket.read,
            });
        }
        self.rows.retain(|row| row.key != ticket.key);
        true
    }

    /// Capture Next/Previous when a gesture starts, then call `open_chat` on
    /// release. Recomputing direction on release could choose a different row.
    pub fn capture_navigation(&self, direction: Direction) -> Option<Ticket> {
        if self.rows.is_empty() {
            return None;
        }
        let selected = self.rows.iter().position(|row| Some(row.key) == self.selected);
        let index = match (selected, direction) {
            (Some(index), Direction::Next) => (index + 1) % self.rows.len(),
            (Some(index), Direction::Previous) => (index + self.rows.len() - 1) % self.rows.len(),
            (None, Direction::Next) => 0,
            (None, Direction::Previous) => self.rows.len() - 1,
        };
        self.capture(self.rows[index].key)
    }
}

#[cfg(test)]
mod tests;
