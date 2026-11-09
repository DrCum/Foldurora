//! Filtered native observations. This journal has no UI preference, forwarding
//! action or receive-channel listener. Only the existing accepted chat path
//! writes it; readers still enforce current native ownership and permissions.
use super::World;
use aurora_net::{ChatMessage, ChatSourceType, ChatType, LoginSource, RegionHandle};
use std::{
    collections::{BTreeMap, VecDeque},
    net::SocketAddr,
    time::Instant,
};
use uuid::Uuid;
#[derive(Clone)]
pub(crate) struct DialogSource {
    pub source: LoginSource,
    pub circuit: Uuid,
    pub region: RegionHandle,
    pub sender: SocketAddr,
}
#[derive(Clone)]
pub(crate) struct Entry {
    pub serial: u64,
    pub object: Uuid,
    pub text: String,
    pub error: bool,
    pub at: Instant,
}
#[derive(Default)]
pub(crate) struct Observations {
    pub enabled: bool,
    pub serial: u64,
    pub epoch: u64,
    pub entries: VecDeque<Entry>,
    pub dialogs: BTreeMap<u64, DialogSource>,
    bytes: usize,
}
impl Observations {
    pub fn set_enabled(&mut self, enabled: bool) {
        if self.enabled != enabled {
            self.clear();
            self.enabled = enabled;
        }
    }
    pub fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
        self.serial = 0;
        self.epoch = self.epoch.saturating_add(1);
    }
    pub fn record(&mut self, message: &ChatMessage, agent: Uuid) {
        if !self.enabled
            || agent.is_nil()
            || message.owner_id != agent
            || message.source_id.is_nil()
            || message.source_type != ChatSourceType::Object
            || !matches!(message.chat_type, ChatType::Debug | ChatType::OwnerSay)
        {
            return;
        }
        let Some(serial) = self.serial.checked_add(1) else { return };
        self.serial = serial;
        let mut end = message.message.len().min(4096);
        while !message.message.is_char_boundary(end) {
            end -= 1;
        }
        let text = message.message[..end].to_owned();
        self.bytes += text.len();
        self.entries.push_back(Entry {
            serial,
            object: message.source_id,
            text,
            error: message.chat_type == ChatType::Debug,
            at: Instant::now(),
        });
        while self.entries.len() > 2048 || self.bytes > 2 * 1024 * 1024 {
            if let Some(old) = self.entries.pop_front() {
                self.bytes = self.bytes.saturating_sub(old.text.len());
            } else {
                break;
            }
        }
    }
}
impl World {
    pub(crate) fn assistant_diagnostic_allowed(&self, object: Uuid, text: &str) -> bool {
        let r = &self.rlva;
        let emote = super::rlva::is_emote(text);
        let (all, specific) = if emote {
            ("recvemote", "recvemotefrom")
        } else {
            ("recvchat", "recvchatfrom")
        };
        !r.has("viewscript")
            && self.shared_chat_privacy_allowed()
            && r.allows(all, &object.to_string())
            && !r.targets(specific, &object.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn journal_is_bounded_filtered_and_gaps_invalidate_prior_capture_epochs() {
        let agent = Uuid::new_v4();
        let object = Uuid::new_v4();
        let mut journal = Observations::default();
        journal.set_enabled(true);
        let mut message = ChatMessage {
            from_name: "untrusted".into(),
            source_id: object,
            owner_id: agent,
            source_type: ChatSourceType::Object,
            chat_type: ChatType::OwnerSay,
            position: glam::Vec3::ZERO,
            message: "é".repeat(4096),
        };
        for _ in 0..3000 {
            journal.record(&message, agent);
        }
        assert!(journal.entries.len() <= 2048);
        assert!(journal.bytes <= 2 * 1024 * 1024);
        assert_eq!(journal.serial, 3000);
        assert!(journal.entries.front().unwrap().serial > 1);
        assert!(journal.entries.iter().all(|e| e.text.len() <= 4096));
        let serial = journal.serial;
        message.owner_id = Uuid::new_v4();
        journal.record(&message, agent);
        message.owner_id = agent;
        message.source_type = ChatSourceType::Agent;
        journal.record(&message, agent);
        message.source_type = ChatSourceType::Object;
        message.chat_type = ChatType::Normal;
        journal.record(&message, agent);
        assert_eq!(journal.serial, serial);
        let epoch = journal.epoch;
        journal.set_enabled(false);
        assert!(journal.entries.is_empty());
        assert_ne!(epoch, journal.epoch);
        journal.set_enabled(true);
        assert_ne!(epoch, journal.epoch);
    }
}
