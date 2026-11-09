//! Small inactive-world renderer. Scheduling here is a local backstop, NOT account/privacy authority
//! or aggregate admission; callers must supply those before scene preparation.
//! No surface, photo readback, transport or input proof is exposed by this API.

use crate::viewport_capture::CaptureCap;
use parking_lot::Mutex;
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

/// Exactly the four intended Monitor cadences. Missed samples are dropped.
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    pub size: CaptureCap,
    pub rate: Rate,
}
impl Config {
    pub fn dimensions(self) -> (u32, u32) {
        self.size.dimensions()
    }
}

/// Apply the same reduced feature policy before cold allocation and on every
/// packet. Pixel/privacy effects remain on FrameParams; presentation settings
/// and the worker's saved graphics preferences are never changed.
pub(crate) fn reduced_settings(mut settings: crate::RenderSettings) -> crate::RenderSettings {
    settings.aa = crate::AntiAliasing::None;
    settings.shadow_resolution = 0;
    settings.projector_shadow_limit = 0;
    settings.probe_slots = 1;
    settings.ssao = 0;
    settings.screen_indirect = false;
    settings.contact_shadows = false;
    settings.water_reflection_scale = 0.0;
    settings.mirror_scale = 0.0;
    settings.water_effects.enabled = false;
    settings.ssr = false;
    settings.occlusion = false;
    settings.render_scale_percent = 100;
    settings
}

/// One cold small-backend allocation per process, shared across renderer/device
/// replacements. View plus actual submission callbacks/borrowed Frames retain
/// the lease. Logical cancellation or constructing another Renderer cannot
/// renew it. Host aggregate credit is independent and remains charged until ACK.
#[derive(Clone, Default)]
pub(crate) struct ResourcePool(Arc<AtomicBool>);
impl ResourcePool {
    fn acquire(&self) -> Option<ResourceLease> {
        self.0.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).ok()?;
        Some(ResourceLease {
            _credit: Arc::new(ResourceCredit(self.clone())),
        })
    }
    fn idle(&self) -> bool {
        !self.0.load(Ordering::Acquire)
    }
}
struct ResourceCredit(ResourcePool);
impl Drop for ResourceCredit {
    fn drop(&mut self) {
        self.0.0.store(false, Ordering::Release);
    }
}
#[derive(Clone)]
pub(crate) struct ResourceLease {
    _credit: Arc<ResourceCredit>,
}
fn process_resources() -> &'static ResourcePool {
    static POOL: OnceLock<ResourcePool> = OnceLock::new();
    POOL.get_or_init(ResourcePool::default)
}
pub(crate) fn resources_idle() -> bool {
    process_resources().idle()
}
pub(crate) fn acquire_resources() -> Option<ResourceLease> {
    process_resources().acquire()
}

/// One configuration lifetime, revoked before promotion/hide/resize/close.
/// Revocation is permanent, including for results already borrowed by a reader.
#[derive(Clone)]
pub(crate) struct Epoch(Arc<AtomicBool>);
impl Epoch {
    fn new() -> Self {
        Self(Arc::new(AtomicBool::new(true)))
    }
    pub(crate) fn current(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
    fn revoke(&self) {
        self.0.store(false, Ordering::Release);
    }
}

/// Test-only facts captured immediately after cold allocation, before the
/// first packet can change settings and hide an oversized transient allocation.
#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct ColdResources {
    pub samples: u32,
    pub probe_slots: u32,
    pub shadow_size: u32,
    pub shadow_count: u8,
}

/// Completed GPU image on this renderer's device, never a presentation proof.
/// Keep this lease until the consumer's GPU use has completed. Holding both
/// output leases makes the producer skip samples instead of allocating more.
/// The App uses the separate credited CaptureRequest service for IPC; this GPU
/// lease itself is never exported as a native/input proof.
pub struct Frame {
    pub(crate) texture: Arc<wgpu::Texture>,
    pub(crate) epoch: Epoch,
    pub(crate) lost: Arc<AtomicBool>,
    pub(crate) index: u64,
    pub(crate) stats: crate::RenderStats,
    #[cfg(test)]
    pub(crate) cold_resources: Option<ColdResources>,
    // Drop GPU handles before returning the process-wide allocation credit.
    pub(crate) _resources: Option<ResourceLease>,
}
impl Frame {
    pub fn texture(&self) -> Option<&wgpu::Texture> {
        (self.epoch.current() && !self.lost.load(Ordering::Acquire)).then_some(&self.texture)
    }
    pub fn index(&self) -> u64 {
        self.index
    }
    /// CPU counters of this sample. As in the normal renderer, GPU timings
    /// are previously harvested queries; they are not fresh admission evidence.
    pub fn stats(&self) -> crate::RenderStats {
        self.stats
    }
}

/// Callback result is separate from the render-thread result: encoding/submission
/// alone cannot make an image available. Cancellation never pretends to drain GPU work.
pub(crate) struct Completion<T> {
    epoch: Epoch,
    result: Mutex<Option<T>>,
    done: AtomicBool,
}
impl<T> Completion<T> {
    fn new(epoch: Epoch) -> Self {
        Self {
            epoch,
            result: Mutex::new(None),
            done: AtomicBool::new(false),
        }
    }
    pub(crate) fn finish(&self, result: Option<T>) {
        *self.result.lock() = result;
        self.done.store(true, Ordering::Release);
    }
}
#[derive(Clone)]
pub(crate) struct Request {
    pub dimensions: (u32, u32),
    pub epoch: Epoch,
    pub completion: Arc<Completion<Frame>>,
}

pub(crate) struct Lane<T> {
    config: Option<Config>,
    epoch: Epoch,
    pending: Option<Arc<Completion<T>>>,
    latest: Option<T>,
    next: Option<Instant>,
}
impl<T> Default for Lane<T> {
    fn default() -> Self {
        Self {
            config: None,
            epoch: Epoch::new(),
            pending: None,
            latest: None,
            next: None,
        }
    }
}
impl<T> Lane<T> {
    pub fn configure(&mut self, config: Option<Config>) {
        if self.config == config {
            return;
        }
        self.epoch.revoke();
        self.epoch = Epoch::new();
        self.config = config;
        self.latest = None;
        // Do not erase the pending GPU reservation or accelerate a rate change.
    }
    pub fn retire(&mut self) -> bool {
        self.epoch.revoke();
        let configured = self.config.take().is_some();
        self.latest = None;
        configured
    }
    pub fn poll(&mut self) {
        if self.pending.as_ref().is_some_and(|p| p.done.load(Ordering::Acquire)) {
            let pending = self.pending.take().unwrap();
            if let Some(frame) = pending.result.lock().take().filter(|_| pending.epoch.current()) {
                self.latest = Some(frame);
            }
        }
    }
    pub fn due(&self, now: Instant) -> Option<Config> {
        (self.pending.is_none() && self.next.is_none_or(|next| now >= next))
            .then_some(self.config)
            .flatten()
    }
    pub fn begin(&mut self, now: Instant) -> Option<(Config, Epoch, Arc<Completion<T>>)> {
        let config = self.due(now)?;
        self.next = now.checked_add(config.rate.interval());
        if self.next.is_none() {
            self.retire();
            return None;
        }
        let completion = Arc::new(Completion::new(self.epoch.clone()));
        self.pending = Some(completion.clone());
        Some((config, self.epoch.clone(), completion))
    }
    pub fn take_latest(&mut self) -> Option<T> {
        self.latest.take()
    }
    pub fn pending(&self) -> bool {
        self.pending.is_some()
    }
}
impl<T> Drop for Lane<T> {
    fn drop(&mut self) {
        self.epoch.revoke();
    }
}

/// Two persistent outputs, no queue of images. Resize waits for every old
/// lease; rapid size changes cannot accumulate target generations.
pub(crate) struct Targets {
    size: (u32, u32),
    slots: [Arc<wgpu::Texture>; 2],
}
impl Targets {
    pub fn new(device: &wgpu::Device, size: (u32, u32), format: wgpu::TextureFormat) -> Self {
        Self {
            size,
            slots: std::array::from_fn(|_| {
                Arc::new(device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("Monitor small world output"),
                    size: wgpu::Extent3d {
                        width: size.0,
                        height: size.1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                }))
            }),
        }
    }
    pub fn size(&self) -> (u32, u32) {
        self.size
    }
    pub fn idle(&self) -> bool {
        self.slots.iter().all(|slot| Arc::strong_count(slot) == 1)
    }
    pub fn acquire(&self) -> Option<Arc<wgpu::Texture>> {
        self.slots.iter().find(|slot| Arc::strong_count(slot) == 1).cloned()
    }
}

#[cfg(test)]
mod tests;
