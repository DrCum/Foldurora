//! Experimental inactive-world renderer. Not connected to the host or enabled
//! by default. Scheduling here is a local backstop, NOT account/privacy authority
//! or aggregate admission; callers must supply those before scene preparation.
//! No surface, photo readback, transport or input proof is exposed by this API.

use crate::viewport_capture::CaptureCap;
use parking_lot::Mutex;
use std::sync::{
    Arc,
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

/// Completed GPU image on this renderer's device, never a presentation proof.
/// Keep this lease until the consumer's GPU use has completed. Holding both
/// output leases makes the producer skip samples instead of allocating more.
/// Cross-process export/readback is deliberately absent from this prototype.
pub struct Frame {
    pub(crate) texture: Arc<wgpu::Texture>,
    pub(crate) epoch: Epoch,
    pub(crate) lost: Arc<AtomicBool>,
    pub(crate) index: u64,
    pub(crate) stats: crate::RenderStats,
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
    pub config: Config,
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
