//! Complete prospective texture-family planner plus mandatory external
//! allocation inventory. It is NOT a complete current renderer allocator:
//! current factories/holders have not been migrated or verified by a recorder.

use super::Error;
use super::descriptor::{Bounded, Buffer, DeviceCaps, Resource, Texture};
use super::inventory::{Class, Entry, Factory, Inventory, MAX_PLAN_RECORDS, Payloads};
use super::layout::{Caps, CoreTargetLayout, FormatSupport, Input};
use wgpu::{TextureDimension, TextureFormat as F, TextureUsages as U};

const ATT: U = U::RENDER_ATTACHMENT;
const SAMPLED: U = ATT.union(U::TEXTURE_BINDING);
const STORAGE: U = U::STORAGE_BINDING.union(U::TEXTURE_BINDING);

#[derive(Clone, Copy)]
pub(super) struct Gtao {
    pub level: u8,
    pub indirect: bool,
}
#[derive(Clone, Copy)]
pub(super) enum Aa {
    None,
    Taa,
    Smaa,
}
#[derive(Clone, Copy)]
pub(super) struct View {
    pub core: Input,
    pub gtao: Option<Gtao>,
    pub blend_depth: bool,
    pub aa: Aa,
    pub hiz: bool,
    pub glow: bool,
    pub bloom: bool,
    pub dof: bool,
    pub privacy_spheres: bool,
    pub reduced_output: Option<[u32; 2]>,
    pub output: Texture,
}

/// The existing capture factory owns a distinct RGBA8 target, row-aligned
/// staging, crop uniform and packed CPU image. Image credits pay all three
/// image payloads; the world buffer domain pays the crop uniform. This is only
/// a descriptor migration aid, not another capture pool or request constructor.
pub(super) struct CaptureLayout {
    pub entries: [Entry; 3],
    pub packed_cpu: u64,
}
impl CaptureLayout {
    pub fn checked(size: [u32; 2], caps: DeviceCaps<'_>) -> Result<Self, Error> {
        if size.contains(&0) || size[0] > 640 || size[1] > 360 {
            return Err(Error::Invalid);
        }
        let row = u64::from(size[0]).checked_mul(4).ok_or(Error::Overflow)?;
        let stride = row.div_ceil(256).checked_mul(256).ok_or(Error::Overflow)?;
        let staging = stride.checked_mul(u64::from(size[1])).ok_or(Error::Overflow)?;
        let packed_cpu = row.checked_mul(u64::from(size[1])).ok_or(Error::Overflow)?;
        let result = Self {
            entries: [
                Entry {
                    factory: Factory::Capture,
                    index: 0,
                    class: Class::ImagePool,
                    resource: Resource::Texture(Texture::d2(size, F::Rgba8Unorm, ATT | U::COPY_SRC)),
                },
                Entry {
                    factory: Factory::Capture,
                    index: 1,
                    class: Class::ImagePool,
                    resource: Resource::Buffer(Buffer {
                        size: staging,
                        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                        mapped: false,
                    }),
                },
                Entry {
                    factory: Factory::Capture,
                    index: 2,
                    class: Class::Buffer,
                    resource: Resource::Buffer(Buffer {
                        size: 32,
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                        mapped: false,
                    }),
                },
            ],
            packed_cpu,
        };
        for entry in result.entries {
            entry.validate(caps)?;
        }
        Ok(result)
    }
    pub fn image_credit(&self, caps: DeviceCaps<'_>) -> Result<u64, Error> {
        self.entries[0]
            .validate(caps)?
            .checked_add(self.entries[1].validate(caps)?)
            .and_then(|gpu| gpu.checked_add(self.packed_cpu))
            .ok_or(Error::Overflow)
    }
}

pub(super) struct WorldViewPlan {
    entries: Bounded<Entry>,
    payloads: Payloads,
    failed: bool,
    components: usize,
    component_limit: usize,
    metadata_storage: u64,
}
impl WorldViewPlan {
    pub fn new(capacity: usize, preparation_bytes: u64) -> Result<Self, Error> {
        let header = u64::try_from(std::mem::size_of::<Self>()).map_err(|_| Error::Overflow)?;
        let entries = Bounded::new(
            capacity,
            MAX_PLAN_RECORDS,
            preparation_bytes.checked_sub(header).ok_or(Error::Capacity)?,
        )?;
        let metadata_storage = entries.storage().checked_add(header).ok_or(Error::Overflow)?;
        Ok(Self {
            entries,
            metadata_storage,
            payloads: Payloads::default(),
            failed: false,
            components: 0,
            component_limit: capacity,
        })
    }
    pub fn entries(&self) -> Result<&[Entry], Error> {
        if self.failed {
            Err(Error::Invalid)
        } else {
            Ok(self.entries.slice())
        }
    }
    pub fn payloads(&self) -> Result<Payloads, Error> {
        self.entries()?;
        Ok(self.payloads)
    }
    pub fn storage(&self) -> u64 {
        self.metadata_storage
    }
    pub fn add(&mut self, entry: Entry, caps: DeviceCaps<'_>) -> Result<(), Error> {
        self.construct(|plan| {
            if plan
                .entries
                .slice()
                .iter()
                .any(|old| old.factory == entry.factory && old.index == entry.index)
            {
                return Err(Error::Invalid);
            }
            let count = entry.components(caps)?.into_iter().flatten().count();
            let components = plan.components.checked_add(count).ok_or(Error::Overflow)?;
            if components > plan.component_limit {
                return Err(Error::Capacity);
            }
            let mut payloads = plan.payloads;
            payloads.add(entry.class, entry.validate(caps)?)?;
            plan.entries.push(entry)?;
            plan.payloads = payloads;
            plan.components = components;
            Ok(())
        })
    }
    fn construct(&mut self, operation: impl FnOnce(&mut Self) -> Result<(), Error>) -> Result<(), Error> {
        if self.failed {
            return Err(Error::Invalid);
        }
        let result = operation(self);
        self.failed |= result.is_err();
        result
    }
    fn texture(&mut self, factory: Factory, index: u16, class: Class, texture: Texture, caps: DeviceCaps<'_>) -> Result<(), Error> {
        self.add(
            Entry {
                factory,
                index,
                class,
                resource: Resource::Texture(texture),
            },
            caps,
        )
    }
    fn view_texture(&mut self, factory: Factory, index: u16, texture: Texture, caps: DeviceCaps<'_>) -> Result<(), Error> {
        self.texture(factory, index, Class::ViewTexture, texture, caps)
    }

    /// Current normal AA/override choices are already resolved. This routine
    /// cannot disable an effect to make a budget fit. It has no settings writer.
    pub fn view(&mut self, view: View, caps: DeviceCaps<'_>) -> Result<(), Error> {
        self.construct(|plan| plan.view_inner(view, caps))
    }
    fn view_inner(&mut self, view: View, caps: DeviceCaps<'_>) -> Result<(), Error> {
        let size = view.core.scene;
        let out = view.output;
        if size.contains(&0)
            || out.size[0] > 640
            || out.size[1] > 360
            || size[0] > out.size[0]
            || size[1] > out.size[1]
            || out.dimension != TextureDimension::D2
            || out.size[2] != 1
            || out.samples != 1
            || out.mips != 1
            || !out.usage.contains(U::RENDER_ATTACHMENT | U::COPY_SRC | U::TEXTURE_BINDING)
            || view.gtao.is_some() != (view.core.ao_enabled && view.core.gtao)
        {
            return Err(Error::Invalid);
        }
        // Preserve the protected core planner and its exact nine tests. Its
        // missing-family result is not consumed as a complete-world receipt.
        let formats = [
            F::R8Unorm,
            F::Rg8Unorm,
            F::R16Float,
            F::Rgba8Unorm,
            F::R32Float,
            F::R32Uint,
            F::Depth32Float,
            F::Rgba16Float,
        ];
        let mut supports = [FormatSupport {
            format: F::R8Unorm,
            features: caps.support(F::R8Unorm)?,
        }; 8];
        for (support, format) in supports.iter_mut().zip(formats) {
            *support = FormatSupport {
                format,
                features: caps.support(format)?,
            };
        }
        let core = CoreTargetLayout::checked(
            view.core,
            &Caps {
                max_dimension_2d: caps.limits.max_texture_dimension_2d,
                max_array_layers: caps.limits.max_texture_array_layers,
                formats: supports,
            },
        )?;
        for (index, spec) in core.specs().enumerate() {
            self.view_texture(
                Factory::Core,
                index as u16,
                Texture {
                    size: spec.size,
                    dimension: TextureDimension::D2,
                    mips: spec.mips,
                    samples: spec.samples,
                    format: spec.format,
                    usage: spec.usage,
                },
                caps,
            )?;
        }
        if view.blend_depth {
            let mut texture = Texture::d2(size, F::Depth32Float, ATT | U::COPY_DST);
            texture.samples = view.core.samples;
            // Actual Layered/Layers texture-to-texture copies preserve the
            // source scene depth's sample count. Pinned wgpu-core permits
            // equal-sample MSAA copies; do not lower samples or omit this cost.
            // Descriptor accounting itself grants no command permission.
            self.view_texture(Factory::BlendDepth, 0, texture, caps)?;
        }
        if let Some(gtao) = view.gtao {
            self.gtao(size, gtao, caps)?;
        }
        if view.core.contact {
            for i in 0..2 {
                self.view_texture(Factory::Contact, i, Texture::d2(size, F::Rgba8Unorm, STORAGE | U::COPY_SRC), caps)?;
            }
            self.view_texture(Factory::Contact, 2, Texture::d2(size, F::Depth32Float, SAMPLED | U::COPY_DST), caps)?;
        }
        match view.aa {
            Aa::None => {}
            Aa::Taa => {
                for i in 0..2 {
                    self.view_texture(Factory::Taa, i, Texture::d2(size, F::Rgba16Float, SAMPLED), caps)?;
                }
                if view.gtao.is_none() {
                    self.view_texture(Factory::Taa, 2, Texture::d2(size, F::Rgba16Float, SAMPLED), caps)?;
                }
            }
            Aa::Smaa => {
                for (index, format) in [F::Rgba8Unorm, F::Rg8Unorm, F::Rgba8Unorm].into_iter().enumerate() {
                    self.view_texture(Factory::Smaa, index as u16, Texture::d2(size, format, SAMPLED), caps)?;
                }
            }
        }
        if view.hiz {
            let half = size.map(|n| n.div_ceil(2));
            let mut texture = Texture::d2(half, F::R32Float, STORAGE);
            texture.mips = 32 - half[0].max(half[1]).leading_zeros();
            self.view_texture(Factory::HiZ, 0, texture, caps)?;
        }
        if view.glow {
            for i in 0..3 {
                self.view_texture(Factory::Glow, i, Texture::d2([512, 512], F::Rgba8Unorm, SAMPLED), caps)?;
            }
        }
        if view.bloom {
            // bloom::levels uses ceil-halving, up to six and min side eight.
            // Keep work bounded without allocating its temporary Vec here.
            let mut level = size.map(|n| n.div_ceil(2));
            for index in 0..6 {
                if index != 0 && level[0].min(level[1]) < 8 {
                    break;
                }
                self.view_texture(Factory::Bloom, index, Texture::d2(level, F::Rgba16Float, SAMPLED), caps)?;
                level = level.map(|n| n.div_ceil(2));
            }
        }
        if view.dof {
            self.view_texture(Factory::Dof, 0, Texture::d2(size, F::Rgba16Float, SAMPLED), caps)?;
            let half = size.map(|n| n.div_ceil(2));
            for i in 1..4 {
                self.view_texture(Factory::Dof, i, Texture::d2(half, F::Rgba16Float, SAMPLED), caps)?;
            }
            for i in 4..6 {
                self.view_texture(
                    Factory::Dof,
                    i,
                    Texture::d2(half.map(|n| n.div_ceil(16)), F::R16Float, SAMPLED),
                    caps,
                )?;
            }
        }
        if view.privacy_spheres {
            for i in 0..2 {
                self.view_texture(
                    Factory::Privacy,
                    i,
                    Texture::d2([out.size[0], out.size[1]], out.format, SAMPLED),
                    caps,
                )?;
            }
        }
        if let Some(reduced) = view.reduced_output {
            if reduced.contains(&0) || reduced[0] > out.size[0] || reduced[1] > out.size[1] {
                return Err(Error::Invalid);
            }
            self.view_texture(Factory::Privacy, 2, Texture::d2(reduced, out.format, SAMPLED | U::COPY_DST), caps)?;
        }
        self.view_texture(Factory::Output, 0, out, caps)
    }

    fn gtao(&mut self, full: [u32; 2], options: Gtao, caps: DeviceCaps<'_>) -> Result<(), Error> {
        let quality = crate::gtao::quality_for(options.level, full[0], full[1]);
        let (w, h) = crate::gtao::target_size(full[0], full[1], quality);
        let size = [w, h];
        let mut depth = Texture::d2(size, F::R32Float, STORAGE | U::COPY_SRC);
        depth.mips = crate::gtao::mip_count(w, h);
        self.view_texture(Factory::Gtao, 0, depth, caps)?;
        for i in 1..4 {
            self.view_texture(Factory::Gtao, i, Texture::d2(size, F::Rgba16Float, STORAGE), caps)?;
        }
        for i in 4..6 {
            self.view_texture(Factory::Gtao, i, Texture::d2(size, F::R32Uint, STORAGE), caps)?;
        }
        self.view_texture(Factory::Gtao, 6, Texture::d2(size, F::Rgba8Unorm, STORAGE), caps)?;
        self.view_texture(Factory::Gtao, 7, Texture::d2(full, F::Rgba8Unorm, STORAGE), caps)?;
        self.view_texture(Factory::Gtao, 8, Texture::d2(full, F::Rgba16Float, SAMPLED), caps)?;
        if quality.half_res {
            self.view_texture(Factory::Gtao, 9, Texture::d2(size, F::Rgba8Unorm, STORAGE), caps)?;
        }
        if options.indirect {
            for i in 10..12 {
                self.view_texture(Factory::Gtao, i, Texture::d2(size, F::Rgba16Float, STORAGE), caps)?;
            }
            self.view_texture(Factory::Gtao, 12, Texture::d2(full, F::Rgba16Float, STORAGE), caps)?;
            if quality.half_res {
                self.view_texture(Factory::Gtao, 13, Texture::d2(size, F::Rgba16Float, STORAGE), caps)?;
            }
        }
        Ok(())
    }

    /// Exact resolved cascade size, never shrunk to output dimensions. The
    /// normal renderer's clamp/quality decisions happen before this boundary.
    pub fn shadows(&mut self, cascade: u32, projectors: u8, caps: DeviceCaps<'_>) -> Result<(), Error> {
        self.construct(|plan| plan.shadows_inner(cascade, projectors, caps))
    }
    fn shadows_inner(&mut self, cascade: u32, projectors: u8, caps: DeviceCaps<'_>) -> Result<(), Error> {
        if !(512..=8192).contains(&cascade) || projectors > 16 {
            return Err(Error::Invalid);
        }
        let width = cascade.checked_mul(2).ok_or(Error::Overflow)?;
        let rows = u32::from(projectors.saturating_sub(4)).div_ceil(4);
        let height = rows
            .checked_mul(cascade / 2)
            .and_then(|extra| width.checked_add(extra))
            .ok_or(Error::Overflow)?;
        self.texture(
            Factory::Shadows,
            0,
            Class::SharedTexture,
            Texture::d2([width, height], F::Depth32Float, SAMPLED),
            caps,
        )?;
        self.texture(
            Factory::ShadowMerge,
            0,
            Class::SharedTexture,
            Texture::d2([cascade / 2; 2], F::Depth32Float, SAMPLED),
            caps,
        )
    }
    pub fn probes(&mut self, slots: u32, copy_source: bool, caps: DeviceCaps<'_>) -> Result<(), Error> {
        self.construct(|plan| plan.probes_inner(slots, copy_source, caps))
    }
    fn probes_inner(&mut self, slots: u32, copy_source: bool, caps: DeviceCaps<'_>) -> Result<(), Error> {
        if !(1..=64).contains(&slots) {
            return Err(Error::Invalid);
        }
        let filtered = if copy_source { STORAGE | U::COPY_SRC } else { STORAGE };
        for (index, size, layers, mips, usage) in [
            (0, 128, slots * 6, 7, filtered),
            (1, 16, slots * 6, 1, filtered),
            (2, 256, 6, 8, STORAGE | ATT),
            (3, 128, 6, 7, STORAGE | ATT),
        ] {
            let mut texture = Texture::d2([size; 2], F::Rgba16Float, usage);
            texture.size[2] = layers;
            texture.mips = mips;
            self.texture(Factory::Probes, index, Class::SharedTexture, texture, caps)?;
        }
        for (index, size) in [(4, 256), (5, 128)] {
            self.texture(
                Factory::Probes,
                index,
                Class::SharedTexture,
                Texture::d2([size; 2], F::Depth32Float, ATT),
                caps,
            )?;
        }
        Ok(())
    }
    /// The old atlas is supplied in Inventory as CopySource/Retiring by exact
    /// allocation ID. Planning a new page count never credits the old one back.
    pub fn impostors(&mut self, pages: u32, caps: DeviceCaps<'_>) -> Result<(), Error> {
        self.construct(|plan| plan.impostors_inner(pages, caps))
    }
    fn impostors_inner(&mut self, pages: u32, caps: DeviceCaps<'_>) -> Result<(), Error> {
        if !matches!(pages, 1 | 2 | 4) {
            return Err(Error::Invalid);
        }
        let mut atlas = Texture::d2([2048; 2], F::Rgba16Float, U::TEXTURE_BINDING | U::COPY_DST | U::COPY_SRC);
        atlas.size[2] = pages;
        self.texture(Factory::Impostors, 0, Class::SharedTexture, atlas, caps)?;
        self.texture(
            Factory::Impostors,
            1,
            Class::SharedTexture,
            Texture::d2([256; 2], F::Rgba16Float, ATT | U::COPY_SRC),
            caps,
        )?;
        self.texture(
            Factory::Impostors,
            2,
            Class::SharedTexture,
            Texture::d2([256; 2], F::Depth32Float, ATT),
            caps,
        )
    }
    pub fn water(&mut self, size: u32, caps: DeviceCaps<'_>) -> Result<(), Error> {
        self.construct(|plan| plan.water_inner(size, caps))
    }
    fn water_inner(&mut self, size: u32, caps: DeviceCaps<'_>) -> Result<(), Error> {
        if size == 0 {
            return Err(Error::Invalid);
        }
        let double = size.checked_mul(2).ok_or(Error::Overflow)?;
        let usage = STORAGE | U::COPY_SRC | U::COPY_DST | ATT;
        self.texture(
            Factory::Water,
            0,
            Class::SharedTexture,
            Texture::d2([size, double], F::Rgba16Float, usage),
            caps,
        )?;
        self.texture(
            Factory::Water,
            1,
            Class::SharedTexture,
            Texture::d2([size; 2], F::Rgba16Float, usage),
            caps,
        )?;
        self.texture(
            Factory::Water,
            2,
            Class::SharedTexture,
            Texture::d2([64; 2], F::Rg32Float, U::TEXTURE_BINDING | U::COPY_DST),
            caps,
        )?;
        self.texture(
            Factory::Water,
            3,
            Class::SharedTexture,
            Texture::d2([256; 2], F::Rgba16Float, SAMPLED),
            caps,
        )
    }
    pub fn immutable(&mut self, caps: DeviceCaps<'_>) -> Result<(), Error> {
        self.construct(|plan| plan.immutable_inner(caps))
    }
    fn immutable_inner(&mut self, caps: DeviceCaps<'_>) -> Result<(), Error> {
        // Actual formats/usages for the helpers. Additional scene/fallback
        // resources still require exact inventory records; no free dummy path.
        for (factory, index, size, format, usage) in [
            (Factory::Smaa, 10, [160, 560], F::Rg8Unorm, U::TEXTURE_BINDING | U::COPY_DST),
            (Factory::Smaa, 11, [64, 16], F::R8Unorm, U::TEXTURE_BINDING | U::COPY_DST),
            (Factory::Bloom, 10, [1, 1], F::Rgba16Float, U::TEXTURE_BINDING),
            (Factory::Gtao, 20, [1, 1], F::R32Float, U::TEXTURE_BINDING),
            (Factory::Immutable, 0, [1, 1], F::Rgba8Unorm, U::TEXTURE_BINDING | U::COPY_DST),
            (Factory::Immutable, 1, [1, 1], F::Rgba16Float, U::TEXTURE_BINDING | U::COPY_DST),
            (Factory::Immutable, 2, [1, 1], F::Depth32Float, SAMPLED),
            (Factory::Immutable, 3, [1, 1], F::Rgba16Float, U::TEXTURE_BINDING),
        ] {
            self.texture(factory, index, Class::SharedTexture, Texture::d2(size, format, usage), caps)?;
        }
        Ok(())
    }
    /// Retained photo-only looks/LUTs can be described for inventory migration;
    /// a future Monitor encoder must not invoke their loading/prepare route.
    pub fn retained_looks(&mut self, size: [u32; 2], format: F, lut_sizes: &[u32], caps: DeviceCaps<'_>) -> Result<(), Error> {
        self.construct(|plan| plan.retained_looks_inner(size, format, lut_sizes, caps))
    }
    fn retained_looks_inner(&mut self, size: [u32; 2], format: F, lut_sizes: &[u32], caps: DeviceCaps<'_>) -> Result<(), Error> {
        if lut_sizes.len() > 4 {
            return Err(Error::Capacity);
        }
        let usage = SAMPLED | U::COPY_DST;
        for i in 0..5 {
            self.texture(
                Factory::Looks,
                i,
                Class::SharedTexture,
                Texture::d2(size, if i == 0 { format } else { F::Rgba8Unorm }, usage),
                caps,
            )?;
        }
        for (index, n) in lut_sizes.iter().copied().enumerate() {
            if !(2..=128).contains(&n) {
                return Err(Error::Invalid);
            }
            self.texture(
                Factory::Looks,
                10 + index as u16,
                Class::SharedTexture,
                Texture {
                    size: [n; 3],
                    dimension: TextureDimension::D3,
                    mips: 1,
                    samples: 1,
                    format: F::Rgba8Unorm,
                    usage: U::TEXTURE_BINDING | U::COPY_DST,
                },
                caps,
            )?;
        }
        self.texture(
            Factory::Looks,
            20,
            Class::SharedTexture,
            Texture::d2([1, 1], F::Depth32Float, SAMPLED),
            caps,
        )?;
        self.texture(
            Factory::Looks,
            21,
            Class::SharedTexture,
            Texture {
                dimension: TextureDimension::D3,
                ..Texture::d2([1, 1], F::Rgba8Unorm, U::TEXTURE_BINDING | U::COPY_DST)
            },
            caps,
        )?;
        Ok(())
    }
    /// Borrowed existing resources are billed once by physical identity. New
    /// construction plus all old/page-copy/retiring allocations are additive.
    /// This method grants no renderer permission or numeric capability.
    pub fn prospective_total(&self, inventory: &Inventory, preparation_limit: u64) -> Result<Payloads, Error> {
        self.entries()?;
        if self.storage().checked_add(inventory.storage()).ok_or(Error::Overflow)? > preparation_limit {
            return Err(Error::Capacity);
        }
        let mut total = inventory.payloads()?;
        for (index, bytes) in self.payloads.0.into_iter().enumerate() {
            total.0[index] = total.0[index].checked_add(bytes).ok_or(Error::Overflow)?;
        }
        Ok(total)
    }
}
