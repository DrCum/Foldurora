//! Endpoint files and listener joins are never performed on the event thread.
use super::*;
use aurora_mcp::ipc::{Discovery, Server};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
    mpsc::{self, SyncSender},
};
use std::time::Duration;

#[derive(Clone, Copy, Default)]
pub(super) struct Snapshot {
    pub installation: Option<aurora_session::InstallationId>,
    pub ready: Option<(u64, HostId)>,
    pub failed: bool,
}
enum Message {
    Start {
        token: u64,
        installation: aurora_session::InstallationId,
        runtime: Runtime,
    },
    Stop,
}
pub(super) struct Owner {
    sender: SyncSender<Message>,
    generation: Arc<AtomicU64>,
    snapshot: Arc<Mutex<Snapshot>>,
}
impl Owner {
    pub(super) fn new() -> Self {
        let (sender, receiver) = mpsc::sync_channel(4);
        let generation = Arc::new(AtomicU64::new(1));
        let current = generation.clone();
        let snapshot = Arc::new(Mutex::new(Snapshot::default()));
        let view = snapshot.clone();
        let _ = std::thread::Builder::new().name("character-mcp-io".into()).spawn(move || {
            let installation = crate::session_host::transport::current_installation().ok();
            if let Ok(mut s) = view.lock() {
                s.installation = installation;
                s.failed = installation.is_none();
            }
            let mut service: Option<(u64, Server)> = None;
            loop {
                if service.as_ref().is_some_and(|(token, _)| *token != current.load(Ordering::Acquire)) {
                    service = None;
                    if let Ok(mut s) = view.lock() {
                        s.ready = None;
                    }
                }
                match receiver.recv_timeout(Duration::from_millis(100)) {
                    Ok(Message::Stop) => {
                        service = None;
                        if let Ok(mut s) = view.lock() {
                            s.ready = None;
                        }
                    }
                    Ok(Message::Start {
                        token,
                        installation,
                        runtime,
                    }) => {
                        if token != current.load(Ordering::Acquire) {
                            continue;
                        }
                        service = None;
                        let result = Discovery::character(installation).and_then(|d| Server::start(d, true, Arc::new(runtime)));
                        match result {
                            Ok(server) if token == current.load(Ordering::Acquire) => {
                                let id = server.instance();
                                service = Some((token, server));
                                if let Ok(mut s) = view.lock() {
                                    s.ready = Some((token, id));
                                    s.failed = false;
                                }
                            }
                            _ => {
                                if let Ok(mut s) = view.lock() {
                                    s.ready = None;
                                    s.failed = true;
                                }
                            }
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
        });
        Self {
            sender,
            generation,
            snapshot,
        }
    }
    /// A disconnected in-memory owner for isolated native UI fixtures. Never
    /// starts a worker, discovers an installation or touches account files.
    #[cfg(test)]
    pub(super) fn inert() -> Self {
        let (sender, _receiver) = mpsc::sync_channel(4);
        Self {
            sender,
            generation: Arc::new(AtomicU64::new(1)),
            snapshot: Arc::new(Mutex::new(Snapshot::default())),
        }
    }
    pub(super) fn snapshot(&self) -> Snapshot {
        self.snapshot.lock().map_or_else(|_| Snapshot::default(), |s| *s)
    }
    pub(super) fn start(&self, runtime: Runtime) -> Option<u64> {
        let installation = self.snapshot().installation?;
        let token = self.generation.fetch_add(1, Ordering::AcqRel).checked_add(1)?;
        self.sender
            .try_send(Message::Start {
                token,
                installation,
                runtime,
            })
            .ok()?;
        Some(token)
    }
    pub(super) fn stop(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        let _ = self.sender.try_send(Message::Stop);
    }
}
impl Drop for Owner {
    fn drop(&mut self) {
        self.stop();
    }
}
