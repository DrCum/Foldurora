//! Closed prospective factory catalogue. These are source audit obligations,
//! NOT evidence that current wgpu constructors report to this inventory.
//! An empty family needs an explicit observation; absence is never zero cost.

use super::Error;
use super::descriptor::{Bounded, DeviceCaps, Resource};
use super::ownership::AllocationId;

pub(super) const MAX_PLAN_RECORDS: usize = 128;
pub(super) const MAX_DEVICE_RECORDS: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub(super) enum Factory {
    Core,
    BlendDepth,
    Gtao,
    Contact,
    Taa,
    Smaa,
    HiZ,
    Glow,
    Bloom,
    Dof,
    Looks,
    Privacy,
    Output,
    Capture,
    Shadows,
    ShadowMerge,
    Probes,
    Impostors,
    Water,
    TexturePages,
    Immutable,
    Geometry,
    Records,
    Palettes,
    Culling,
    Particles,
    Upload,
    Uniforms,
    Readbacks,
    Exposure,
    Histogram,
    Excluded,
}

/// Each entry names the actual factories whose migration must be reviewed.
/// Dynamic/growth/old-copy holders need records as well as current handles.
pub(super) const FACTORY_SITES: [(Factory, &str); 32] = [
    (Factory::Core, "renderer::Backend::make_targets"),
    (Factory::BlendDepth, "renderer::Backend::render_view rigged blend_layers lazy depth"),
    (Factory::Gtao, "gtao::Gtao::{new,targets,record_context} (context photo-only)"),
    (
        Factory::Contact,
        "contact_shadows::ContactShadows::{new,targets} own_depth and copy groups",
    ),
    (Factory::Taa, "renderer::Backend::make_taa (normals alias by allocation identity)"),
    (Factory::Smaa, "renderer::Smaa::{new,make_targets} lookup uploads"),
    (Factory::HiZ, "occlusion::Occlusion::ensure_hiz and mip views"),
    (Factory::Glow, "renderer::Glow::new fixed 512 targets"),
    (Factory::Bloom, "bloom::Bloom::{new,ensure} black and old level groups"),
    (Factory::Dof, "dof::Dof::{new,ensure_targets} old post bind group"),
    (
        Factory::Looks,
        "looks::Looks::{new,prepare,table} photo-only; retained LUT/depth/input",
    ),
    (
        Factory::Privacy,
        "rlva::Spheres::{new,prepare}; rlva::resolution::Scaled::new; overlay buffers",
    ),
    (Factory::Output, "future no-present output factory (does not exist yet)"),
    (
        Factory::Capture,
        "viewport_capture_gpu::{GpuCapture::encode,PairSlot} target/staging/pack",
    ),
    (
        Factory::Shadows,
        "renderer::Backend::make_shadow_atlas; projector_shadows::atlas_size",
    ),
    (Factory::ShadowMerge, "shadow_merge::ShadowMerge::{new,make_scratch,make_groups}"),
    (Factory::Probes, "probes::Probes::new arrays/scratch/depth/filter groups"),
    (
        Factory::Impostors,
        "renderer::Impostors::{new,make_atlas,prepare} old atlas copy/sprites",
    ),
    (Factory::Water, "water_effects::WaterGpu::new fields/bathymetry/caustics/events"),
    (
        Factory::TexturePages,
        "textures::{make_page,TextureTable} resident/spare/compaction/page worker",
    ),
    (
        Factory::Immutable,
        "renderer dummy white/black/depth; motion_debug empty; helper fallbacks",
    ),
    (
        Factory::Geometry,
        "arena::GeometryArena::{make,try_make} growth; packet::SceneGpu geometry",
    ),
    (
        Factory::Records,
        "arena::RecordStore::{new,make,grown_buffers}; gpu_cull::Table; textures locations",
    ),
    (
        Factory::Palettes,
        "main_thread::{new,make_skin_bind_buffer,grow_table,make_palette_buffer,set_palettes}",
    ),
    (
        Factory::Culling,
        "gpu_cull::{new,prepare,make_storage,make_args}; occlusion buffers/visibility/indirect",
    ),
    (Factory::Particles, "renderer particle_buffer; scene particle instance growth"),
    (
        Factory::Upload,
        "upload::UploadQueue chunks; write_staging; queued writes/copy encoders",
    ),
    (
        Factory::Uniforms,
        "all renderer/helper uniform buffers, stars, probe lists, sampler owners",
    ),
    (Factory::Readbacks, "renderer timers/depth/counters; submitted/map callbacks"),
    (
        Factory::Exposure,
        "exposure::Exposure::new params/state/readback; per-view temporal owner",
    ),
    (
        Factory::Histogram,
        "histogram::Histogram::new diagnostics params/bins/readback; retained baseline",
    ),
    (
        Factory::Excluded,
        "ui_frame/ui_blur/ui_glass/selection/drawn_pick/pick/label_depth/preview/HUD/photo/XR/device_lost::demo_hang",
    ),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub(super) enum Class {
    ViewTexture,
    SharedTexture,
    Buffer,
    // Image credits remain separate from world-target credits. No free alias.
    ImagePool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Payloads(pub [u64; 4]);
impl Payloads {
    pub fn add(&mut self, class: Class, bytes: u64) -> Result<(), Error> {
        self.0[class as usize] = self.0[class as usize].checked_add(bytes).ok_or(Error::Overflow)?;
        Ok(())
    }
    pub fn within(self, limits: Self) -> bool {
        self.0.into_iter().zip(limits.0).all(|(used, limit)| used <= limit)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Entry {
    pub factory: Factory,
    pub index: u16,
    pub class: Class,
    pub resource: Resource,
}
impl Entry {
    pub fn validate(self, caps: DeviceCaps<'_>) -> Result<u64, Error> {
        // ImagePool owns both the capture texture and staging buffer.
        if self.class != Class::ImagePool && matches!(self.resource, Resource::Buffer(_)) != (self.class == Class::Buffer) {
            return Err(Error::Invalid);
        }
        Ok(self.resource.payload(caps)?.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Holding {
    Resident,
    Spare,
    Retiring,
    CopySource,
    // Retained UI/photo/XR allocations still consume physical memory although
    // a future Monitor packet is forbidden to invoke those prepare factories.
    Baseline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Record {
    id: AllocationId,
    entry: Entry,
    holding: Holding,
}
impl Record {
    /// Descriptors and identity come from the same opaque allocation owner;
    /// copied metadata cannot associate a cheaper descriptor with another ID.
    pub fn held<T>(lease: &super::ownership::Lease<T>, holding: Holding) -> Self {
        Self {
            id: lease.id(),
            entry: lease.entry(),
            holding,
        }
    }
    pub fn holding(self) -> Holding {
        self.holding
    }
}

pub(super) struct Inventory {
    records: Bounded<Record>,
    observed: [bool; FACTORY_SITES.len()],
    payloads: Payloads,
    failed: bool,
}
impl Inventory {
    pub fn new(capacity: usize, preparation_bytes: u64) -> Result<Self, Error> {
        Ok(Self {
            records: Bounded::new(capacity, MAX_DEVICE_RECORDS, preparation_bytes)?,
            observed: [false; FACTORY_SITES.len()],
            payloads: Payloads::default(),
            failed: false,
        })
    }
    /// Exact owner observation, including an actually empty family. Merely
    /// setting this bit is not proof of actual factory coverage in production.
    pub fn observe(&mut self, factory: Factory) {
        self.observed[factory as usize] = true;
    }
    pub fn record(&mut self, record: Record, caps: DeviceCaps<'_>) -> Result<(), Error> {
        if self.failed {
            return Err(Error::Invalid);
        }
        // Ignoring a failed observation cannot later yield a closed inventory.
        self.failed = true;
        if self.records.slice().iter().any(|old| old.id == record.id) {
            return Err(Error::Invalid);
        }
        let bytes = record.entry.validate(caps)?;
        let mut payloads = self.payloads;
        payloads.add(record.entry.class, bytes)?;
        self.records.push(record)?;
        self.payloads = payloads;
        self.observe(record.entry.factory);
        self.failed = false;
        Ok(())
    }
    pub fn checked_records(&self) -> Result<&[Record], Error> {
        if self.failed || self.observed.contains(&false) {
            return Err(Error::MissingDependencies);
        }
        Ok(self.records.slice())
    }
    pub fn payloads(&self) -> Result<Payloads, Error> {
        self.checked_records()?;
        Ok(self.payloads)
    }
    pub fn storage(&self) -> u64 {
        self.records.storage()
    }
}
