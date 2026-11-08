//! Synthetic source fixtures only. None invokes current wgpu factories or
//! establishes native authority, GPU retirement, measured VRAM or support.

use super::descriptor::{Bounded, Buffer, DeviceCaps, Resource, Texture};
use super::inventory::{Class, Entry, FACTORY_SITES, Factory, Holding, Inventory, Payloads, Record};
use super::layout::FormatSupport;
use super::ownership::{BufferParts, DeviceIds, Ledger, Limits, Transaction};
use super::world::{Aa, CaptureLayout, Gtao, View, WorldViewPlan};
use super::{Error, PayloadCost};
use std::cell::Cell;
use std::rc::Rc;
use wgpu::{BufferUsages as B, TextureDimension as D, TextureFormat as F, TextureUsages as U};

fn formats() -> [FormatSupport; 13] {
    [
        F::R8Unorm,
        F::Rg8Unorm,
        F::R16Float,
        F::Rgba8Unorm,
        F::R32Float,
        F::R32Uint,
        F::Depth32Float,
        F::Rgba16Float,
        F::Rg32Float,
        F::Bc7RgbaUnorm,
        F::Rgba8UnormSrgb,
        F::Bgra8Unorm,
        F::Bgra8UnormSrgb,
    ]
    .map(|format| FormatSupport {
        format,
        features: wgpu::TextureFormatFeatures {
            allowed_usages: U::RENDER_ATTACHMENT | U::TEXTURE_BINDING | U::STORAGE_BINDING | U::COPY_SRC | U::COPY_DST,
            flags: wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X2
                | wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X4
                | wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X8
                | wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X16,
        },
    })
}
fn limits() -> wgpu::Limits {
    wgpu::Limits {
        max_texture_dimension_1d: 16384,
        max_texture_dimension_2d: 32768,
        max_texture_dimension_3d: 2048,
        max_texture_array_layers: 2048,
        max_buffer_size: u64::MAX,
        ..Default::default()
    }
}
fn caps<'a>(limits: &'a wgpu::Limits, formats: &'a [FormatSupport]) -> DeviceCaps<'a> {
    DeviceCaps {
        limits,
        formats,
        mappable_primary_buffers: false,
    }
}
fn plan() -> WorldViewPlan {
    WorldViewPlan::new(128, 1_000_000).unwrap()
}
fn view(size: [u32; 2]) -> View {
    View {
        core: super::tests::input(size),
        gtao: None,
        blend_depth: false,
        aa: Aa::None,
        hiz: false,
        glow: false,
        bloom: false,
        dof: false,
        privacy_spheres: false,
        reduced_output: None,
        output: Texture::d2(size, F::Rgba8Unorm, U::RENDER_ATTACHMENT | U::TEXTURE_BINDING | U::COPY_SRC),
    }
}
fn family(plan: &WorldViewPlan, factory: Factory, caps: DeviceCaps<'_>) -> u64 {
    plan.entries()
        .unwrap()
        .iter()
        .filter(|entry| entry.factory == factory)
        .map(|entry| entry.validate(caps).unwrap())
        .sum()
}
fn test_ledger(payload: u64) -> Ledger {
    // Deliberately synthetic fixture limits, never configuration/defaults.
    Ledger::new(Limits {
        payloads: Payloads([payload; 4]),
        preparation: 1_000_000,
        driver_headroom: 1,
    })
    .unwrap()
}
fn entry(index: u16, class: Class) -> Entry {
    Entry {
        factory: Factory::Output,
        index,
        class,
        resource: Resource::Texture(Texture::d2([1, 1], F::Rgba8Unorm, U::TEXTURE_BINDING)),
    }
}
fn observe_all(inventory: &mut Inventory) {
    for (factory, _) in FACTORY_SITES {
        inventory.observe(factory);
    }
}

#[test]
fn prospective_dimensions_distinguish_volume_mips_array_layers_and_bc7_blocks() {
    let (l, f) = (limits(), formats());
    let c = caps(&l, &f);
    let texture = Texture {
        size: [8, 8, 8],
        dimension: D::D3,
        mips: 4,
        samples: 1,
        format: F::Rgba8Unorm,
        usage: U::TEXTURE_BINDING,
    };
    assert_eq!(texture.payload(c), Ok(PayloadCost((512 + 64 + 8 + 1) * 4)));
    assert_eq!(
        Texture {
            dimension: D::D2,
            ..texture
        }
        .payload(c),
        Ok(PayloadCost((64 + 16 + 4 + 1) * 8 * 4))
    );
    assert_eq!(
        Texture {
            size: [64, 32, 4],
            dimension: D::D2,
            mips: 6,
            format: F::Bc7RgbaUnorm,
            ..texture
        }
        .payload(c),
        Ok(PayloadCost((128 + 32 + 8 + 2 + 1 + 1) * 16 * 4))
    );
    let d1 = Texture {
        size: [8, 1, 1],
        dimension: D::D1,
        mips: 1,
        ..texture
    };
    assert_eq!(d1.payload(c), Ok(PayloadCost(32)));
    assert_eq!(Texture { mips: 2, ..d1 }.payload(c), Err(Error::Invalid));
    assert_eq!(Texture { size: [8, 2, 1], ..d1 }.payload(c), Err(Error::Limits));
}

#[test]
fn prospective_descriptor_validation_closes_unknown_format_support_usage_samples_and_overflow() {
    let (mut l, mut f) = (limits(), formats());
    let base = Texture::d2([4, 4], F::Rgba8Unorm, U::TEXTURE_BINDING);
    assert_eq!(base.payload(caps(&l, &[f[0]; 65])), Err(Error::Capacity));
    for bad in [
        Texture { size: [0, 4, 1], ..base },
        Texture { mips: 4, ..base },
        Texture { samples: 3, ..base },
        Texture { usage: U::empty(), ..base },
        Texture { samples: 4, ..base },
        Texture {
            size: [5, 4, 1],
            format: F::Bc7RgbaUnorm,
            ..base
        },
        Texture {
            dimension: D::D3,
            samples: 4,
            usage: U::RENDER_ATTACHMENT,
            ..base
        },
    ] {
        assert_eq!(bad.payload(caps(&l, &f)), Err(Error::Invalid));
    }
    assert_eq!(
        Texture {
            format: F::Depth24Plus,
            ..base
        }
        .payload(caps(&l, &f)),
        Err(Error::Format)
    );
    f[3].features.allowed_usages = U::TEXTURE_BINDING;
    assert_eq!(
        Texture {
            usage: U::COPY_DST,
            ..base
        }
        .payload(caps(&l, &f)),
        Err(Error::Limits)
    );
    assert_eq!(
        Texture {
            usage: U::RENDER_ATTACHMENT,
            samples: 4,
            ..base
        }
        .payload(caps(&l, &f)),
        Err(Error::Limits)
    );
    f[3].features.allowed_usages = U::RENDER_ATTACHMENT;
    f[3].features.flags = wgpu::TextureFormatFeatureFlags::empty();
    assert_eq!(
        Texture {
            usage: U::RENDER_ATTACHMENT,
            samples: 4,
            ..base
        }
        .payload(caps(&l, &f)),
        Err(Error::Limits)
    );
    f[0].format = F::Rgba8Unorm;
    assert_eq!(base.payload(caps(&l, &f)), Err(Error::Format));
    f = formats();
    l.max_texture_dimension_3d = u32::MAX;
    assert_eq!(
        Texture {
            size: [u32::MAX; 3],
            dimension: D::D3,
            ..base
        }
        .payload(caps(&l, &f)),
        Err(Error::Overflow)
    );
    assert_eq!(
        Texture {
            size: [32769, 1, 1],
            ..base
        }
        .payload(caps(&l, &f)),
        Err(Error::Limits)
    );
}

#[test]
fn prospective_buffer_mapping_limits_and_separate_pool_class_are_checked() {
    let (mut l, f) = (limits(), formats());
    let mut buffer = Buffer {
        size: 256,
        usage: B::MAP_READ | B::COPY_DST,
        mapped: true,
    };
    assert_eq!(buffer.payload(caps(&l, &f)), Ok(PayloadCost(512)));
    buffer.usage |= B::STORAGE;
    assert_eq!(buffer.payload(caps(&l, &f)), Err(Error::Limits));
    let c = DeviceCaps {
        mappable_primary_buffers: true,
        ..caps(&l, &f)
    };
    assert_eq!(buffer.payload(c), Ok(PayloadCost(512)));
    buffer.size = 255;
    assert_eq!(buffer.payload(caps(&l, &f)), Err(Error::Invalid));
    buffer.mapped = false;
    buffer.usage = B::STORAGE;
    l.max_buffer_size = 254;
    assert_eq!(buffer.payload(caps(&l, &f)), Err(Error::Limits));
    assert_eq!(
        Entry {
            resource: Resource::Buffer(buffer),
            ..entry(0, Class::ViewTexture)
        }
        .validate(caps(&l, &f)),
        Err(Error::Invalid)
    );
}

#[test]
fn prospective_d1_render_attachments_refuse_even_with_genuine_format_usage_support() {
    let l = limits();
    let f = [FormatSupport {
        format: F::Rgba8Unorm,
        features: F::Rgba8Unorm.guaranteed_format_features(wgpu::Features::empty()),
    }];
    let c = caps(&l, &f);
    assert!(f[0].features.allowed_usages.contains(U::RENDER_ATTACHMENT));
    let d1 = Texture {
        dimension: D::D1,
        ..Texture::d2([8, 1], F::Rgba8Unorm, U::TEXTURE_BINDING)
    };
    assert_eq!(d1.payload(c), Ok(PayloadCost(32)));
    assert_eq!(
        Texture {
            usage: U::RENDER_ATTACHMENT,
            ..d1
        }
        .payload(c),
        Err(Error::Invalid)
    );
    for dimension in [D::D2, D::D3] {
        assert_eq!(
            Texture {
                dimension,
                usage: U::RENDER_ATTACHMENT,
                ..d1
            }
            .payload(c),
            Ok(PayloadCost(32))
        );
    }
}

#[test]
fn prospective_transient_usage_dimension_mips_and_layers_obey_pinned_constraints() {
    let l = limits();
    let f = [FormatSupport {
        format: F::Rgba8Unorm,
        features: F::Rgba8Unorm.guaranteed_format_features(wgpu::Features::empty()),
    }];
    let c = caps(&l, &f);
    let transient = Texture::d2([8, 8], F::Rgba8Unorm, U::RENDER_ATTACHMENT | U::TRANSIENT_ATTACHMENT);
    assert!(f[0].features.allowed_usages.contains(transient.usage));
    assert_eq!(transient.payload(c), Ok(PayloadCost(256)));
    assert_eq!(Texture { samples: 4, ..transient }.payload(c), Ok(PayloadCost(1024)));
    for invalid in [
        Texture {
            usage: U::TRANSIENT_ATTACHMENT,
            ..transient
        },
        Texture {
            usage: transient.usage | U::TEXTURE_BINDING,
            ..transient
        },
        Texture {
            usage: transient.usage | U::COPY_DST,
            ..transient
        },
        Texture {
            size: [8, 1, 1],
            dimension: D::D1,
            ..transient
        },
        Texture {
            dimension: D::D3,
            ..transient
        },
        Texture { mips: 2, ..transient },
        Texture {
            size: [8, 8, 2],
            ..transient
        },
    ] {
        assert_eq!(invalid.payload(c), Err(Error::Invalid));
    }
}

#[test]
fn prospective_mip_count_keeps_pinned_hal_ceiling_below_larger_copied_dimension_limit() {
    let (mut l, f) = (limits(), formats());
    l.max_texture_dimension_2d = 65536;
    let texture = Texture {
        mips: 16,
        ..Texture::d2([65536, 1], F::Rgba8Unorm, U::TEXTURE_BINDING)
    };
    // The sixteen widths are 65536 through 2, totaling 131070 texels.
    assert_eq!(texture.payload(caps(&l, &f)), Ok(PayloadCost(524_280)));
    // Seventeen levels fit this extent and copied limit, but not pinned HAL.
    assert_eq!(Texture { mips: 17, ..texture }.payload(caps(&l, &f)), Err(Error::Invalid));
}

#[test]
fn prospective_multisampled_copy_destination_keeps_attachment_storage_mip_and_layer_rules() {
    let l = limits();
    let f = [FormatSupport {
        format: F::Rgba8Unorm,
        features: F::Rgba8Unorm.guaranteed_format_features(wgpu::Features::empty()),
    }];
    let c = caps(&l, &f);
    let texture = Texture {
        samples: 4,
        ..Texture::d2([8, 8], F::Rgba8Unorm, U::RENDER_ATTACHMENT | U::COPY_DST)
    };
    assert_eq!(texture.payload(c), Ok(PayloadCost(1024)));
    for invalid in [
        Texture {
            usage: U::COPY_DST,
            ..texture
        },
        Texture {
            usage: texture.usage | U::STORAGE_BINDING,
            ..texture
        },
        Texture { mips: 2, ..texture },
        Texture {
            size: [8, 8, 2],
            ..texture
        },
    ] {
        assert_eq!(invalid.payload(c), Err(Error::Invalid));
    }
}

#[test]
fn prospective_buffer_unknown_usage_bits_refuse_before_backing_or_mapping_credit() {
    let (l, f) = (limits(), formats());
    let buffer = Buffer {
        size: 4,
        usage: B::COPY_DST,
        mapped: false,
    };
    let unknown = B::from_bits_retain(!B::all().bits());
    assert!(unknown.contains_unknown_bits());
    assert_eq!(buffer.payload(caps(&l, &f)), Ok(PayloadCost(4)));
    for mappable_primary_buffers in [false, true] {
        assert_eq!(
            Buffer {
                usage: buffer.usage | unknown,
                ..buffer
            }
            .payload(DeviceCaps {
                mappable_primary_buffers,
                ..caps(&l, &f)
            }),
            Err(Error::Invalid)
        );
    }
}

#[test]
fn prospective_buffer_backing_charges_vertex_end_byte_and_alignment_with_checked_overflow() {
    let (l, f) = (limits(), formats());
    let c = caps(&l, &f);
    for (size, usage, expected) in [
        (1, B::COPY_DST, 4),
        (3, B::COPY_DST, 4),
        (4, B::COPY_DST, 4),
        (5, B::COPY_DST, 8),
        (1, B::VERTEX, 4),
        (3, B::VERTEX, 4),
        (4, B::VERTEX, 8),
        (7, B::VERTEX, 8),
        (8, B::VERTEX, 12),
        (u64::MAX - 3, B::COPY_DST, u64::MAX - 3),
        (u64::MAX - 4, B::VERTEX, u64::MAX - 3),
    ] {
        assert_eq!(
            Buffer {
                size,
                usage,
                mapped: false
            }
            .payload(c),
            Ok(PayloadCost(expected))
        );
    }
    for (size, usage) in [
        (u64::MAX, B::VERTEX),     // the mandatory end byte overflows
        (u64::MAX - 1, B::VERTEX), // the end byte fits, rounding overflows
        (u64::MAX - 3, B::VERTEX),
        (u64::MAX, B::COPY_DST), // rounding alone overflows
    ] {
        assert_eq!(
            Buffer {
                size,
                usage,
                mapped: false
            }
            .payload(c),
            Err(Error::Overflow)
        );
    }
    assert_eq!(
        Buffer {
            size: 0,
            usage: B::COPY_DST,
            mapped: false,
        }
        .payload(c),
        Ok(PayloadCost(4)),
        "pinned core gives zero-size descriptors four backing bytes"
    );
}

#[test]
fn prospective_buffer_backing_is_reserved_before_factory_and_retained_by_derived_holder() {
    let (l, f) = (limits(), formats());
    let c = caps(&l, &f);
    let e = Entry {
        resource: Resource::Buffer(Buffer {
            size: 4,
            usage: B::VERTEX,
            mapped: false,
        }),
        ..entry(0, Class::Buffer)
    };
    assert_eq!(e.validate(c), Ok(8));
    let mut p = plan();
    p.add(e, c).unwrap();
    assert_eq!(p.payloads().unwrap().0, [0, 0, 8, 0]);
    let insufficient = test_ledger(7);
    assert!(matches!(
        Transaction::<[u8; 8]>::begin(DeviceIds::new().unwrap(), &insufficient, p.entries().unwrap(), c),
        Err(Error::Capacity)
    ));
    assert_eq!(insufficient.used(), Payloads::default());
    assert_eq!(insufficient.preparation(), 0);
    let ledger = test_ledger(8);
    let mut transaction = Transaction::begin(DeviceIds::new().unwrap(), &ledger, p.entries().unwrap(), c).unwrap();
    assert_eq!(ledger.used().0, [0, 0, 8, 0]);
    let lease = transaction.create(0, e, || Ok([0u8; 8])).unwrap();
    let holder = lease.clone().hold([0u8; 1]);
    drop(transaction.finish().unwrap());
    drop(lease);
    assert_eq!(ledger.used().0, [0, 0, 8, 0]);
    drop(holder);
    assert_eq!(ledger.used(), Payloads::default());
    assert_eq!(ledger.preparation(), 0);
}

#[test]
fn prospective_core_gtao_contact_and_aa_match_independent_math_at_all_caps_and_qualities() {
    let (l, f) = (limits(), formats());
    let c = caps(&l, &f);
    for [w, h] in [[320, 180], [480, 270], [640, 360], [181, 319], [1, 1]] {
        let n = u64::from(w) * u64::from(h);
        for level in 0..4 {
            for indirect in [false, true] {
                let mut v = view([w, h]);
                v.core.ao_enabled = true;
                v.core.gtao = true;
                v.core.legacy_ao = true; // retained override pair coexists
                v.core.contact = true;
                v.gtao = Some(Gtao { level, indirect });
                v.aa = Aa::Taa;
                let mut p = plan();
                p.view(v, c).unwrap();
                let half = level == 0;
                let (qw, qh) = if half { (w.div_ceil(2), h.div_ceil(2)) } else { (w, h) };
                let q = u64::from(qw) * u64::from(qh);
                let mip_levels = (32 - qw.max(qh).leading_zeros()).min(5);
                let depth: u64 = (0..mip_levels)
                    .map(|mip| u64::from((qw >> mip).max(1)) * u64::from((qh >> mip).max(1)))
                    .sum();
                let expected = 4 * depth
                    + 36 * q
                    + 12 * n
                    + if half { 4 * q } else { 0 }
                    + if indirect {
                        16 * q + 8 * n + if half { 8 * q } else { 0 }
                    } else {
                        0
                    };
                assert_eq!(family(&p, Factory::Gtao, c), expected);
                assert_eq!(
                    family(&p, Factory::Core, c),
                    29 * n + 2 * u64::from(w.div_ceil(2)) * u64::from(h.div_ceil(2))
                );
                assert_eq!(family(&p, Factory::Contact, c), 12 * n);
                assert_eq!(family(&p, Factory::Taa, c), 16 * n); // exact normals alias, no second copy
                assert_eq!(family(&p, Factory::Output, c), 4 * n);
            }
        }
    }
}

#[test]
fn prospective_samples_ssr_reflections_and_lazy_depth_never_change_effective_quality() {
    let (l, f) = (limits(), formats());
    let c = caps(&l, &f);
    for samples in [1, 2, 4, 8, 16] {
        let mut v = view([319, 179]);
        v.core.samples = samples;
        v.core.ssr = true;
        v.core.reflection_scales = [0.01, 0.75, 1.0];
        let mut p = plan();
        p.view(v, c).unwrap();
        let n = 319 * 179u64;
        let reflected = 12 * (16 * 16 + 239 * 134 + n);
        assert_eq!(
            family(&p, Factory::Core, c),
            29 * n + reflected + if samples == 1 { 0 } else { 16 * u64::from(samples) * n }
        );
        v.blend_depth = true;
        let mut lazy = plan();
        if samples == 1 {
            lazy.view(v, c).unwrap();
            assert_eq!(family(&lazy, Factory::BlendDepth, c), 4 * n);
        } else {
            // The actual Layered/Layers texture-to-texture route preserves
            // equal MSAA counts. Its descriptor is valid in pinned wgpu-core.
            lazy.view(v, c).unwrap();
            assert_eq!(family(&lazy, Factory::BlendDepth, c), 4 * n * u64::from(samples));
        }
        let blend = lazy
            .entries()
            .unwrap()
            .iter()
            .find(|entry| entry.factory == Factory::BlendDepth)
            .unwrap();
        assert_eq!(
            blend.resource,
            Resource::Texture(Texture {
                samples,
                ..Texture::d2([319, 179], F::Depth32Float, U::RENDER_ATTACHMENT | U::COPY_DST)
            })
        );
        assert!(lazy.entries().unwrap().iter().any(|entry| {
            entry.factory == Factory::Core
                && matches!(entry.resource, Resource::Texture(texture)
                    if texture.size == [319, 179, 1]
                        && texture.format == F::Depth32Float
                        && texture.samples == samples
                        && texture.usage.contains(U::RENDER_ATTACHMENT | U::COPY_SRC))
        }));
    }
}

#[test]
fn prospective_post_graph_counts_fixed_glow_ceil_levels_tiles_hiz_and_privacy() {
    let (l, f) = (limits(), formats());
    let c = caps(&l, &f);
    let mut v = view([319, 179]);
    v.aa = Aa::Smaa;
    v.hiz = true;
    v.glow = true;
    v.bloom = true;
    v.dof = true;
    v.privacy_spheres = true;
    v.reduced_output = Some([159, 89]);
    let mut p = plan();
    p.view(v, c).unwrap();
    let n = 319 * 179u64;
    assert_eq!(family(&p, Factory::Smaa, c), 10 * n);
    assert_eq!(family(&p, Factory::Glow, c), 3 * 512 * 512 * 4);
    assert_eq!(family(&p, Factory::Bloom, c), 8 * (160 * 90 + 80 * 45 + 40 * 23 + 20 * 12));
    assert_eq!(family(&p, Factory::Dof, c), 8 * n + 24 * 160 * 90 + 4 * 10 * 6);
    assert_eq!(family(&p, Factory::Privacy, c), 8 * n + 4 * 159 * 89);
    assert_eq!(
        family(&p, Factory::HiZ, c),
        4 * (160 * 90 + 80 * 45 + 40 * 22 + 20 * 11 + 10 * 5 + 5 * 2 + 2 + 1)
    );
    let mut taa = plan();
    v.aa = Aa::Taa;
    taa.view(v, c).unwrap();
    assert_eq!(family(&taa, Factory::Taa, c), 24 * n);
    assert_eq!(family(&taa, Factory::Smaa, c), 0);
}

#[test]
fn prospective_fixed_shared_graph_includes_shadow_probe_impostor_water_and_retained_lut_costs() {
    let (l, f) = (limits(), formats());
    let c = caps(&l, &f);
    for projectors in [0, 4, 10, 16] {
        let mut p = plan();
        p.shadows(4096, projectors, c).unwrap();
        assert_eq!(
            family(&p, Factory::Shadows, c),
            match projectors {
                0 | 4 => 268_435_456,
                10 => 402_653_184,
                _ => 469_762_048,
            }
        );
        assert_eq!(family(&p, Factory::ShadowMerge, c), 16_777_216);
    }
    let mut p = plan();
    p.probes(64, false, c).unwrap();
    assert_eq!(
        family(&p, Factory::Probes, c),
        67_104_768 + 786_432 + 4_194_240 + 1_048_512 + 327_680
    );
    p.impostors(4, c).unwrap();
    assert_eq!(family(&p, Factory::Impostors, c), 134_217_728 + 256 * 256 * 12);
    p.water(256, c).unwrap();
    assert_eq!(family(&p, Factory::Water, c), 24 * 256 * 256 + 64 * 64 * 8 + 256 * 256 * 8);
    p.retained_looks([640, 360], F::Bgra8UnormSrgb, &[128; 4], c).unwrap();
    assert_eq!(family(&p, Factory::Looks, c), 20 * 640 * 360 + 33_554_432 + 8);
    p.immutable(c).unwrap();
    assert_eq!(family(&p, Factory::Smaa, c), 160 * 560 * 2 + 64 * 16);
    assert_eq!(family(&p, Factory::Bloom, c), 8);
    assert_eq!(family(&p, Factory::Gtao, c), 4);
    assert_eq!(family(&p, Factory::Immutable, c), 24);
    let mut probe_fixture = plan();
    probe_fixture.probes(1, true, c).unwrap();
    assert!(
        probe_fixture
            .entries()
            .unwrap()
            .iter()
            .filter(|entry| entry.index < 2)
            .all(|entry| { matches!(entry.resource, Resource::Texture(texture) if texture.usage.contains(U::COPY_SRC)) })
    );
}

#[test]
fn prospective_full_plan_composes_with_generic_recorder_and_refuses_small_shadow_budget_before_factory() {
    let (l, f) = (limits(), formats());
    let c = caps(&l, &f);
    let mut v = view([640, 360]);
    v.core.ao_enabled = true;
    v.core.gtao = true;
    v.core.legacy_ao = true;
    v.core.contact = true;
    v.core.ssr = true;
    v.core.reflection_scales = [1.0, 0.75, 1.0];
    v.gtao = Some(Gtao { level: 0, indirect: true });
    v.aa = Aa::Taa;
    v.blend_depth = true;
    v.hiz = true;
    v.glow = true;
    v.bloom = true;
    v.dof = true;
    v.privacy_spheres = true;
    let mut p = plan();
    p.view(v, c).unwrap();
    p.shadows(4096, 10, c).unwrap();
    p.probes(64, false, c).unwrap();
    p.impostors(4, c).unwrap();
    p.water(256, c).unwrap();
    p.immutable(c).unwrap();
    let ledger = test_ledger(u64::MAX / 8);
    let mut transaction = Transaction::begin(DeviceIds::new().unwrap(), &ledger, p.entries().unwrap(), c).unwrap();
    assert_eq!(ledger.used(), p.payloads().unwrap());
    // These tiny fake values record descriptor calls only; no actual texture
    // allocation or measured resource-byte evidence is implied by this test.
    for (index, entry) in p.entries().unwrap().iter().copied().enumerate() {
        drop(transaction.create(index, entry, || Ok([0u8; 1])).unwrap());
    }
    let bundle = transaction.finish().unwrap();
    assert_eq!(bundle.ids().count(), p.entries().unwrap().len());
    drop(bundle);
    assert_eq!(ledger.used(), Payloads::default());
    assert_eq!(ledger.preparation(), 0);
    let mut shadows = plan();
    shadows.shadows(4096, 4, c).unwrap();
    // Negative fixture only: 64 MiB can never admit this 256 MiB atlas.
    let too_small = test_ledger(64 * 1024 * 1024);
    assert!(matches!(
        Transaction::<[u8; 1]>::begin(DeviceIds::new().unwrap(), &too_small, shadows.entries().unwrap(), c),
        Err(Error::Capacity)
    ));
    assert_eq!(too_small.used(), Payloads::default());
    assert_eq!(too_small.preparation(), 0);
}

#[test]
fn prospective_capture_keeps_row_padding_uniform_and_packed_cpu_credits_separate() {
    let (l, f) = (limits(), formats());
    let c = caps(&l, &f);
    for size in [[320, 180], [480, 270], [640, 360], [319, 179]] {
        let capture = CaptureLayout::checked(size, c).unwrap();
        let pixels = u64::from(size[0]) * u64::from(size[1]) * 4;
        let padded = (u64::from(size[0]) * 4).div_ceil(256) * 256 * u64::from(size[1]);
        assert_eq!(capture.packed_cpu, pixels);
        assert_eq!(capture.image_credit(c), Ok(2 * pixels + padded));
        assert_eq!(capture.entries[2].validate(c), Ok(32));
        let mut p = plan();
        for entry in capture.entries {
            p.add(entry, c).unwrap();
        }
        assert_eq!(p.payloads().unwrap().0, [0, 0, 32, pixels + padded]);
    }
    assert!(matches!(CaptureLayout::checked([0, 1], c), Err(Error::Invalid)));
}

#[test]
fn prospective_missing_inventory_unknown_family_and_partial_plan_never_look_complete() {
    let (l, f) = (limits(), formats());
    let c = caps(&l, &f);
    let mut p = plan();
    p.view(view([320, 180]), c).unwrap();
    let mut inventory = Inventory::new(32, 1_000_000).unwrap();
    inventory.observe(Factory::TexturePages);
    assert_eq!(p.prospective_total(&inventory, 1_000_000), Err(Error::MissingDependencies));
    observe_all(&mut inventory);
    let total = p.prospective_total(&inventory, 1_000_000).unwrap();
    assert_eq!(total.0, [29 * 320 * 180, 0, 0, 0]);
    assert_eq!(p.prospective_total(&inventory, 1), Err(Error::Capacity));
    assert!(total.within(Payloads([u64::MAX; 4])));
    assert!(!total.within(Payloads::default()));
    let mut bad = plan();
    assert_eq!(bad.probes(65, false, c), Err(Error::Invalid));
    assert_eq!(bad.view(view([320, 180]), c), Err(Error::Invalid));
    assert_eq!(bad.payloads(), Err(Error::Invalid));
    assert_eq!(bad.prospective_total(&inventory, 1_000_000), Err(Error::Invalid));
}

#[test]
fn prospective_capacity_limits_and_failed_options_refuse_before_any_resource_factory() {
    assert!(matches!(WorldViewPlan::new(129, u64::MAX), Err(Error::Capacity)));
    assert!(matches!(Inventory::new(4097, u64::MAX), Err(Error::Capacity)));
    assert!(matches!(Bounded::<u64>::new(0, 256, 1_000_000), Err(Error::Capacity)));
    assert!(matches!(Bounded::<u64>::new(256, 256, 1), Err(Error::Capacity)));
    let (l, f) = (limits(), formats());
    let c = caps(&l, &f);
    let mut small = WorldViewPlan::new(1, 1_000_000).unwrap();
    assert_eq!(small.view(view([320, 180]), c), Err(Error::Capacity));
    assert_eq!(small.entries(), Err(Error::Invalid));
    for bad in [view([641, 360]), view([640, 361]), view([0, 0])] {
        assert_eq!(plan().view(bad, c), Err(Error::Invalid));
    }
    let mut v = view([320, 180]);
    v.core.gtao = true;
    v.core.ao_enabled = true;
    assert_eq!(plan().view(v, c), Err(Error::Invalid));
    assert_eq!(plan().water(u32::MAX, c), Err(Error::Overflow));
    assert_eq!(plan().impostors(3, c), Err(Error::Invalid));
    assert_eq!(plan().shadows(4096, 17, c), Err(Error::Invalid));
    assert_eq!(plan().retained_looks([1, 1], F::Rgba8Unorm, &[128; 5], c), Err(Error::Capacity));
}

struct Storage {
    ledger: Ledger,
    drops: Rc<Cell<usize>>,
    _pixels: [u8; 4],
}
impl Drop for Storage {
    fn drop(&mut self) {
        // Observe the actual drop order: credit still covers this physical
        // storage while its destructor runs, including failed partial bundles.
        assert!(self.ledger.used().0.iter().sum::<u64>() >= 4);
        self.drops.set(self.drops.get() + 1);
    }
}

#[test]
fn recorder_matches_each_descriptor_once_and_refuses_mismatch_duplicate_and_omission() {
    let (l, f) = (limits(), formats());
    let c = caps(&l, &f);
    let ledger = test_ledger(100);
    let ids = DeviceIds::new().unwrap();
    let entries = [entry(0, Class::ViewTexture), entry(1, Class::ViewTexture)];
    let calls = Cell::new(0);
    let mut transaction = Transaction::<[u8; 4]>::begin(ids, &ledger, &entries, c).unwrap();
    assert_eq!(ledger.used().0[0], 8);
    assert!(matches!(
        transaction.create(0, entry(9, Class::ViewTexture), || {
            calls.set(1);
            Ok([0; 4])
        }),
        Err(Error::Invalid)
    ));
    assert_eq!(calls.get(), 0);
    let held = transaction.create(0, entries[0], || Ok([0; 4])).unwrap();
    assert_eq!(held.entry(), entries[0]);
    assert!(matches!(
        transaction.create(0, entries[0], || panic!("duplicate must not allocate")),
        Err(Error::Invalid)
    ));
    assert!(matches!(transaction.finish(), Err(Error::MissingDependencies)));
    assert_eq!(ledger.used().0[0], 4); // held real allocation, not current-bundle sum
    drop(held);
    assert_eq!(ledger.used(), Payloads::default());
    assert_eq!(ledger.preparation(), 0);
}

#[test]
fn recorder_failed_nth_factory_and_unwind_drop_physical_storage_before_refund() {
    let (l, f) = (limits(), formats());
    let c = caps(&l, &f);
    for fail_at in 0..3 {
        for panic in [false, true] {
            let ledger = test_ledger(100);
            let drops = Rc::new(Cell::new(0));
            let entries = [
                entry(0, Class::ViewTexture),
                entry(1, Class::ViewTexture),
                entry(2, Class::ViewTexture),
            ];
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut transaction = Transaction::begin(DeviceIds::new().unwrap(), &ledger, &entries, c).unwrap();
                for (index, item) in entries.into_iter().enumerate() {
                    let created = transaction.create(index, item, || {
                        if index == fail_at {
                            if panic {
                                panic!("synthetic factory unwind");
                            }
                            return Err(Error::Allocation);
                        }
                        Ok(Storage {
                            ledger: ledger.clone(),
                            drops: drops.clone(),
                            _pixels: [0; 4],
                        })
                    });
                    if created.is_err() {
                        return;
                    }
                }
                panic!("one factory must fail");
            }));
            assert_eq!(result.is_err(), panic);
            assert_eq!(drops.get(), fail_at);
            assert_eq!(ledger.used(), Payloads::default());
            assert_eq!(ledger.preparation(), 0);
        }
    }
}

#[test]
fn recorder_derived_storage_and_synthetic_last_use_keep_old_charge_across_owner_replacement() {
    let (l, f) = (limits(), formats());
    let c = caps(&l, &f);
    let ledger = test_ledger(8);
    let ids = DeviceIds::new().unwrap();
    let drops = Rc::new(Cell::new(0));
    let make = || {
        Ok(Storage {
            ledger: ledger.clone(),
            drops: drops.clone(),
            _pixels: [0; 4],
        })
    };
    let mut old = Transaction::begin(ids.clone(), &ledger, &[entry(0, Class::ViewTexture)], c).unwrap();
    let old_lease = old.create(0, entry(0, Class::ViewTexture), make).unwrap();
    let old_id = old_lease.id();
    let binding = old_lease.hold([0u8; 16]);
    let old = old.finish().unwrap();
    assert_eq!(old.ids().collect::<Vec<_>>(), [old_id]);
    let submitted = old.synthetic_retirement();
    assert_eq!(submitted.allocation_count(), 1);
    let mut replacement = Transaction::begin(ids, &ledger, &[entry(0, Class::ViewTexture)], c).unwrap();
    let new = replacement.create(0, entry(0, Class::ViewTexture), make).unwrap();
    assert_ne!(old_id, new.id());
    let replacement = replacement.finish().unwrap();
    assert_eq!(ledger.used().0[0], 8);
    assert!(matches!(
        Transaction::<Storage>::begin(DeviceIds::new().unwrap(), &ledger, &[entry(0, Class::ViewTexture)], c),
        Err(Error::Capacity)
    ));
    drop(submitted); // synthetic callback-owner storage retired, NOT GPU proof
    assert_eq!(ledger.used().0[0], 8);
    drop(binding); // independently retained old bind-group owner
    assert_eq!(ledger.used().0[0], 4);
    assert_eq!(drops.get(), 1);
    drop(replacement);
    assert_eq!(ledger.used().0[0], 4);
    drop(new);
    assert_eq!(drops.get(), 2);
    assert_eq!(ledger.used(), Payloads::default());
    assert_eq!(ledger.preparation(), 0);
}

#[test]
fn recorder_shared_and_image_pools_never_spend_view_credit_or_skip_preflight() {
    let (l, f) = (limits(), formats());
    let c = caps(&l, &f);
    let ledger = test_ledger(8);
    let ids = DeviceIds::new().unwrap();
    for class in [Class::SharedTexture, Class::ImagePool] {
        let e = entry(0, class);
        let mut transaction = Transaction::begin(ids.clone(), &ledger, &[e], c).unwrap();
        let lease = transaction.create(0, e, || Ok([0u8; 4])).unwrap();
        assert_eq!(ledger.used().0[class as usize], 4);
        assert_eq!(ledger.used().0[0], 0);
        drop(transaction.finish().unwrap());
        drop(lease);
    }
    let e = Entry {
        resource: Resource::Buffer(Buffer {
            size: 8,
            usage: B::COPY_DST,
            mapped: false,
        }),
        ..entry(0, Class::Buffer)
    };
    let mut transaction = Transaction::begin(ids, &ledger, &[e], c).unwrap();
    let lease = transaction.create(0, e, || Ok([0u8; 8])).unwrap();
    assert_eq!(ledger.used().0, [0, 0, 8, 0]);
    drop(transaction.finish().unwrap());
    drop(lease);
    let absent = Limits {
        payloads: Payloads([8; 4]),
        preparation: 100,
        driver_headroom: 0,
    };
    assert!(matches!(Ledger::new(absent), Err(Error::MissingDependencies)));
    assert!(matches!(
        Ledger::new(Limits {
            payloads: Payloads([u64::MAX; 4]),
            driver_headroom: 1,
            ..absent
        }),
        Err(Error::Overflow)
    ));
    let tiny = Ledger::new(Limits {
        driver_headroom: 1,
        preparation: 1,
        ..absent
    })
    .unwrap();
    assert!(matches!(
        Transaction::<[u8; 4]>::begin(DeviceIds::new().unwrap(), &tiny, &[entry(0, Class::ViewTexture)], c),
        Err(Error::Capacity)
    ));
    assert_eq!(tiny.used(), Payloads::default());
}

#[test]
fn inventory_charges_every_old_spare_page_copy_and_retiring_physical_id_once() {
    let (l, f) = (limits(), formats());
    let c = caps(&l, &f);
    let ledger = test_ledger(100);
    let ids = DeviceIds::new().unwrap();
    let mut inventory = Inventory::new(5, 1_000_000).unwrap();
    let mut holders = Vec::new();
    for (index, holding) in [
        Holding::Resident,
        Holding::Spare,
        Holding::Retiring,
        Holding::CopySource,
        Holding::Baseline,
    ]
    .into_iter()
    .enumerate()
    {
        let e = Entry {
            factory: Factory::TexturePages,
            ..entry(index as u16, Class::SharedTexture)
        };
        let mut transaction = Transaction::begin(ids.clone(), &ledger, &[e], c).unwrap();
        let lease = transaction.create(0, e, || Ok([0u8; 4])).unwrap();
        let record = Record::held(&lease, holding);
        assert_eq!(record.holding(), holding);
        inventory.record(record, c).unwrap();
        let mut duplicate = Inventory::new(2, 1_000_000).unwrap();
        duplicate.record(record, c).unwrap();
        assert_eq!(duplicate.record(record, c), Err(Error::Invalid));
        observe_all(&mut duplicate);
        assert!(matches!(duplicate.checked_records(), Err(Error::MissingDependencies)));
        holders.push(lease);
    }
    observe_all(&mut inventory);
    assert_eq!(inventory.checked_records().unwrap().len(), 5);
    assert_eq!(inventory.payloads().unwrap().0, [0, 20, 0, 0]);
    let mut replacement = plan();
    replacement.add(entry(0, Class::SharedTexture), c).unwrap();
    assert_eq!(replacement.prospective_total(&inventory, 1_000_000).unwrap().0, [0, 24, 0, 0]);
    drop(holders);
    assert_eq!(ledger.used(), Payloads::default());
    // A copied inventory is metadata only: it never substitutes for actual
    // resource leases or proves those allocations are still live.
    assert_eq!(inventory.payloads().unwrap().0[1], 20);
}

fn initialized_entry(index: u16) -> Entry {
    Entry {
        factory: Factory::Uniforms,
        index,
        class: Class::Buffer,
        resource: Resource::Buffer(Buffer {
            size: 4,
            usage: B::UNIFORM,
            mapped: true,
        }),
    }
}

#[test]
fn initialized_buffer_plan_counts_zero_util_padding_mapping_and_total_overflow() {
    let (l, f) = (limits(), formats());
    let c = caps(&l, &f);
    for (size, usage, mapped, primary, stage) in [
        (0, B::UNIFORM, false, 4, None),
        (0, B::UNIFORM, true, 4, Some(4)),
        (4, B::UNIFORM, true, 4, Some(4)),
        (4, B::VERTEX, true, 8, Some(8)),
        (4, B::MAP_WRITE | B::COPY_SRC, true, 4, None),
        (256, B::MAP_READ | B::COPY_DST, true, 256, Some(256)),
    ] {
        let b = Buffer { size, usage, mapped };
        let p = b.creation_plan(c).unwrap();
        assert_eq!(p.source(), b);
        assert_eq!(p.primary(), primary);
        assert_eq!(p.init_staging(), stage);
        assert_eq!(b.payload(c), Ok(PayloadCost(primary + stage.unwrap_or(0))));
    }
    for (length, usage, effective, total) in [
        (0, B::UNIFORM, 0, 4),
        (1, B::UNIFORM, 4, 8),
        (5, B::UNIFORM, 8, 16),
        (1, B::VERTEX, 4, 16),
    ] {
        let b = Buffer::from_init_len(length, usage).unwrap();
        assert_eq!(b.size, effective);
        assert_eq!(b.mapped, length != 0);
        assert_eq!(b.payload(c), Ok(PayloadCost(total)));
    }
    for b in [
        Buffer::from_init_len(0, B::UNIFORM).unwrap(),
        Buffer {
            size: 4,
            usage: B::MAP_WRITE | B::COPY_SRC,
            mapped: true,
        },
    ] {
        let e = Entry {
            resource: Resource::Buffer(b),
            ..initialized_entry(0)
        };
        let ledger = test_ledger(4);
        let mut t = Transaction::begin(DeviceIds::new().unwrap(), &ledger, &[e], c).unwrap();
        let made = t.create_buffer(0, e, || Ok(BufferParts::new([0u8; 4], None))).unwrap();
        assert!(made.records()[1].is_none());
        let (primary, pending) = made.synthetic_unmap();
        assert!(pending.is_none());
        drop(t.finish().unwrap());
        assert_eq!(ledger.used().0[2], 4);
        drop(primary);
        assert_eq!(ledger.preparation(), 0);
    }
    assert_eq!(Buffer::from_init_len(u64::MAX, B::UNIFORM), Err(Error::Overflow));
    assert_eq!(
        Buffer::from_init_len(u64::MAX - 3, B::UNIFORM).unwrap().payload(c),
        Err(Error::Overflow)
    );
}

#[test]
fn initialized_components_reserve_both_ids_credits_and_capacity_before_factories() {
    let (l, f) = (limits(), formats());
    let c = caps(&l, &f);
    let entries: Vec<_> = (0..64).map(initialized_entry).collect();
    let ledger = test_ledger(10_000);
    let mut transaction = Transaction::begin(DeviceIds::new().unwrap(), &ledger, &entries, c).unwrap();
    assert_eq!(ledger.used().0, [0, 0, 512, 0]);
    let calls = Cell::new(0);
    assert!(matches!(
        transaction.create(0, entries[0], || {
            calls.set(1);
            Ok([0u8; 4])
        }),
        Err(Error::MissingDependencies)
    ));
    assert_eq!(calls.get(), 0);
    for (i, e) in entries.iter().copied().enumerate() {
        let made = transaction
            .create_buffer(i, e, || Ok(BufferParts::new([0u8; 4], Some([0u8; 4]))))
            .unwrap();
        let records = made.records();
        assert_ne!(records[0].unwrap().id(), records[1].unwrap().id());
        drop(made);
    }
    let bundle = transaction.finish().unwrap();
    assert_eq!(bundle.ids().count(), 128);
    drop(bundle);
    assert_eq!(ledger.used(), Payloads::default());
    assert_eq!(ledger.preparation(), 0);
    let mut oversized = entries;
    oversized.push(Entry {
        resource: Resource::Buffer(Buffer {
            mapped: false,
            ..Buffer {
                size: 4,
                usage: B::UNIFORM,
                mapped: true,
            }
        }),
        ..initialized_entry(64)
    });
    assert!(matches!(
        Transaction::<[u8; 4]>::begin(DeviceIds::new().unwrap(), &ledger, &oversized, c),
        Err(Error::Capacity)
    ));
    assert_eq!(calls.get(), 0);
    let mut small_plan = WorldViewPlan::new(1, 100_000).unwrap();
    assert_eq!(small_plan.add(initialized_entry(0), c), Err(Error::Capacity));
    assert_eq!(small_plan.entries(), Err(Error::Invalid));
    assert_eq!(ledger.preparation(), 0);
}

struct InitStorage {
    ledger: Ledger,
    drops: Rc<Cell<usize>>,
    _bytes: [u8; 4],
}
impl Drop for InitStorage {
    fn drop(&mut self) {
        assert!(
            self.ledger.used().0[Class::Buffer as usize] >= 4,
            "each component storage drops before its last refund"
        );
        self.drops.set(self.drops.get() + 1);
    }
}

#[test]
fn initialized_upload_owners_outlive_primary_and_logical_retirement_with_independent_last_holder() {
    let (l, f) = (limits(), formats());
    let c = caps(&l, &f);
    let ledger = test_ledger(8);
    let drops = Rc::new(Cell::new(0));
    let e = initialized_entry(0);
    let make = || InitStorage {
        ledger: ledger.clone(),
        drops: drops.clone(),
        _bytes: [0; 4],
    };
    let mut t = Transaction::begin(DeviceIds::new().unwrap(), &ledger, &[e], c).unwrap();
    let created = t.create_buffer(0, e, || Ok(BufferParts::new(make(), Some(make())))).unwrap();
    assert_eq!(ledger.used().0[2], 8);
    let logical = t.finish().unwrap().synthetic_retirement();
    assert_eq!(logical.allocation_count(), 2);
    let (primary, pending) = created.synthetic_unmap();
    let pending = pending.unwrap();
    let pending_records = pending.records();
    assert_ne!(pending_records[0].id(), pending_records[1].id());
    let submitted = pending.synthetic_submit();
    assert_eq!(submitted.records()[1].id(), pending_records[1].id());
    let derived = submitted.hold_staging([0u8; 3]);
    drop(primary);
    drop(logical);
    assert_eq!(ledger.used().0[2], 8);
    assert_eq!(drops.get(), 0);
    drop(submitted); // MODEL storage retirement, not actual queue completion
    assert_eq!(ledger.used().0[2], 4);
    assert_eq!(drops.get(), 1);
    let mut stage_only = Inventory::new(1, 100_000).unwrap();
    stage_only.record(derived.record(Holding::Retiring), c).unwrap();
    observe_all(&mut stage_only);
    assert_eq!(stage_only.payloads().unwrap().0, [0, 0, 4, 0]);
    drop(derived);
    assert_eq!(drops.get(), 2);
    assert_eq!(ledger.used(), Payloads::default());
    assert_eq!(ledger.preparation(), 0);
}

#[test]
fn initialized_companion_inventory_refuses_missing_live_stage_and_stale_snapshots_remain_conservative() {
    let (l, f) = (limits(), formats());
    let c = caps(&l, &f);
    let ledger = test_ledger(8);
    let e = initialized_entry(0);
    let mut t = Transaction::begin(DeviceIds::new().unwrap(), &ledger, &[e], c).unwrap();
    let made = t.create_buffer(0, e, || Ok(BufferParts::new([0u8; 4], Some([0u8; 4])))).unwrap();
    drop(t.finish().unwrap());
    let (primary, pending) = made.synthetic_unmap();
    let pending = pending.unwrap();
    let stale = Record::held(&primary, Holding::Resident);
    let mut incomplete = Inventory::new(2, 100_000).unwrap();
    incomplete.record(stale, c).unwrap();
    observe_all(&mut incomplete);
    assert_eq!(incomplete.payloads(), Err(Error::MissingDependencies));
    incomplete.record(pending.records()[1], c).unwrap();
    assert_eq!(incomplete.payloads().unwrap().0, [0, 0, 8, 0]);
    let retained_stage = pending.hold_staging([0u8; 1]);
    drop(retained_stage);
    drop(pending.synthetic_submit());
    assert_eq!(ledger.used().0[2], 4);
    let mut old = Inventory::new(1, 100_000).unwrap();
    old.record(stale, c).unwrap();
    observe_all(&mut old);
    assert_eq!(old.checked_records(), Err(Error::MissingDependencies));
    let mut fresh = Inventory::new(1, 100_000).unwrap();
    fresh.record(Record::held(&primary, Holding::Resident), c).unwrap();
    observe_all(&mut fresh);
    assert_eq!(fresh.payloads().unwrap().0, [0, 0, 4, 0]);
    drop(primary);
    assert_eq!(ledger.used(), Payloads::default());
    assert_eq!(ledger.preparation(), 0);
}

#[test]
fn initialized_factory_mismatch_missing_stage_error_and_unwind_drop_storage_before_credit() {
    let (l, f) = (limits(), formats());
    let c = caps(&l, &f);
    let e = initialized_entry(0);
    for mode in 0..3 {
        let ledger = test_ledger(8);
        let drops = Rc::new(Cell::new(0));
        let make = || InitStorage {
            ledger: ledger.clone(),
            drops: drops.clone(),
            _bytes: [0; 4],
        };
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut t = Transaction::begin(DeviceIds::new().unwrap(), &ledger, &[e], c).unwrap();
            let calls = Cell::new(0);
            assert!(matches!(
                t.create_buffer(0, initialized_entry(1), || {
                    calls.set(1);
                    Ok(BufferParts::new(make(), Some(make())))
                }),
                Err(Error::Invalid)
            ));
            assert_eq!(calls.get(), 0);
            let made = t.create_buffer(0, e, || {
                assert_eq!(ledger.used().0[2], 8);
                let primary = make();
                if mode == 0 {
                    return Ok(BufferParts::new(primary, None));
                }
                if mode == 1 {
                    return Err(Error::Allocation);
                }
                let _stage = make();
                panic!("synthetic mapped-init stage unwind");
            });
            assert!(made.is_err());
            assert!(matches!(t.finish(), Err(Error::MissingDependencies)));
        }));
        assert_eq!(outcome.is_err(), mode == 2);
        assert_eq!(drops.get(), if mode == 2 { 2 } else { 1 });
        assert_eq!(ledger.used(), Payloads::default());
        assert_eq!(ledger.preparation(), 0);
    }
    let ledger = test_ledger(8);
    let drops = Rc::new(Cell::new(0));
    let single = Entry {
        resource: Resource::Buffer(Buffer {
            size: 4,
            usage: B::UNIFORM,
            mapped: false,
        }),
        ..e
    };
    let make = || InitStorage {
        ledger: ledger.clone(),
        drops: drops.clone(),
        _bytes: [0; 4],
    };
    let mut t = Transaction::begin(DeviceIds::new().unwrap(), &ledger, &[single], c).unwrap();
    assert!(matches!(
        t.create_buffer(0, single, || Ok(BufferParts::new(make(), Some(make())))),
        Err(Error::MissingDependencies)
    ));
    assert_eq!(drops.get(), 2, "unexpected factory-owned storage drops before refund");
    assert!(matches!(t.finish(), Err(Error::MissingDependencies)));
    assert_eq!(ledger.used(), Payloads::default());
    assert_eq!(ledger.preparation(), 0);
}

#[test]
fn physical_id_index_keeps_ordered_reverse_interleaved_membership_bounded_and_source_record_order() {
    let (l, f) = (limits(), formats());
    let c = caps(&l, &f);
    let ledger = test_ledger(64);
    let allocator = DeviceIds::new().unwrap();
    let mut ids = Vec::with_capacity(4097);
    for _ in 0..4097 {
        let e = entry(0, Class::ViewTexture);
        let mut t = Transaction::begin(allocator.clone(), &ledger, &[e], c).unwrap();
        let lease = t.create(0, e, || Ok([0u8; 4])).unwrap();
        ids.push(lease.id());
        drop(t.finish().unwrap());
        drop(lease);
    }
    let absent = ids.pop().unwrap();
    assert!(matches!(super::index::IdIndex::new(4097, u64::MAX), Err(Error::Capacity)));
    assert!(matches!(super::index::IdIndex::new(4, 1), Err(Error::Capacity)));
    assert!(matches!(Inventory::new(8, 1), Err(Error::Capacity)));

    for order in 0..3 {
        let mut index = super::index::IdIndex::new(4096, 1_000_000).unwrap();
        let source = |position: usize| match order {
            0 => position,
            1 => 4095 - position,
            _ if position % 2 == 0 => position / 2,
            _ => 4095 - position / 2,
        };
        for position in 0..4096 {
            index.insert(ids[source(position)], position).unwrap();
        }
        for position in 0..4096 {
            let (found, steps) = index.probe(ids[source(position)]);
            assert_eq!(found, Some(position));
            assert!(steps <= 24, "fixed-stack bound at full capacity");
        }
        assert_eq!(index.position(absent), None);
        assert_eq!(index.insert(ids[100], 0), Err(Error::Invalid));
        assert_eq!(index.insert(absent, 4096), Err(Error::Capacity));
        assert_eq!(index.position(ids[source(17)]), Some(17));
        assert_eq!(index.position(absent), None);
    }
    let mut inventory = Inventory::new(8, 100_000).unwrap();
    let mut leases = Vec::new();
    let mut expected = Vec::new();
    for n in 0..8 {
        let e = entry(n, Class::ViewTexture);
        let mut t = Transaction::begin(allocator.clone(), &ledger, &[e], c).unwrap();
        let lease = t.create(0, e, || Ok([0u8; 4])).unwrap();
        drop(t.finish().unwrap());
        leases.push(lease);
    }
    for lease in leases.iter().rev() {
        let record = Record::held(lease, Holding::Resident);
        expected.push(record.id());
        inventory.record(record, c).unwrap();
    }
    observe_all(&mut inventory);
    assert_eq!(
        inventory.checked_records().unwrap().iter().map(|r| r.id()).collect::<Vec<_>>(),
        expected
    );
    drop(leases);
    assert_eq!(ledger.preparation(), 0);
}

#[test]
fn initialized_later_factory_failure_or_omission_preserves_independent_upload_components() {
    let (l, f) = (limits(), formats());
    let c = caps(&l, &f);
    for omit in [false, true] {
        let ledger = test_ledger(16);
        let drops = Rc::new(Cell::new(0));
        let make = || InitStorage {
            ledger: ledger.clone(),
            drops: drops.clone(),
            _bytes: [0; 4],
        };
        let entries = [initialized_entry(0), initialized_entry(1)];
        let mut t = Transaction::begin(DeviceIds::new().unwrap(), &ledger, &entries, c).unwrap();
        let first = t
            .create_buffer(0, entries[0], || Ok(BufferParts::new(make(), Some(make()))))
            .unwrap();
        let calls = Cell::new(0);
        assert!(matches!(
            t.create_buffer(0, entries[0], || {
                calls.set(1);
                Ok(BufferParts::new(make(), Some(make())))
            }),
            Err(Error::Invalid)
        ));
        assert_eq!(calls.get(), 0);
        let (primary, pending) = first.synthetic_unmap();
        let pending = pending.unwrap();
        if !omit {
            assert!(matches!(
                t.create_buffer(1, entries[1], || {
                    let _primary = make();
                    Err(Error::Allocation)
                }),
                Err(Error::Allocation)
            ));
        }
        assert!(matches!(t.finish(), Err(Error::MissingDependencies)));
        assert_eq!(ledger.used().0[2], 8);
        assert_eq!(drops.get(), usize::from(!omit));
        drop(primary);
        assert_eq!(ledger.used().0[2], 8);
        drop(pending);
        assert_eq!(drops.get(), 2 + usize::from(!omit));
        assert_eq!(ledger.used(), Payloads::default());
        assert_eq!(ledger.preparation(), 0);
    }
}
