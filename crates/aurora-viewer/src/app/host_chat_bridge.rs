//! Bounded native chat work on the owning App thread. Socket IO and codecs
//! remain on a dedicated worker; no world activation or native focus is changed.

use super::App;
use crate::session_host::chat::{MAX_REPLY, WorkerChat};
use crate::session_host::chat_transport::LaneError;
use crate::session_host::worker::WorkerBridge;
use aurora_session::chat::{ChatPacket, ChatSource};
use std::collections::VecDeque;

const MAX_PENDING: usize = 16;
// Poll: gap1 + rotating policy1 + three*(policy1+line1) + read batch1
// + send receipt1 = 10. Reserve 11 before each FIFO native operation.
fn reply_room(pending: usize) -> bool {
    pending <= MAX_PENDING - MAX_REPLY
}

#[derive(Default)]
pub(super) struct Driver {
    native: WorkerChat,
    outgoing: VecDeque<ChatPacket>,
    incoming: VecDeque<ChatPacket>,
}

impl Driver {
    fn flush(&mut self, bridge: &WorkerBridge) {
        while let Some(packet) = self.outgoing.front() {
            match bridge.chat_send(packet) {
                Ok(()) => {
                    self.outgoing.pop_front();
                }
                Err(LaneError::Busy) => break,
                Err(_) => {
                    self.outgoing.clear();
                    break;
                }
            }
        }
    }

    fn step(&mut self, app: &mut App, bridge: &WorkerBridge) {
        let authority = || bridge.bound_session().and_then(|binding| ChatSource::try_from(binding).ok());
        let Some(source) = authority() else {
            self.native.clear();
            self.outgoing.clear();
            self.incoming.clear();
            return;
        };
        if self.native.current_source() != Some(source) {
            self.outgoing.clear();
            self.incoming.clear();
        }
        // A socket reconnect never clears the native submission ledger. It
        // retains uncertain receipts and global sequences for this source.
        if app.host_chat_sync(&mut self.native, &authority).is_err() {
            self.outgoing.clear();
            return;
        }
        if !bridge.chat_ready() {
            self.outgoing.clear();
            return;
        }
        self.flush(bridge);
        if !reply_room(self.outgoing.len()) {
            return;
        }
        if self.incoming.is_empty() {
            // chat_commands has a hard one-command-per-call bound. Retain its
            // FIFO locally until reply capacity is reserved, never drain/drop
            // a native mutation because a previous response filled the queue.
            self.incoming.extend(bridge.chat_commands());
        }
        if reply_room(self.outgoing.len())
            && let Some(packet) = self.incoming.pop_front()
        {
            if authority() != Some(packet.source) {
                return;
            }
            let previous = self.native.outgoing_sequence();
            if let Ok(replies) = app.host_chat_packet(&mut self.native, &authority, &packet) {
                let ordered = replies
                    .iter()
                    .try_fold(previous, |previous, reply| {
                        (reply.source == source && reply.sequence > previous).then_some(reply.sequence)
                    })
                    .is_some();
                if !ordered || replies.len() > MAX_REPLY || self.outgoing.len() + replies.len() > MAX_PENDING {
                    // Never replay the mutation to recover an overflowing
                    // response. A later Poll may recover its immutable receipt.
                    self.outgoing.clear();
                    return;
                }
                self.outgoing.extend(replies);
            }
        }
        self.flush(bridge);
    }
}

impl App {
    pub(super) fn tick_host_chat(&mut self) {
        let Some(bridge) = self.host_session.take() else {
            return;
        };
        let mut driver = self.host_chat.take().unwrap_or_default();
        driver.step(self, &bridge);
        self.host_chat = Some(driver);
        self.host_session = Some(bridge);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn host_chat_driver_reserves_full_reply_before_taking_a_fifo_command() {
        assert!(reply_room(5));
        assert!(!reply_room(6));
        assert!(!reply_room(MAX_PENDING));
    }
}
