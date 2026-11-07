//! Render-owner viewport sampling. Frontend and backend share one pure queue;
//! only try_lock is used, never across GPU APIs or pixel packing. No caller in
//! App/native hosting enables this service yet. It grants no world authority.

use crate::WorldViewport;
use crate::pipeline_cache::CachedPipelines;
use crate::viewport_capture::{
    CaptureBoundary, CaptureBudget, CaptureCap, CaptureError, CaptureLayout, CaptureOutcome, CapturePixels, CaptureQueue, CaptureRequest,
    CaptureToken, CapturedViewport, MapCallback, WORKER_CAPTURE_BYTES,
};
use bytemuck::{Pod, Zeroable};
use parking_lot::{Mutex, MutexGuard};
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicU64, Ordering},
};
use std::time::Instant;
use wgpu::util::DeviceExt;

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
}

pub(crate) struct Shared {
    state: Mutex<State>,
    budget: CaptureBudget,
    generation: AtomicU64,
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
        frame.generation != 0 && frame.generation == self.generation.load(Ordering::Acquire)
    }
    pub fn new(size: (u32, u32)) -> Result<Arc<Self>, CaptureError> {
        // Renderer/device replacement does not mint fresh image credit while
        // the old driver/callback/results still retain physical allocations.
        let budget =
            WORKER_BUDGET.get_or_init(|| CaptureBudget::new(WORKER_CAPTURE_BYTES).expect("fixed positive worker capture bound is valid"));
        Self::with_budget(size, budget.clone())
    }

    fn with_budget(size: (u32, u32), budget: CaptureBudget) -> Result<Arc<Self>, CaptureError> {
        let device = NEXT_DEVICE
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |next| next.checked_add(1))
            .map_err(|_| CaptureError::Exhausted)?;
        let boundary = CaptureBoundary::new(device, 1, size)?;
        Ok(Arc::new(Self {
            state: Mutex::new(State {
                queue: CaptureQueue::new(boundary, budget.clone())?,
                generation: 1,
            }),
            budget,
            generation: AtomicU64::new(1),
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
        let mut state = self.state.try_lock().ok_or(CaptureError::Busy)?;
        let generation = self.generation.load(Ordering::Acquire);
        if generation == 0 {
            return Err(CaptureError::Exhausted);
        }
        if state.generation != generation {
            state.queue.invalidate()?;
            state.generation = generation;
        }
        Ok(state)
    }

    /// Logical revocation never waits for a contended queue or render job.
    pub fn invalidate(&self) {
        if self
            .generation
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1).filter(|_| value != 0)
            })
            .is_err()
        {
            self.generation.store(0, Ordering::Release);
        }
    }

    pub fn request(&self, boundary: CaptureBoundary, viewport: WorldViewport, cap: CaptureCap) -> Result<FrameCapture, CaptureError> {
        let mut state = self.lock()?;
        state.queue.synchronize_boundary(boundary)?;
        let request = state.queue.request(viewport, cap, self.now())?;
        let generation = state.generation;
        if generation != self.generation.load(Ordering::Acquire) {
            return Err(CaptureError::Cancelled);
        }
        Ok(FrameCapture { request, generation })
    }

    pub fn cancel(&self, token: CaptureToken) -> Result<(), CaptureError> {
        let mut state = self.lock()?;
        state.queue.cancel(token)?;
        self.invalidate();
        Ok(())
    }

    pub fn current(&self, frame: &FrameCapture) -> Result<bool, CaptureError> {
        if !self.generation_current(frame) {
            return Ok(false);
        }
        let mut state = self.lock()?;
        state.queue.poll_map(self.now())?;
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
        let result = state.queue.take_result(self.now())?;
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
        state.queue.can_publish(result, self.now()) && generation == self.generation.load(Ordering::Acquire)
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
    staging: wgpu::Buffer,
    _target: wgpu::Texture,
    mapped: bool,
    packed: Option<CapturePixels>,
}
impl Drop for Pending {
    fn drop(&mut self) {
        self.staging.unmap();
    }
}

pub(crate) struct GpuCapture {
    pub shared: Arc<Shared>,
    pipeline: Option<wgpu::RenderPipeline>,
    pending: Option<Pending>,
}

impl GpuCapture {
    pub fn new(shared: Arc<Shared>) -> Self {
        Self {
            shared,
            pipeline: None,
            pending: None,
        }
    }

    /// Only the render owner calls this, at the pre-label/HUD composition tap.
    pub fn encode(
        &mut self,
        device: &wgpu::Device,
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
        let pipeline = self.pipeline.get_or_insert_with(|| pipeline(device));
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("viewport capture crop"),
            contents: bytemuck::bytes_of(&params(&frame.request, source.format())),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
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
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniform.as_entire_binding(),
                },
            ],
        });
        let target_view = target.create_view(&Default::default());
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
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &staging,
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
            staging,
            _target: target,
            mapped: false,
            packed: None,
        });
    }

    pub fn after_submit(&mut self, queue: &wgpu::Queue) {
        if let Some(pending) = &mut self.pending
            && !pending.mapped
        {
            // Mapping can be cancelled before its copy completes. The ordered
            // queue callback retains GPU allocation credit through real work
            // completion independently of map/queue/UI cancellation.
            let submitted_credit = pending.frame.request.clone();
            queue.on_submitted_work_done(move || drop(submitted_credit));
            let callback = pending.callback.clone();
            pending
                .staging
                .map_async(wgpu::MapMode::Read, .., move |result| callback.complete(result.is_ok()));
            pending.mapped = true;
        }
    }

    /// Device polling occurs in Backend, on its render owner, even without
    /// frame pacing or another presentation. This method never waits for maps.
    pub fn maintain(&mut self) {
        let Some(pending) = &mut self.pending else {
            return;
        };
        match self.shared.current(&pending.frame) {
            Ok(true) => {}
            Err(CaptureError::Busy) => return,
            _ => {
                self.pending = None;
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
            if !ready || !pending.mapped {
                return;
            }
            let pixels = match self.shared.budget.pixels(pending.frame.request.layout()) {
                Ok(pixels) => pixels,
                Err(error) => {
                    self.shared.fail(&pending.frame, error);
                    self.pending = None;
                    return;
                }
            };
            let mut pixels = pixels;
            let result = match pending.staging.get_mapped_range(..) {
                Ok(mapped) => pack_rows(&mapped, pixels.bytes_mut(), pending.frame.request.layout()),
                Err(_) => Err(CaptureError::MapFailed),
            };
            pending.staging.unmap();
            pending.packed = match result.and_then(|_| pixels.finish()) {
                Ok(pixels) => Some(pixels),
                Err(error) => {
                    self.shared.fail(&pending.frame, error);
                    self.pending = None;
                    return;
                }
            };
        }
        let Some(pixels) = pending.packed.as_ref() else {
            return;
        };
        match self.shared.finish(&pending.frame, pixels.clone()) {
            Err(CaptureError::Busy) => {}
            _ => self.pending = None,
        }
    }

    pub fn invalidate(&mut self) {
        self.shared.invalidate();
        self.pending = None;
    }
    pub fn has_pending(&self) -> bool {
        self.pending.is_some()
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
        for pixel in pixels.bytes_mut().chunks_exact_mut(4) {
            pixel.copy_from_slice(&[23, 47, 91, 255]);
        }
        pixels.finish().unwrap()
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
        for pixel in allocation.bytes_mut().chunks_exact_mut(4) {
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
        let callback = first.begin(&frame, frame.request.boundary(), 1).unwrap();
        callback.complete(true);
        first.finish(&frame, fake_pixels(&first, &frame)).unwrap();
        let image = first.take().unwrap().unwrap();
        drop((first, frame, callback));
        let replacement = Shared::with_budget((640, 360), budget.clone()).unwrap();
        replacement.clock_override.store(0, Ordering::Release);
        let boundary = replacement.boundary(1, (640, 360)).unwrap();
        assert!(matches!(
            replacement.request(boundary, view(), CaptureCap::Small),
            Err(CaptureError::Capacity)
        ));
        assert_eq!(budget.used(), bytes * 3);
        drop(image);
        assert_eq!(budget.used(), 0);
        assert!(replacement.request(boundary, view(), CaptureCap::Small).is_ok());
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
            for pixel in row[..layout.row_bytes() as usize].chunks_exact_mut(4) {
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
        let source = include_str!("viewport_capture_gpu.rs")
            .split("#[cfg(test)]\nmod tests")
            .next()
            .unwrap();
        for forbidden in [
            "state.lock()",
            "wait_indefinitely",
            "PollType::Wait",
            "finish_until",
            "std::fs",
            "TcpStream",
            "thread::sleep",
        ] {
            assert!(!source.contains(forbidden), "asynchronous capture introduced {forbidden}");
        }
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
        ] {
            let shared = Shared::new((64, 64)).unwrap();
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
                    if format == wgpu::TextureFormat::Bgra8Unorm {
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
            capture.encode(&device, &mut encoder, &source, frame.clone(), frame.request.boundary(), viewport, 1);
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
}
