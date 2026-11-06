//! Bounded native chat work on the owning App thread. Socket IO and codecs
//! remain on a dedicated worker; no world activation or native focus is changed.

use super::App;
use crate::session_host::chat::WorkerChat;
use crate::session_host::chat_transport::LaneError;
use crate::session_host::worker::WorkerBridge;
use aurora_session::chat::{ChatPacket, ChatSource};
use std::collections::VecDeque;

const MAX_PENDING: usize = 16;
// One policy per line plus metadata/gap/receipt for the four-line native page.
const MAX_REPLY: usize = 11;

#[derive(Default)]
pub(super) struct Driver {
    native: WorkerChat,
    outgoing: VecDeque<ChatPacket>,
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
            return;
        };
        if self.native.current_source() != Some(source) {
            self.outgoing.clear();
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
        if self.outgoing.len() > MAX_PENDING - MAX_REPLY {
            return;
        }
        for packet in bridge.chat_commands() {
            if authority() != Some(packet.source) {
                continue;
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
                    break;
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
