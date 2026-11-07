//! Actual bounded metadata IO consumer. NativeDirectory owns all private-path,
//! owner/ACL/no-follow/link-count/CAS/atomic publication work; no asset IO here.
use super::{
    model::{self, Catalogue, Change},
    playback::Facts,
};
use crate::session_host::profile::{NativeDirectory, NativeStoreCancellation, NativeStoreKind};
use std::{
    cell::Cell,
    sync::{
        Arc,
        mpsc::{self, Receiver, SyncSender},
    },
};
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct Owner {
    pub instance: u64,
    pub epoch: u64,
    pub facts: Facts,
}
#[derive(Clone)]
pub(crate) struct Snapshot {
    pub data: Arc<Catalogue>,
    original: Option<Arc<[u8]>>,
}
impl Snapshot {
    pub(crate) fn memory(data: Catalogue) -> Self {
        Self {
            data: Arc::new(data),
            original: None,
        }
    }
    pub fn original(&self) -> Option<Arc<[u8]>> {
        self.original.clone()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Error {
    Private,
    Cancelled,
    Conflict,
    Invalid,
    Model(model::Error),
}
pub(crate) struct Request {
    pub owner: Owner,
    pub operation: u64,
    pub directory: NativeDirectory,
    pub cancellation: NativeStoreCancellation,
    pub edit: Option<(Option<Arc<[u8]>>, Change)>,
}
pub(crate) struct Reply {
    pub owner: Owner,
    pub operation: u64,
    pub result: Result<Snapshot, Error>,
}
pub(crate) struct Lane {
    send: SyncSender<Request>,
    receive: Receiver<Reply>,
    busy: Cell<bool>,
}
impl Lane {
    pub fn new() -> std::io::Result<Self> {
        let (send, requests) = mpsc::sync_channel::<Request>(1);
        let (results, receive) = mpsc::sync_channel(1);
        std::thread::Builder::new().name("aurora-gesture-boards".into()).spawn(move || {
            while let Ok(request) = requests.recv() {
                let result = perform(&request);
                if results
                    .send(Reply {
                        owner: request.owner,
                        operation: request.operation,
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
            busy: Cell::new(false),
        })
    }
    pub fn try_submit(&self, request: Request) -> bool {
        if self.busy.get() {
            return false;
        }
        if self.send.try_send(request).is_ok() {
            self.busy.set(true);
            true
        } else {
            false
        }
    }
    pub fn poll(&self) -> Result<Option<Reply>, ()> {
        match self.receive.try_recv() {
            Ok(reply) => {
                self.busy.set(false);
                Ok(Some(reply))
            }
            Err(mpsc::TryRecvError::Empty) => Ok(None),
            Err(mpsc::TryRecvError::Disconnected) => {
                self.busy.set(false);
                Err(())
            }
        }
    }
}
fn io_error(error: std::io::Error) -> Error {
    if error.kind() == std::io::ErrorKind::Interrupted {
        Error::Cancelled
    } else {
        Error::Private
    }
}
fn perform(r: &Request) -> Result<Snapshot, Error> {
    if r.owner.instance == 0
        || r.owner.epoch == 0
        || r.operation == 0
        || r.owner.facts.agent.is_nil()
        || r.owner.facts.login.is_nil()
        || r.owner.facts.logical_generation == 0
        || r.owner.facts.profile_generation == 0
        || NativeStoreKind::Boards.max_bytes() != model::MAX_FILE
        || !r
            .directory
            .matches(NativeStoreKind::Boards, r.owner.facts.agent, &r.owner.facts.grid.to_string())
    {
        return Err(Error::Invalid);
    }
    if r.cancellation.is_cancelled() {
        return Err(Error::Cancelled);
    }
    let _lock = r.directory.lock().map_err(io_error)?;
    if r.cancellation.is_cancelled() {
        return Err(Error::Cancelled);
    }
    let original = r.directory.read().map_err(io_error)?;
    // Some invalid/unknown bytes are NEVER a missing empty catalogue.
    let current = decode(original.as_deref(), r.owner.facts)?;
    if r.cancellation.is_cancelled() {
        return Err(Error::Cancelled);
    }
    let Some((expected, change)) = &r.edit else {
        return Ok(Snapshot {
            data: Arc::new(current),
            original: original.map(Arc::from),
        });
    };
    let (next, bytes) = prepare(&current, original.as_deref(), expected.as_deref(), change)?;
    if r.cancellation.is_cancelled() {
        return Err(Error::Cancelled);
    }
    r.directory
        .publish_cancellable(&bytes, original.as_deref(), &r.cancellation)
        .map_err(io_error)?;
    Ok(Snapshot {
        data: Arc::new(next),
        original: Some(Arc::from(bytes)),
    })
}

fn decode(original: Option<&[u8]>, facts: Facts) -> Result<Catalogue, Error> {
    match original {
        Some(bytes) => Catalogue::decode(bytes, facts.agent, facts.grid),
        None => Catalogue::empty(facts.agent, facts.grid),
    }
    .map_err(Error::Model)
}
fn prepare(current: &Catalogue, original: Option<&[u8]>, expected: Option<&[u8]>, change: &Change) -> Result<(Catalogue, Vec<u8>), Error> {
    if expected != original {
        return Err(Error::Conflict);
    }
    let next = current.apply(change.clone()).map_err(Error::Model)?;
    let bytes = next.encode().map_err(Error::Model)?;
    Ok((next, bytes))
}
#[cfg(test)]
mod tests;
