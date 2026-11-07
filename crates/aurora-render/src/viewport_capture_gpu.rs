//! Render-owner viewport sampling. Frontend and backend share one pure queue;
//! only try_lock is used, never across GPU APIs or pixel packing. No caller in
//! App/native hosting enables this service yet. It grants no world authority.

use crate::WorldViewport;
use crate::pipeline_cache::CachedPipelines;
use crate::viewport_capture::{
    CaptureBoundary, CaptureBudget, CaptureCap, CaptureError, CaptureLayout, CaptureOutcome, CapturePixels, CapturePublicationGuard,
    CaptureQueue, CaptureRequest, CaptureResources, CaptureRetentionGuard, CaptureToken, CapturedViewport, MapCallback,
    WORKER_CAPTURE_BYTES,
};
use bytemuck::{Pod, Zeroable};
use parking_lot::{Mutex, MutexGuard};
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

static NEXT_DEVICE: AtomicU64 = AtomicU64::new(1);
static WORKER_BUDGET: OnceLock<CaptureBudget> = OnceLock::new();

#[derive(Clone)]
pub(crate) struct FrameCapture {
    pub request: CaptureRequest,
    generation: u64,
}

struct State {
    queue: CaptureQueue,
    generation: u64,
    publication_token: Option<CaptureToken>,
}

pub(crate) struct Shared {
    state: Mutex<State>,
    budget: CaptureBudget,
    generation: Arc<AtomicU64>,
    ticket: Arc<AtomicU64>,
    physical: Arc<AtomicU64>,
    lost: Arc<AtomicBool>,
    instance: u64,
    device: u64,
    began: Instant,
    #[cfg(test)]
    clock_override: AtomicU64,
}

impl Shared {
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }
    pub fn generation_current(&self, frame: &FrameCapture) -> bool {
        frame.generation != 0 && frame.generation == self.generation.load(Ordering::Acquire) && !self.lost.load(Ordering::Acquire)
    }
    pub fn new(size: (u32, u32), lost: Arc<AtomicBool>) -> Result<Arc<Self>, CaptureError> {
        // Renderer/device replacement does not mint fresh image credit while
        // the old driver/callback/results still retain physical allocations.
        let budget =
            WORKER_BUDGET.get_or_init(|| CaptureBudget::new(WORKER_CAPTURE_BYTES).expect("fixed positive worker capture bound is valid"));
        Self::with_budget_and_loss(size, budget.clone(), lost)
    }

    #[cfg(test)]
    fn with_budget(size: (u32, u32), budget: CaptureBudget) -> Result<Arc<Self>, CaptureError> {
        Self::with_budget_and_loss(size, budget, Arc::new(AtomicBool::new(false)))
    }

    fn with_budget_and_loss(size: (u32, u32), budget: CaptureBudget, lost: Arc<AtomicBool>) -> Result<Arc<Self>, CaptureError> {
        let device = NEXT_DEVICE
            .try_update(Ordering::AcqRel, Ordering::Acquire, |next| next.checked_add(1))
            .map_err(|_| CaptureError::Exhausted)?;
        let boundary = CaptureBoundary::new(device, 1, size)?;
        let queue = CaptureQueue::new(boundary, budget.clone())?;
        let instance = queue.instance();
        Ok(Arc::new(Self {
            state: Mutex::new(State {
                queue,
                generation: 1,
                publication_token: None,
            }),
            budget,
            generation: Arc::new(AtomicU64::new(1)),
            ticket: Arc::new(AtomicU64::new(0)),
            physical: Arc::new(AtomicU64::new(1)),
            lost,
            instance,
            device,
            began: Instant::now(),
            #[cfg(test)]
            clock_override: AtomicU64::new(u64::MAX),
        }))
    }

    pub fn boundary(&self, surface: u64, size: (u32, u32)) -> Result<CaptureBoundary, CaptureError> {
        CaptureBoundary::new(self.device, surface, size)
    }

    fn now(&self) -> u64 {
        #[cfg(test)]
        if self.clock_override.load(Ordering::Acquire) != u64::MAX {
            return self.clock_override.load(Ordering::Acquire);
        }
        self.began.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
    }

    fn lock(&self) -> Result<MutexGuard<'_, State>, CaptureError> {
        if self.lost.load(Ordering::Acquire) {
            return Err(CaptureError::DeviceLost);
        }
        let mut state = self.state.try_lock().ok_or(CaptureError::Busy)?;
        let generation = self.generation.load(Ordering::Acquire);
        if generation == 0 || self.physical.load(Ordering::Acquire) == 0 {
            return Err(CaptureError::Exhausted);
        }
        if state.generation != generation {
            state.queue.invalidate()?;
            state.generation = generation;
            state.publication_token = None;
        }
        Ok(state)
    }

    /// Logical revocation never waits for a contended queue or render job.
    pub fn invalidate(&self) {
        self.ticket.store(0, Ordering::Release);
        if self
            .generation
            .try_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1).filter(|_| value != 0)
            })
            .is_err()
        {
            self.generation.store(0, Ordering::Release);
        }
    }

    /// Real resize/configuration/surface failure retires historical physical
    /// identity as well as publication. It never waits for queue contention.
    pub fn invalidate_physical(&self) {
        self.invalidate();
        if self
            .physical
            .try_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1).filter(|_| value != 0)
            })
            .is_err()
        {
            self.physical.store(0, Ordering::Release);
        }
    }

    /// Frontend ownership is distinct from helper backend lifetime. No later
    /// queue/device with repeated numeric dimensions can revive these atomics.
    pub fn retire_owner(&self) {
        self.ticket.store(0, Ordering::Release);
        self.generation.store(0, Ordering::Release);
        self.physical.store(0, Ordering::Release);
    }

    pub fn retention_guard(&self) -> Option<CaptureRetentionGuard> {
        CaptureRetentionGuard::new(self.instance, self.device, self.physical.clone(), self.lost.clone())
    }

    pub fn physical_boundary(&self, surface: u64, size: (u32, u32)) -> Option<CaptureBoundary> {
        let guard = self.retention_guard()?;
        let boundary = self.boundary(surface, size).ok()?;
        guard.current().then_some(boundary)
    }

    pub fn publication_guard(&self, result: &CapturedViewport) -> Option<CapturePublicationGuard> {
        let state = self.lock().ok()?;
        let generation = state.generation;
        if state.publication_token != Some(result.request().token()) || !state.queue.can_publish(result, self.now()) {
            return None;
        }
        // The request deadline is measured from this Shared's original clock,
        // not guard creation or IO enqueue. checked_add fails closed.
        let deadline = self.began.checked_add(Duration::from_millis(result.deadline_millis()))?;
        CapturePublicationGuard::new(
            result.request().token(),
            generation,
            self.generation.clone(),
            self.ticket.clone(),
            self.retention_guard()?,
            deadline,
        )
    }

    pub fn request(&self, boundary: CaptureBoundary, viewport: WorldViewport, cap: CaptureCap) -> Result<FrameCapture, CaptureError> {
        let mut state = self.lock()?;
        // Frontend physical changes revoke synchronously before this request.
        // Queue reconciliation must not bump physical identity a second time
        // after the native adapter has already captured its new witness.
        state.queue.synchronize_boundary(boundary)?;
        let request = match state.queue.request_metadata(viewport, cap, self.now()) {
            Ok(request) => request,
            Err(error) => {
                if matches!(error, CaptureError::Clock | CaptureError::Exhausted) {
                    self.invalidate();
                }
                return Err(error);
            }
        };
        let generation = state.generation;
        state.publication_token = Some(request.token());
        self.ticket.store(request.token().ticket(), Ordering::Release);
        if generation != self.generation.load(Ordering::Acquire) {
            return Err(CaptureError::Cancelled);
        }
        Ok(FrameCapture { request, generation })
    }

    pub fn cancel(&self, token: CaptureToken) -> Result<(), CaptureError> {
        let mut state = self.lock()?;
        if state.publication_token != Some(token) {
            return Err(CaptureError::Stale);
        }
        // A consumed result can still be held by IO. Revoke its exact token;
        // an old cancellation must never retire a newer issued capture.
        // Atomic revocation precedes fallible epoch retirement: exhaustion
        // must not leave an already minted IO guard eligible.
        self.invalidate();
        state.queue.invalidate()?;
        Ok(())
    }

    pub fn current(&self, frame: &FrameCapture) -> Result<bool, CaptureError> {
        if !self.generation_current(frame) {
            return Ok(false);
        }
        let mut state = self.lock()?;
        if let Err(error) = state.queue.poll_map(self.now()) {
            self.invalidate();
            return Err(error);
        }
        Ok(state.queue.request_current(&frame.request) && frame.generation == self.generation.load(Ordering::Acquire))
    }

    fn begin(&self, frame: &FrameCapture, boundary: CaptureBoundary, index: u64) -> Result<MapCallback, CaptureError> {
        let mut state = self.lock()?;
        if frame.generation != state.generation || frame.generation != self.generation.load(Ordering::Acquire) {
            return Err(CaptureError::Cancelled);
        }
        state.queue.begin_map(frame.request.token(), boundary, index, self.now())
    }

    fn finish(&self, frame: &FrameCapture, pixels: CapturePixels) -> Result<(), CaptureError> {
        let mut state = self.lock()?;
        if frame.generation != state.generation || frame.generation != self.generation.load(Ordering::Acquire) {
            return Err(CaptureError::Cancelled);
        }
        state
            .queue
            .finish_map(frame.request.token(), frame.request.boundary(), pixels, self.now())?;
        if frame.generation != self.generation.load(Ordering::Acquire) {
            state.queue.invalidate()?;
            return Err(CaptureError::Cancelled);
        }
        Ok(())
    }

    pub fn fail(&self, frame: &FrameCapture, error: CaptureError) {
        if let Ok(mut state) = self.lock() {
            state.queue.fail_request(frame.request.token(), error);
        }
    }

    pub fn take(&self) -> Result<Option<CaptureOutcome>, CaptureError> {
        let mut state = self.lock()?;
        let generation = state.generation;
        let result = match state.queue.take_result(self.now()) {
            Ok(result) => result,
            Err(error) => {
                self.invalidate();
                return Err(error);
            }
        };
        if generation != self.generation.load(Ordering::Acquire) {
            return Err(CaptureError::Cancelled);
        }
        Ok(result)
    }

    pub fn can_publish(&self, result: &CapturedViewport) -> bool {
        let Ok(state) = self.lock() else {
            return false;
        };
        let generation = state.generation;
        state.publication_token == Some(result.request().token())
            && state.queue.can_publish(result, self.now())
            && generation == self.generation.load(Ordering::Acquire)
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        self.retire_owner();
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    rect: [f32; 4],
    source_size: [f32; 2],
    encode_srgb: u32,
    _pad: u32,
}

fn params(request: &CaptureRequest, format: wgpu::TextureFormat) -> Params {
    let size = request.boundary().size();
    let view = request.viewport();
    Params {
        rect: [
            view.x as f32 / size.0 as f32,
            view.y as f32 / size.1 as f32,
            view.width as f32 / size.0 as f32,
            view.height as f32 / size.1 as f32,
        ],
        source_size: [size.0 as f32, size.1 as f32],
        encode_srgb: u32::from(format.is_srgb()),
        _pad: 0,
    }
}

pub(crate) fn sampleable(format: wgpu::TextureFormat, usage: wgpu::TextureUsages) -> bool {
    usage.contains(wgpu::TextureUsages::TEXTURE_BINDING)
        && matches!(
            format,
            wgpu::TextureFormat::Rgba8Unorm
                | wgpu::TextureFormat::Rgba8UnormSrgb
                | wgpu::TextureFormat::Bgra8Unorm
                | wgpu::TextureFormat::Bgra8UnormSrgb
        )
}

struct Pending {
    frame: FrameCapture,
    callback: MapCallback,
    mapping: Mapping,
    packed: Option<CapturePixels>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mapping {
    NotRequested,
    Requested,
    Released,
}
impl Mapping {
    fn release(&mut self, failed: bool) -> bool {
        let requested = *self == Self::Requested;
        *self = Self::Released;
        // A failed callback has already returned wgpu's buffer to Idle.
        // Unmapping Idle is a validation error, not an idempotent operation.
        requested && !failed
    }
}
impl Pending {
    fn release_mapping(&mut self, staging: &wgpu::Buffer) {
        // The render owner also polls this device. Native map callbacks finish
        // on that polling thread before it can run this release. Mark Released
        // before unmap: canceling a pending map can call its callback inline.
        if self.mapping.release(self.callback.failed()) {
            staging.unmap();
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct PairKey {
    boundary: CaptureBoundary,
    layout: CaptureLayout,
    generation: u64,
}
struct Flight {
    retired: Arc<AtomicBool>,
    submitted: bool,
    unmapped: bool,
}

/// The actual backend and fake allocator tests use the same single-pair
/// lifecycle. Readiness requires both unmapping and submitted-work retirement;
/// cancellation never manufactures completion or admits a second pair.
struct PairSlot<T> {
    key: Option<PairKey>,
    value: Option<T>,
    flight: Option<Flight>,
    discard: bool,
}
impl<T> Default for PairSlot<T> {
    fn default() -> Self {
        Self {
            key: None,
            value: None,
            flight: None,
            discard: false,
        }
    }
}
impl<T> PairSlot<T> {
    fn poll(&mut self, generation: u64) {
        if self.key.is_some_and(|key| key.generation != generation) {
            self.discard = true;
        }
        if self
            .flight
            .as_ref()
            .is_some_and(|flight| flight.unmapped && (!flight.submitted || flight.retired.load(Ordering::Acquire)))
        {
            self.flight = None;
        }
        if self.discard && self.flight.is_none() {
            self.value = None;
            self.key = None;
            self.discard = false;
        }
    }
    fn acquire(&mut self, key: PairKey, create: impl FnOnce() -> Result<T, CaptureError>) -> Result<(), CaptureError> {
        self.poll(key.generation);
        if self.flight.is_some() {
            return Err(CaptureError::Busy);
        }
        if self.key != Some(key) {
            self.value = None;
            self.key = None;
        }
        if self.value.is_none() {
            self.value = Some(create()?);
            self.key = Some(key);
        }
        self.flight = Some(Flight {
            retired: Arc::new(AtomicBool::new(false)),
            submitted: false,
            unmapped: false,
        });
        Ok(())
    }
    fn submitted(&mut self) -> Option<Arc<AtomicBool>> {
        let flight = self.flight.as_mut()?;
        flight.submitted = true;
        Some(flight.retired.clone())
    }
    fn unmapped(&mut self) {
        if let Some(flight) = &mut self.flight {
            flight.unmapped = true;
        }
    }
    fn retire(&mut self, generation: u64) {
        self.discard = true;
        self.poll(generation);
    }
    fn needs_maintenance(&self, generation: u64) -> bool {
        self.flight.is_some() || self.discard || self.key.is_some_and(|key| key.generation != generation)
    }
}

struct ResourcePair {
    target: wgpu::Texture,
    staging: wgpu::Buffer,
    uniform: wgpu::Buffer,
    sampler: wgpu::Sampler,
    lease: CaptureResources,
}

pub(crate) struct GpuCapture {
    pub shared: Arc<Shared>,
    pipeline: Option<wgpu::RenderPipeline>,
    pending: Option<Pending>,
    pair: PairSlot<ResourcePair>,
}

impl GpuCapture {
    pub fn new(shared: Arc<Shared>) -> Self {
        Self {
            shared,
            pipeline: None,
            pending: None,
            pair: PairSlot::default(),
        }
    }

    /// Only the render owner calls this, at the pre-label/HUD composition tap.
    pub fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::Texture,
        frame: FrameCapture,
        boundary: CaptureBoundary,
        viewport: WorldViewport,
        index: u64,
    ) {
        if self.pending.is_some()
            || frame.request.boundary() != boundary
            || frame.request.viewport() != viewport
            || source.width() != boundary.size().0
            || source.height() != boundary.size().1
            || !sampleable(source.format(), source.usage())
        {
            self.shared.fail(&frame, CaptureError::Unsupported);
            return;
        }
        if !matches!(self.shared.current(&frame), Ok(true)) {
            return;
        }
        let callback = match self.shared.begin(&frame, boundary, index) {
            Ok(callback) => callback,
            Err(CaptureError::Busy) => return,
            Err(error) => {
                self.shared.fail(&frame, error);
                return;
            }
        };
        if frame.generation != self.shared.generation.load(Ordering::Acquire) {
            return;
        }
        let layout = frame.request.layout();
        let (width, height) = layout.size();
        let key = PairKey {
            boundary,
            layout,
            generation: frame.generation,
        };
        if let Err(error) = self.pair.acquire(key, || {
            let lease = self.shared.budget.resources(layout)?;
            let target = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("viewport capture target"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let staging = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("viewport capture staging"),
                size: layout.staging_bytes() as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let uniform = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("viewport capture crop"),
                size: std::mem::size_of::<Params>() as u64,
                mapped_at_creation: false,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
            let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            });
            Ok(ResourcePair {
                target,
                staging,
                uniform,
                sampler,
                lease,
            })
        }) {
            self.shared.fail(&frame, error);
            return;
        }
        let Some(pair) = self.pair.value.as_ref() else {
            return;
        };
        debug_assert_eq!(pair.lease.layout(), layout);
        queue.write_buffer(&pair.uniform, 0, bytemuck::bytes_of(&params(&frame.request, source.format())));
        let pipeline = self.pipeline.get_or_insert_with(|| pipeline(device));
        let source_view = source.create_view(&Default::default());
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("viewport capture source"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&source_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&pair.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: pair.uniform.as_entire_binding(),
                },
            ],
        });
        let target_view = pair.target.create_view(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("viewport capture sample"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.draw(0..3, 0..1);
        }
        encoder.copy_texture_to_buffer(
            pair.target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &pair.staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(layout.padded_row_bytes()),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.pending = Some(Pending {
            frame,
            callback,
            mapping: Mapping::NotRequested,
            packed: None,
        });
    }

    pub fn after_submit(&mut self, queue: &wgpu::Queue) {
        if let Some(pending) = &mut self.pending
            && pending.mapping == Mapping::NotRequested
        {
            let Some(retired) = self.pair.submitted() else {
                return;
            };
            let Some(pair) = &self.pair.value else {
                return;
            };
            // Mapping can be cancelled before its copy completes. The ordered
            // queue callback retains GPU allocation credit through real work
            // completion independently of map/queue/UI cancellation.
            let submitted_credit = pair.lease.clone();
            queue.on_submitted_work_done(move || {
                retired.store(true, Ordering::Release);
                drop(submitted_credit);
            });
            let callback = pending.callback.clone();
            let map_credit = pair.lease.clone();
            pending.mapping = Mapping::Requested;
            pair.staging.map_async(wgpu::MapMode::Read, .., move |result| {
                callback.complete(result.is_ok());
                drop(map_credit);
            });
        }
    }

    /// Device polling occurs in Backend, on its render owner, even without
    /// frame pacing or another presentation. This method never waits for maps.
    pub fn maintain(&mut self) {
        self.pair.poll(self.shared.generation());
        let Some(pending) = &mut self.pending else {
            return;
        };
        match self.shared.current(&pending.frame) {
            Ok(true) => {}
            Err(CaptureError::Busy) => return,
            _ => {
                self.release_pending(true);
                return;
            }
        }
        if pending.packed.is_none() {
            let ready = {
                let Ok(mut state) = self.shared.lock() else {
                    return;
                };
                matches!(state.queue.poll_map(self.shared.now()), Ok(Some(request)) if request.token() == pending.frame.request.token())
            };
            if !ready || pending.mapping != Mapping::Requested {
                return;
            }
            let pixels = match self.shared.budget.pixels(pending.frame.request.layout()) {
                Ok(pixels) => pixels,
                Err(error) => {
                    self.shared.fail(&pending.frame, error);
                    self.release_pending(true);
                    return;
                }
            };
            let mut pixels = pixels;
            let Some(pair) = self.pair.value.as_ref() else {
                return;
            };
            let result = match pair.staging.get_mapped_range(..) {
                Ok(mapped) => pack_rows(&mapped, pixels.bytes_mut(), pending.frame.request.layout()),
                Err(_) => Err(CaptureError::MapFailed),
            };
            pending.release_mapping(&pair.staging);
            self.pair.unmapped();
            let packed = match result.and_then(|_| pixels.finish()) {
                Ok(pixels) => pixels,
                Err(error) => {
                    self.shared.fail(&pending.frame, error);
                    self.release_pending(true);
                    return;
                }
            };
            pending.packed = Some(packed);
        }
        let Some(pixels) = pending.packed.as_ref() else {
            return;
        };
        match self.shared.finish(&pending.frame, pixels.clone()) {
            Err(CaptureError::Busy) => {}
            Ok(()) => self.release_pending(false),
            Err(_) => self.release_pending(true),
        }
    }

    pub fn invalidate(&mut self) {
        self.shared.invalidate();
        self.release_pending(true);
    }
    pub fn invalidate_physical(&mut self) {
        self.shared.invalidate_physical();
        self.release_pending(true);
    }
    /// Occlusion/minimization and temporary acquisition timeout stop new
    /// captures without destroying historical physical identity. All other
    /// acquisition failures are physical or unknown and fail closed.
    pub fn surface_unavailable(&mut self, reason: wgpu::CurrentSurfaceTexture) {
        if matches!(reason, wgpu::CurrentSurfaceTexture::Occluded | wgpu::CurrentSurfaceTexture::Timeout) {
            self.invalidate();
        } else {
            self.invalidate_physical();
        }
    }
    pub fn has_pending(&self) -> bool {
        self.pending.is_some() || self.pair.needs_maintenance(self.shared.generation())
    }

    /// Cleanup-only: never allocate packed pixels, read mapped bytes or publish
    /// a result. A delayed old probe cannot release a newer request's mapping.
    pub fn maintain_retirement(&mut self, origin: CaptureToken) {
        self.pair.poll(self.shared.generation());
        if self.pending.as_ref().is_some_and(|pending| pending.frame.request.token() == origin) {
            self.release_pending(true);
        }
    }

    fn release_pending(&mut self, retire: bool) {
        if let Some(mut pending) = self.pending.take() {
            if let Some(pair) = &self.pair.value {
                pending.release_mapping(&pair.staging);
            }
            self.pair.unmapped();
        }
        if retire {
            self.pair.retire(self.shared.generation());
        } else {
            self.pair.poll(self.shared.generation());
        }
    }
}

impl Drop for GpuCapture {
    fn drop(&mut self) {
        self.release_pending(true);
        // Submitted/map callback leases retain physical allocation credit even
        // if renderer teardown drops this cache before GPU retirement.
    }
}

fn pack_rows(source: &[u8], target: &mut [u8], layout: CaptureLayout) -> Result<(), CaptureError> {
    if source.len() != layout.staging_bytes() || target.len() != layout.payload_bytes() {
        return Err(CaptureError::Invalid);
    }
    let row = layout.row_bytes() as usize;
    let padded = layout.padded_row_bytes() as usize;
    for (from, to) in source.chunks_exact(padded).zip(target.chunks_exact_mut(row)) {
        to.copy_from_slice(&from[..row]);
    }
    Ok(())
}

fn pipeline(device: &wgpu::Device) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("viewport capture"),
        source: wgpu::ShaderSource::Wgsl(include_str!("shaders/viewport_capture.wgsl").into()),
    });
    device.create_render_pipeline_cached(&wgpu::RenderPipelineDescriptor {
        label: Some("viewport capture"),
        layout: None,
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba8Unorm,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shared() -> Arc<Shared> {
        let shared = Shared::with_budget((640, 360), CaptureBudget::new(WORKER_CAPTURE_BYTES).unwrap()).unwrap();
        shared.clock_override.store(0, Ordering::Release);
        shared
    }
    fn view() -> WorldViewport {
        WorldViewport {
            x: 0,
            y: 0,
            width: 640,
            height: 360,
        }
    }
    fn request(shared: &Shared) -> FrameCapture {
        shared
            .request(shared.boundary(1, (640, 360)).unwrap(), view(), CaptureCap::Small)
            .unwrap()
    }
    fn fake_pixels(shared: &Shared, frame: &FrameCapture) -> CapturePixels {
        let mut pixels = shared.budget.pixels(frame.request.layout()).unwrap();
        for pixel in pixels.bytes_mut().as_chunks_mut::<4>().0.iter_mut() {
            pixel.copy_from_slice(&[23, 47, 91, 255]);
        }
        pixels.finish().unwrap()
    }

    fn ready_image(shared: &Shared) -> CapturedViewport {
        let frame = request(shared);
        let callback = shared.begin(&frame, frame.request.boundary(), 31).unwrap();
        callback.complete(true);
        shared.finish(&frame, fake_pixels(shared, &frame)).unwrap();
        let Some(CaptureOutcome::Ready(image)) = shared.take().unwrap() else {
            panic!("checked synthetic map result");
        };
        image
    }

    #[test]
    fn viewport_guard_uses_exact_token_and_original_result_deadline_without_retaining_pixels_or_queue() {
        let shared = shared();
        shared.clock_override.store(25, Ordering::Release);
        let image = ready_image(&shared);
        shared.clock_override.store(1000, Ordering::Release);
        let guard = shared.publication_guard(&image).unwrap();
        assert_eq!(guard.token(), image.request().token());
        assert_eq!(guard.token().instance(), shared.instance);
        assert_eq!(guard.deadline(), shared.began + Duration::from_millis(1025));
        assert!(guard.current_at(shared.began + Duration::from_millis(1024)));
        assert!(!guard.current_at(shared.began + Duration::from_millis(1025)));
        assert!(
            !guard.current_at(shared.began - Duration::from_millis(1)),
            "a guard cannot be queried before it was issued"
        );
        let queue_lock = shared.state.try_lock().unwrap();
        assert!(guard.current(), "IO never needs the contended queue mutex");
        drop(queue_lock);
        shared.clock_override.store(1025, Ordering::Release);
        assert!(
            shared.publication_guard(&image).is_none(),
            "late guard creation cannot renew the old request"
        );
        drop(image);
        assert_eq!(shared.budget.used(), 0, "a guard retains no image allocation credit");
        let weak = Arc::downgrade(&shared);
        drop(shared);
        assert!(weak.upgrade().is_none(), "a guard retains no Shared/queue owner");
        assert!(!guard.current());
    }

    #[test]
    fn viewport_guard_next_request_and_stale_cancel_cannot_share_or_retarget_publication() {
        let shared = shared();
        let image = ready_image(&shared);
        let old = shared.publication_guard(&image).unwrap();
        let physical = shared.retention_guard().unwrap();
        let next = request(&shared);
        assert_ne!(old.token(), next.request.token());
        assert!(!old.current());
        assert!(shared.publication_guard(&image).is_none());
        assert!(matches!(shared.cancel(old.token()), Err(CaptureError::Stale)));
        assert_eq!(shared.current(&next), Ok(true));
        assert!(physical.current());
    }

    #[test]
    fn viewport_guard_cancel_and_session_stop_preserve_history_but_physical_change_retires_it() {
        let shared = shared();
        let image = ready_image(&shared);
        let published = shared.publication_guard(&image).unwrap();
        let retained = shared.retention_guard().unwrap();
        shared.cancel(image.request().token()).unwrap();
        assert!(!published.current(), "cancel also fences an already consumed result");
        assert!(retained.current());
        let next = ready_image(&shared);
        let published = shared.publication_guard(&next).unwrap();
        let locked = shared.state.try_lock().unwrap();
        shared.invalidate(); // The same production path as fence/stop/privacy.
        assert!(!published.current());
        assert!(retained.current(), "capture revocation cannot retire parked historical pixels");
        shared.invalidate_physical();
        assert!(!retained.current(), "physical invalidation needs no queue mutex");
        drop(locked);
        assert!(shared.retention_guard().unwrap().generation() > retained.generation());
    }

    #[test]
    fn viewport_guard_frontend_retirement_revokes_held_guards_while_helper_drop_does_not() {
        let shared = shared();
        let image = ready_image(&shared);
        let published = shared.publication_guard(&image).unwrap();
        let retained = shared.retention_guard().unwrap();
        drop(GpuCapture::new(shared.clone()));
        assert!(published.current());
        assert!(retained.current(), "an unrelated XR helper is not the frontend owner");
        let helper = GpuCapture::new(shared.clone());
        shared.retire_owner(); // Exact atomic path called by Renderer::drop.
        assert!(!published.current());
        assert!(!retained.current());
        assert!(shared.retention_guard().is_none());
        assert!(matches!(shared.take(), Err(CaptureError::Exhausted)));
        drop(helper);
    }

    #[test]
    fn viewport_guard_replacement_same_surface_size_uses_distinct_instance_and_device() {
        let first = shared();
        let image = ready_image(&first);
        let published = first.publication_guard(&image).unwrap();
        let retained = first.retention_guard().unwrap();
        let boundary = image.request().boundary();
        drop(first);
        let replacement = shared();
        let current = replacement.retention_guard().unwrap();
        assert_eq!(replacement.boundary(1, (640, 360)).unwrap().size(), boundary.size());
        assert_eq!(
            replacement.boundary(1, (640, 360)).unwrap().surface_epoch(),
            boundary.surface_epoch()
        );
        assert_ne!(current.instance(), retained.instance());
        assert_ne!(current.device_epoch(), retained.device_epoch());
        assert_eq!(current.generation(), retained.generation());
        assert!(!retained.current());
        assert!(!published.current());
        assert!(replacement.publication_guard(&image).is_none());
    }

    #[test]
    fn viewport_guard_physical_loss_without_pending_capture_is_immediately_visible() {
        let shared = shared();
        let retained = shared.retention_guard().unwrap();
        assert!(shared.take().unwrap().is_none());
        shared.invalidate_physical();
        assert!(!retained.current());
        let retained = shared.retention_guard().unwrap();
        let locked = shared.state.try_lock().unwrap();
        // This is the same atomic held by the actual DeviceLost callback;
        // there is no App tick, capture, device poll or queue lock acquisition.
        shared.lost.store(true, Ordering::Release);
        assert!(!retained.current());
        assert!(shared.retention_guard().is_none());
        drop(locked);
    }

    #[test]
    fn viewport_guard_physical_boundary_survives_capture_only_revocation_and_keeps_actual_surface() {
        let shared = shared();
        let retained = shared.retention_guard().unwrap();
        let boundary = shared.physical_boundary(9, (640, 360)).unwrap();
        assert_eq!(boundary.device_epoch(), retained.device_epoch());
        assert_eq!(boundary.surface_epoch(), 9, "never synthesize an initial surface epoch");
        let locked = shared.state.try_lock().unwrap();
        shared.invalidate(); // Same capture-only path used for XR/park/stop.
        assert!(retained.current());
        assert_eq!(shared.physical_boundary(9, (640, 360)), Some(boundary));
        shared.invalidate_physical();
        assert!(!retained.current());
        assert!(shared.retention_guard().unwrap().generation() > retained.generation());
        assert_eq!(
            shared.physical_boundary(9, (640, 360)),
            Some(boundary),
            "new physical generation may keep the actual numeric surface; native render epoch handles the replacement"
        );
        drop(locked);
    }

    #[test]
    fn viewport_guard_physical_boundary_requires_current_owner_and_checked_actual_geometry() {
        let shared = shared();
        assert!(shared.physical_boundary(0, (640, 360)).is_none());
        assert!(shared.physical_boundary(9, (0, 360)).is_none());
        assert!(shared.physical_boundary(9, (640, 0)).is_none());
        shared.lost.store(true, Ordering::Release);
        assert!(shared.physical_boundary(9, (640, 360)).is_none());
        let replacement = Shared::with_budget((640, 360), CaptureBudget::new(WORKER_CAPTURE_BYTES).unwrap()).unwrap();
        assert_ne!(replacement.physical_boundary(1, (640, 360)).unwrap().device_epoch(), shared.device);
        replacement.retire_owner();
        assert!(replacement.physical_boundary(1, (640, 360)).is_none());
    }

    #[test]
    fn viewport_guard_surface_occlusion_and_timeout_preserve_history_while_loss_or_unknown_error_retires_it() {
        for reason in [wgpu::CurrentSurfaceTexture::Occluded, wgpu::CurrentSurfaceTexture::Timeout] {
            let shared = shared();
            let image = ready_image(&shared);
            let published = shared.publication_guard(&image).unwrap();
            let retained = shared.retention_guard().unwrap();
            GpuCapture::new(shared.clone()).surface_unavailable(reason);
            assert!(!published.current());
            assert!(retained.current());
        }
        for reason in [
            wgpu::CurrentSurfaceTexture::Outdated,
            wgpu::CurrentSurfaceTexture::Lost,
            wgpu::CurrentSurfaceTexture::Validation,
        ] {
            let shared = shared();
            let retained = shared.retention_guard().unwrap();
            assert!(shared.take().unwrap().is_none());
            GpuCapture::new(shared.clone()).surface_unavailable(reason);
            assert!(
                !retained.current(),
                "physical acquisition loss also retires with no pending capture"
            );
        }
    }

    #[test]
    fn viewport_guard_publication_and_physical_counter_exhaustion_never_revive_old_witnesses() {
        let shared = shared();
        let image = ready_image(&shared);
        let published = shared.publication_guard(&image).unwrap();
        let retained = shared.retention_guard().unwrap();
        shared.generation.store(u64::MAX, Ordering::Release);
        shared.invalidate();
        shared.invalidate();
        assert_eq!(shared.generation(), 0);
        assert!(!published.current());
        assert!(retained.current(), "capture-only exhaustion does not invent a physical loss");
        shared.physical.store(u64::MAX, Ordering::Release);
        let last = shared.retention_guard().unwrap();
        shared.invalidate_physical();
        shared.invalidate_physical();
        assert_eq!(shared.physical.load(Ordering::Acquire), 0);
        assert!(!last.current());
        assert!(shared.retention_guard().is_none());
        assert!(matches!(shared.take(), Err(CaptureError::Exhausted)));
    }

    #[test]
    fn viewport_gpu_mapping_release_distinguishes_unrequested_failed_pending_and_already_unmapped() {
        for completed in [None, Some(false), Some(true)] {
            let shared = shared();
            let frame = request(&shared);
            let callback = shared.begin(&frame, frame.request.boundary(), 1).unwrap();
            if let Some(success) = completed {
                callback.complete(success);
            }
            let mut idle = Mapping::NotRequested;
            assert!(!idle.release(callback.failed()), "no map request means no unmap");
            assert_eq!(idle, Mapping::Released);
            let mut requested = Mapping::Requested;
            assert_eq!(requested.release(callback.failed()), completed != Some(false));
            assert_eq!(requested, Mapping::Released);
            assert!(!requested.release(callback.failed()), "release is exactly once");
            assert_ne!(
                requested,
                Mapping::NotRequested,
                "packing/retry cannot remap an already released buffer"
            );
        }
    }

    #[test]
    fn viewport_guard_exact_cancel_revokes_publication_even_when_queue_epoch_retirement_exhausts() {
        let shared = shared();
        shared.state.try_lock().unwrap().queue.last_epoch_for_test();
        let image = ready_image(&shared);
        let published = shared.publication_guard(&image).unwrap();
        let retained = shared.retention_guard().unwrap();
        assert!(published.current());
        assert!(matches!(shared.cancel(image.request().token()), Err(CaptureError::Exhausted)));
        assert!(!published.current(), "an epoch error cannot skip the exact atomic cancellation");
        assert!(shared.publication_guard(&image).is_none());
        assert!(retained.current(), "capture counter exhaustion is not physical loss");
    }

    #[test]
    fn viewport_gpu_shared_queue_uses_frontend_token_and_completes_without_more_frames() {
        let shared = shared();
        let frame = request(&shared);
        let callback = shared.begin(&frame, frame.request.boundary(), 31).unwrap();
        assert_eq!(
            callback.token(),
            frame.request.token(),
            "backend never allocates a second queue/token"
        );
        assert!(shared.take().unwrap().is_none());
        callback.complete(true);
        let pixels = fake_pixels(&shared, &frame);
        shared.finish(&frame, pixels).unwrap();
        let Some(CaptureOutcome::Ready(image)) = shared.take().unwrap() else {
            panic!("one map result");
        };
        assert_eq!(image.request().token(), frame.request.token());
        assert_eq!(image.frame(), 31);
        assert!(shared.can_publish(&image));
        shared.invalidate();
        assert!(!shared.can_publish(&image), "held bytes are ineligible after owner revocation");
    }

    #[test]
    fn viewport_gpu_atomic_revoke_wins_during_queue_contention_and_late_callback() {
        let shared = shared();
        let old = request(&shared);
        let callback = shared.begin(&old, old.request.boundary(), 1).unwrap();
        callback.complete(true);
        shared.finish(&old, fake_pixels(&shared, &old)).unwrap();
        let guard = shared.state.try_lock().unwrap();
        shared.invalidate();
        assert!(matches!(shared.take(), Err(CaptureError::Busy)));
        assert_eq!(shared.current(&old), Ok(false), "atomic revoke needs no queue mutex");
        drop(guard);
        assert!(
            shared.take().unwrap().is_none(),
            "reconcile retires mailbox before any result can escape"
        );
        let current = request(&shared);
        let fresh = shared.begin(&current, current.request.boundary(), 2).unwrap();
        callback.complete(true);
        let mut state = shared.lock().unwrap();
        assert!(
            state.queue.poll_map(shared.now()).unwrap().is_none(),
            "old callback cannot complete new map"
        );
        drop(state);
        fresh.complete(true);
        shared.finish(&current, fake_pixels(&shared, &current)).unwrap();
        assert!(matches!(shared.take().unwrap(), Some(CaptureOutcome::Ready(_))));
    }

    #[test]
    fn viewport_gpu_timeout_and_same_size_foreign_pool_cannot_publish() {
        let shared = shared();
        let frame = request(&shared);
        let callback = shared.begin(&frame, frame.request.boundary(), 1).unwrap();
        callback.complete(true);
        let foreign = CaptureBudget::new(WORKER_CAPTURE_BYTES).unwrap();
        let mut allocation = foreign.pixels(frame.request.layout()).unwrap();
        for pixel in allocation.bytes_mut().as_chunks_mut::<4>().0.iter_mut() {
            pixel[3] = 255;
        }
        assert!(matches!(
            shared.finish(&frame, allocation.finish().unwrap()),
            Err(CaptureError::Invalid)
        ));
        shared.clock_override.store(1000, Ordering::Release);
        assert!(
            matches!(shared.take().unwrap(), Some(CaptureOutcome::Failed(token, CaptureError::TimedOut)) if token == frame.request.token())
        );
        callback.complete(true);
        assert_eq!(shared.current(&frame), Ok(false));
    }

    #[test]
    fn viewport_gpu_boundary_replacement_and_generation_exhaustion_never_restore_old_capture() {
        let shared = shared();
        let old = request(&shared);
        shared.invalidate();
        let next_boundary = shared.boundary(2, (640, 360)).unwrap();
        let next = shared.request(next_boundary, view(), CaptureCap::Small).unwrap();
        assert_eq!(shared.current(&old), Ok(false));
        assert!(shared.begin(&next, old.request.boundary(), 2).is_err());
        assert!(shared.begin(&old, next_boundary, 2).is_err());
        shared.generation.store(u64::MAX, Ordering::Release);
        shared.invalidate();
        assert_eq!(shared.generation.load(Ordering::Acquire), 0);
        assert!(matches!(shared.take(), Err(CaptureError::Exhausted)));
        shared.invalidate();
        assert_eq!(shared.generation.load(Ordering::Acquire), 0);
        assert_eq!(shared.current(&next), Ok(false));
    }

    #[test]
    fn viewport_gpu_replacement_service_keeps_old_result_credit_charged() {
        let bytes = CaptureLayout::fit(view(), CaptureCap::Small).unwrap().payload_bytes();
        let budget = CaptureBudget::new(bytes * 3).unwrap();
        let first = Shared::with_budget((640, 360), budget.clone()).unwrap();
        first.clock_override.store(0, Ordering::Release);
        let frame = request(&first);
        let resources = budget.resources(frame.request.layout()).unwrap();
        let callback = first.begin(&frame, frame.request.boundary(), 1).unwrap();
        callback.complete(true);
        first.finish(&frame, fake_pixels(&first, &frame)).unwrap();
        let image = first.take().unwrap().unwrap();
        drop((first, frame, callback));
        let replacement = Shared::with_budget((640, 360), budget.clone()).unwrap();
        replacement.clock_override.store(0, Ordering::Release);
        let boundary = replacement.boundary(1, (640, 360)).unwrap();
        replacement.request(boundary, view(), CaptureCap::Small).unwrap();
        assert!(
            matches!(budget.resources(resources.layout()), Err(CaptureError::Capacity)),
            "metadata requests no longer double-charge GPU bytes; physical pair allocation still fails at the shared cap"
        );
        assert_eq!(budget.used(), bytes * 3);
        drop(image);
        assert_eq!(budget.used(), bytes * 2);
        drop(resources);
        assert_eq!(budget.used(), 0);
        assert!(budget.resources(CaptureLayout::fit(view(), CaptureCap::Small).unwrap()).is_ok());
    }

    #[test]
    fn viewport_gpu_pair_cache_reuses_matching_captures_with_fresh_tokens_and_one_allocation() {
        let shared = shared();
        let mut slot: PairSlot<CaptureResources> = PairSlot::default();
        let mut allocations = 0;
        let mut previous = None;
        for sequence in 1..=3 {
            let frame = request(&shared);
            assert_ne!(previous, Some(frame.request.token()));
            previous = Some(frame.request.token());
            let key = PairKey {
                boundary: frame.request.boundary(),
                layout: frame.request.layout(),
                generation: frame.generation,
            };
            slot.acquire(key, || {
                allocations += 1;
                shared.budget.resources(key.layout)
            })
            .unwrap();
            let submitted = slot.submitted().unwrap();
            let callback = shared.begin(&frame, key.boundary, sequence).unwrap();
            callback.complete(true);
            shared.finish(&frame, fake_pixels(&shared, &frame)).unwrap();
            drop(shared.take().unwrap().unwrap());
            slot.unmapped();
            assert!(matches!(slot.acquire(key, || panic!("still in flight")), Err(CaptureError::Busy)));
            submitted.store(true, Ordering::Release);
            slot.poll(frame.generation);
            assert_eq!(allocations, 1, "matching target/staging pair is allocated once");
            assert_eq!(shared.budget.used(), key.layout.payload_bytes() + key.layout.staging_bytes());
        }
        let old = slot.key.unwrap();
        let resized = PairKey {
            layout: CaptureLayout::new(480, 270).unwrap(),
            ..old
        };
        slot.acquire(resized, || {
            allocations += 1;
            shared.budget.resources(resized.layout)
        })
        .unwrap();
        let done = slot.submitted().unwrap();
        done.store(true, Ordering::Release);
        slot.unmapped();
        slot.poll(old.generation);
        assert_eq!(allocations, 2);
        let new_device = PairKey {
            boundary: CaptureBoundary::new(old.boundary.device_epoch() + 1, 1, old.boundary.size()).unwrap(),
            ..resized
        };
        slot.acquire(new_device, || {
            allocations += 1;
            shared.budget.resources(new_device.layout)
        })
        .unwrap();
        assert_eq!(allocations, 3, "device replacement cannot reuse old resources");
    }

    #[test]
    fn viewport_gpu_pair_reuse_waits_for_unmap_and_exact_submission_even_after_cancel() {
        let shared = shared();
        let frame = request(&shared);
        let key = PairKey {
            boundary: frame.request.boundary(),
            layout: frame.request.layout(),
            generation: frame.generation,
        };
        let mut slot: PairSlot<CaptureResources> = PairSlot::default();
        let mut allocations = 0;
        slot.acquire(key, || {
            allocations += 1;
            shared.budget.resources(key.layout)
        })
        .unwrap();
        let old = slot.submitted().unwrap();
        old.store(true, Ordering::Release);
        assert!(matches!(slot.acquire(key, || panic!("map not unmapped")), Err(CaptureError::Busy)));
        slot.unmapped();
        slot.poll(key.generation);
        slot.acquire(key, || panic!("reuse expected")).unwrap();
        let cancelled = slot.submitted().unwrap();
        slot.unmapped();
        slot.retire(key.generation + 1);
        let next = PairKey {
            generation: key.generation + 1,
            ..key
        };
        assert!(matches!(
            slot.acquire(next, || panic!("cancel is not GPU completion")),
            Err(CaptureError::Busy)
        ));
        cancelled.store(true, Ordering::Release);
        slot.poll(next.generation);
        slot.acquire(next, || {
            allocations += 1;
            shared.budget.resources(next.layout)
        })
        .unwrap();
        let fresh = slot.submitted().unwrap();
        slot.unmapped();
        old.store(true, Ordering::Release);
        cancelled.store(true, Ordering::Release);
        assert!(matches!(
            slot.acquire(next, || panic!("old callbacks cannot retire new submission")),
            Err(CaptureError::Busy)
        ));
        fresh.store(true, Ordering::Release);
        slot.poll(next.generation);
        slot.acquire(next, || panic!("new safely retired pair is reusable")).unwrap();
        assert_eq!(allocations, 2);
    }

    #[test]
    fn viewport_gpu_cached_pair_and_callback_leases_survive_renderer_teardown() {
        let shared = shared();
        let frame = request(&shared);
        let layout = frame.request.layout();
        let key = PairKey {
            boundary: frame.request.boundary(),
            layout,
            generation: frame.generation,
        };
        let bytes = layout.payload_bytes() + layout.staging_bytes();
        let budget = CaptureBudget::new(bytes).unwrap();
        let mut slot: PairSlot<CaptureResources> = PairSlot::default();
        slot.acquire(key, || budget.resources(layout)).unwrap();
        let retired = slot.submitted().unwrap();
        let callback_lease = slot.value.as_ref().unwrap().clone();
        slot.unmapped();
        slot.retire(key.generation + 1);
        drop(slot);
        assert_eq!(budget.used(), bytes);
        assert!(matches!(budget.resources(layout), Err(CaptureError::Capacity)));
        retired.store(true, Ordering::Release);
        drop(callback_lease);
        assert_eq!(budget.used(), 0);
        assert!(budget.resources(layout).is_ok());
    }

    #[test]
    fn viewport_gpu_crop_format_and_padded_rows_preserve_top_down_opaque_bytes() {
        let shared = shared();
        let rect = WorldViewport {
            x: 80,
            y: 45,
            width: 320,
            height: 180,
        };
        let frame = shared
            .request(shared.boundary(1, (640, 360)).unwrap(), rect, CaptureCap::Medium)
            .unwrap();
        let encoded = params(&frame.request, wgpu::TextureFormat::Bgra8Unorm);
        assert_eq!(encoded.rect, [0.125, 0.125, 0.5, 0.5]);
        assert_eq!(encoded.encode_srgb, 0);
        assert_eq!(params(&frame.request, wgpu::TextureFormat::Rgba8UnormSrgb).encode_srgb, 1);
        assert!(!sampleable(wgpu::TextureFormat::Rgba8Unorm, wgpu::TextureUsages::COPY_SRC));
        assert!(!sampleable(wgpu::TextureFormat::Rgba16Float, wgpu::TextureUsages::TEXTURE_BINDING));
        let layout = frame.request.layout();
        assert!(layout.padded_row_bytes() > layout.row_bytes());
        let mut source = vec![0xCC; layout.staging_bytes()];
        for (row_index, row) in source.chunks_exact_mut(layout.padded_row_bytes() as usize).enumerate() {
            for pixel in row[..layout.row_bytes() as usize].as_chunks_mut::<4>().0.iter_mut() {
                pixel.copy_from_slice(&[row_index as u8, 19, 73, 255]);
            }
        }
        let mut packed = shared.budget.pixels(layout).unwrap();
        pack_rows(&source, packed.bytes_mut(), layout).unwrap();
        let image = packed.finish().unwrap();
        assert_eq!(&image.bytes()[..4], &[0, 19, 73, 255]);
        assert_eq!(
            &image.bytes()[layout.row_bytes() as usize..layout.row_bytes() as usize + 4],
            &[1, 19, 73, 255]
        );
        assert!(pack_rows(&source[..source.len() - 1], &mut vec![0; layout.payload_bytes()], layout).is_err());
    }

    #[test]
    fn viewport_gpu_shader_validates_without_a_driver_and_capture_path_has_no_waits() {
        let shader = naga::front::wgsl::parse_str(include_str!("shaders/viewport_capture.wgsl")).expect("viewport sample shader parses");
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
            .validate(&shader)
            .expect("viewport sample shader validates");
        const FORBIDDEN: [&str; 7] = [
            "state.lock()",
            "wait_indefinitely",
            "PollType::Wait",
            "finish_until",
            "std::fs",
            "TcpStream",
            "thread::sleep",
        ];
        fn production_prefix(source: &str) -> String {
            // Windows checkouts may use CRLF. A missed boundary would scan
            // these test literals instead of the asynchronous production path.
            let normalized = source.replace("\r\n", "\n");
            normalized
                .split_once("#[cfg(test)]\nmod tests")
                .expect("capture source has a test module boundary")
                .0
                .to_owned()
        }
        fn forbidden_operation(source: &str) -> Option<&'static str> {
            FORBIDDEN.into_iter().find(|forbidden| source.contains(*forbidden))
        }
        for newline in ["\n", "\r\n"] {
            let fixture = format!("state.try_lock();\n#[cfg(test)]\nmod tests {{\n{}\n}}\n", FORBIDDEN.join("\n")).replace('\n', newline);
            let prefix = production_prefix(&fixture);
            assert!(prefix.contains("state.try_lock()"));
            assert_eq!(
                forbidden_operation(&prefix),
                None,
                "test literals are outside the {newline:?} production prefix"
            );
            for forbidden in FORBIDDEN {
                let fixture = format!("state.try_lock();\n{forbidden};\n#[cfg(test)]\nmod tests {{}}\n").replace('\n', newline);
                assert_eq!(
                    forbidden_operation(&production_prefix(&fixture)),
                    Some(forbidden),
                    "the {newline:?} production prefix must still reject {forbidden}"
                );
            }
        }
        let source = production_prefix(include_str!("viewport_capture_gpu.rs"));
        assert!(
            forbidden_operation(&source).is_none(),
            "asynchronous capture introduced {:?}",
            forbidden_operation(&source)
        );
        assert!(source.contains("state.try_lock()"));
    }

    #[test]
    #[ignore = "requires an explicitly requested offline GPU run; synthetic textures only"]
    fn viewport_gpu_synthetic_crop_colors_srgb_channels_and_alpha() {
        let _gpu = crate::test_gpu::lock();
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).expect("synthetic capture GPU adapter");
        let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).expect("synthetic capture GPU device");
        for format in [
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            wgpu::TextureFormat::Bgra8Unorm,
            wgpu::TextureFormat::Bgra8UnormSrgb,
        ] {
            let shared = Shared::new((64, 64), Arc::new(AtomicBool::new(false))).unwrap();
            shared.clock_override.store(0, Ordering::Release);
            let viewport = WorldViewport {
                x: 8,
                y: 8,
                width: 48,
                height: 48,
            };
            let frame = shared
                .request(shared.boundary(1, (64, 64)).unwrap(), viewport, CaptureCap::Small)
                .unwrap();
            let source = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("synthetic capture colors"),
                size: wgpu::Extent3d {
                    width: 64,
                    height: 64,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let colors = [[64, 128, 192, 0], [192, 64, 128, 0], [128, 192, 64, 0], [32, 96, 160, 0]];
            let mut bytes = vec![0; 64 * 64 * 4];
            for y in 0..64 {
                for x in 0..64 {
                    let mut color = if (8..56).contains(&x) && (8..56).contains(&y) {
                        colors[usize::from(x >= 32) + usize::from(y >= 32) * 2]
                    } else {
                        [255, 0, 255, 0]
                    };
                    if matches!(format, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb) {
                        color.swap(0, 2);
                    }
                    bytes[(y * 64 + x) * 4..(y * 64 + x + 1) * 4].copy_from_slice(&color);
                }
            }
            queue.write_texture(
                source.as_image_copy(),
                &bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: Some(64),
                },
                source.size(),
            );
            let mut capture = GpuCapture::new(shared.clone());
            let mut encoder = device.create_command_encoder(&Default::default());
            capture.encode(
                &device,
                &queue,
                &mut encoder,
                &source,
                frame.clone(),
                frame.request.boundary(),
                viewport,
                1,
            );
            queue.submit([encoder.finish()]);
            capture.after_submit(&queue);
            let deadline = Instant::now() + std::time::Duration::from_secs(2);
            let result = loop {
                device.poll(wgpu::PollType::Poll).expect("nonblocking synthetic map poll");
                capture.maintain();
                if let Some(result) = shared.take().unwrap() {
                    break result;
                }
                assert!(Instant::now() < deadline, "synthetic capture did not complete");
                std::thread::yield_now();
            };
            let CaptureOutcome::Ready(image) = result else {
                panic!("synthetic capture failed");
            };
            let (width, height) = image.pixels().layout().size();
            for (index, (x, y)) in [(0, 0), (width - 1, 0), (0, height - 1), (width - 1, height - 1)]
                .into_iter()
                .enumerate()
            {
                let offset = ((y * width + x) * 4) as usize;
                let actual = &image.pixels().bytes()[offset..offset + 4];
                for channel in 0..3 {
                    assert!(actual[channel].abs_diff(colors[index][channel]) <= 2, "{format:?}: {actual:?}");
                }
                assert_eq!(actual[3], 255);
            }
        }
    }

    fn poll_gpu_until(device: &wgpu::Device, mut ready: impl FnMut() -> bool) {
        let deadline = Instant::now() + std::time::Duration::from_secs(2);
        loop {
            device.poll(wgpu::PollType::Poll).expect("nonblocking synthetic lifecycle poll");
            if ready() {
                return;
            }
            assert!(Instant::now() < deadline, "synthetic lifecycle did not retire");
            std::thread::yield_now();
        }
    }

    fn gpu_image(device: &wgpu::Device, capture: &mut GpuCapture) -> CapturedViewport {
        let mut result = None;
        poll_gpu_until(device, || {
            capture.maintain();
            result = capture.shared.take().unwrap();
            result.is_some()
        });
        let Some(CaptureOutcome::Ready(image)) = result else {
            panic!("synthetic lifecycle capture failed");
        };
        image
    }

    #[test]
    #[ignore = "requires an explicitly requested offline GPU run; synthetic textures only"]
    fn viewport_gpu_synthetic_mapping_success_reuse_cancel_and_drop_have_no_validation_errors() {
        let _gpu = crate::test_gpu::lock();
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).expect("synthetic lifecycle GPU adapter");
        let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).expect("synthetic lifecycle GPU device");
        // Keep wgpu's default validation handler: any invalid map/unmap panics.
        let source = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("synthetic lifecycle source"),
            size: wgpu::Extent3d {
                width: 64,
                height: 64,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let bytes = [47, 83, 127, 255].repeat(64 * 64);
        queue.write_texture(
            source.as_image_copy(),
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: Some(64),
            },
            source.size(),
        );
        let make_shared = || {
            let shared = Shared::with_budget((64, 64), CaptureBudget::new(WORKER_CAPTURE_BYTES).unwrap()).unwrap();
            shared.clock_override.store(0, Ordering::Release);
            shared
        };
        let encode = |capture: &mut GpuCapture, index| {
            let shared = capture.shared.clone();
            let viewport = WorldViewport::full((64, 64));
            let frame = shared
                .request(shared.boundary(1, (64, 64)).unwrap(), viewport, CaptureCap::Small)
                .unwrap();
            let mut encoder = device.create_command_encoder(&Default::default());
            capture.encode(
                &device,
                &queue,
                &mut encoder,
                &source,
                frame.clone(),
                frame.request.boundary(),
                viewport,
                index,
            );
            assert_eq!(capture.pending.as_ref().unwrap().mapping, Mapping::NotRequested);
            (frame, encoder)
        };

        // Two successful requests reuse the exact real staging allocation.
        // Completed packing and idle cache Drop must not call unmap again.
        let shared = make_shared();
        let mut capture = GpuCapture::new(shared.clone());
        let (first, encoder) = encode(&mut capture, 1);
        queue.submit([encoder.finish()]);
        capture.after_submit(&queue);
        let image = gpu_image(&device, &mut capture);
        assert!(
            image
                .pixels()
                .bytes()
                .as_chunks::<4>()
                .0
                .iter()
                .all(|pixel| *pixel == [47, 83, 127, 255])
        );
        drop(image);
        poll_gpu_until(&device, || {
            capture.maintain();
            !capture.has_pending()
        });
        let staging = capture.pair.value.as_ref().unwrap().staging.clone();
        let (second, encoder) = encode(&mut capture, 2);
        assert_ne!(first.request.token(), second.request.token());
        assert_eq!(capture.pair.value.as_ref().unwrap().staging, staging);
        queue.submit([encoder.finish()]);
        capture.after_submit(&queue);
        drop(gpu_image(&device, &mut capture));
        poll_gpu_until(&device, || {
            capture.maintain();
            !capture.has_pending()
        });
        drop(staging);
        drop(capture);
        assert_eq!(shared.budget.used(), 0);

        // No map was requested and no encoded work is submitted in this case.
        // Discard the encoder before retiring its unsubmitted resource pair.
        let shared = make_shared();
        let mut capture = GpuCapture::new(shared.clone());
        let (_, encoder) = encode(&mut capture, 3);
        drop(encoder);
        capture.invalidate();
        assert!(capture.pending.is_none());
        drop(capture);
        assert_eq!(shared.budget.used(), 0);

        for (index, wait_ready, drop_owner) in [(4, false, false), (5, true, false), (6, false, true), (7, true, true)] {
            let shared = make_shared();
            let mut capture = Some(GpuCapture::new(shared.clone()));
            let (_, encoder) = encode(capture.as_mut().unwrap(), index);
            queue.submit([encoder.finish()]);
            capture.as_mut().unwrap().after_submit(&queue);
            if wait_ready {
                // Callback success has actually mapped the buffer, but packing
                // has not run. Both cancel and Drop must unmap it exactly once.
                poll_gpu_until(&device, || shared.lock().unwrap().queue.poll_map(shared.now()).unwrap().is_some());
            }
            let retired = capture.as_ref().unwrap().pair.flight.as_ref().unwrap().retired.clone();
            let layout = capture.as_ref().unwrap().pending.as_ref().unwrap().frame.request.layout();
            if drop_owner {
                drop(capture.take());
                assert!(
                    shared.budget.used() >= layout.payload_bytes() + layout.staging_bytes() || retired.load(Ordering::Acquire),
                    "submitted GPU credit cannot disappear before its retirement callback"
                );
                poll_gpu_until(&device, || shared.budget.used() == 0);
            } else {
                capture.as_mut().unwrap().invalidate();
                poll_gpu_until(&device, || {
                    capture.as_mut().unwrap().maintain();
                    !capture.as_ref().unwrap().has_pending()
                });
                drop(capture.take());
            }
            assert_eq!(shared.budget.used(), 0);
            assert!(
                !matches!(shared.take().unwrap(), Some(CaptureOutcome::Ready(_))),
                "late callback cannot publish canceled bytes"
            );
        }

        // Retirement-only maintenance cannot pack an already mapped eligible
        // request, and an old origin cannot release a newer mapping.
        let shared = make_shared();
        let mut capture = GpuCapture::new(shared.clone());
        let (current, encoder) = encode(&mut capture, 8);
        queue.submit([encoder.finish()]);
        capture.after_submit(&queue);
        poll_gpu_until(&device, || shared.lock().unwrap().queue.poll_map(shared.now()).unwrap().is_some());
        let before = shared.budget.used();
        capture.maintain_retirement(first.request.token());
        assert!(capture.pending.as_ref().unwrap().packed.is_none());
        assert_eq!(capture.pending.as_ref().unwrap().frame.request.token(), current.request.token());
        capture.maintain_retirement(current.request.token());
        assert!(capture.pending.is_none());
        assert!(shared.budget.used() <= before, "cleanup allocated no packed pixels");
        assert!(!matches!(shared.take().unwrap(), Some(CaptureOutcome::Ready(_))));
        poll_gpu_until(&device, || {
            capture.maintain_retirement(current.request.token());
            !capture.has_pending()
        });
        drop(capture);
        assert_eq!(shared.budget.used(), 0);
    }
}
