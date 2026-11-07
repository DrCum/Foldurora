//! Native read bookkeeping, independent of transcript export order. Only the
//! bounded conversations observed by the host are retained; native counters
//! and native read-on-draw behavior remain authoritative.

use super::World;
use std::collections::BTreeMap;
use uuid::Uuid;

pub(crate) const MAX_INTEREST: usize = 64;
pub(crate) type Conversation = Option<Uuid>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ReadState {
    pub epoch: u64,
    pub incoming: u64,
    pub revision: u64,
    pub unread: u64,
}

struct Entry {
    token: u64,
    raw: usize,
    badge: usize,
    state: Option<ReadState>,
}

#[derive(Default)]
pub(crate) struct ReadLedger {
    entries: BTreeMap<Conversation, Entry>,
    epoch: u64,
}

impl ReadLedger {
    pub fn reconcile(&mut self, current: &[(Conversation, u64, usize, usize)]) {
        self.entries
            .retain(|key, _| current.iter().take(MAX_INTEREST).any(|row| row.0 == *key));
        for &(key, token, raw, badge) in current.iter().take(MAX_INTEREST) {
            if self.entries.get(&key).is_none_or(|entry| entry.token != token) {
                let epoch = self.epoch.checked_add(1);
                // Exhaustion never permits an old observation to become valid.
                self.epoch = epoch.unwrap_or(u64::MAX);
                self.entries.insert(
                    key,
                    Entry {
                        token,
                        raw,
                        badge,
                        state: epoch.map(|epoch| ReadState {
                            epoch,
                            incoming: 0,
                            revision: 1,
                            unread: badge as u64,
                        }),
                    },
                );
            } else {
                self.record(key, token, raw, badge, false);
            }
        }
    }

    pub fn record(&mut self, key: Conversation, token: u64, raw: usize, badge: usize, incoming: bool) {
        let Some(entry) = self.entries.get_mut(&key) else { return };
        // Reconcile issues a fresh epoch before the next operation. Never
        // relabel a new list's arrival with an old list's identity meanwhile.
        if entry.token != token {
            entry.state = None;
            return;
        }
        if entry.raw == raw && entry.badge == badge && !incoming {
            return;
        }
        entry.raw = raw;
        entry.badge = badge;
        entry.state = entry.state.and_then(|state| {
            Some(ReadState {
                incoming: if incoming { state.incoming.checked_add(1)? } else { state.incoming },
                revision: state.revision.checked_add(1)?,
                unread: u64::try_from(badge).ok()?,
                ..state
            })
        });
    }

    pub fn interested(&self, key: Conversation) -> bool {
        self.entries.contains_key(&key)
    }

    pub fn state(&self, key: Conversation) -> Option<ReadState> {
        self.entries.get(&key)?.state
    }

    pub fn retire(&mut self, key: Conversation) {
        self.entries.remove(&key);
    }
}

impl World {
    pub(crate) fn read_counters(&self, key: Conversation) -> Option<(u64, usize, usize)> {
        match key {
            None => Some((self.chat_clock.token, self.chat_unread, self.chat_badge)),
            Some(id) => self
                .social
                .ims
                .iter()
                .find(|session| session.other == id)
                .map(|session| (session.clock.token, session.unread, session.unread)),
        }
    }

    pub(super) fn record_read_change(&mut self, key: Conversation, incoming: bool) {
        if !self.read_ledger.interested(key) {
            return;
        }
        if let Some((token, raw, badge)) = self.read_counters(key) {
            self.read_ledger.record(key, token, raw, badge, incoming);
        }
    }

    /// Common counter mutation used by both native chat styles and the
    /// explicit host action. No session creation or offer response occurs.
    pub(crate) fn mark_conversation_read(&mut self, key: Conversation) -> Option<(usize, usize)> {
        let result = match key {
            None => {
                self.chat_badge = 0;
                (std::mem::take(&mut self.chat_unread), self.chat.len())
            }
            Some(id) => {
                let session = self.social.ims.iter_mut().find(|session| session.other == id)?;
                (std::mem::take(&mut session.unread), session.lines.len())
            }
        };
        self.record_read_change(key, false);
        Some(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_read_ledger_detects_equal_count_replacement_and_ignores_zero_reads() {
        let mut ledger = ReadLedger::default();
        ledger.reconcile(&[(None, 1, 1, 1)]);
        let a = ledger.state(None).unwrap();
        ledger.record(None, 1, 0, 0, false);
        let read = ledger.state(None).unwrap();
        ledger.record(None, 1, 0, 0, false);
        assert_eq!(ledger.state(None), Some(read));
        ledger.record(None, 1, 1, 1, true);
        let b = ledger.state(None).unwrap();
        assert_eq!(a.unread, b.unread);
        assert!(b.incoming > a.incoming && b.revision > a.revision);
        ledger.reconcile(&[(None, 2, 1, 1)]);
        assert_ne!(ledger.state(None).unwrap().epoch, a.epoch);
    }

    #[test]
    fn native_read_ledger_bounds_interest_and_fails_closed_on_overflow() {
        let mut ledger = ReadLedger::default();
        let rows: Vec<_> = (0..100).map(|i| (Some(Uuid::from_u128(i + 1)), 1, 0, 0)).collect();
        ledger.reconcile(&rows);
        assert_eq!(ledger.entries.len(), MAX_INTEREST);
        ledger.epoch = u64::MAX;
        ledger.reconcile(&[(None, 1, 0, 0)]);
        assert_eq!(ledger.state(None), None);
    }
}
