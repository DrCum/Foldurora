//! Prospective descriptors, not a device allocator. All inputs are copied
//! effective factory decisions. No default capabilities or budget are supplied.

use super::{Error, PayloadCost};
use wgpu::{BufferUsages, TextureDimension, TextureFormat as Format, TextureUsages as Usage};

// Pinned wgpu-hal 30.0.1::MAX_MIP_LEVELS, also applied by wgpu-core's
// create_texture_inner. Dimensional/caller-copied limits cannot raise it.
const MAX_MIP_LEVELS: u32 = 16;

#[derive(Clone, Copy)]
pub(super) struct DeviceCaps<'a> {
    pub limits: &'a wgpu::Limits,
    pub formats: &'a [super::layout::FormatSupport],
    pub mappable_primary_buffers: bool,
}

impl DeviceCaps<'_> {
    pub fn support(self, format: Format) -> Result<wgpu::TextureFormatFeatures, Error> {
        if self.formats.len() > 64 {
            return Err(Error::Capacity);
        }
        let mut found = self.formats.iter().filter(|item| item.format == format);
        let result = found.next().ok_or(Error::Format)?.features;
        if found.next().is_some() {
            return Err(Error::Format);
        }
        Ok(result)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Texture {
    pub size: [u32; 3],
    pub dimension: TextureDimension,
    pub mips: u32,
    pub samples: u32,
    pub format: Format,
    pub usage: Usage,
}

impl Texture {
    pub fn d2(size: [u32; 2], format: Format, usage: Usage) -> Self {
        Self {
            size: [size[0], size[1], 1],
            dimension: TextureDimension::D2,
            mips: 1,
            samples: 1,
            format,
            usage,
        }
    }

    /// Requested blocks only. Unknown/implementation-defined texel sizes fail
    /// closed. D2 array layers stay constant; D3 depth shrinks at each mip.
    pub fn payload(self, caps: DeviceCaps<'_>) -> Result<PayloadCost, Error> {
        let (block, bytes) = match self.format {
            Format::R8Unorm => (1, 1u64),
            Format::Rg8Unorm | Format::R16Float => (1, 2),
            Format::Rgba8Unorm
            | Format::Rgba8UnormSrgb
            | Format::Bgra8Unorm
            | Format::Bgra8UnormSrgb
            | Format::R32Float
            | Format::R32Uint
            | Format::Depth32Float => (1, 4),
            Format::Rgba16Float | Format::Rg32Float => (1, 8),
            Format::Bc7RgbaUnorm | Format::Bc7RgbaUnormSrgb => (4, 16),
            _ => return Err(Error::Format),
        };
        let [w, h, z] = self.size;
        if self.size.contains(&0) || self.mips == 0 || self.usage.is_empty() || !matches!(self.samples, 1 | 2 | 4 | 8 | 16) {
            return Err(Error::Invalid);
        }
        let max_edge = match self.dimension {
            TextureDimension::D1 => {
                if h != 1 || z != 1 || w > caps.limits.max_texture_dimension_1d {
                    return Err(Error::Limits);
                }
                w
            }
            TextureDimension::D2 => {
                if w.max(h) > caps.limits.max_texture_dimension_2d || z > caps.limits.max_texture_array_layers {
                    return Err(Error::Limits);
                }
                w.max(h)
            }
            TextureDimension::D3 => {
                if w.max(h).max(z) > caps.limits.max_texture_dimension_3d {
                    return Err(Error::Limits);
                }
                w.max(h).max(z)
            }
        };
        if self.mips > (32 - max_edge.leading_zeros()).min(MAX_MIP_LEVELS)
            || self.dimension == TextureDimension::D1 && self.mips != 1
            || self.dimension == TextureDimension::D1 && self.usage.contains(Usage::RENDER_ATTACHMENT)
            || self.dimension != TextureDimension::D2 && (self.samples > 1 || block > 1 || self.format == Format::Depth32Float)
            || block > 1 && (!w.is_multiple_of(block) || !h.is_multiple_of(block))
            // Pinned wgpu-core 30.0.1::create_texture_inner. Descriptors here
            // have no additional view formats; those need a separate review.
            || self.usage.contains(Usage::TRANSIENT_ATTACHMENT)
                && (self.dimension != TextureDimension::D2
                    || self.usage != (Usage::RENDER_ATTACHMENT | Usage::TRANSIENT_ATTACHMENT)
                    || self.mips != 1
                    || z != 1)
            || self.samples > 1
                && (self.mips != 1
                    || z != 1
                    || !self.usage.contains(Usage::RENDER_ATTACHMENT)
                    || self.usage.contains(Usage::STORAGE_BINDING))
        {
            return Err(Error::Invalid);
        }
        let support = caps.support(self.format)?;
        if !support.allowed_usages.contains(self.usage) || !support.flags.sample_count_supported(self.samples) {
            return Err(Error::Limits);
        }
        let mut blocks = 0u64;
        for mip in 0..self.mips {
            let depth = if self.dimension == TextureDimension::D3 {
                (z >> mip).max(1)
            } else {
                z
            };
            let level = u64::from((w >> mip).max(1).div_ceil(block))
                .checked_mul(u64::from((h >> mip).max(1).div_ceil(block)))
                .and_then(|area| area.checked_mul(u64::from(depth)))
                .ok_or(Error::Overflow)?;
            blocks = blocks.checked_add(level).ok_or(Error::Overflow)?;
        }
        blocks
            .checked_mul(bytes)
            .and_then(|value| value.checked_mul(u64::from(self.samples)))
            .map(PayloadCost)
            .ok_or(Error::Overflow)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Buffer {
    pub size: u64,
    pub usage: BufferUsages,
    pub mapped: bool,
}
/// Known core allocations for one effective create_buffer descriptor. Init
/// staging uses a separate HAL MAP_WRITE|COPY_SRC / TRANSIENT allocation of
/// exactly primary bytes, not another public mapped-buffer constructor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct BufferCreationPlan {
    source: Buffer,
    primary: u64,
    init_staging: Option<u64>,
}
impl BufferCreationPlan {
    pub fn source(self) -> Buffer {
        self.source
    }
    pub fn primary(self) -> u64 {
        self.primary
    }
    pub fn init_staging(self) -> Option<u64> {
        self.init_staging
    }
    pub fn total(self) -> Result<PayloadCost, Error> {
        self.primary
            .checked_add(self.init_staging.unwrap_or(0))
            .map(PayloadCost)
            .ok_or(Error::Overflow)
    }
}
impl Buffer {
    /// Effective descriptor used by pinned util::create_buffer_init, without
    /// reading/copying input data. Empty input uses an unmapped zero descriptor.
    pub fn from_init_len(length: u64, usage: BufferUsages) -> Result<Self, Error> {
        let remainder = length % wgpu::COPY_BUFFER_ALIGNMENT;
        let size = if remainder == 0 {
            length
        } else {
            length.checked_add(wgpu::COPY_BUFFER_ALIGNMENT - remainder).ok_or(Error::Overflow)?
        };
        Ok(Self {
            size,
            usage,
            mapped: length != 0,
        })
    }
    /// Complete deterministic creation bill, including core's separate Init
    /// staging. Driver heaps/tiling remain a different, unproven cost.
    pub fn payload(self, caps: DeviceCaps<'_>) -> Result<PayloadCost, Error> {
        self.creation_plan(caps)?.total()
    }
    pub fn creation_plan(self, caps: DeviceCaps<'_>) -> Result<BufferCreationPlan, Error> {
        if self.usage.is_empty()
            || self.usage.contains_unknown_bits()
            || self.mapped && !self.size.is_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT)
        {
            return Err(Error::Invalid);
        }
        if self.size > caps.limits.max_buffer_size {
            return Err(Error::Limits);
        }
        if !caps.mappable_primary_buffers
            && (self.usage.contains(BufferUsages::MAP_READ) && !(BufferUsages::MAP_READ | BufferUsages::COPY_DST).contains(self.usage)
                || self.usage.contains(BufferUsages::MAP_WRITE) && !(BufferUsages::MAP_WRITE | BufferUsages::COPY_SRC).contains(self.usage))
        {
            return Err(Error::Limits);
        }
        let backing = if self.size == 0 {
            wgpu::COPY_BUFFER_ALIGNMENT
        } else if self.usage.contains(BufferUsages::VERTEX) {
            self.size.checked_add(1).ok_or(Error::Overflow)?
        } else {
            self.size
        };
        let remainder = backing % wgpu::COPY_BUFFER_ALIGNMENT;
        let primary = if remainder == 0 {
            backing
        } else {
            backing
                .checked_add(wgpu::COPY_BUFFER_ALIGNMENT - remainder)
                .ok_or(Error::Overflow)?
        };
        let plan = BufferCreationPlan {
            source: self,
            primary,
            init_staging: (self.mapped && !self.usage.contains(BufferUsages::MAP_WRITE)).then_some(primary),
        };
        plan.total()?;
        Ok(plan)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ComponentRole {
    Primary,
    InitStaging,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Resource {
    Texture(Texture),
    Buffer(Buffer),
}
impl Resource {
    pub fn payload(self, caps: DeviceCaps<'_>) -> Result<PayloadCost, Error> {
        match self {
            Self::Texture(texture) => texture.payload(caps),
            Self::Buffer(buffer) => buffer.payload(caps),
        }
    }
}

/// Every new vector in this prerequisite uses this bounded constructor. The
/// caller pays for actual reserved capacity, not only occupied entries. The
/// hard cap bounds work even when a caller supplies an excessive byte limit.
pub(super) struct Bounded<T> {
    items: Vec<T>,
    limit: usize,
    storage: u64,
}
impl<T> Bounded<T> {
    pub fn new(limit: usize, hard_cap: usize, preparation_bytes: u64) -> Result<Self, Error> {
        if limit == 0 || limit > hard_cap || std::mem::size_of::<T>() == 0 {
            return Err(Error::Capacity);
        }
        let bytes = limit.checked_mul(std::mem::size_of::<T>()).ok_or(Error::Overflow)?;
        if u64::try_from(bytes).map_err(|_| Error::Overflow)? > preparation_bytes {
            return Err(Error::Capacity);
        }
        let mut items = Vec::new();
        items.try_reserve_exact(limit).map_err(|_| Error::Allocation)?;
        let storage = items.capacity().checked_mul(std::mem::size_of::<T>()).ok_or(Error::Overflow)?;
        let storage = u64::try_from(storage).map_err(|_| Error::Overflow)?;
        if items.capacity() > hard_cap || storage > preparation_bytes {
            return Err(Error::Capacity);
        }
        Ok(Self { items, limit, storage })
    }
    pub fn push(&mut self, value: T) -> Result<(), Error> {
        if self.items.len() == self.limit {
            return Err(Error::Capacity);
        }
        self.items.push(value);
        Ok(())
    }
    pub fn slice(&self) -> &[T] {
        &self.items
    }
    pub fn items_mut(&mut self) -> &mut [T] {
        &mut self.items
    }
    pub fn storage(&self) -> u64 {
        self.storage
    }
}
