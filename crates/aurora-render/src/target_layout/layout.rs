//! Direct allocation formulas from Backend::make_targets. Inputs are already
//! resolved settings/override decisions; this module never reads environment,
//! camera/world state or a GPU device. Nested component planners remain missing.

use super::{Error, PayloadCost};
use wgpu::{TextureFormat as Format, TextureUsages as Usage};

const MAX_TEXTURES: usize = 18;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Key {
    Color,
    ColorMsaa,
    DepthMsaa,
    Depth,
    SceneCopy,
    WaterInterface,
    WaterMask,
    GbufferMsaa,
    Gbuffer,
    AoDepth,
    Ao,
    AoBlur,
    WaterReflectionColor,
    WaterReflectionDepth,
    WaterAirColor,
    WaterAirDepth,
    MirrorColor,
    MirrorDepth,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct TextureSpec {
    pub key: Key,
    pub size: [u32; 3],
    pub mips: u32,
    pub samples: u32,
    pub format: Format,
    pub usage: Usage,
}
#[derive(Clone, Copy)]
pub(super) struct FormatSupport {
    pub format: Format,
    pub features: wgpu::TextureFormatFeatures,
}
/// Copied metadata, never a device/queue or capture/native authority.
pub(super) struct Caps {
    pub max_dimension_2d: u32,
    pub max_array_layers: u32,
    pub formats: [FormatSupport; 8],
}
impl TextureSpec {
    pub fn payload(self, caps: &Caps) -> Result<PayloadCost, Error> {
        let texel = match self.format {
            Format::R8Unorm => 1u64,
            Format::Rg8Unorm | Format::R16Float => 2,
            Format::Rgba8Unorm | Format::R32Float | Format::R32Uint | Format::Depth32Float => 4,
            Format::Rgba16Float => 8,
            _ => return Err(Error::Format),
        };
        let [w, h, layers] = self.size;
        let max_mips = 32 - w.max(h).leading_zeros();
        if self.usage.is_empty()
            || self.size.contains(&0)
            || self.mips == 0
            || self.mips > max_mips
            || !matches!(self.samples, 1 | 2 | 4 | 8 | 16)
            || self.samples > 1 && (self.mips != 1 || layers != 1)
        {
            return Err(Error::Invalid);
        }
        if w > caps.max_dimension_2d || h > caps.max_dimension_2d || layers > caps.max_array_layers {
            return Err(Error::Limits);
        }
        let mut matching = caps.formats.iter().filter(|support| support.format == self.format);
        let support = matching.next().ok_or(Error::Format)?;
        if matching.next().is_some()
            || !support.features.allowed_usages.contains(self.usage)
            || !support.features.flags.sample_count_supported(self.samples)
        {
            return Err(Error::Limits);
        }
        let mut pixels = 0u64;
        for mip in 0..self.mips {
            let area = u64::from((w >> mip).max(1))
                .checked_mul(u64::from((h >> mip).max(1)))
                .ok_or(Error::Overflow)?;
            pixels = pixels.checked_add(area).ok_or(Error::Overflow)?;
        }
        let bytes = pixels
            .checked_mul(u64::from(layers))
            .and_then(|v| v.checked_mul(u64::from(self.samples)))
            .and_then(|v| v.checked_mul(texel))
            .ok_or(Error::Overflow)?;
        Ok(PayloadCost(bytes))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Missing {
    pub gtao: bool,
    pub contact: bool,
}
#[derive(Clone, Copy)]
pub(super) struct Input {
    pub scene: [u32; 2],
    pub samples: u32,
    pub ssr: bool,
    pub ao_enabled: bool,
    pub legacy_ao: bool,
    pub gtao: bool,
    pub contact: bool,
    // Resolved water reflection, exterior and mirror scales, in that order.
    pub reflection_scales: [f32; 3],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CoreTargetLayout {
    specs: [Option<TextureSpec>; MAX_TEXTURES],
    len: usize,
    missing: Missing,
    cost: PayloadCost,
}
impl CoreTargetLayout {
    pub fn checked(input: Input, caps: &Caps) -> Result<Self, Error> {
        let [w, h] = input.scene.map(|edge| edge.max(1));
        if input.reflection_scales.iter().any(|scale| !scale.is_finite()) {
            return Err(Error::Invalid);
        }
        let mut result = Self {
            specs: [None; MAX_TEXTURES],
            len: 0,
            missing: Missing {
                gtao: input.ao_enabled && input.gtao,
                contact: input.contact,
            },
            cost: PayloadCost(0),
        };
        let attachment = Usage::RENDER_ATTACHMENT;
        let sampled = attachment | Usage::TEXTURE_BINDING;
        let mut add = |key, size: [u32; 2], format, samples, usage| -> Result<(), Error> {
            let spec = TextureSpec {
                key,
                size: [size[0], size[1], 1],
                mips: 1,
                format,
                samples,
                usage,
            };
            let bytes = spec.payload(caps)?.0;
            let slot = result.specs.get_mut(result.len).ok_or(Error::Overflow)?;
            *slot = Some(spec);
            result.len += 1;
            result.cost.0 = result.cost.0.checked_add(bytes).ok_or(Error::Overflow)?;
            Ok(())
        };
        add(Key::Color, [w, h], Format::Rgba16Float, 1, sampled | Usage::COPY_SRC)?;
        if input.samples > 1 {
            add(Key::ColorMsaa, [w, h], Format::Rgba16Float, input.samples, attachment)?;
            add(
                Key::DepthMsaa,
                [w, h],
                Format::Depth32Float,
                input.samples,
                sampled | Usage::COPY_SRC,
            )?;
        } else if input.samples != 1 {
            return Err(Error::Invalid);
        }
        add(Key::Depth, [w, h], Format::Depth32Float, 1, sampled | Usage::COPY_SRC)?;
        add(
            Key::SceneCopy,
            [w, h],
            Format::Rgba16Float,
            1,
            Usage::TEXTURE_BINDING | Usage::COPY_DST,
        )?;
        add(Key::WaterInterface, [w, h], Format::Depth32Float, 1, sampled)?;
        add(Key::WaterMask, [w, h], Format::R8Unorm, 1, sampled)?;
        if input.ssr {
            if input.samples > 1 {
                add(Key::GbufferMsaa, [w, h], Format::Rgba8Unorm, input.samples, attachment)?;
            }
            add(Key::Gbuffer, [w, h], Format::Rgba8Unorm, 1, sampled)?;
        }
        if input.ao_enabled && (input.legacy_ao || input.gtao) {
            add(
                Key::AoDepth,
                [w, h],
                Format::Depth32Float,
                1,
                sampled | Usage::COPY_DST | Usage::COPY_SRC,
            )?;
        }
        if input.ao_enabled && input.legacy_ao {
            add(Key::Ao, [w.div_ceil(2), h.div_ceil(2)], Format::R8Unorm, 1, sampled)?;
            add(Key::AoBlur, [w.div_ceil(2), h.div_ceil(2)], Format::R8Unorm, 1, sampled)?;
        }
        for (index, scale) in input.reflection_scales.into_iter().enumerate() {
            if scale <= 0.0 {
                continue;
            }
            let size = [
                ((w as f32 * scale.min(1.0)) as u32).max(16),
                ((h as f32 * scale.min(1.0)) as u32).max(16),
            ];
            let (color, depth) = [
                (Key::WaterReflectionColor, Key::WaterReflectionDepth),
                (Key::WaterAirColor, Key::WaterAirDepth),
                (Key::MirrorColor, Key::MirrorDepth),
            ][index];
            add(color, size, Format::Rgba16Float, 1, sampled)?;
            add(depth, size, Format::Depth32Float, 1, if index == 1 { sampled } else { attachment })?;
        }
        Ok(result)
    }
    pub fn specs(&self) -> impl Iterator<Item = &TextureSpec> {
        self.specs[..self.len].iter().flatten()
    }
    pub fn missing(self) -> Missing {
        self.missing
    }
    pub fn accounted_payload(self) -> PayloadCost {
        self.cost
    }
    /// Reserves only this core bundle. There is deliberately no complete-view
    /// receipt API; other optional/history/shared resources remain out of scope.
    pub fn reservable_core_payload(self) -> Result<PayloadCost, Error> {
        if self.missing != Missing::default() {
            Err(Error::MissingDependencies)
        } else {
            Ok(self.cost)
        }
    }
}
