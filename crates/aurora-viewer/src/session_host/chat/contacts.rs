//! Bounded native Friends snapshots and once-only model Open, on the private
//! authenticated chat lane. Native state remains the operation authority.
use super::*;
use aurora_session::chat::{
    ContactChannel, ContactKey, ContactPage, ContactPageRequest, ContactRow, ContactsPolicy, MAX_CONTACT_PAGE_ROWS, MAX_FRIEND_CONTACTS,
    OpenDirectLedger, OpenDirectOutcome, OpenDirectReply, OpenDirectRequest, OpenDirectState, OpenDirectView,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NativeContactPolicy {
    pub stamp: [u64; 4],
    pub can_browse: bool,
}
#[derive(Clone)]
pub(crate) struct NativeContacts {
    pub rows: Vec<ContactRow>,
    pub limited: bool,
}
struct Friends {
    key: ContactKey,
    data: NativeContacts,
}
#[derive(Default)]
pub(super) struct WorkerContacts {
    pub(super) policy: Option<ContactsPolicy>,
    native: Option<NativeContactPolicy>,
    highwater: u64,
    friends: VecDeque<Friends>,
    opens: Option<OpenDirectLedger>,
}
impl WorkerContacts {
    pub(super) fn sync(&mut self, source: ChatSource, native: &impl NativeChat) -> Result<(), ChatError> {
        let policy = native.contacts_policy();
        if policy.is_none() && self.policy.is_none() {
            return Ok(());
        }
        let changed = policy != self.native || self.policy.is_none();
        if changed {
            let revision = self
                .policy
                .map_or(Some(1), |old| old.revision.checked_add(1))
                .ok_or(ChatError::Sequence)?;
            self.policy = Some(ContactsPolicy {
                source,
                revision,
                can_browse: policy.is_some_and(|policy| policy.can_browse),
            });
            self.native = policy;
            self.friends.clear();
        }
        if self.opens.is_none() {
            self.opens = Some(OpenDirectLedger::new(source)?);
        }
        Ok(())
    }
    fn selection(&self, request: OpenDirectRequest) -> bool {
        self.policy
            .is_some_and(|policy| policy.can_browse && policy.revision == request.policy_revision)
            && self.friends.iter().any(|friends| {
                friends.key == request.selection && friends.data.rows.iter().any(|row| row.resident == request.resident && row.can_open)
            })
    }
    fn page(&mut self, request: ContactPageRequest, native: &impl NativeChat) -> Result<ContactPage, ChatError> {
        let policy = self.policy.filter(|policy| policy.can_browse).ok_or(ChatError::Denied)?;
        if request.key.source != policy.source || request.key.channel != ContactChannel::Friends {
            return Err(ChatError::Binding);
        }
        if !self.friends.iter().any(|friends| friends.key == request.key) {
            if request.offset != 0 || request.key.token <= self.highwater {
                return Err(ChatError::StaleRequest);
            }
            self.highwater = request.key.token;
            let data = native.friends()?;
            if data.rows.len() > MAX_FRIEND_CONTACTS {
                return Err(ChatError::Capacity);
            }
            let mut seen = BTreeSet::new();
            for row in &data.rows {
                row.validate()?;
                if !seen.insert(row.resident) {
                    return Err(ChatError::Invalid);
                }
            }
            // One retained completed generation can remain selected during a
            // refresh. This is bounded, not an unbounded token history.
            if self.friends.len() == 2 {
                self.friends.pop_front();
            }
            self.friends.push_back(Friends { key: request.key, data });
        }
        let friends = self
            .friends
            .iter()
            .find(|friends| friends.key == request.key)
            .ok_or(ChatError::StaleRequest)?;
        let offset = usize::from(request.offset);
        if offset > friends.data.rows.len() {
            return Err(ChatError::Invalid);
        }
        Ok(ContactPage {
            key: request.key,
            policy_revision: policy.revision,
            offset: request.offset,
            total: friends.data.rows.len() as u16,
            limited: friends.data.limited,
            pending: false,
            rows: friends.data.rows.iter().skip(offset).take(MAX_CONTACT_PAGE_ROWS).cloned().collect(),
        })
    }
}
impl WorkerChat {
    fn contact_page(
        &mut self,
        native: &impl NativeChat,
        packet: &ChatPacket,
        request: ContactPageRequest,
    ) -> Result<Vec<ChatPacket>, ChatError> {
        let source = self.context(native)?;
        self.contacts.sync(source, native)?;
        let event = self
            .contacts
            .page(request, native)
            .map_or_else(|_| ChatEvent::ContactPageFailed(request.key), ChatEvent::ContactPage);
        if self.context(native)? != source {
            return Err(ChatError::Binding);
        }
        let policy = self.contacts.policy.ok_or(ChatError::Unavailable)?;
        let policy_packet = self.packet(packet.sequence, ChatEvent::ContactsPolicy(policy))?;
        let reply = self.packet(packet.sequence, event)?;
        let reply = if matches!(reply.message, ChatMessage::Event(ChatEvent::ContactPage(_))) {
            aurora_session::chat::fit_contact_page(reply)?
        } else {
            reply
        };
        self.received = packet.sequence;
        Ok(vec![policy_packet, reply])
    }
    fn open_direct(
        &mut self,
        native: &mut impl NativeChat,
        packet: &ChatPacket,
        request: OpenDirectRequest,
        received_at: Option<Instant>,
        now: Instant,
        query: bool,
    ) -> Result<Vec<ChatPacket>, ChatError> {
        let source = self.context(native)?;
        self.contacts.sync(source, native)?;
        let ledger = self.contacts.opens.as_mut().ok_or(ChatError::Unavailable)?;
        let receipt = if query {
            ledger.query(request)?
        } else if let Some(receipt) = ledger.reserve(request)? {
            receipt
        } else {
            let age = received_at.and_then(|received| now.checked_duration_since(received));
            let outcome = if age.is_none_or(|age| age > Duration::from_secs(2)) {
                OpenDirectOutcome::Expired
            } else if !self.contacts.selection(request) {
                OpenDirectOutcome::StaleTarget
            } else if self.context(native)? != source {
                OpenDirectOutcome::RejectedNative
            } else {
                native.open_direct(request)
            };
            self.contacts
                .opens
                .as_mut()
                .ok_or(ChatError::Unavailable)?
                .finish(request, outcome)?
        };
        // Recovering a receipt never re-runs the native operation. Presentation
        // is independently reauthorized now; the receipt itself contains no name.
        if self.context(native)? != source {
            return Err(ChatError::Binding);
        }
        self.contacts.sync(source, native)?;
        let mut view = None;
        if let OpenDirectState::Terminal(OpenDirectOutcome::Opened { key, .. }) = &receipt.state
            && self.contacts.policy.is_some_and(|policy| policy.can_browse)
            && self.require_read_authority(native, *key).is_ok()
            && let Some(state) = self.states.get(&key.conversation)
        {
            view = Some(OpenDirectView {
                policy: state.policy,
                title: state.snapshot.title.clone(),
                unread: native.read_metadata(key.conversation).map_or(0, |read| read.unread),
            });
        }
        if self.context(native)? != source {
            return Err(ChatError::Binding);
        }
        let policy = self.contacts.policy.ok_or(ChatError::Unavailable)?;
        let replies = vec![
            self.packet(packet.sequence, ChatEvent::ContactsPolicy(policy))?,
            self.packet(packet.sequence, ChatEvent::OpenDirect(OpenDirectReply { receipt, view }))?,
        ];
        self.received = packet.sequence;
        Ok(replies)
    }
}

#[cfg(test)]
mod tests;
