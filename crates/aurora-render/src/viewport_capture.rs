//! Bounded state and read-only guards for an asynchronous viewport readback.
//! This module performs no rendering, mapping, polling, waits or IO and grants no session
//! ownership. Geometry follows `fssessionpreview.h::fittedPreview` (Legacy
//! Viewer); an eventual backend must separately prove composition and color.

use crate::WorldViewport;
pub use crate::viewport_capture_retirement::CaptureRetirementProgress;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU8, AtomicU64, AtomicUsize, Ordering},
};
use std::time::Instant;

pub const MAX_CAPTURE_BYTES: usize = 640 * 360 * 4;
/// Process-wide target/staging/packed-pixel pool. The remaining fixed MiB is
/// withheld for worker image IO/auth/framing, never another pixel pool.
pub const WORKER_CAPTURE_BYTES: usize = 7 * 1024 * 1024;
pub const WORKER_CAPTURE_IO_HEADROOM_BYTES: usize = 1024 * 1024;
pub const WORKER_CAPTURE_TOTAL_BYTES: usize = WORKER_CAPTURE_BYTES + WORKER_CAPTURE_IO_HEADROOM_BYTES;
pub const CAPTURE_TIMEOUT_MILLIS: u64 = 1000;
static NEXT_QUEUE: AtomicU64 = AtomicU64::new(1);

fn allocate_queue_instance(counter: &AtomicU64) -> Result<u64, CaptureError> {
    counter
        .try_update(Ordering::AcqRel, Ordering::Acquire, |next| next.checked_add(1))
        .map_err(|_| CaptureError::Exhausted)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureFormat {
    Rgba8OpaqueSrgb,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CaptureCap {
    #[default]
    Small,
    Medium,
    Large,
}

impl CaptureCap {
    pub fn dimensions(self) -> (u32, u32) {
        match self {
            Self::Small => (320, 180),
            Self::Medium => (480, 270),
            Self::Large => (640, 360),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureError {
    Unsupported,
    DeviceLost,
    Invalid,
    Busy,
    Capacity,
    Stale,
    Cancelled,
    MapFailed,
    TimedOut,
    Exhausted,
    Clock,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureLayout {
    size: (u32, u32),
    row_bytes: u32,
    padded_row_bytes: u32,
    payload_bytes: usize,
    staging_bytes: usize,
}

impl CaptureLayout {
    pub fn fit(viewport: WorldViewport, cap: CaptureCap) -> Result<Self, CaptureError> {
        if viewport.width == 0 || viewport.height == 0 {
            return Err(CaptureError::Invalid);
        }
        let (max_w, max_h) = cap.dimensions();
        let mut w = u64::from(max_w);
        let mut h = w * u64::from(viewport.height) / u64::from(viewport.width);
        if h > u64::from(max_h) {
            h = u64::from(max_h);
            w = h * u64::from(viewport.width) / u64::from(viewport.height);
        }
        w -= w % 4;
        Self::new(w as u32, h as u32)
    }

    pub fn new(width: u32, height: u32) -> Result<Self, CaptureError> {
        if !(4..=640).contains(&width) || !(4..=360).contains(&height) || !width.is_multiple_of(4) {
            return Err(CaptureError::Invalid);
        }
        let row_bytes = width.checked_mul(4).ok_or(CaptureError::Invalid)?;
        let padded_row_bytes = row_bytes.checked_add(255).ok_or(CaptureError::Invalid)? / 256 * 256;
        let payload_bytes = usize::try_from(u64::from(row_bytes) * u64::from(height)).map_err(|_| CaptureError::Invalid)?;
        let staging_bytes = usize::try_from(u64::from(padded_row_bytes) * u64::from(height)).map_err(|_| CaptureError::Invalid)?;
        if payload_bytes > MAX_CAPTURE_BYTES || staging_bytes > MAX_CAPTURE_BYTES {
            return Err(CaptureError::Invalid);
        }
        Ok(Self {
            size: (width, height),
            row_bytes,
            padded_row_bytes,
            payload_bytes,
            staging_bytes,
        })
    }

    pub fn size(self) -> (u32, u32) {
        self.size
    }
    pub fn row_bytes(self) -> u32 {
        self.row_bytes
    }
    pub fn padded_row_bytes(self) -> u32 {
        self.padded_row_bytes
    }
    pub fn payload_bytes(self) -> usize {
        self.payload_bytes
    }
    pub fn staging_bytes(self) -> usize {
        self.staging_bytes
    }
}

/// Epochs must come from the owning renderer. Device epochs increase across
/// renderer recreation; surface epochs increase before geometry replacement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureBoundary {
    device_epoch: u64,
    surface_epoch: u64,
    size: (u32, u32),
}

impl CaptureBoundary {
    pub fn new(device_epoch: u64, surface_epoch: u64, size: (u32, u32)) -> Result<Self, CaptureError> {
        if device_epoch == 0 || surface_epoch == 0 || size.0 == 0 || size.1 == 0 {
            return Err(CaptureError::Invalid);
        }
        Ok(Self {
            device_epoch,
            surface_epoch,
            size,
        })
    }
    pub fn device_epoch(self) -> u64 {
        self.device_epoch
    }
    pub fn surface_epoch(self) -> u64 {
        self.surface_epoch
    }
    pub fn size(self) -> (u32, u32) {
        self.size
    }

    fn contains(self, view: WorldViewport) -> bool {
        view.width != 0
            && view.height != 0
            && view.x.checked_add(view.width).is_some_and(|right| right <= self.size.0)
            && view.y.checked_add(view.height).is_some_and(|bottom| bottom <= self.size.1)
    }
}

/// Allocated exclusively by CaptureQueue. Cancellation never reuses its epoch
/// or ticket; the device epoch separates recreated renderer instances.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureToken {
    instance: u64,
    device: u64,
    epoch: u64,
    ticket: u64,
}

impl CaptureToken {
    pub fn instance(self) -> u64 {
        self.instance
    }
    pub fn device_epoch(self) -> u64 {
        self.device
    }
    pub fn epoch(self) -> u64 {
        self.epoch
    }
    pub fn ticket(self) -> u64 {
        self.ticket
    }
}

/// Frame-free physical renderer witness. These atomics retain no renderer,
/// queue, mutex, callback, pixel allocation or GPU resource. Normal capture
/// cancellation/session fencing does not retire already admitted history.
#[derive(Clone, Debug)]
pub struct CaptureRetentionGuard {
    instance: u64,
    device: u64,
    generation: u64,
    physical: Arc<AtomicU64>,
    lost: Arc<AtomicBool>,
}
impl CaptureRetentionGuard {
    pub(crate) fn new(instance: u64, device: u64, physical: Arc<AtomicU64>, lost: Arc<AtomicBool>) -> Option<Self> {
        let guard = Self {
            instance,
            device,
            generation: physical.load(Ordering::Acquire),
            physical,
            lost,
        };
        guard.current().then_some(guard)
    }
    pub fn instance(&self) -> u64 {
        self.instance
    }
    pub fn device_epoch(&self) -> u64 {
        self.device
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn current(&self) -> bool {
        self.instance != 0
            && self.device != 0
            && self.generation != 0
            && !self.lost.load(Ordering::Acquire)
            && self.physical.load(Ordering::Acquire) == self.generation
            && !self.lost.load(Ordering::Acquire)
    }
}

/// Read-only publication lease for one exact result. Construction is confined
/// to the renderer after current boundary/token checks. IO only loads atomics
/// and the same-process clock; it cannot refresh the original capture deadline.
#[derive(Clone, Debug)]
pub struct CapturePublicationGuard {
    token: CaptureToken,
    generation: u64,
    publication: Arc<AtomicU64>,
    ticket: Arc<AtomicU64>,
    retention: CaptureRetentionGuard,
    issued: Instant,
    deadline: Instant,
}
impl CapturePublicationGuard {
    pub(crate) fn new(
        token: CaptureToken,
        generation: u64,
        publication: Arc<AtomicU64>,
        ticket: Arc<AtomicU64>,
        retention: CaptureRetentionGuard,
        deadline: Instant,
    ) -> Option<Self> {
        let guard = Self {
            token,
            generation,
            publication,
            ticket,
            retention,
            issued: Instant::now(),
            deadline,
        };
        guard.current().then_some(guard)
    }
    pub fn token(&self) -> CaptureToken {
        self.token
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn deadline(&self) -> Instant {
        self.deadline
    }
    pub fn current(&self) -> bool {
        self.current_at(Instant::now())
    }
    pub(crate) fn current_at(&self, now: Instant) -> bool {
        self.generation != 0
            && self.token.epoch != 0
            && self.token.ticket != 0
            && self.token.instance == self.retention.instance
            && self.token.device == self.retention.device
            && self.retention.current()
            && now >= self.issued
            && now < self.deadline
            && self.publication.load(Ordering::Acquire) == self.generation
            && self.ticket.load(Ordering::Acquire) == self.token.ticket
            && self.retention.current()
    }
}

#[derive(Clone)]
pub struct CaptureRequest {
    token: CaptureToken,
    boundary: CaptureBoundary,
    viewport: WorldViewport,
    layout: CaptureLayout,
    // The simple public request API carries allocation credit through clones.
    // The backend-cache metadata path instead uses CaptureResources so fresh
    // tokens do not repeatedly reserve a retained physical pair.
    _credit: Option<Arc<Credit>>,
}

impl CaptureRequest {
    pub fn token(&self) -> CaptureToken {
        self.token
    }
    pub fn boundary(&self) -> CaptureBoundary {
        self.boundary
    }
    pub fn viewport(&self) -> WorldViewport {
        self.viewport
    }
    pub fn layout(&self) -> CaptureLayout {
        self.layout
    }
}

#[derive(Clone)]
pub struct CaptureBudget(Arc<Budget>);
struct Budget {
    limit: usize,
    used: AtomicUsize,
}
struct Credit {
    budget: CaptureBudget,
    bytes: usize,
}

/// Token-independent credit for one physical cached target/staging pair.
/// Backend storage and submitted/map callbacks retain clones until retirement.
#[derive(Clone)]
pub(crate) struct CaptureResources {
    layout: CaptureLayout,
    _credit: Arc<Credit>,
}
impl CaptureResources {
    pub(crate) fn layout(&self) -> CaptureLayout {
        self.layout
    }
}

impl CaptureBudget {
    pub(crate) fn resources(&self, layout: CaptureLayout) -> Result<CaptureResources, CaptureError> {
        Ok(CaptureResources {
            layout,
            _credit: Arc::new(self.reserve(layout.payload_bytes + layout.staging_bytes)?),
        })
    }
    /// One process-wide pool for targets/staging/packed pixels, bounded to
    /// seven MiB so the worker's fixed one-MiB IO headroom stays inside eight.
    /// Replacement queues reuse it while old callbacks/images remain held.
    pub fn new(limit: usize) -> Result<Self, CaptureError> {
        if limit == 0 || limit > WORKER_CAPTURE_BYTES {
            return Err(CaptureError::Capacity);
        }
        Ok(Self(Arc::new(Budget {
            limit,
            used: AtomicUsize::new(0),
        })))
    }
    pub fn used(&self) -> usize {
        self.0.used.load(Ordering::Acquire)
    }

    fn reserve(&self, bytes: usize) -> Result<Credit, CaptureError> {
        self.0
            .used
            .try_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes).filter(|next| *next <= self.0.limit)
            })
            .map_err(|_| CaptureError::Capacity)?;
        Ok(Credit {
            budget: self.clone(),
            bytes,
        })
    }

    /// Reserve before allocating CPU pixels, including every actual copy.
    /// The same budget must be shared by the queue and its output buffers.
    pub fn pixels(&self, layout: CaptureLayout) -> Result<PixelReservation, CaptureError> {
        let credit = self.reserve(layout.payload_bytes)?;
        Ok(PixelReservation {
            layout,
            bytes: vec![0; layout.payload_bytes],
            credit,
        })
    }
}

impl Drop for Credit {
    fn drop(&mut self) {
        self.budget.0.used.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

/// Checked allocation, not a wire DTO or an image decoder. A backend fills the
/// bytes, then validates opacity. It must separately prove sRGB composition.
pub struct PixelReservation {
    layout: CaptureLayout,
    bytes: Vec<u8>,
    credit: Credit,
}
impl PixelReservation {
    pub fn bytes_mut(&mut self) -> &mut [u8] {
        &mut self.bytes
    }
    pub fn finish(self) -> Result<CapturePixels, CaptureError> {
        if self.bytes.as_chunks::<4>().0.iter().any(|pixel| pixel[3] != 255) {
            return Err(CaptureError::Invalid);
        }
        Ok(CapturePixels(Arc::new(Pixels {
            layout: self.layout,
            bytes: self.bytes,
            _credit: self.credit,
        })))
    }
}

struct Pixels {
    layout: CaptureLayout,
    bytes: Vec<u8>,
    _credit: Credit,
}
#[derive(Clone)]
pub struct CapturePixels(Arc<Pixels>);
impl CapturePixels {
    pub fn layout(&self) -> CaptureLayout {
        self.0.layout
    }
    pub fn bytes(&self) -> &[u8] {
        &self.0.bytes
    }
    pub fn format(&self) -> CaptureFormat {
        CaptureFormat::Rgba8OpaqueSrgb
    }
}

#[derive(Clone)]
pub struct CapturedViewport {
    request: CaptureRequest,
    frame: u64,
    pixels: CapturePixels,
    deadline: u64,
}
impl CapturedViewport {
    pub(crate) fn deadline_millis(&self) -> u64 {
        self.deadline
    }
    pub fn request(&self) -> &CaptureRequest {
        &self.request
    }
    pub fn frame(&self) -> u64 {
        self.frame
    }
    pub fn pixels(&self) -> &CapturePixels {
        &self.pixels
    }
}
pub enum CaptureOutcome {
    Ready(CapturedViewport),
    Failed(CaptureToken, CaptureError),
}

/// The map callback captures this handle once. Completing it can only change
/// its own atomic slot, including after cancellation or renderer replacement.
#[derive(Clone)]
pub struct MapCallback {
    request: CaptureRequest,
    status: Arc<AtomicU8>,
}
impl MapCallback {
    pub(crate) fn failed(&self) -> bool {
        self.status.load(Ordering::Acquire) == 2
    }

    pub fn token(&self) -> CaptureToken {
        self.request.token
    }
    pub fn complete(&self, success: bool) {
        let _ = self
            .status
            .compare_exchange(0, if success { 1 } else { 2 }, Ordering::Release, Ordering::Relaxed);
    }
}

struct Pending {
    request: CaptureRequest,
    deadline: u64,
    map: Option<(u64, MapCallback)>,
}

/// One outstanding request OR terminal outcome. `poll_map` advances completion
/// without another frame; the future render owner must separately nonblocking
/// poll its device and call Buffer::unmap on cancellation. This pure state never
/// claims that dropping a request has drained submitted GPU work.
pub struct CaptureQueue {
    boundary: CaptureBoundary,
    instance: u64,
    epoch: u64,
    next_ticket: u64,
    now: u64,
    budget: CaptureBudget,
    pending: Option<Pending>,
    outcome: Option<CaptureOutcome>,
}

impl CaptureQueue {
    #[cfg(test)]
    pub(crate) fn last_epoch_for_test(&mut self) {
        self.epoch = u64::MAX;
    }

    pub(crate) fn instance(&self) -> u64 {
        self.instance
    }
    pub(crate) fn request_current(&self, request: &CaptureRequest) -> bool {
        self.epoch != 0
            && request.boundary == self.boundary
            && self.pending.as_ref().is_some_and(|pending| pending.request.token == request.token)
    }

    pub(crate) fn fail_request(&mut self, token: CaptureToken, error: CaptureError) {
        if self.pending.as_ref().is_some_and(|pending| pending.request.token == token) {
            self.pending = None;
            self.outcome = Some(CaptureOutcome::Failed(token, error));
        }
    }
    pub fn new(boundary: CaptureBoundary, budget: CaptureBudget) -> Result<Self, CaptureError> {
        let instance = allocate_queue_instance(&NEXT_QUEUE)?;
        Ok(Self {
            boundary,
            instance,
            epoch: 1,
            next_ticket: 1,
            now: 0,
            budget,
            pending: None,
            outcome: None,
        })
    }

    pub fn request(&mut self, viewport: WorldViewport, cap: CaptureCap, now: u64) -> Result<CaptureRequest, CaptureError> {
        self.request_inner(viewport, cap, now, true)
    }

    /// The render-owner cache independently reserves physical pair credit.
    /// This path freezes metadata only and never reserves the same GPU pair
    /// again for every fresh request token. Pixel allocation remains separate.
    pub(crate) fn request_metadata(&mut self, viewport: WorldViewport, cap: CaptureCap, now: u64) -> Result<CaptureRequest, CaptureError> {
        self.request_inner(viewport, cap, now, false)
    }

    fn request_inner(
        &mut self,
        viewport: WorldViewport,
        cap: CaptureCap,
        now: u64,
        reserve_gpu: bool,
    ) -> Result<CaptureRequest, CaptureError> {
        self.clock(now)?;
        if self.epoch == 0 {
            return Err(CaptureError::Exhausted);
        }
        if self.pending.is_some() || self.outcome.is_some() {
            return Err(CaptureError::Busy);
        }
        if !self.boundary.contains(viewport) {
            return Err(CaptureError::Invalid);
        }
        let layout = CaptureLayout::fit(viewport, cap)?;
        let deadline = now.checked_add(CAPTURE_TIMEOUT_MILLIS).ok_or_else(|| {
            self.epoch = 0;
            CaptureError::Clock
        })?;
        let next = self.next_ticket.checked_add(1).ok_or_else(|| {
            self.epoch = 0;
            CaptureError::Exhausted
        })?;
        // GPU target and staging reservations precede their eventual creation.
        let credit = if reserve_gpu {
            Some(self.budget.resources(layout)?._credit)
        } else {
            None
        };
        let request = CaptureRequest {
            token: CaptureToken {
                instance: self.instance,
                device: self.boundary.device_epoch,
                epoch: self.epoch,
                ticket: self.next_ticket,
            },
            boundary: self.boundary,
            viewport,
            layout,
            _credit: credit,
        };
        self.next_ticket = next;
        self.pending = Some(Pending {
            request: request.clone(),
            deadline,
            map: None,
        });
        Ok(request)
    }

    pub fn begin_map(&mut self, token: CaptureToken, boundary: CaptureBoundary, frame: u64, now: u64) -> Result<MapCallback, CaptureError> {
        self.poll_map(now)?;
        let pending = self.pending.as_mut().ok_or(CaptureError::Stale)?;
        if pending.request.token != token || pending.request.boundary != boundary {
            return Err(CaptureError::Stale);
        }
        if pending.map.is_some() {
            return Err(CaptureError::Busy);
        }
        let callback = MapCallback {
            request: pending.request.clone(),
            status: Arc::new(AtomicU8::new(0)),
        };
        pending.map = Some((frame, callback.clone()));
        Ok(callback)
    }

    /// Some(request) means a successful map is ready for bounded packing;
    /// repeated calls do not consume it or allocate a replacement buffer.
    pub fn poll_map(&mut self, now: u64) -> Result<Option<CaptureRequest>, CaptureError> {
        self.clock(now)?;
        let Some(pending) = &self.pending else {
            return Ok(None);
        };
        let error = if now >= pending.deadline {
            Some(CaptureError::TimedOut)
        } else if pending
            .map
            .as_ref()
            .is_some_and(|(_, callback)| callback.status.load(Ordering::Acquire) == 2)
        {
            Some(CaptureError::MapFailed)
        } else {
            None
        };
        if let Some(error) = error {
            let token = pending.request.token;
            self.pending = None;
            self.outcome = Some(CaptureOutcome::Failed(token, error));
            return Ok(None);
        }
        Ok(pending
            .map
            .as_ref()
            .filter(|(_, callback)| callback.status.load(Ordering::Acquire) == 1)
            .map(|_| pending.request.clone()))
    }

    pub fn finish_map(
        &mut self,
        token: CaptureToken,
        boundary: CaptureBoundary,
        pixels: CapturePixels,
        now: u64,
    ) -> Result<(), CaptureError> {
        let request = self.poll_map(now)?.ok_or(CaptureError::Stale)?;
        if request.token != token || request.boundary != boundary || pixels.layout() != request.layout {
            return Err(CaptureError::Stale);
        }
        // A caller cannot import allocations charged to a different budget.
        if !Arc::ptr_eq(&pixels.0._credit.budget.0, &self.budget.0) {
            return Err(CaptureError::Invalid);
        }
        let pending = self.pending.take().ok_or(CaptureError::Stale)?;
        let frame = pending.map.as_ref().map(|(frame, _)| *frame).ok_or(CaptureError::Stale)?;
        self.outcome = Some(CaptureOutcome::Ready(CapturedViewport {
            request,
            frame,
            pixels,
            deadline: pending.deadline,
        }));
        Ok(())
    }

    pub fn take_result(&mut self, now: u64) -> Result<Option<CaptureOutcome>, CaptureError> {
        self.poll_map(now)?;
        Ok(self.outcome.take().map(|outcome| match outcome {
            CaptureOutcome::Ready(result) if now >= result.deadline => CaptureOutcome::Failed(result.request.token, CaptureError::TimedOut),
            outcome => outcome,
        }))
    }

    /// Recheck after consuming a result and immediately before its handoff to
    /// the native policy adapter. Revocation cannot erase already borrowed
    /// pixels; the adapter must also revalidate current source/Active authority.
    pub fn can_publish(&self, result: &CapturedViewport, now: u64) -> bool {
        let token = result.request.token;
        self.epoch != 0
            && token.instance == self.instance
            && token.epoch == self.epoch
            && token.device == self.boundary.device_epoch
            && result.request.boundary == self.boundary
            && now >= self.now
            && now < result.deadline
    }

    /// Cancel only the captured operation. A delayed cancellation cannot retire
    /// a later request or terminal result. Owner policy loss uses invalidate.
    pub fn cancel(&mut self, token: CaptureToken) -> Result<(), CaptureError> {
        let current = self
            .pending
            .as_ref()
            .map(|pending| pending.request.token)
            .or_else(|| self.outcome_token());
        if current != Some(token) {
            return Err(CaptureError::Stale);
        }
        self.invalidate()?;
        Ok(())
    }

    /// Owner-wide policy/boundary invalidation, including an already ready
    /// mailbox. Returns its retired token so the real backend can unmap/drop
    /// resources; retained request clones still hold their allocation credit.
    pub fn invalidate(&mut self) -> Result<Option<CaptureToken>, CaptureError> {
        let retired = self
            .pending
            .take()
            .map(|pending| pending.request.token)
            .or_else(|| self.outcome_token());
        self.outcome = None;
        if self.epoch == 0 {
            return Err(CaptureError::Exhausted);
        }
        self.epoch = self.epoch.checked_add(1).filter(|epoch| *epoch != 0).unwrap_or(0);
        if self.epoch == 0 {
            return Err(CaptureError::Exhausted);
        }
        Ok(retired)
    }

    pub fn synchronize_boundary(&mut self, boundary: CaptureBoundary) -> Result<Option<CaptureToken>, CaptureError> {
        if boundary == self.boundary {
            return Ok(None);
        }
        let ordered = boundary.device_epoch > self.boundary.device_epoch
            || boundary.device_epoch == self.boundary.device_epoch && boundary.surface_epoch > self.boundary.surface_epoch;
        let retired = self.invalidate()?;
        if !ordered {
            self.epoch = 0;
            return Err(CaptureError::Stale);
        }
        self.boundary = boundary;
        Ok(retired)
    }

    fn clock(&mut self, now: u64) -> Result<(), CaptureError> {
        if now < self.now {
            let _ = self.invalidate();
            self.epoch = 0;
            return Err(CaptureError::Clock);
        }
        self.now = now;
        Ok(())
    }

    fn outcome_token(&self) -> Option<CaptureToken> {
        self.outcome.as_ref().map(|outcome| match outcome {
            CaptureOutcome::Ready(result) => result.request.token,
            CaptureOutcome::Failed(token, _) => *token,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewport_capture_pool_withholds_worker_io_headroom_even_when_all_capture_credit_is_held() {
        assert_eq!(WORKER_CAPTURE_TOTAL_BYTES, 8 * 1024 * 1024);
        assert_eq!(WORKER_CAPTURE_IO_HEADROOM_BYTES, 1024 * 1024);
        assert!(matches!(CaptureBudget::new(WORKER_CAPTURE_BYTES + 1), Err(CaptureError::Capacity)));
        let budget = CaptureBudget::new(WORKER_CAPTURE_BYTES).unwrap();
        let held = budget.reserve(WORKER_CAPTURE_BYTES).unwrap();
        assert!(matches!(budget.reserve(1), Err(CaptureError::Capacity)));
        assert_eq!(budget.used(), WORKER_CAPTURE_TOTAL_BYTES - WORKER_CAPTURE_IO_HEADROOM_BYTES);
        drop(held);
        assert_eq!(budget.used(), 0);
    }

    fn boundary() -> CaptureBoundary {
        CaptureBoundary::new(1, 1, (1920, 1080)).unwrap()
    }
    fn view() -> WorldViewport {
        WorldViewport {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        }
    }
    fn queue(budget: &CaptureBudget) -> CaptureQueue {
        CaptureQueue::new(boundary(), budget.clone()).unwrap()
    }
    fn pixels(budget: &CaptureBudget, layout: CaptureLayout) -> CapturePixels {
        let mut allocation = budget.pixels(layout).unwrap();
        for pixel in allocation.bytes_mut().as_chunks_mut::<4>().0.iter_mut() {
            pixel.copy_from_slice(&[19, 31, 57, 255]);
        }
        allocation.finish().unwrap()
    }

    #[test]
    fn viewport_capture_fits_caps_and_checked_staging_without_clamping_invalid_views() {
        for (cap, dimensions) in [
            (CaptureCap::Small, (320, 180)),
            (CaptureCap::Medium, (480, 270)),
            (CaptureCap::Large, (640, 360)),
        ] {
            let layout = CaptureLayout::fit(view(), cap).unwrap();
            assert_eq!(layout.size(), dimensions);
            assert_eq!(layout.row_bytes(), dimensions.0 * 4);
            assert!(layout.padded_row_bytes().is_multiple_of(256));
            assert!(layout.payload_bytes() <= MAX_CAPTURE_BYTES && layout.staging_bytes() <= MAX_CAPTURE_BYTES);
        }
        assert_eq!(CaptureCap::default(), CaptureCap::Small);
        let tall = WorldViewport {
            width: 100,
            height: 200,
            ..view()
        };
        assert_eq!(CaptureLayout::fit(tall, CaptureCap::Small).unwrap().size(), (88, 180));
        for invalid in [
            WorldViewport { width: 0, ..view() },
            WorldViewport {
                width: 1,
                height: u32::MAX,
                ..view()
            },
            WorldViewport {
                width: u32::MAX,
                height: 1,
                ..view()
            },
        ] {
            assert!(CaptureLayout::fit(invalid, CaptureCap::Large).is_err());
        }
        for (width, height) in [(0, 180), (3, 4), (638, 360), (644, 360), (640, 361), (u32::MAX, u32::MAX)] {
            assert!(CaptureLayout::new(width, height).is_err());
        }
        assert!(CaptureBoundary::new(0, 1, (1, 1)).is_err());
        let budget = CaptureBudget::new(WORKER_CAPTURE_BYTES).unwrap();
        let mut queue = queue(&budget);
        assert!(matches!(
            queue.request(WorldViewport { x: u32::MAX, ..view() }, CaptureCap::Small, 0),
            Err(CaptureError::Invalid)
        ));
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn viewport_capture_fake_backend_completes_on_maintenance_without_another_frame() {
        let budget = CaptureBudget::new(WORKER_CAPTURE_BYTES).unwrap();
        let mut queue = queue(&budget);
        let request = queue.request(view(), CaptureCap::Small, 10).unwrap();
        assert!(queue.poll_map(11).unwrap().is_none(), "request alone is not a completed map");
        let callback = queue.begin_map(request.token(), boundary(), 42, 12).unwrap();
        for tick in 13..30 {
            assert!(queue.poll_map(tick).unwrap().is_none());
        }
        callback.complete(true);
        // No frame submission or waiting exists in these maintenance calls.
        let ready = queue.poll_map(30).unwrap().unwrap();
        queue
            .finish_map(ready.token(), ready.boundary(), pixels(&budget, ready.layout()), 31)
            .unwrap();
        assert!(
            matches!(queue.request(view(), CaptureCap::Small, 32), Err(CaptureError::Busy)),
            "result mailbox also consumes the one slot"
        );
        let Some(CaptureOutcome::Ready(result)) = queue.take_result(32).unwrap() else {
            panic!("ready frame");
        };
        assert_eq!(result.frame(), 42);
        assert_eq!(result.request().token(), request.token());
        assert_eq!(result.pixels().bytes().len(), request.layout().payload_bytes());
        assert!(queue.take_result(32).unwrap().is_none());
        assert!(queue.can_publish(&result, 32));
        queue
            .synchronize_boundary(CaptureBoundary::new(1, 2, (1920, 1080)).unwrap())
            .unwrap();
        assert!(
            !queue.can_publish(&result, 33),
            "held result cannot publish after surface invalidation"
        );
    }

    #[test]
    fn viewport_capture_ready_pixels_expire_before_delayed_publication() {
        let budget = CaptureBudget::new(WORKER_CAPTURE_BYTES).unwrap();
        let mut queue = queue(&budget);
        let request = queue.request(view(), CaptureCap::Small, 10).unwrap();
        let callback = queue.begin_map(request.token(), boundary(), 1, 11).unwrap();
        callback.complete(true);
        queue
            .finish_map(request.token(), boundary(), pixels(&budget, request.layout()), 12)
            .unwrap();
        assert!(
            matches!(queue.take_result(1010).unwrap(), Some(CaptureOutcome::Failed(token, CaptureError::TimedOut)) if token == request.token())
        );
    }

    #[test]
    fn viewport_capture_cancel_old_callback_and_new_queue_never_share_tokens_or_credit() {
        let budget = CaptureBudget::new(WORKER_CAPTURE_BYTES).unwrap();
        let mut first_queue = queue(&budget);
        let old = first_queue.request(view(), CaptureCap::Small, 0).unwrap();
        let old_callback = first_queue.begin_map(old.token(), boundary(), 1, 1).unwrap();
        first_queue.cancel(old.token()).unwrap();
        assert!(
            budget.used() > 0,
            "retired backend request/callback still own physical resource credit"
        );
        let current = first_queue.request(view(), CaptureCap::Small, 2).unwrap();
        assert_ne!(old.token(), current.token());
        assert_eq!(first_queue.cancel(old.token()), Err(CaptureError::Stale));
        old_callback.complete(true);
        assert!(first_queue.poll_map(3).unwrap().is_none());
        assert!(first_queue.begin_map(old.token(), boundary(), 2, 3).is_err());
        drop(first_queue);
        let mut recreated = queue(&budget);
        let replacement = recreated.request(view(), CaptureCap::Small, 0).unwrap();
        assert_ne!(replacement.token(), old.token());
        assert_ne!(replacement.token(), current.token());
        old_callback.complete(false);
        assert!(recreated.poll_map(1).unwrap().is_none());
        drop((old_callback, old, current, replacement, recreated));
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn viewport_capture_checks_surface_device_and_current_token_at_map_and_publication() {
        let budget = CaptureBudget::new(WORKER_CAPTURE_BYTES).unwrap();
        let mut queue = queue(&budget);
        let old = queue.request(view(), CaptureCap::Small, 0).unwrap();
        let callback = queue.begin_map(old.token(), boundary(), 1, 1).unwrap();
        let resized = CaptureBoundary::new(1, 2, (1600, 900)).unwrap();
        assert_eq!(queue.synchronize_boundary(resized).unwrap(), Some(old.token()));
        callback.complete(true);
        assert!(queue.finish_map(old.token(), boundary(), pixels(&budget, old.layout()), 2).is_err());
        let resized_view = WorldViewport {
            width: 1600,
            height: 900,
            ..view()
        };
        let current = queue.request(resized_view, CaptureCap::Small, 3).unwrap();
        assert!(queue.begin_map(current.token(), boundary(), 2, 4).is_err());
        let callback = queue.begin_map(current.token(), resized, 2, 4).unwrap();
        callback.complete(true);
        assert!(
            queue
                .finish_map(current.token(), boundary(), pixels(&budget, current.layout()), 5)
                .is_err()
        );
        let restored_device = CaptureBoundary::new(2, 1, (1600, 900)).unwrap();
        queue.synchronize_boundary(restored_device).unwrap();
        assert!(
            queue
                .finish_map(current.token(), resized, pixels(&budget, current.layout()), 6)
                .is_err()
        );
        assert!(queue.synchronize_boundary(resized).is_err(), "device rollback poisons admission");
        assert!(matches!(
            queue.request(resized_view, CaptureCap::Small, 7),
            Err(CaptureError::Exhausted)
        ));
    }

    #[test]
    fn viewport_capture_retained_clones_hold_budget_and_bad_opacity_is_rejected() {
        let budget = CaptureBudget::new(3 * MAX_CAPTURE_BYTES).unwrap();
        let mut queue = queue(&budget);
        let request = queue.request(view(), CaptureCap::Large, 0).unwrap();
        let callback = queue.begin_map(request.token(), boundary(), 1, 1).unwrap();
        callback.complete(true);
        let mut invalid = budget.pixels(request.layout()).unwrap();
        assert!(invalid.bytes_mut().as_chunks::<4>().0.iter().all(|pixel| pixel[3] == 0));
        assert!(invalid.finish().is_err());
        queue
            .finish_map(request.token(), boundary(), pixels(&budget, request.layout()), 2)
            .unwrap();
        let Some(CaptureOutcome::Ready(result)) = queue.take_result(2).unwrap() else {
            panic!("pixels");
        };
        let held = result.clone();
        drop((result, callback, request));
        assert_eq!(budget.used(), 3 * MAX_CAPTURE_BYTES);
        assert!(matches!(queue.request(view(), CaptureCap::Small, 3), Err(CaptureError::Capacity)));
        drop(held);
        assert_eq!(budget.used(), 0);
        assert!(queue.request(view(), CaptureCap::Small, 4).is_ok());
    }

    #[test]
    fn viewport_capture_failure_timeout_clock_and_counter_exhaustion_fail_closed() {
        let instance = AtomicU64::new(u64::MAX - 1);
        assert_eq!(allocate_queue_instance(&instance), Ok(u64::MAX - 1));
        assert_eq!(allocate_queue_instance(&instance), Err(CaptureError::Exhausted));
        assert_eq!(allocate_queue_instance(&instance), Err(CaptureError::Exhausted));
        let budget = CaptureBudget::new(WORKER_CAPTURE_BYTES).unwrap();
        let mut queue = queue(&budget);
        let first = queue.request(view(), CaptureCap::Small, 0).unwrap();
        let callback = queue.begin_map(first.token(), boundary(), 0, 1).unwrap();
        callback.complete(false);
        callback.complete(true);
        queue.poll_map(2).unwrap();
        assert!(
            matches!(queue.take_result(2).unwrap(), Some(CaptureOutcome::Failed(token, CaptureError::MapFailed)) if token == first.token())
        );
        let second = queue.request(view(), CaptureCap::Small, 3).unwrap();
        queue.poll_map(1003).unwrap();
        assert!(
            matches!(queue.take_result(1003).unwrap(), Some(CaptureOutcome::Failed(token, CaptureError::TimedOut)) if token == second.token())
        );
        queue.next_ticket = u64::MAX;
        assert!(matches!(
            queue.request(view(), CaptureCap::Small, 1004),
            Err(CaptureError::Exhausted)
        ));
        assert_eq!(queue.invalidate(), Err(CaptureError::Exhausted));
        assert!(matches!(
            queue.request(view(), CaptureCap::Small, 1005),
            Err(CaptureError::Exhausted)
        ));
        let mut clock_queue = super::tests::queue(&budget);
        clock_queue.request(view(), CaptureCap::Small, 10).unwrap();
        assert!(matches!(clock_queue.poll_map(9), Err(CaptureError::Clock)));
        assert!(matches!(
            clock_queue.request(view(), CaptureCap::Small, 11),
            Err(CaptureError::Exhausted)
        ));
        let mut epoch_queue = super::tests::queue(&budget);
        epoch_queue.epoch = u64::MAX;
        assert_eq!(epoch_queue.invalidate(), Err(CaptureError::Exhausted));
    }

    #[test]
    fn viewport_capture_pure_module_has_no_blocking_or_gpu_work_entrypoint() {
        let implementation = include_str!("viewport_capture.rs").split("#[cfg(test)]").next().unwrap();
        for forbidden in [
            "Condvar",
            "thread::sleep",
            "PollType::Wait",
            "wait_indefinitely",
            "wgpu::",
            "std::fs",
            "TcpStream",
        ] {
            assert!(!implementation.contains(forbidden), "pure capture model introduced {forbidden}");
        }
    }
}
