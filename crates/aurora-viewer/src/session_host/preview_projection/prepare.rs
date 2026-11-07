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
        self.run_before_publish(|| {})
    }
    fn run_before_publish(self, before_publish: impl FnOnce()) -> Completion {
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
            before_publish();
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
struct Queued {
    job: Job,
    sink: std::sync::Weak<CompletionSink>,
}
struct Mailbox {
    jobs: VecDeque<Queued>,
}
struct Worker {
    mailbox: Mutex<Mailbox>,
    wake: Condvar,
}
struct CompletionSink {
    owner: Arc<Frontdoor>,
    done: Mutex<Vec<Completion>>,
}
/// The Broker owns its bounded results. The global worker holds only a Weak
/// sink, so terminal stop cannot orphan finished payloads in a static cache.
pub(super) struct Preparer {
    worker: Arc<Worker>,
    sink: Arc<CompletionSink>,
}
impl Preparer {
    /// Called lazily by the Broker after real owned admission, never from UI.
    /// The one stack+bounded metadata fit inside the host's 8MiB service reserve;
    /// raw32/UI24 pixel credits are separate and are retained through errors.
    pub fn shared(owner: Arc<Frontdoor>) -> Result<Self, PreviewError> {
        static WORKER: OnceLock<Result<Arc<Worker>, PreviewError>> = OnceLock::new();
        let worker = WORKER
            .get_or_init(|| {
                let worker = Arc::new(Worker {
                    mailbox: Mutex::new(Mailbox {
                        jobs: VecDeque::with_capacity(MAX_WORKERS),
                    }),
                    wake: Condvar::new(),
                });
                let runner = worker.clone();
                std::thread::Builder::new()
                    .name("session-preview-prepare".into())
                    .stack_size(256 * 1024)
                    .spawn(move || runner.run())
                    .map_err(|_| PreviewError::Unavailable)?;
                Ok(worker)
            })
            .clone()?;
        Ok(Self::owner(worker, owner))
    }
    fn owner(worker: Arc<Worker>, owner: Arc<Frontdoor>) -> Self {
        Self {
            worker,
            sink: Arc::new(CompletionSink {
                owner,
                done: Mutex::new(Vec::with_capacity(MAX_WORKERS)),
            }),
        }
    }
    #[expect(
        clippy::result_large_err,
        reason = "return the bounded indivisible owned job on contention; do not allocate a metadata box under the queue lock"
    )]
    pub fn submit(&self, job: Job) -> Result<(), Job> {
        if !Arc::ptr_eq(&job.owner, &self.sink.owner) || !self.sink.owner.current() {
            return Err(job);
        }
        let sink = Arc::downgrade(&self.sink);
        let retired;
        {
            let Ok(mut mailbox) = self.worker.mailbox.try_lock() else {
                return Err(job);
            };
            retired = mailbox
                .jobs
                .iter()
                .position(|old| old.job.source == job.source && old.sink.ptr_eq(&sink))
                .and_then(|at| mailbox.jobs.remove(at));
            if mailbox.jobs.len() >= MAX_WORKERS {
                return Err(job);
            }
            mailbox.jobs.push_back(Queued { job, sink });
        }
        drop(retired);
        self.worker.wake.notify_one();
        Ok(())
    }
    pub fn take(&self) -> Result<Vec<Completion>, PreviewError> {
        let mut done = self.sink.done.try_lock().map_err(|_| PreviewError::Busy)?;
        Ok(std::mem::replace(&mut *done, Vec::with_capacity(MAX_WORKERS)))
    }
}
impl Worker {
    fn complete(&self, queued: Queued) {
        self.publish(queued.sink, queued.job.run());
    }
    fn publish(&self, sink: std::sync::Weak<CompletionSink>, done: Completion) {
        let Some(sink) = sink.upgrade() else { return };
        let retired;
        {
            let mut mailbox = sink.done.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            retired = mailbox
                .iter()
                .position(|old| old.source == done.source)
                .map(|at| mailbox.swap_remove(at));
            if mailbox.len() < MAX_WORKERS && done.lifetime.current() && sink.owner.current() {
                mailbox.push(done);
            }
        }
        drop(retired);
        // A rejected completion and the final transient upgraded sink are both
        // destroyed here, after releasing the completion mutex.
    }
    fn run(&self) {
        loop {
            let queued = {
                let mut mailbox = self.mailbox.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                while mailbox.jobs.is_empty() {
                    mailbox = self.wake.wait(mailbox).unwrap_or_else(std::sync::PoisonError::into_inner);
                }
                mailbox.jobs.pop_front().expect("nonempty bounded preparation queue")
            };
            self.complete(queued);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::store::tests::Owned;
    use super::*;

    fn worker() -> Arc<Worker> {
        Arc::new(Worker {
            mailbox: Mutex::new(Mailbox {
                jobs: VecDeque::with_capacity(MAX_WORKERS),
            }),
            wake: Condvar::new(),
        })
    }
    fn pop(worker: &Worker) -> Queued {
        worker.mailbox.lock().unwrap().jobs.pop_front().unwrap()
    }
    fn submit(preparer: &Preparer, job: Job) {
        assert!(preparer.submit(job).is_ok());
    }

    #[test]
    fn preview_host_completed_conversion_owner_drop_releases_real_raw_and_private_pixel_credits() {
        let (mut fixture, mut job, raw) = Owned::preparation_fixture();
        // Inject a separate real credit pool only to measure this ownership
        // case deterministically alongside parallel tests. Production always
        // uses Shared's ONE process pool, covered by the actual reader test.
        let pixels = UiPixelBudget::new();
        job.budget = pixels.clone();
        let worker = worker();
        let preparer = Preparer::owner(worker.clone(), job.owner.clone());
        submit(&preparer, job);
        worker.complete(pop(&worker));
        assert_eq!(preparer.sink.done.lock().unwrap().len(), 1);
        assert!(preparer.sink.done.lock().unwrap()[0].payload.is_some());
        assert!(raw.used() > 0);
        assert!(pixels.used() > 0);
        let weak = Arc::downgrade(&preparer.sink);
        fixture.stop_preparation_fixture();
        drop(preparer); // Same result-owner disposal performed by runtime.stop.
        assert!(weak.upgrade().is_none());
        assert_eq!(raw.used(), 0);
        assert_eq!(pixels.used(), 0);
    }

    #[test]
    fn preview_host_queued_and_running_conversion_after_stop_cannot_publish_or_release_credits_early() {
        for running in [false, true] {
            let (mut fixture, mut job, raw) = Owned::preparation_fixture();
            let pixels = UiPixelBudget::new();
            job.budget = pixels.clone();
            let worker = worker();
            let preparer = Preparer::owner(worker.clone(), job.owner.clone());
            submit(&preparer, job);
            let queued = running.then(|| pop(&worker));
            let weak = Arc::downgrade(&preparer.sink);
            fixture.stop_preparation_fixture();
            drop(preparer);
            assert!(weak.upgrade().is_none());
            assert!(raw.used() > 0, "actual queued/running owner still holds its raw allocation");
            worker.complete(queued.unwrap_or_else(|| pop(&worker)));
            assert_eq!(raw.used(), 0);
            assert_eq!(pixels.used(), 0);
            assert!(worker.mailbox.lock().unwrap().jobs.is_empty());
        }
    }

    #[test]
    fn preview_host_two_completion_owners_cannot_steal_or_coalesce_each_others_results() {
        let (mut fixture, mut job, raw) = Owned::preparation_fixture();
        let pixels = UiPixelBudget::new();
        job.budget = pixels.clone();
        let worker = worker();
        let first = Preparer::owner(worker.clone(), job.owner.clone());
        let other = Preparer::owner(worker.clone(), job.owner.clone());
        let mut another = fixture.actual_preparation_job();
        another.budget = pixels.clone();
        submit(&first, job);
        submit(&other, another);
        assert_eq!(
            worker.mailbox.lock().unwrap().jobs.len(),
            2,
            "same-source jobs of different owners never coalesce"
        );
        worker.complete(pop(&worker));
        assert!(other.take().unwrap().is_empty());
        assert_eq!(first.sink.done.lock().unwrap().len(), 1);
        let results = first.take().unwrap();
        assert!(first.take().unwrap().is_empty());
        assert_eq!(results.len(), 1);
        assert!(results[0].payload.is_some());
        worker.complete(pop(&worker));
        let other_results = other.take().unwrap();
        assert_eq!(other_results.len(), 1);
        assert!(other_results[0].payload.is_some());
        fixture.stop_preparation_fixture();
        drop(first);
        drop(other);
        assert!(raw.used() > 0, "a handed-out result is a genuine physical owner");
        assert!(pixels.used() > 0);
        drop(results);
        assert!(raw.used() > 0, "the other owner still holds its result");
        drop(other_results);
        assert_eq!(raw.used(), 0);
        assert_eq!(pixels.used(), 0);
    }
    #[test]
    fn preview_host_stop_during_actual_conversion_drops_unpublished_pixels_after_final_original_guard_check() {
        let (mut fixture, mut job, raw) = Owned::preparation_fixture();
        let pixels = UiPixelBudget::new();
        job.budget = pixels.clone();
        let worker = worker();
        let preparer = Preparer::owner(worker.clone(), job.owner.clone());
        submit(&preparer, job);
        let Queued { job, sink } = pop(&worker);
        let done = job.run_before_publish(|| {
            assert!(pixels.used() > 0, "the actual private conversion has completed");
            fixture.stop_preparation_fixture();
            drop(preparer);
            assert!(pixels.used() > 0, "in-flight converted allocation is still physically owned");
            assert!(raw.used() > 0);
        });
        assert!(done.payload.is_none(), "original owner/lifetime is checked AFTER conversion");
        worker.publish(sink, done);
        assert_eq!(raw.used(), 0);
        assert_eq!(pixels.used(), 0);
    }
}
