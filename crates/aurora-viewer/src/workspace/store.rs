//! One bounded native IO lane. Replies never apply a layout, and every job
//! retains its originally selected private profile directory.
use super::model::{Catalogue, MAX_FILE};
use crate::session_host::profile::WorkspaceDirectory;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use uuid::Uuid;

pub(crate) struct Request {
    pub operation: u64,
    pub binding: u64,
    pub directory: WorkspaceDirectory,
    pub account: Uuid,
    pub grid: String,
    pub write: Option<(Option<Catalogue>, Catalogue)>,
}
pub(crate) struct Reply {
    pub operation: u64,
    pub binding: u64,
    pub result: Result<Catalogue, ()>,
}
pub(crate) struct Lane {
    send: SyncSender<Request>,
    receive: Receiver<Reply>,
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
        Ok(Self { send, receive })
    }
    pub(crate) fn submit(&self, r: Request) -> bool {
        self.send.try_send(r).is_ok()
    }
    pub(crate) fn poll(&self) -> Option<Reply> {
        self.receive.try_recv().ok()
    }
}
fn perform(r: &Request) -> Result<Catalogue, ()> {
    let _lock = r.directory.lock().map_err(|_| ())?;
    let current = r
        .directory
        .read(MAX_FILE)
        .map_err(|_| ())?
        .map(|b| Catalogue::decode(&b, r.account, &r.grid))
        .transpose()
        .map_err(|_| ())?;
    match &r.write {
        None => Ok(current.unwrap_or_else(|| Catalogue::empty(r.account, r.grid.clone()))),
        Some((expected, next)) => {
            if current != *expected || next.account != r.account || next.grid != r.grid {
                return Err(());
            }
            let old_revision = current.as_ref().map_or(0, |c| c.revision);
            if next.revision != old_revision.checked_add(1).ok_or(())? {
                return Err(());
            }
            let b = next.encode().map_err(|_| ())?;
            r.directory.publish(&b).map_err(|_| ())?;
            Ok(next.clone())
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
            operation: 1,
            binding: 1,
            directory: WorkspaceDirectory::select(account, &grid).unwrap(),
            account,
            grid: grid.clone(),
            write,
        };
        let empty = perform(&make(None)).unwrap();
        assert_eq!(empty.records.len(), 0);
        let mut next = empty;
        next.revision = 1;
        assert_eq!(perform(&make(Some((None, next.clone())))).unwrap(), next);
        assert!(perform(&make(Some((None, next.clone())))).is_err());
        let directory = WorkspaceDirectory::select(account, &grid).unwrap();
        let unknown = b"{\"version\":999,\"future\":true}";
        directory.publish(unknown).unwrap();
        assert!(perform(&make(None)).is_err());
        assert!(perform(&make(Some((Some(next.clone()), next)))).is_err());
        assert_eq!(directory.read(MAX_FILE).unwrap().unwrap(), unknown);
        assert_eq!(
            perform(&Request {
                grid: "e".repeat(64),
                directory: WorkspaceDirectory::select(account, &"e".repeat(64)).unwrap(),
                ..make(None)
            })
            .unwrap()
            .records
            .len(),
            0
        );
    }
}
