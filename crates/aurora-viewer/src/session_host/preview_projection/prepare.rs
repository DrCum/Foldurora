//! One process-wide off-GUI converter. It borrows authenticated admitted bytes
//! inside the real retention callback and creates one credited opaque image.
use super::*;
use crate::session_host::preview_io::{EpochFence, PublicationGuard, RetainedFrame};
use std::collections::VecDeque;
use std::sync::{Condvar, OnceLock};

pub(super) struct Payload {
    pub revision: u64,
    pub image: PrivateImage,
    pub admitted: AdmittedImage,
    pub raw: Arc<RetainedFrame>,
}
pub(super) struct ReadScope {
    pub control: ControlGuard,
    pub control_head: EpochFence,
    pub metadata_head: PublicationGuard,
    pub view: EpochFence,
    pub shared: Arc<Shared>,
    pub binding: (View, ViewNamespace),
    pub service: u64,
}
/// An immutable payload pair may get a new read wrapper under a genuinely fresh
/// accepted control receipt, without recopying pixels or renewing that receipt.
pub(in crate::session_host) struct PreparedImage {
    payload: Arc<Payload>,
    control: ControlGuard,
    control_head: EpochFence,
    metadata_head: PublicationGuard,
    view: EpochFence,
    shared: Arc<Shared>,
    binding: (View, ViewNamespace),
    service: u64,
}
impl PreparedImage {
    pub(super) fn for_read(payload: Arc<Payload>, scope: ReadScope) -> Self {
        let ReadScope {
            control,
            control_head,
            metadata_head,
            view,
            shared,
            binding,
            service,
        } = scope;
        Self {
            payload,
            control,
            control_head,
            metadata_head,
            view,
            shared,
            binding,
            service,
        }
    }
    pub fn revision(&self) -> u64 {
        self.payload.revision
    }
    pub fn image(&self) -> PrivateImage {
        self.payload.image.clone()
    }
    pub fn admitted(&self) -> &AdmittedImage {
        &self.payload.admitted
    }
    pub fn current(&self) -> bool {
        let now = self.shared.now();
        self.shared.frontdoor.current()
            && self.shared.io_generation.load(Ordering::Acquire) == self.service
            && self.view.current()
            && self.shared.io_generation.load(Ordering::Acquire) == self.service
            && self.control_head.current()
            && self.metadata_head.current()
            && self.payload.admitted.current(now)
            && self.payload.raw.with_pixels(&self.control, now, |_| ()).is_some()
            && self.metadata_head.current()
            && self.control_head.current()
            && self.view.current()
    }
    pub(super) fn refresh(&self, read: &PreviewRead) -> Option<Self> {
        let fresh = read.image_guard.as_ref()?;
        if !read.view_current()
            || !fresh.metadata_head.current()
            || self.shared.instance != read.shared.instance
            || self.service != fresh.service
            || self.binding != (read.view.view, read.view.namespace)
            || self.payload.admitted.stamp().claim().source() != read.view.source
            || !self.payload.admitted.current(read.now())
        {
            return None;
        }
        let refreshed = Self::for_read(
            self.payload.clone(),
            ReadScope {
                control: fresh.control.clone(),
                control_head: read.control_head.clone(),
                metadata_head: fresh.metadata_head.clone(),
                view: read.view_guard.clone(),
                shared: read.shared.clone(),
                binding: self.binding,
                service: fresh.service,
            },
        );
        refreshed.current().then_some(refreshed)
    }
    #[cfg(test)]
    pub(super) fn same_payload(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.payload, &other.payload)
    }
}

pub(super) struct Job {
    pub source: PreviewSource,
    pub revision: u64,
    pub raw: Arc<RetainedFrame>,
    pub admitted: AdmittedImage,
    pub control: ControlGuard,
    pub candidate: EpochFence,
    pub lifetime: EpochFence,
    pub clock: Instant,
    pub budget: UiPixelBudget,
    pub owner: Arc<Frontdoor>,
    pub view_generation: EpochFence,
}
impl Job {
    fn now(&self) -> u64 {
        self.clock.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
    }
    fn current(&self) -> bool {
        self.owner.current()
            && self.view_generation.current()
            && self.lifetime.current()
            && self.candidate.current()
            && self.admitted.current(self.now())
    }
    fn run(self) -> Completion {
        let payload = (|| {
            if !self.current() {
                return None;
            }
            let size = self.admitted.stamp().geometry().size().map(|side| side as usize);
            let image = self
                .raw
                .with_pixels(&self.control, self.now(), |bytes| {
                    PrivateImage::from_rgba(&self.budget, size, bytes)
                })?
                .ok()?;
            if !self.current() {
                return None;
            }
            Some(Arc::new(Payload {
                revision: self.revision,
                image,
                admitted: self.admitted.clone(),
                raw: self.raw.clone(),
            }))
        })();
        Completion {
            source: self.source,
            revision: self.revision,
            lifetime: self.lifetime,
            payload,
        }
    }
}
pub(super) struct Completion {
    pub source: PreviewSource,
    pub revision: u64,
    pub lifetime: EpochFence,
    pub payload: Option<Arc<Payload>>,
}
struct Mailbox {
    jobs: VecDeque<Job>,
    done: Vec<Completion>,
}
struct Worker {
    mailbox: Mutex<Mailbox>,
    wake: Condvar,
}
#[derive(Clone)]
pub(super) struct Preparer(Arc<Worker>);
impl Preparer {
    /// Called lazily by the Broker after real owned admission, never from UI.
    /// The one stack+bounded metadata fit inside the host's 8MiB service reserve;
    /// raw32/UI24 pixel credits are separate and are retained through errors.
    pub fn shared() -> Result<Self, PreviewError> {
        static WORKER: OnceLock<Result<Preparer, PreviewError>> = OnceLock::new();
        WORKER
            .get_or_init(|| {
                let worker = Arc::new(Worker {
                    mailbox: Mutex::new(Mailbox {
                        jobs: VecDeque::with_capacity(MAX_WORKERS),
                        done: Vec::with_capacity(MAX_WORKERS),
                    }),
                    wake: Condvar::new(),
                });
                let owner = worker.clone();
                std::thread::Builder::new()
                    .name("session-preview-prepare".into())
                    .stack_size(256 * 1024)
                    .spawn(move || owner.run())
                    .map_err(|_| PreviewError::Unavailable)?;
                Ok(Preparer(worker))
            })
            .clone()
    }
    pub fn submit(&self, job: Job) -> Result<(), Job> {
        let retired;
        {
            let Ok(mut mailbox) = self.0.mailbox.try_lock() else {
                return Err(job);
            };
            retired = mailbox
                .jobs
                .iter()
                .position(|old| old.source == job.source)
                .and_then(|at| mailbox.jobs.remove(at));
            if mailbox.jobs.len() >= MAX_WORKERS {
                return Err(job);
            }
            mailbox.jobs.push_back(job);
        }
        drop(retired);
        self.0.wake.notify_one();
        Ok(())
    }
    pub fn take(&self) -> Result<Vec<Completion>, PreviewError> {
        let mut mailbox = self.0.mailbox.try_lock().map_err(|_| PreviewError::Busy)?;
        Ok(std::mem::replace(&mut mailbox.done, Vec::with_capacity(MAX_WORKERS)))
    }
}
impl Worker {
    fn run(&self) {
        loop {
            let job = {
                let mut mailbox = self.mailbox.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                while mailbox.jobs.is_empty() {
                    mailbox = self.wake.wait(mailbox).unwrap_or_else(std::sync::PoisonError::into_inner);
                }
                mailbox.jobs.pop_front().expect("nonempty bounded preparation queue")
            };
            let done = job.run();
            let retired;
            {
                let mut mailbox = self.mailbox.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                retired = mailbox
                    .done
                    .iter()
                    .position(|old| old.source == done.source)
                    .map(|at| mailbox.done.swap_remove(at));
                if mailbox.done.len() < MAX_WORKERS && done.lifetime.current() {
                    mailbox.done.push(done);
                }
            }
            drop(retired);
        }
    }
}
