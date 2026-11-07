//! Bounded native chat work on the owning App thread. Socket IO and codecs
//! remain on a dedicated worker. Explicit Review alone may open native chat.

use super::App;
use crate::session_host::chat::{MAX_REPLY, WorkerChat};
use crate::session_host::chat_io::ReceivedCommand;
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

fn take_command(incoming: &mut VecDeque<ReceivedCommand>, pending_replies: usize) -> Option<ReceivedCommand> {
    if reply_room(pending_replies) { incoming.pop_front() } else { None }
}

#[derive(Default)]
pub(super) struct Driver {
    native: WorkerChat,
    outgoing: VecDeque<ChatPacket>,
    incoming: VecDeque<ReceivedCommand>,
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
        if app.host_chat_sync(&mut self.native, bridge).is_err() {
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
        if let Some(command) = take_command(&mut self.incoming, self.outgoing.len()) {
            let packet = command.packet;
            if authority() != Some(packet.source) {
                return;
            }
            let previous = self.native.outgoing_sequence();
            if let Ok(replies) = app.host_chat_packet(&mut self.native, bridge, &packet, command.received_at) {
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
        use aurora_session::chat::{ChatCommand, ChatMessage, VERSION};
        use aurora_session::{AccountKey, GridKey, HostId, InstallationId, SessionId, SessionIdentity, WorkerId, WorkerIdentity};
        let source = ChatSource {
            worker: WorkerIdentity {
                installation: InstallationId::from_bytes([1; 32]).unwrap(),
                host: HostId::from_bytes([2; 16]).unwrap(),
                worker: WorkerId::from_bytes([3; 16]).unwrap(),
            },
            session: SessionIdentity {
                session: SessionId::from_bytes([4; 16]).unwrap(),
                account: AccountKey::from_bytes([5; 32]).unwrap(),
                grid: GridKey::from_bytes([6; 32]).unwrap(),
                generation: 1,
            },
            lease_epoch: 1,
        };
        let packet = |sequence| ChatPacket {
            version: VERSION,
            source,
            sequence,
            reply_to: 0,
            message: ChatMessage::Command(ChatCommand::Poll { after_event: 0 }),
        };
        let mut incoming = VecDeque::from([packet(1), packet(2)].map(|packet| ReceivedCommand {
            packet,
            received_at: std::time::Instant::now(),
        }));
        assert_eq!(take_command(&mut incoming, 6), None);
        assert_eq!(incoming.len(), 2);
        assert_eq!(take_command(&mut incoming, 5).unwrap().packet.sequence, 1);
        assert_eq!(take_command(&mut incoming, MAX_PENDING), None);
        assert_eq!(incoming.front().unwrap().packet.sequence, 2);
        assert_eq!(take_command(&mut incoming, 0).unwrap().packet.sequence, 2);
    }
}
