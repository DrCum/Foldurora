//! Source-bound contact transfers and acknowledged model-only Direct opening.
//! Folderstorm abbe3cb0 fssessioncontacts.h/fssessionworker.cpp (LGPL-2.1).
//! Native friendship/search membership, privacy and permission remain with the
//! worker. A row's can_open bit is presentation, never native authorization.
use super::{ChatError, ChatKey, ChatSource, Conversation, MAX_RECEIPTS, NativeChatPolicy, changed_source};
use crate::{ConversationId, Label};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::{BTreeSet, VecDeque};

pub const MAX_FRIEND_CONTACTS: usize = 2048;
pub const MAX_SEARCH_CONTACTS: usize = 100;
pub const MAX_CONTACT_PAGE_ROWS: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct ContactText<const LIMIT: usize>(String);
impl<const LIMIT: usize> ContactText<LIMIT> {
    pub fn new(text: &str) -> Result<Self, ChatError> {
        if text.len() > LIMIT || text.chars().any(char::is_control) {
            return Err(ChatError::Invalid);
        }
        Ok(Self(text.into()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl<'de, const LIMIT: usize> Deserialize<'de> for ContactText<LIMIT> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(&String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContactChannel {
    Friends,
    Search,
}
impl ContactChannel {
    fn index(self) -> usize {
        match self {
            Self::Friends => 0,
            Self::Search => 1,
        }
    }
    pub fn limit(self) -> usize {
        match self {
            Self::Friends => MAX_FRIEND_CONTACTS,
            Self::Search => MAX_SEARCH_CONTACTS,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContactKey {
    pub source: ChatSource,
    pub channel: ContactChannel,
    pub token: u64,
}
impl ContactKey {
    pub fn validate(self) -> Result<(), ChatError> {
        self.source.validate()?;
        if self.token == 0 {
            return Err(ChatError::Sequence);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContactRow {
    pub resident: ConversationId,
    pub display: ContactText<255>,
    pub username: ContactText<127>,
    pub online: bool,
    pub can_open: bool,
}
impl ContactRow {
    pub fn validate(&self) -> Result<(), ChatError> {
        if self.display.as_str().is_empty() {
            return Err(ChatError::Invalid);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContactsPolicy {
    pub source: ChatSource,
    pub revision: u64,
    pub can_browse: bool,
}
impl ContactsPolicy {
    pub fn validate(self) -> Result<(), ChatError> {
        self.source.validate()?;
        if self.revision == 0 {
            return Err(ChatError::StalePolicy);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContactPageRequest {
    pub key: ContactKey,
    pub offset: u16,
}
impl ContactPageRequest {
    pub fn validate(self) -> Result<(), ChatError> {
        self.key.validate()?;
        if usize::from(self.offset) > self.key.channel.limit() {
            return Err(ChatError::Invalid);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContactPage {
    pub key: ContactKey,
    pub policy_revision: u64,
    pub offset: u16,
    pub total: u16,
    pub limited: bool,
    pub pending: bool,
    pub rows: Vec<ContactRow>,
}
impl ContactPage {
    pub fn validate(&self) -> Result<(), ChatError> {
        self.key.validate()?;
        if self.policy_revision == 0 {
            return Err(ChatError::StalePolicy);
        }
        let (offset, total) = (usize::from(self.offset), usize::from(self.total));
        if self.rows.len() > MAX_CONTACT_PAGE_ROWS || total > self.key.channel.limit() || offset > total || self.rows.len() > total - offset
        {
            return Err(ChatError::Capacity);
        }
        if self.pending {
            if offset != 0 || total != 0 || self.limited || !self.rows.is_empty() {
                return Err(ChatError::Invalid);
            }
        } else if offset < total && self.rows.is_empty() {
            return Err(ChatError::Invalid);
        }
        let mut seen = BTreeSet::new();
        for row in &self.rows {
            row.validate()?;
            if !seen.insert(row.resident) {
                return Err(ChatError::Invalid);
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletedContacts {
    pub key: ContactKey,
    pub policy_revision: u64,
    pub limited: bool,
    pub rows: Vec<ContactRow>,
}
struct Transfer {
    key: ContactKey,
    total: Option<(u16, bool)>,
    rows: Vec<ContactRow>,
}

/// Owned by the source broker, not by a disposable chooser window. Hiding or
/// revoking rows retains token high-water marks, so an old gesture cannot revive.
pub struct ContactDirectory {
    source: ChatSource,
    policy: Option<ContactsPolicy>,
    highwater: [u64; 2],
    transfers: [Option<Transfer>; 2],
    completed: [Option<CompletedContacts>; 2],
}
impl ContactDirectory {
    pub fn new(source: ChatSource) -> Result<Self, ChatError> {
        source.validate()?;
        Ok(Self {
            source,
            policy: None,
            highwater: [0; 2],
            transfers: [None, None],
            completed: [None, None],
        })
    }
    pub fn reset(&mut self, source: ChatSource) -> Result<bool, ChatError> {
        if !changed_source(self.source, source)? {
            return Ok(false);
        }
        *self = Self::new(source)?;
        Ok(true)
    }
    pub fn clear(&mut self) {
        self.transfers = [None, None];
        self.completed = [None, None];
    }
    pub fn set_policy(&mut self, policy: ContactsPolicy) -> Result<(), ChatError> {
        policy.validate()?;
        if policy.source != self.source {
            return Err(ChatError::Binding);
        }
        if let Some(old) = self.policy {
            if policy.revision < old.revision {
                return Err(ChatError::StalePolicy);
            }
            if policy.revision == old.revision && policy != old {
                return Err(ChatError::StalePolicy);
            }
        }
        if self.policy != Some(policy) || !policy.can_browse {
            self.clear();
        }
        self.policy = Some(policy);
        Ok(())
    }
    pub fn begin(&mut self, channel: ContactChannel, token: u64) -> Result<ContactPageRequest, ChatError> {
        if !self.policy.is_some_and(|policy| policy.can_browse) {
            return Err(ChatError::Denied);
        }
        let key = ContactKey {
            source: self.source,
            channel,
            token,
        };
        key.validate()?;
        let index = channel.index();
        if token <= self.highwater[index] {
            return Err(ChatError::StaleRequest);
        }
        self.highwater[index] = token;
        // Folderstorm beginSearch discards the previous query's membership;
        // beginFriends may keep its completed list during a refresh.
        if channel == ContactChannel::Search {
            self.completed[index] = None;
        }
        self.transfers[index] = Some(Transfer {
            key,
            total: None,
            rows: Vec::new(),
        });
        Ok(ContactPageRequest { key, offset: 0 })
    }
    pub fn fail(&mut self, key: ContactKey) -> Result<(), ChatError> {
        key.validate()?;
        if key.source != self.source {
            return Err(ChatError::Binding);
        }
        let index = key.channel.index();
        if self.transfers[index].as_ref().is_none_or(|transfer| transfer.key != key) {
            return Err(ChatError::StaleRequest);
        }
        self.transfers[index] = None;
        Ok(())
    }
    pub fn request(&self, channel: ContactChannel) -> Option<ContactPageRequest> {
        let transfer = self.transfers[channel.index()].as_ref()?;
        Some(ContactPageRequest {
            key: transfer.key,
            offset: transfer.rows.len() as u16,
        })
    }
    pub fn completed(&self, channel: ContactChannel) -> Option<&CompletedContacts> {
        self.policy.filter(|policy| policy.can_browse)?;
        self.completed[channel.index()].as_ref()
    }
    pub fn selected(&self, key: ContactKey, resident: ConversationId) -> bool {
        self.completed(key.channel)
            .is_some_and(|complete| complete.key == key && complete.rows.iter().any(|row| row.resident == resident && row.can_open))
    }
    pub fn accept(&mut self, page: &ContactPage) -> Result<bool, ChatError> {
        page.validate()?;
        if page.key.source != self.source {
            return Err(ChatError::Binding);
        }
        let policy = self.policy.ok_or(ChatError::Denied)?;
        if !policy.can_browse {
            return Err(ChatError::Denied);
        }
        if page.policy_revision != policy.revision {
            return Err(ChatError::StalePolicy);
        }
        let index = page.key.channel.index();
        let transfer = self.transfers[index].as_mut().ok_or(ChatError::StaleRequest)?;
        if transfer.key != page.key || transfer.rows.len() != usize::from(page.offset) {
            return Err(ChatError::StaleRequest);
        }
        if page.pending {
            return Ok(false);
        }
        if transfer.total.is_some_and(|total| total != (page.total, page.limited)) {
            return Err(ChatError::Invalid);
        }
        let mut seen: BTreeSet<_> = transfer.rows.iter().map(|row| row.resident).collect();
        if page.rows.iter().any(|row| !seen.insert(row.resident)) {
            return Err(ChatError::Invalid);
        }
        transfer.total = Some((page.total, page.limited));
        transfer.rows.extend_from_slice(&page.rows);
        if transfer.rows.len() != usize::from(page.total) {
            return Ok(false);
        }
        let mut transfer = self.transfers[index].take().unwrap();
        transfer.rows.sort_by(|a, b| {
            b.online
                .cmp(&a.online)
                .then_with(|| {
                    a.display
                        .as_str()
                        .to_ascii_lowercase()
                        .cmp(&b.display.as_str().to_ascii_lowercase())
                })
                .then_with(|| a.username.as_str().cmp(b.username.as_str()))
                .then_with(|| a.resident.cmp(&b.resident))
        });
        self.completed[index] = Some(CompletedContacts {
            key: page.key,
            policy_revision: page.policy_revision,
            limited: page.limited,
            rows: transfer.rows,
        });
        Ok(true)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenDirectRequest {
    pub selection: ContactKey,
    pub resident: ConversationId,
    pub request: u64,
    pub policy_revision: u64,
}
impl OpenDirectRequest {
    pub fn validate(self) -> Result<(), ChatError> {
        self.selection.validate()?;
        if self.request == 0 {
            return Err(ChatError::Sequence);
        }
        if self.policy_revision == 0 {
            return Err(ChatError::StalePolicy);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "data", rename_all = "snake_case", deny_unknown_fields)]
pub enum OpenDirectOutcome {
    Opened { resident: ConversationId, key: ChatKey },
    RejectedNative,
    StaleTarget,
    Expired,
    Capacity,
    Uncertain,
}
impl OpenDirectOutcome {
    fn validate(&self, request: OpenDirectRequest) -> Result<(), ChatError> {
        if let Self::Opened { resident, key, .. } = self {
            key.validate()?;
            if *resident != request.resident
                || key.source != request.selection.source
                || key.conversation != Conversation::Direct(*resident)
            {
                return Err(ChatError::Binding);
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum OpenDirectState {
    Pending,
    Terminal(OpenDirectOutcome),
    Unknown,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenDirectReceipt {
    pub request: OpenDirectRequest,
    pub state: OpenDirectState,
}
impl OpenDirectReceipt {
    pub fn validate(&self) -> Result<(), ChatError> {
        self.request.validate()?;
        if let OpenDirectState::Terminal(outcome) = &self.state {
            outcome.validate(self.request)?;
        }
        Ok(())
    }
}

/// Fresh presentation is separate from the immutable operation receipt. A
/// later query must not disclose retained names/unread after privacy revocation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenDirectView {
    pub policy: NativeChatPolicy,
    pub title: Label,
    pub unread: u32,
}
impl OpenDirectView {
    pub fn validate(&self, receipt: &OpenDirectReceipt) -> Result<(), ChatError> {
        receipt.validate()?;
        self.policy.validate()?;
        let OpenDirectState::Terminal(OpenDirectOutcome::Opened { key, .. }) = &receipt.state else {
            return Err(ChatError::Invalid);
        };
        if self.policy.key != *key {
            return Err(ChatError::Binding);
        }
        if !self.policy.lease_current || !self.policy.ready || !self.policy.can_read {
            return Err(ChatError::Denied);
        }
        Ok(())
    }
}

/// Reserve before native creation. Receipt queries never re-run an open, and
/// eviction retains the high-water mark. Exactly one pending operation per source.
pub struct OpenDirectLedger {
    source: ChatSource,
    highwater: u64,
    records: VecDeque<OpenDirectReceipt>,
}
impl OpenDirectLedger {
    pub fn new(source: ChatSource) -> Result<Self, ChatError> {
        source.validate()?;
        Ok(Self {
            source,
            highwater: 0,
            records: VecDeque::new(),
        })
    }
    pub fn reserve(&mut self, request: OpenDirectRequest) -> Result<Option<OpenDirectReceipt>, ChatError> {
        request.validate()?;
        if request.selection.source != self.source {
            return Err(ChatError::Binding);
        }
        if let Some(receipt) = self.records.iter().find(|record| record.request.request == request.request) {
            if receipt.request != request {
                return Err(ChatError::RequestChanged);
            }
            return Ok(Some(receipt.clone()));
        }
        if request.request <= self.highwater {
            return Err(ChatError::StaleRequest);
        }
        if self.records.iter().any(|receipt| receipt.state == OpenDirectState::Pending) {
            return Err(ChatError::Busy);
        }
        if self.records.len() == MAX_RECEIPTS {
            self.records.pop_front();
        }
        self.highwater = request.request;
        self.records.push_back(OpenDirectReceipt {
            request,
            state: OpenDirectState::Pending,
        });
        Ok(None)
    }
    pub fn finish(&mut self, request: OpenDirectRequest, outcome: OpenDirectOutcome) -> Result<OpenDirectReceipt, ChatError> {
        request.validate()?;
        outcome.validate(request)?;
        if request.selection.source != self.source {
            return Err(ChatError::Binding);
        }
        let receipt = self
            .records
            .iter_mut()
            .find(|record| record.request.request == request.request)
            .ok_or(ChatError::UnknownReceipt)?;
        if receipt.request != request {
            return Err(ChatError::RequestChanged);
        }
        let next = OpenDirectState::Terminal(outcome);
        if receipt.state != OpenDirectState::Pending && receipt.state != next {
            return Err(ChatError::FinalReceipt);
        }
        receipt.state = next;
        Ok(receipt.clone())
    }
    pub fn query(&self, request: OpenDirectRequest) -> Result<OpenDirectReceipt, ChatError> {
        request.validate()?;
        if request.selection.source != self.source {
            return Err(ChatError::Binding);
        }
        if let Some(receipt) = self.records.iter().find(|record| record.request.request == request.request) {
            if receipt.request != request {
                return Err(ChatError::RequestChanged);
            }
            return Ok(receipt.clone());
        }
        Ok(OpenDirectReceipt {
            request,
            state: OpenDirectState::Unknown,
        })
    }
}

#[cfg(test)]
mod tests;
