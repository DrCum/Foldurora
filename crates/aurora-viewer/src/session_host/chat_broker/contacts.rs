//! Controller-owned bounded contact transfer and original-source Open records.
//! Socket recovery can query a receipt, never resubmit native Open.
use super::*;
use aurora_session::ConversationId;
use aurora_session::chat::{
    CompletedContacts, ContactChannel, ContactDirectory, ContactKey, ContactPage, ContactPageRequest, ContactsPolicy, MAX_RECEIPTS,
    OpenDirectOutcome, OpenDirectReply, OpenDirectRequest, OpenDirectState,
};
use aurora_session::chat::{ContactSearchRequest, ContactText, NativeChatPolicy};
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum OpenStatus {
    Pending,
    Uncertain,
    Completed(OpenDirectOutcome),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OpenOperation {
    pub request: OpenDirectRequest,
    pub status: OpenStatus,
    pub presented: Option<NativeChatPolicy>,
}
#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) struct ContactsSnapshot {
    pub policy: Option<ContactsPolicy>,
    pub friends: Option<CompletedContacts>,
    pub search: Option<CompletedContacts>,
    pub search_key: Option<ContactKey>,
    pub search_query: Option<ContactText<127>>,
    pub search_loading: bool,
    pub search_failed: bool,
    pub loading: bool,
    pub failed: bool,
    pub open_busy: bool,
    pub opens: Vec<OpenOperation>,
}
impl ContactsSnapshot {
    pub fn selected(&self, key: ContactKey, resident: ConversationId, revision: u64) -> bool {
        let rows = match key.channel {
            ContactChannel::Friends => &self.friends,
            ContactChannel::Search => &self.search,
        };
        self.policy
            .is_some_and(|policy| policy.source == key.source && policy.can_browse && policy.revision == revision)
            && rows.as_ref().is_some_and(|rows| {
                rows.key == key && rows.policy_revision == revision && rows.rows.iter().any(|row| row.resident == resident && row.can_open)
            })
    }
}
fn channel_index(channel: ContactChannel) -> usize {
    match channel {
        ContactChannel::Friends => 0,
        ContactChannel::Search => 1,
    }
}
struct PageFlight {
    request: ContactPageRequest,
    sequence: u64,
    since: Instant,
}
struct OpenRecord {
    operation: OpenOperation,
    close_through: u64,
}
struct OpenFlight {
    request: OpenDirectRequest,
    since: Instant,
    query_at: Instant,
    queries: u8,
}
#[derive(Default)]
pub(super) struct ContactsBook {
    directory: Option<ContactDirectory>,
    policy: Option<ContactsPolicy>,
    serial: u64,
    flight: [Option<PageFlight>; 2],
    page_at: [Option<Instant>; 2],
    failed: [bool; 2],
    search_key: Option<ContactKey>,
    search_query: Option<ContactText<127>>,
    opens: VecDeque<OpenRecord>,
    open_highwater: u64,
    pending: Option<OpenFlight>,
}
impl ContactsBook {
    pub(super) fn revoke(&mut self) {
        if let Some(directory) = &mut self.directory {
            directory.clear();
        }
        self.flight = [None, None];
        self.page_at = [None, None];
        self.failed = [false, false];
        self.search_key = None;
        self.search_query = None;
        for record in &mut self.opens {
            record.operation.presented = None;
        }
        if let Some(pending) = &self.pending {
            let request = pending.request;
            self.open_status(request, OpenStatus::Uncertain);
        }
    }
    pub(super) fn snapshot(&self, active: bool) -> ContactsSnapshot {
        let policy = self.policy.filter(|policy| active && policy.can_browse);
        ContactsSnapshot {
            policy,
            friends: policy.and_then(|_| self.directory.as_ref()?.completed(ContactChannel::Friends).cloned()),
            loading: active
                && self
                    .directory
                    .as_ref()
                    .is_some_and(|directory| directory.request(ContactChannel::Friends).is_some()),
            search: policy.and_then(|_| self.directory.as_ref()?.completed(ContactChannel::Search).cloned()),
            search_key: policy.and(self.search_key),
            search_query: policy.and(self.search_query.clone()),
            search_loading: active
                && self
                    .directory
                    .as_ref()
                    .is_some_and(|directory| directory.request(ContactChannel::Search).is_some()),
            search_failed: active && self.failed[1],
            failed: active && self.failed[0],
            open_busy: self.pending.is_some(),
            opens: self.opens.iter().map(|record| record.operation.clone()).collect(),
        }
    }
    pub(super) fn policy(&mut self, policy: ContactsPolicy) -> Result<(), ChatError> {
        if self.directory.is_none() {
            self.directory = Some(ContactDirectory::new(policy.source)?);
        }
        self.directory.as_mut().ok_or(ChatError::Unavailable)?.set_policy(policy)?;
        if self.policy != Some(policy) {
            for record in &mut self.opens {
                record.operation.presented = None;
            }
            self.flight = [None, None];
            self.page_at = [None, None];
            self.failed = [false, false];
            self.search_key = None;
            self.search_query = None;
        }
        self.policy = Some(policy);
        Ok(())
    }
    pub(super) fn page(&mut self, reply_to: u64, page: &ContactPage) -> Result<(), ChatError> {
        let index = channel_index(page.key.channel);
        let flight = self.flight[index].as_ref().ok_or(ChatError::StaleRequest)?;
        if flight.sequence != reply_to || flight.request.key != page.key || flight.request.offset != page.offset {
            return Err(ChatError::StaleRequest);
        }
        self.directory.as_mut().ok_or(ChatError::Unavailable)?.accept(page)?;
        self.flight[index] = None;
        self.page_at[index] = page.pending.then(|| Instant::now() + Duration::from_millis(250));
        self.failed[index] = false;
        Ok(())
    }
    pub(super) fn failed(&mut self, reply_to: u64, key: ContactKey) -> Result<(), ChatError> {
        let index = channel_index(key.channel);
        let flight = self.flight[index].as_ref().ok_or(ChatError::StaleRequest)?;
        if flight.sequence != reply_to || flight.request.key != key {
            return Err(ChatError::StaleRequest);
        }
        self.directory.as_mut().ok_or(ChatError::Unavailable)?.fail(key)?;
        self.flight[index] = None;
        self.page_at[index] = None;
        self.failed[index] = true;
        Ok(())
    }
    fn open_status(&mut self, request: OpenDirectRequest, status: OpenStatus) {
        if let Some(record) = self.opens.iter_mut().find(|record| record.operation.request == request) {
            if !matches!(record.operation.status, OpenStatus::Completed(_)) || record.operation.status == status {
                record.operation.status = status;
            }
        }
    }
    pub(super) fn presented(&mut self, request: OpenDirectRequest, policy: NativeChatPolicy) {
        if let Some(record) = self.opens.iter_mut().find(|record| record.operation.request == request) {
            record.operation.presented = Some(policy);
        }
        if self.pending.as_ref().is_some_and(|pending| pending.request == request) {
            self.pending = None;
        }
    }
    pub(super) fn receipt(&mut self, reply: &OpenDirectReply) -> Result<Option<(ChatKey, u64)>, ChatError> {
        reply.validate()?;
        let request = reply.receipt.request;
        let record = self
            .opens
            .iter_mut()
            .find(|record| record.operation.request == request)
            .ok_or(ChatError::UnknownReceipt)?;
        let status = match &reply.receipt.state {
            OpenDirectState::Pending => return Ok(None),
            OpenDirectState::Unknown => OpenStatus::Uncertain,
            OpenDirectState::Terminal(outcome) => OpenStatus::Completed(outcome.clone()),
        };
        if matches!(record.operation.status, OpenStatus::Completed(_)) && record.operation.status != status {
            return Err(ChatError::FinalReceipt);
        }
        record.operation.status = status;
        record.operation.presented = None;
        let reopen = if matches!(reply.receipt.state, OpenDirectState::Terminal(OpenDirectOutcome::Opened { .. })) {
            reply.view.as_ref().map(|view| (view.policy.key, record.close_through))
        } else {
            None
        };
        if matches!(reply.receipt.state, OpenDirectState::Terminal(ref outcome) if !matches!(outcome, OpenDirectOutcome::Opened { .. }))
            && self.pending.as_ref().is_some_and(|pending| pending.request == request)
        {
            self.pending = None;
        }
        Ok(reopen)
    }
}
impl Peer {
    fn queue_contact_command(&mut self, command: ChatCommand) -> Result<u64, HostError> {
        if !self.live {
            return Err(HostError::Unavailable);
        }
        let source = self.source.ok_or(HostError::Unavailable)?;
        let sequence = self.sent.checked_add(1).ok_or(HostError::Unavailable)?;
        let packet = ChatPacket {
            version: VERSION,
            source,
            sequence,
            reply_to: 0,
            message: ChatMessage::Command(command),
        };
        self.io
            .as_mut()
            .ok_or(HostError::Unavailable)?
            .queue(&packet)
            .map_err(|_| HostError::Unavailable)?;
        self.sent = sequence;
        Ok(sequence)
    }
    pub(in crate::session_host) fn refresh_contacts(&mut self, source: ChatSource, now: Instant) -> Result<(), HostError> {
        if !self.live || self.io.is_none() || self.source != Some(source) || !self.contacts.policy.is_some_and(|policy| policy.can_browse) {
            return Err(HostError::Unavailable);
        }
        let directory = self.contacts.directory.as_mut().ok_or(HostError::Unavailable)?;
        if directory.request(ContactChannel::Friends).is_some() {
            return Err(HostError::Busy);
        }
        let serial = self.contacts.serial.checked_add(1).ok_or(HostError::Unavailable)?;
        directory
            .begin(ContactChannel::Friends, serial)
            .map_err(|_| HostError::Unavailable)?;
        self.contacts.serial = serial;
        self.contacts.failed[0] = false;
        self.poll_contacts(now);
        self.publish();
        Ok(())
    }
    pub(in crate::session_host) fn search_contacts(
        &mut self,
        source: ChatSource,
        revision: u64,
        query: ContactText<127>,
        now: Instant,
    ) -> Result<(), HostError> {
        if !self.live
            || self.io.is_none()
            || self.source != Some(source)
            || !self
                .contacts
                .policy
                .is_some_and(|policy| policy.can_browse && policy.revision == revision)
        {
            return Err(HostError::Unavailable);
        }
        let directory = self.contacts.directory.as_mut().ok_or(HostError::Unavailable)?;
        if directory.request(ContactChannel::Search).is_some() {
            return Err(HostError::Busy);
        }
        let serial = self.contacts.serial.checked_add(1).ok_or(HostError::Unavailable)?;
        let key = ContactKey {
            source,
            channel: ContactChannel::Search,
            token: serial,
        };
        let request = ContactSearchRequest {
            key,
            policy_revision: revision,
            query: query.clone(),
        };
        request.validate().map_err(|_| HostError::Unavailable)?;
        directory
            .begin(ContactChannel::Search, serial)
            .map_err(|_| HostError::Unavailable)?;
        self.contacts.serial = serial;
        self.contacts.search_key = Some(key);
        self.contacts.search_query = Some(query);
        self.contacts.failed[1] = false;
        match self.queue_contact_command(ChatCommand::SearchContacts(request)) {
            Ok(sequence) => {
                self.contacts.flight[1] = Some(PageFlight {
                    request: ContactPageRequest { key, offset: 0 },
                    sequence,
                    since: now,
                })
            }
            Err(_) => {
                let _ = self.contacts.directory.as_mut().ok_or(HostError::Unavailable)?.fail(key);
                self.contacts.failed[1] = true;
            }
        }
        self.publish();
        Ok(())
    }
    pub(super) fn poll_contacts(&mut self, now: Instant) -> bool {
        let mut changed = false;
        for channel in [ContactChannel::Friends, ContactChannel::Search] {
            let index = channel_index(channel);
            if let Some(flight) = &self.contacts.flight[index] {
                if now.saturating_duration_since(flight.since) >= RECEIPT_WAIT {
                    let (sequence, key) = (flight.sequence, flight.request.key);
                    let _ = self.contacts.failed(sequence, key);
                    changed = true;
                }
                continue;
            }
            if self.contacts.page_at[index].is_some_and(|when| now < when) {
                continue;
            }
            let Some(request) = self.contacts.directory.as_ref().and_then(|directory| directory.request(channel)) else {
                continue;
            };
            if let Ok(sequence) = self.queue_contact_command(ChatCommand::ContactPage(request)) {
                self.contacts.flight[index] = Some(PageFlight {
                    request,
                    sequence,
                    since: now,
                });
            }
        }
        changed
    }
    pub(in crate::session_host) fn admit_open_direct(&mut self, request: OpenDirectRequest, now: Instant) -> Result<(), HostError> {
        if self.source != Some(request.selection.source) || request.validate().is_err() || request.request <= self.contacts.open_highwater {
            return Err(HostError::Unavailable);
        }
        let allowed = self.live
            && self.io.is_some()
            && self.contacts.pending.is_none()
            && self
                .contacts
                .snapshot(true)
                .selected(request.selection, request.resident, request.policy_revision);
        if self.contacts.opens.len() == MAX_RECEIPTS {
            let active = self.contacts.pending.as_ref().map(|pending| pending.request);
            let index = self
                .contacts
                .opens
                .iter()
                .position(|record| Some(record.operation.request) != active)
                .ok_or(HostError::Busy)?;
            self.contacts.opens.remove(index);
        }
        self.contacts.open_highwater = request.request;
        self.contacts.opens.push_back(OpenRecord {
            operation: OpenOperation {
                request,
                presented: None,
                status: if allowed {
                    OpenStatus::Pending
                } else {
                    OpenStatus::Completed(OpenDirectOutcome::RejectedNative)
                },
            },
            close_through: self.closes.highwater(),
        });
        if !allowed {
            self.publish();
            return Err(HostError::Unavailable);
        }
        self.contacts.pending = Some(OpenFlight {
            request,
            since: now,
            query_at: now,
            queries: 0,
        });
        if self.queue_contact_command(ChatCommand::OpenDirect(request)).is_err() {
            self.contacts.open_status(request, OpenStatus::Uncertain);
        }
        self.publish();
        Ok(())
    }
    pub(super) fn poll_open_direct(&mut self, now: Instant) -> bool {
        let Some(pending) = &mut self.contacts.pending else {
            return false;
        };
        if now.saturating_duration_since(pending.since) < Duration::from_secs(1) {
            return false;
        }
        let (request, uncertain) = (pending.request, now.saturating_duration_since(pending.since) >= RECEIPT_WAIT);
        if pending.queries >= REVIEW_QUERY_LIMIT {
            self.contacts.open_status(request, OpenStatus::Uncertain);
            self.contacts.pending = None;
            return true;
        }
        if now.saturating_duration_since(pending.query_at) < Duration::from_secs(1) {
            return false;
        }
        pending.query_at = now;
        pending.queries += 1;
        if uncertain {
            self.contacts.open_status(request, OpenStatus::Uncertain);
        }
        // Never retry Open; even an unknown lookup has no mutation authority.
        let _ = self.queue_contact_command(ChatCommand::QueryOpenDirect(request));
        uncertain
    }
}

#[cfg(test)]
mod tests;
