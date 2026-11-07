//! One bounded native IO lane. Replies never apply a layout, and every job
//! retains its originally selected private profile directory.
use super::model::{Catalogue, MAX_FILE};
use crate::session_host::profile::WorkspaceDirectory;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use uuid::Uuid;

pub(crate) struct Request {
    pub source: super::controller::Source,
    pub operation: u64,
    pub binding: u64,
    pub directory: WorkspaceDirectory,
    pub account: Uuid,
    pub grid: String,
    pub write: Option<(Option<Catalogue>, Catalogue)>,
}
pub(crate) struct Reply {
    pub source: super::controller::Source,
    pub operation: u64,
    pub binding: u64,
    pub result: Result<Option<Catalogue>, ()>,
}
pub(crate) struct Lane {
    send: SyncSender<Request>,
    receive: Receiver<Reply>,
    busy: std::cell::Cell<bool>,
}
impl Lane {
    pub(crate) fn new() -> std::io::Result<Self> {
        let (send, requests) = sync_channel::<Request>(1);
        let (results, receive) = sync_channel(1);
        std::thread::Builder::new().name("aurora-workspaces".into()).spawn(move || {
            while let Ok(r) = requests.recv() {
                let result = perform(&r);
                if results
                    .send(Reply {
                        source: r.source.clone(),
                        operation: r.operation,
                        binding: r.binding,
                        result,
                    })
                    .is_err()
                {
                    break;
                }
            }
        })?;
        Ok(Self {
            send,
            receive,
            busy: std::cell::Cell::new(false),
        })
    }
    pub(crate) fn submit(&self, r: Request) -> bool {
        if self.busy.replace(true) {
            return false;
        }
        if self.send.try_send(r).is_ok() {
            true
        } else {
            self.busy.set(false);
            false
        }
    }
    pub(crate) fn poll(&self) -> Result<Option<Reply>, ()> {
        match self.receive.try_recv() {
            Ok(reply) => {
                self.busy.set(false);
                Ok(Some(reply))
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => Ok(None),
            Err(_) => {
                self.busy.set(false);
                Err(())
            }
        }
    }
}
fn perform(r: &Request) -> Result<Option<Catalogue>, ()> {
    if r.operation == 0
        || r.binding == 0
        || r.source.login.is_nil()
        || r.source.agent != r.account
        || r.source.grid != r.grid
        || !r.directory.matches(r.account, &r.grid)
    {
        return Err(());
    }
    let _lock = r.directory.lock().map_err(|_| ())?;
    let current_bytes = r.directory.read(MAX_FILE).map_err(|_| ())?;
    let current = current_bytes
        .as_deref()
        .map(|b| Catalogue::decode(b, r.account, &r.grid))
        .transpose()
        .map_err(|_| ())?;
    match &r.write {
        None => Ok(current),
        Some((expected, next)) => {
            if current != *expected || next.account != r.account || next.grid != r.grid {
                return Err(());
            }
            let old_revision = current.as_ref().map_or(0, |c| c.revision);
            if next.revision != old_revision.checked_add(1).ok_or(())? {
                return Err(());
            }
            let b = next.encode().map_err(|_| ())?;
            r.directory.publish(&b, current_bytes.as_deref()).map_err(|_| ())?;
            Ok(Some(next.clone()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn workspace_store_missing_load_atomic_write_revision_and_unknown_bytes_preservation() {
        let account = Uuid::new_v4();
        let grid = "d".repeat(64);
        let make = |write| Request {
            source: super::super::controller::Source {
                agent: account,
                login: Uuid::new_v4(),
                grid: grid.clone(),
                hosted: None,
            },
            operation: 1,
            binding: 1,
            directory: WorkspaceDirectory::select(account, &grid).unwrap(),
            account,
            grid: grid.clone(),
            write,
        };
        assert!(perform(&make(None)).unwrap().is_none());
        let next = Catalogue::empty(account, grid.clone());
        assert_eq!(perform(&make(Some((None, next.clone())))).unwrap(), Some(next.clone()));
        assert!(perform(&make(Some((None, next.clone())))).is_err());
        let directory = WorkspaceDirectory::select(account, &grid).unwrap();
        let unknown = b"{\"version\":999,\"future\":true}";
        let old = directory.read(MAX_FILE).unwrap().unwrap();
        directory.publish(unknown, Some(&old)).unwrap();
        assert!(perform(&make(None)).is_err());
        assert!(perform(&make(Some((Some(next.clone()), next)))).is_err());
        assert_eq!(directory.read(MAX_FILE).unwrap().unwrap(), unknown);
        assert!(directory.publish(b"replacement", Some(&old)).is_err());
        assert_eq!(directory.read(MAX_FILE).unwrap().unwrap(), unknown);
        assert!(
            perform(&Request {
                source: super::super::controller::Source {
                    agent: account,
                    login: Uuid::new_v4(),
                    grid: "e".repeat(64),
                    hosted: None
                },
                grid: "e".repeat(64),
                directory: WorkspaceDirectory::select(account, &"e".repeat(64)).unwrap(),
                ..make(None)
            })
            .unwrap()
            .is_none()
        );
    }
    #[test]
    fn workspace_io_lane_has_one_inflight_and_one_exact_result_without_extra_admission() {
        let (send, requests) = sync_channel(1);
        let (results, receive) = sync_channel(1);
        let lane = Lane {
            send,
            receive,
            busy: std::cell::Cell::new(false),
        };
        let account = Uuid::new_v4();
        let grid = "f".repeat(64);
        let source = super::super::controller::Source {
            agent: account,
            login: Uuid::new_v4(),
            grid: grid.clone(),
            hosted: None,
        };
        let request = || Request {
            source: source.clone(),
            operation: 1,
            binding: 1,
            directory: WorkspaceDirectory::select(account, &grid).unwrap(),
            account,
            grid: grid.clone(),
            write: None,
        };
        assert!(lane.submit(request()));
        let accepted = requests.recv().unwrap();
        assert!(
            !lane.submit(request()),
            "removing a queued request does not release its result credit"
        );
        results
            .send(Reply {
                source: accepted.source,
                operation: accepted.operation,
                binding: accepted.binding,
                result: Ok(None),
            })
            .unwrap();
        let reply = lane.poll().unwrap().unwrap();
        assert_eq!(reply.source, source);
        assert_eq!(reply.operation, 1);
        assert!(lane.submit(request()));
        drop(results);
        assert!(lane.poll().is_err());
    }
    #[test]
    fn workspace_request_identity_cannot_publish_into_another_empty_account_directory() {
        let account = Uuid::new_v4();
        let other = Uuid::new_v4();
        let grid = "e".repeat(64);
        let r = Request {
            source: super::super::controller::Source {
                agent: account,
                login: Uuid::new_v4(),
                grid: grid.clone(),
                hosted: None,
            },
            operation: 1,
            binding: 1,
            directory: WorkspaceDirectory::select(other, &grid).unwrap(),
            account,
            grid: grid.clone(),
            write: Some((None, Catalogue::empty(account, grid.clone()))),
        };
        assert!(perform(&r).is_err());
        let _lock = r.directory.lock().unwrap();
        assert!(r.directory.read(MAX_FILE).unwrap().is_none());
    }
}
