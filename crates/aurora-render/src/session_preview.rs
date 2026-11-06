//! Opt-in standby world previews. No native surface, interface, pick, photo
//! capture, or session-presentation proof is produced by this path.
//!
//! The caller supplies a fresh account/vision/physical-budget decision and a
//! consumed renderer fence. An admitted request draws into persistent small
//! targets on the existing device/queue. One asynchronous readback is allowed;
//! busy requests are dropped before their scene journal is consumed.

use crate::packet::{FramePacket, Spent};
use crate::{AntiAliasing, DrawLists, FrameParams, PhotoContext, RenderSettings, SessionCompletion};
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};
use std::time::{Duration, Instant};

pub const GPU_RESERVE_BYTES: u64 = 128 << 20;
pub(crate) const MAX_SCENE_SLOTS: usize = 65_536;
pub(crate) const MAX_JOURNAL_BYTES: usize = 4 << 20;
const ADMISSION_AGE: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    Small,
    Medium,
    Large,
}
impl Size {
    pub fn dimensions(self) -> (u32, u32) {
        match self {
            Self::Small => (320, 180),
            Self::Medium => (480, 270),
            Self::Large => (640, 360),
        }
    }
    pub fn rgba_bytes(self) -> usize {
        let (w, h) = self.dimensions();
        w as usize * h as usize * 4
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rate {
    Half,
    One,
    Two,
    Five,
}
impl Rate {
    pub fn interval(self) -> Duration {
        Duration::from_millis(match self {
            Self::Half => 2000,
            Self::One => 1000,
            Self::Two => 500,
            Self::Five => 200,
        })
    }
}

/// Local, short-lived admission, never a wire credential. The worker must
/// derive these flags from its current exact session/lease, visibility, native
/// capture/vision restrictions and shared GPU pressure policy. A rate allowance
/// alone is insufficient. Unknown policy must set its corresponding flag false.
#[derive(Clone, Copy, Debug)]
pub struct Admission {
    /// Strictly advance on every account/session/lease or vision-policy
    /// change. Callers must immediately purge displayed old-epoch pixels too.
    pub epoch: u64,
    pub fence: SessionCompletion,
    pub observed_at: Instant,
    pub standby: bool,
    pub visible: bool,
    /// Include native no-capture policy and locked/required HUD vision. This
    /// world-only path cannot reproduce a mandatory attachment HUD; deny it
    /// rather than removing that restriction from a preview.
    pub vision_allowed: bool,
    pub gpu_safe: bool,
    pub extra_gpu_bytes: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct Request {
    pub admission: Admission,
    pub sequence: u64,
    pub size: Size,
    pub rate: Rate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Unsupported,
    NotAdmitted,
    Stale,
    Busy,
    Budget,
    SceneNotReady,
    RendererFailed,
    Readback,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "standby preview: {self:?}")
    }
}
impl std::error::Error for Error {}

/// Tightly packed, opaque RGBA8 at the actual rendered size. The caller must
/// retain only its latest matching session/policy frame and never retry input
/// or attach viewer interactivity to these pixels.
#[derive(Debug)]
pub struct Frame {
    pub epoch: u64,
    pub sequence: u64,
    pub size: Size,
    pub rgba: Vec<u8>,
    /// Encoding through map completion, excluding one-time initialization.
    pub elapsed: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Key {
    pub epoch: u64,
    pub sequence: u64,
    pub revision: u64,
}

pub(crate) enum Job {
    Init {
        size: Size,
        observed_at: Instant,
        scene: Box<crate::packet::SceneGpu>,
    },
    WarmPoll,
    Render {
        key: Key,
        size: Size,
        observed_at: Instant,
        packet: Box<FramePacket>,
    },
    Poll(Key),
    Release,
}
pub(crate) enum Reply {
    Warming,
    Warmed,
    InitFailed(Error),
    Submitted(Key, Box<Spent>),
    Pending(Key),
    Ready(Key, Frame),
    Failed(Key, Error, Option<Box<Spent>>),
    Released,
}

#[derive(Default)]
pub(crate) struct Owner {
    revision: u64,
    candidate: Option<(u64, u64)>,
    ready: Option<SessionCompletion>,
    epoch: u64,
    sequence: u64,
    last_submit: Option<Instant>,
    flight: Option<Key>,
    handed: bool,
    allocated: bool,
    warming: bool,
    exhausted: bool,
    release: bool,
    latest: Option<Frame>,
    error: Option<Error>,
}

impl Owner {
    pub fn invalidate(&mut self) {
        if let Some(next) = self.revision.checked_add(1) {
            self.revision = next;
        } else {
            self.exhausted = true;
        }
        self.ready = None;
        self.candidate = None;
        self.latest = None;
        self.release |= self.allocated;
    }
    pub fn fence_requested(&mut self, ticket: u64) {
        if self.candidate.is_some_and(|(old, _)| old == ticket) {
            return;
        }
        self.invalidate();
        self.candidate = Some((ticket, self.revision));
    }
    pub fn fence_consumed(&mut self, proof: SessionCompletion) {
        if self.revision != 0 && self.candidate == Some((proof.ticket, self.revision)) {
            self.ready = Some(proof);
        }
    }
    pub fn authorize(&self, admission: &Admission, now: Instant) -> Result<(), Error> {
        if !fresh(admission.observed_at, now) {
            return Err(Error::Stale);
        }
        if admission.epoch < self.epoch {
            return Err(Error::Stale);
        }
        if self.exhausted
            || admission.epoch == 0
            || !admission.standby
            || !admission.visible
            || !admission.vision_allowed
            || !admission.gpu_safe
        {
            return Err(Error::NotAdmitted);
        }
        if admission.extra_gpu_bytes < GPU_RESERVE_BYTES {
            return Err(Error::Budget);
        }
        if self.ready != Some(admission.fence) || admission.fence.ticket == 0 || admission.fence.surface_epoch == 0 {
            return Err(Error::NotAdmitted);
        }
        Ok(())
    }
    pub fn check(&self, request: &Request, now: Instant) -> Result<(), Error> {
        self.authorize(&request.admission, now)?;
        if self.handed || self.warming || self.flight.is_some() || self.release {
            return Err(Error::Busy);
        }
        if request.sequence == 0
            || request.admission.epoch < self.epoch
            || (request.admission.epoch == self.epoch && request.sequence <= self.sequence)
        {
            return Err(Error::Stale);
        }
        if self
            .last_submit
            .is_some_and(|at| now.saturating_duration_since(at) < request.rate.interval())
        {
            return Err(Error::Busy);
        }
        Ok(())
    }
    pub fn key(&self, request: &Request) -> Key {
        Key {
            epoch: request.admission.epoch,
            sequence: request.sequence,
            revision: self.revision,
        }
    }
    pub fn needs_init(&self) -> bool {
        !self.allocated
    }
    pub fn initializing(&mut self) {
        self.allocated = true;
        self.warming = true;
        self.handed = true;
    }
    pub fn submitted(&mut self, request: &Request, now: Instant) {
        self.epoch = request.admission.epoch;
        self.sequence = request.sequence;
        self.last_submit = Some(now);
        self.flight = Some(self.key(request));
        self.handed = true;
        self.allocated = true;
        self.error = None;
    }
    pub fn next_job(&self) -> Option<Job> {
        if self.handed {
            None
        } else if self.release {
            Some(Job::Release)
        } else if self.warming {
            Some(Job::WarmPoll)
        } else {
            self.flight.map(Job::Poll)
        }
    }
    pub fn handed(&mut self) {
        self.handed = true;
    }
    pub fn receive(&mut self, reply: Reply) -> Option<Box<Spent>> {
        self.handed = false;
        match reply {
            Reply::Warming => {
                self.warming = true;
                None
            }
            Reply::Warmed => {
                self.warming = false;
                None
            }
            Reply::InitFailed(error) => {
                self.warming = false;
                self.error = Some(error);
                self.release = true;
                self.ready = None;
                None
            }
            Reply::Submitted(key, spent) => {
                if self.flight != Some(key) {
                    self.release = true;
                }
                Some(spent)
            }
            Reply::Pending(key) => {
                if self.flight != Some(key) {
                    self.release = true;
                }
                None
            }
            Reply::Ready(key, frame) => {
                if self.flight == Some(key) {
                    self.flight = None;
                }
                if key.revision == self.revision
                    && key.epoch == self.epoch
                    && !self.release
                    && frame.epoch == key.epoch
                    && frame.sequence == key.sequence
                    && frame.rgba.len() == frame.size.rgba_bytes()
                {
                    if frame.elapsed > ADMISSION_AGE {
                        self.error = Some(Error::Budget);
                        self.release = true;
                        self.ready = None;
                    } else {
                        self.latest = Some(frame);
                    }
                }
                None
            }
            Reply::Failed(key, error, spent) => {
                if self.flight == Some(key) {
                    self.flight = None;
                    self.error = Some(error);
                }
                self.release = true;
                self.ready = None;
                spent
            }
            Reply::Released => {
                self.allocated = false;
                self.warming = false;
                self.release = false;
                self.flight = None;
                None
            }
        }
    }
    pub fn take(&mut self, admission: &Admission, now: Instant) -> Result<Option<Frame>, Error> {
        if let Some(error) = self.error.take() {
            return Err(error);
        }
        self.authorize(admission, now)?;
        Ok(self.latest.take().filter(|frame| frame.epoch == admission.epoch))
    }
}

pub(crate) fn fresh(observed_at: Instant, now: Instant) -> bool {
    now.checked_duration_since(observed_at).is_some_and(|age| age <= ADMISSION_AGE)
}

pub(crate) fn memory_safe(memory: Option<crate::vram::VideoMemory>) -> bool {
    memory.is_some_and(|m| m.budget.saturating_sub(m.usage.max(m.allocated)) >= GPU_RESERVE_BYTES + (m.budget / 8).max(256 << 20))
}

pub(crate) fn settings() -> RenderSettings {
    RenderSettings {
        aa: AntiAliasing::None,
        render_scale_percent: 100,
        shadow_resolution: 0,
        projector_shadow_limit: 0,
        probe_slots: 1,
        ssao: 0,
        screen_indirect: false,
        contact_shadows: false,
        water_reflection_scale: 0.0,
        mirror_scale: 0.0,
        ssr: false,
        occlusion: false,
        water_effects: crate::WaterEffects {
            enabled: false,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Build the camera before CPU culling for this view; a narrower native-window
/// draw list cannot supply geometry outside its original frustum.
pub fn projection(fov_y: f32, near: f32, size: Size) -> Result<glam::Mat4, Error> {
    if !fov_y.is_finite() || !(0.01..3.13).contains(&fov_y) || !near.is_finite() || near <= 0.0 {
        return Err(Error::SceneNotReady);
    }
    let (w, h) = size.dimensions();
    Ok(glam::camera::rh::proj::directx::perspective_infinite_reverse(
        fov_y,
        w as f32 / h as f32,
        near,
    ))
}

pub(crate) fn lists_ready(src: &DrawLists) -> bool {
    src.total() <= MAX_SCENE_SLOTS
        && src.blend_glow.len() <= MAX_SCENE_SLOTS
        && src.blend_double_sided.len() <= MAX_SCENE_SLOTS
        && src.blend_under_double_sided.len() <= MAX_SCENE_SLOTS
        && src.blend_depth_runs.len() <= MAX_SCENE_SLOTS
        && src.water_ground.heights.len() <= crate::water_effects::WATER_GROUND_SIZE.pow(2) as usize
        && src.particles.len() <= 4096
        && src.impostor_sprites.len() <= 1024
}

pub(crate) fn world_lists(src: &DrawLists) -> DrawLists {
    // Do not clone large HUD/selection/reflection/capture lists just to clear
    // them again. Every copied geometry list is covered by the request bounds.
    DrawLists {
        portal_clipping: src.portal_clipping,
        water_exclusion: src.water_exclusion.clone(),
        terrain: src.terrain.clone(),
        terrain_pbr: src.terrain_pbr.clone(),
        terrain_mask: src.terrain_mask.clone(),
        opaque: src.opaque.clone(),
        opaque_two_sided: src.opaque_two_sided.clone(),
        mask: src.mask.clone(),
        mask_two_sided: src.mask_two_sided.clone(),
        water: src.water.clone(),
        water_ground: src.water_ground.clone(),
        blend: src.blend.clone(),
        blend_under_water: src.blend_under_water.clone(),
        blend_depth: src.blend_depth,
        blend_depth_mode: src.blend_depth_mode,
        blend_depth_runs: src.blend_depth_runs.clone(),
        blend_glow: src.blend_glow.clone(),
        blend_double_sided: src.blend_double_sided.clone(),
        blend_under_double_sided: src.blend_under_double_sided.clone(),
        particles: src.particles.clone(),
        impostor_sprites: src.impostor_sprites.clone(),
        gpu: src.gpu,
        ..Default::default()
    }
}

pub(crate) fn validate_params(params: &FrameParams) -> Result<(), Error> {
    if !params.proj.is_finite()
        || !params.view.is_finite()
        || !params.fov_y.is_finite()
        || !(0.01..3.13).contains(&params.fov_y)
        || !params.near.is_finite()
        || params.near <= 0.0
        || !params.camera_pos.is_finite()
        || params.rlva_spheres.len() > 64
        || params.rlva_overlays.len() > 64
        || params.rlva_resolution_divisor == 0
        || params.photo_context != PhotoContext::None
        || params.photo_tile != glam::Mat4::IDENTITY
        || params.cutout
    {
        return Err(Error::SceneNotReady);
    }
    Ok(())
}

pub(crate) fn strip(params: &mut FrameParams, lists: &mut DrawLists, size: Size) -> Result<(), Error> {
    validate_params(params)?;
    let (_, height) = size.dimensions();
    params.proj = projection(params.fov_y, params.near, size)?;
    params.world_viewport = None;
    params.photo_scale = 1.0;
    params.dof = None;
    params.look = None;
    params.bloom = 0.0;
    params.glow = false;
    params.shadows = false;
    params.mirror = None;
    params.probes.enabled = false;
    params.probes.capture = None;
    params.probes.probes.clear();
    params.wireframe = false;
    params.debug_glow = false;
    params.debug = Default::default();
    // Vision effects, including resolution reduction and overlays, remain.
    lists.strip_viewer_overlays();
    lists.impostor_captures.clear();
    lists.shadow_casters.clear();
    lists.terrain_shadow_casters.clear();
    lists.reflection.clear();
    lists.probe.clear();
    lists.reflection_terrain.clear();
    lists.reflection_terrain_pbr.clear();
    lists.glow.clear();
    lists.glow_alpha.clear();
    lists.water_events.clear();
    lists.water_contacts.clear();
    if let Some(gpu) = &mut lists.gpu {
        gpu.planes = crate::gpu_cull::view_planes(params.proj * params.view);
        gpu.pixel_scale = height as f32 / (params.fov_y * 0.5).tan();
        gpu.eye = params.camera_pos;
        gpu.shadows = false;
        gpu.reflections = false;
    }
    Ok(())
}

pub(crate) struct View {
    pub backend: Box<crate::renderer::Backend>,
    output: wgpu::Texture,
    readback: wgpu::Buffer,
    size: Size,
    format: wgpu::TextureFormat,
    pending: Option<(Key, Arc<AtomicU8>)>,
    began: Option<Instant>,
    warm: Arc<std::sync::atomic::AtomicBool>,
}

impl View {
    pub fn new(init: crate::renderer::BackendInit, size: Size) -> Result<Self, Error> {
        let device = init.device.clone();
        let format = init.output_format.ok_or(Error::Unsupported)?;
        if !matches!(
            format,
            wgpu::TextureFormat::Rgba8Unorm
                | wgpu::TextureFormat::Rgba8UnormSrgb
                | wgpu::TextureFormat::Bgra8Unorm
                | wgpu::TextureFormat::Bgra8UnormSrgb
        ) {
            return Err(Error::Unsupported);
        }
        let backend = Box::new(crate::renderer::Backend::new(init).map_err(|_| Error::RendererFailed)?);
        let (output, readback) = Self::targets(&device, format, size);
        Ok(Self {
            backend,
            output,
            readback,
            size,
            format,
            pending: None,
            began: None,
            warm: Default::default(),
        })
    }
    pub fn warm(&self, queue: &wgpu::Queue) {
        let ready = self.warm.clone();
        queue.on_submitted_work_done(move || ready.store(true, Ordering::Release));
    }
    pub fn warm_poll(&self, device: &wgpu::Device, lost: &crate::device_lost::DeviceLost) -> Reply {
        lost.poll(device);
        if lost.get() {
            Reply::InitFailed(Error::RendererFailed)
        } else if self.warm.load(Ordering::Acquire) {
            Reply::Warmed
        } else {
            Reply::Warming
        }
    }
    fn targets(device: &wgpu::Device, format: wgpu::TextureFormat, size: Size) -> (wgpu::Texture, wgpu::Buffer) {
        let (width, height) = size.dimensions();
        let output = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("standby preview output"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("standby preview readback"),
            size: u64::from(row_pitch(width)) * u64::from(height),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        (output, readback)
    }
    pub fn render(
        &mut self,
        key: Key,
        size: Size,
        observed_at: Instant,
        mut packet: FramePacket,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        lost: &crate::device_lost::DeviceLost,
    ) -> Reply {
        if self.pending.is_some() {
            return self.backend.reject_session_preview(key, Error::Busy, packet);
        }
        if self.size != size {
            (self.output, self.readback) = Self::targets(device, self.format, size);
            self.size = size;
        }
        if !fresh(observed_at, Instant::now()) {
            return self.backend.reject_session_preview(key, Error::Stale, packet);
        }
        self.began = Some(Instant::now());
        packet.output = Some(self.output.clone());
        let result = self.backend.render(packet);
        if lost.get() || !result.rendered {
            return Reply::Failed(key, Error::RendererFailed, Some(Box::new(result.spent)));
        }
        let (width, height) = size.dimensions();
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("standby preview copy"),
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.output,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row_pitch(width)),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        crate::device_lost::submit_tracked(lost, queue, key.sequence, [encoder.finish()]);
        let state = Arc::new(AtomicU8::new(0));
        let answer = state.clone();
        self.readback.map_async(wgpu::MapMode::Read, .., move |result| {
            answer.store(if result.is_ok() { 1 } else { 2 }, Ordering::Release);
        });
        self.pending = Some((key, state));
        Reply::Submitted(key, Box::new(result.spent))
    }
    pub fn poll(&mut self, key: Key, device: &wgpu::Device, lost: &crate::device_lost::DeviceLost) -> Reply {
        lost.poll(device);
        if lost.get() {
            return Reply::Failed(key, Error::RendererFailed, None);
        }
        let Some((pending, state)) = &self.pending else {
            return Reply::Failed(key, Error::Stale, None);
        };
        if *pending != key {
            return Reply::Failed(key, Error::Stale, None);
        }
        match state.load(Ordering::Acquire) {
            0 => Reply::Pending(key),
            1 => {
                let data = self
                    .readback
                    .get_mapped_range(..)
                    .ok()
                    .and_then(|mapped| unpack(&mapped, self.size, self.format));
                self.readback.unmap();
                self.pending = None;
                match data {
                    Some(rgba) => Reply::Ready(
                        key,
                        Frame {
                            epoch: key.epoch,
                            sequence: key.sequence,
                            size: self.size,
                            rgba,
                            elapsed: self.began.take().map_or(Duration::MAX, |at| at.elapsed()),
                        },
                    ),
                    None => Reply::Failed(key, Error::Readback, None),
                }
            }
            _ => {
                self.readback.unmap();
                self.pending = None;
                Reply::Failed(key, Error::Readback, None)
            }
        }
    }
}

impl Drop for View {
    fn drop(&mut self) {
        self.readback.unmap();
    }
}
fn row_pitch(width: u32) -> u32 {
    (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT
}
fn unpack(bytes: &[u8], size: Size, format: wgpu::TextureFormat) -> Option<Vec<u8>> {
    let bgra = match format {
        wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb => true,
        wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Rgba8UnormSrgb => false,
        _ => return None,
    };
    let (width, height) = size.dimensions();
    let row = row_pitch(width) as usize;
    if bytes.len() != row * height as usize {
        return None;
    }
    let mut rgba = Vec::with_capacity(size.rgba_bytes());
    for y in 0..height as usize {
        for pixel in bytes[y * row..y * row + width as usize * 4].as_chunks::<4>().0 {
            let pixel = if bgra {
                [pixel[2], pixel[1], pixel[0], 255]
            } else {
                [pixel[0], pixel[1], pixel[2], 255]
            };
            rgba.extend_from_slice(&pixel);
        }
    }
    Some(rgba)
}

#[cfg(test)]
mod tests;
