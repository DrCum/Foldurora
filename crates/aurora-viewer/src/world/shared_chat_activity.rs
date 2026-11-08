//! Actual native live-incoming provenance for a close-in-host fence. This
//! neither changes native unread nor removes a native IM. Legacy reference:
//! FSSessionWorker::connectChat / host ChatView::accept (abbe3cb0, LGPL-2.1).
use super::{ChatKind, ChatLine, Origin, World};
use aurora_session::chat::NativeIncoming;
use std::sync::atomic::{AtomicU64, Ordering};
use uuid::Uuid;

static NEXT_STREAM: AtomicU64 = AtomicU64::new(1);

impl World {
    pub(crate) fn bind_shared_chat_activity(&mut self, agent: Uuid, session: Uuid) {
        self.shared_chat_incoming_owner = None;
        self.shared_chat_incoming = None;
        if agent.is_nil() || session.is_nil() {
            return;
        }
        let Ok(stream) = NEXT_STREAM.try_update(Ordering::AcqRel, Ordering::Acquire, |next| next.checked_add(1)) else {
            return;
        };
        self.shared_chat_incoming_owner = Some((agent, session));
        self.shared_chat_incoming = Some(NativeIncoming { stream, serial: 0 });
    }
    fn shared_chat_activity_current(&self) -> Option<NativeIncoming> {
        let (agent, session) = self.shared_chat_incoming_owner?;
        let login = self.login.as_ref()?;
        (self.agent_id == agent && login.agent_id == agent && login.session_id == session)
            .then_some(self.shared_chat_incoming)
            .flatten()
    }
    /// Shared by the producer and actual App export policy, not a UI hint.
    pub(crate) fn shared_chat_privacy_allowed(&self) -> bool {
        let r = &self.rlva;
        !(r.enabled() && (r.has("shownames") || r.has("showloc") || r.has("shownearby")))
    }
    pub(crate) fn shared_chat_direct_readable(&self, peer: Uuid) -> bool {
        !self.mutes.is_muted_id(&peer) && self.shared_chat_privacy_allowed() && self.rlva_allows_im("recvim", peer)
    }
    /// Called only by actual conversation insertion, including its first line.
    /// History, cards, own echoes, typing/system callbacks and scans never mint.
    pub(super) fn record_shared_chat_incoming(&mut self, session: Uuid, line: &ChatLine) {
        // Session type comes from the native owner, never from a host tag. In
        // particular a conference or uninitialized group cannot fall through
        // to the resident path. Group activity is ordinary live session text.
        let readable = if self.is_group_session(&session) {
            matches!(line.kind, ChatKind::Im) && self.shared_group_readable(session)
        } else {
            !self.groups.is_member(&session) && self.shared_chat_direct_readable(session)
        };
        if self.shared_chat_activity_current().is_none()
            || self.agent_id.is_nil()
            || !self.mutes.is_loaded()
            || line.origin != Origin::Live
            || !matches!(line.kind, ChatKind::Im | ChatKind::ObjectIm)
            || line.source.is_nil()
            || line.source == self.agent_id
            || !readable
            || !self.rlva_allows_im("recvim", line.source)
            || self.mutes.text_muted(&line.source, &line.from)
        {
            return;
        }
        // Exhaustion is permanent until actual World/source reset. It cannot
        // manufacture zero or revive an earlier stream after a missing session.
        self.shared_chat_incoming = self.shared_chat_incoming.and_then(NativeIncoming::checked_next);
        if let Some(incoming) = self.shared_chat_incoming {
            self.session_lines(session).shared_chat_incoming = incoming;
        }
    }
    pub(crate) fn shared_chat_activity(&self, peer: Uuid) -> Option<NativeIncoming> {
        let current = self.shared_chat_activity_current()?;
        self.social
            .ims
            .iter()
            .find(|session| session.other == peer)
            .map(|session| session.shared_chat_incoming)
            .filter(|incoming| incoming.stream == current.stream)
    }
    pub(crate) fn shared_chat_close_fence(&self) -> Option<NativeIncoming> {
        self.shared_chat_activity_current()
    }
}

#[cfg(test)]
mod tests;
