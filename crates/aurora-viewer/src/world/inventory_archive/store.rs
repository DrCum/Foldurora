//! Closed native metadata IO. All filesystem policy and atomic publication
//! belong to NativeDirectory; this module accepts no path or read-limit input.
use super::{Document, Error, LoadTicket, MAX_BYTES, SaveIntent, Source};
use crate::session_host::profile::{NativeDirectory, NativeStoreCancellation, NativeStoreKind};
use std::{
    io,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
};

// Separate from Favorites. Replacing World or canceling an owner never resets
// these credits. IO ordering cannot be acquired by a GUI observer or poll.
const MAX_WORKERS: usize = 2;
static WORKERS: OnceLock<WorkerPool> = OnceLock::new();
static IO_ORDER: Mutex<()> = Mutex::new(());

#[derive(Clone)]
struct WorkerPool(Arc<AtomicUsize>);
impl Default for WorkerPool {
    fn default() -> Self {
        WORKERS.get_or_init(|| Self(Arc::new(AtomicUsize::new(0)))).clone()
    }
}
impl WorkerPool {
    fn admit(&self) -> Option<Permit> {
        self.0
            .try_update(Ordering::AcqRel, Ordering::Acquire, |used| (used < MAX_WORKERS).then(|| used + 1))
            .ok()?;
        Some(Permit(self.0.clone()))
    }
}
struct Permit(Arc<AtomicUsize>);
impl Drop for Permit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Clone)]
pub(super) enum Request {
    Load(LoadTicket),
    Save { intent: SaveIntent, original: Option<Arc<[u8]>> },
}
impl Request {
    fn source(&self) -> Source {
        match self {
            Self::Load(ticket) => ticket.source,
            Self::Save { intent, .. } => intent.source(),
        }
    }
}
pub(super) struct Loaded {
    pub document: Option<Document>,
    pub original: Option<Arc<[u8]>>,
}
pub(super) enum Reply {
    Loaded {
        ticket: LoadTicket,
        result: Result<Loaded, Error>,
    },
    Saved {
        intent: SaveIntent,
        result: Result<Arc<[u8]>, Error>,
    },
}
pub(super) struct Worker {
    requests: SyncSender<Request>,
    replies: Receiver<Reply>,
    cancellation: NativeStoreCancellation,
}

// Field order is intentional: actual worker-owned directory, queues, payloads
// and cancellation references drop before the sole permit, including unwind
// and an unstarted FnOnce dropped by Builder::spawn on error.
struct Job {
    resources: Resources,
    _permit: Permit,
}
struct Resources {
    directory: NativeDirectory,
    source: Source,
    cancellation: NativeStoreCancellation,
    requests: Receiver<Request>,
    replies: SyncSender<Reply>,
    #[cfg(test)]
    entering: Option<SyncSender<()>>,
    #[cfg(test)]
    _dropping: Option<tests::DropObservation>,
}
impl Worker {
    /// Busy is retriable without allocating a path, channel, cancellation or
    /// thread. Selection is native-only and is captured once by this worker.
    pub(super) fn start(source: Source) -> Result<Option<Self>, Error> {
        Self::start_in(source, &WorkerPool::default())
    }
    fn start_in(source: Source, pool: &WorkerPool) -> Result<Option<Self>, Error> {
        let Some(permit) = pool.admit() else { return Ok(None) };
        let directory =
            NativeDirectory::select(NativeStoreKind::InventoryArchive, source.account(), &source.grid().namespace()).map_err(io_error)?;
        let (requests, incoming) = mpsc::sync_channel(1);
        let (outgoing, replies) = mpsc::sync_channel(1);
        let cancellation = NativeStoreCancellation::new();
        let job = Job {
            resources: Resources {
                directory,
                source,
                cancellation: cancellation.clone(),
                requests: incoming,
                replies: outgoing,
                #[cfg(test)]
                entering: None,
                #[cfg(test)]
                _dropping: None,
            },
            _permit: permit,
        };
        std::thread::Builder::new()
            .name("aurora-inventory-archive".into())
            .spawn(move || job.run())
            .map_err(io_error)?;
        Ok(Some(Self {
            requests,
            replies,
            cancellation,
        }))
    }
    pub(super) fn submit(&self, request: Request) -> Result<(), Error> {
        self.requests.try_send(request).map_err(|_| Error::Unavailable)
    }
    pub(super) fn poll(&self) -> Result<Option<Reply>, Error> {
        match self.replies.try_recv() {
            Ok(reply) => Ok(Some(reply)),
            Err(mpsc::TryRecvError::Empty) => Ok(None),
            Err(mpsc::TryRecvError::Disconnected) => Err(Error::Unavailable),
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.cancellation.cancel();
    } // Disconnect wakes an idle worker; there is no GUI join or IO mutex.
}
impl Job {
    fn run(self) {
        while !self.resources.cancellation.is_cancelled() {
            let Ok(request) = self.resources.requests.recv() else { break };
            if self.resources.cancellation.is_cancelled() {
                break;
            }
            let result = self.resources.perform(&request);
            let reply = match request {
                Request::Load(ticket) => Reply::Loaded {
                    ticket,
                    result: result.map(|(document, original)| Loaded { document, original }),
                },
                Request::Save { intent, .. } => Reply::Saved {
                    intent,
                    result: result.and_then(|(_, bytes)| bytes.ok_or(Error::Invalid)),
                },
            };
            // No blocking reply send. A broken/full sink is terminal; its
            // remaining reply can be drained, then disconnect fails pending.
            if self.resources.cancellation.is_cancelled() || self.resources.replies.try_send(reply).is_err() {
                break;
            }
        }
    }
}
fn io_error(_: io::Error) -> Error {
    Error::Unavailable
}
fn decode(original: Option<&[u8]>, source: Source) -> Result<Option<Document>, Error> {
    original.map(|bytes| Document::decode(bytes, source)).transpose()
}
fn prepare(request: &Request, current: Option<&[u8]>) -> Result<Vec<u8>, Error> {
    let Request::Save { intent, original } = request else {
        return Err(Error::Invalid);
    };
    if original.as_deref() != current {
        return Err(Error::Stale);
    }
    let document = decode(current, intent.source())?.unwrap_or_else(|| Document::empty(intent.source()));
    if document != *intent.expected()
        || !intent.next().matches(intent.source())
        || document.publication().checked_add(1) != Some(intent.next().publication())
    {
        return Err(Error::Stale);
    }
    intent.next().encode()
}
type Outcome = (Option<Document>, Option<Arc<[u8]>>);
impl Resources {
    fn check(&self) -> Result<(), Error> {
        if self.cancellation.is_cancelled() {
            Err(Error::Unavailable)
        } else {
            Ok(())
        }
    }
    fn perform(&self, request: &Request) -> Result<Outcome, Error> {
        if match request {
            Request::Load(ticket) => ticket.instance == 0,
            Request::Save { intent, .. } => intent.operation() == 0,
        } {
            return Err(Error::Invalid);
        }
        if request.source() != self.source
            || NativeStoreKind::InventoryArchive.max_bytes() != MAX_BYTES
            || !self.directory.matches(
                NativeStoreKind::InventoryArchive,
                self.source.account(),
                &self.source.grid().namespace(),
            )
        {
            return Err(Error::Scope);
        }
        self.check()?;
        #[cfg(test)]
        if let Some(entering) = &self.entering {
            let _ = entering.try_send(());
        }
        let _order = IO_ORDER.lock().map_err(|_| Error::Unavailable)?;
        self.check()?;
        if matches!(request, Request::Load(_)) {
            // Initial load never takes store.lock, creates the private store,
            // repairs ACLs or publishes an empty file. Only actual absence is
            // Missing; unknown/nonprivate/oversized bytes remain unavailable.
            let original = match self.directory.read() {
                Ok(original) => original,
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => return Err(io_error(error)),
            };
            let document = decode(original.as_deref(), self.source)?;
            self.check()?;
            return Ok((document, original.map(Arc::from)));
        }
        let _lock = self.directory.lock().map_err(io_error)?;
        self.check()?;
        let original = self.directory.read().map_err(io_error)?;
        let bytes = prepare(request, original.as_deref())?;
        self.check()?;
        self.directory
            .publish_cancellable(&bytes, original.as_deref(), &self.cancellation)
            .map_err(io_error)?;
        Ok((None, Some(Arc::from(bytes))))
    }
}

#[cfg(test)]
mod tests;
