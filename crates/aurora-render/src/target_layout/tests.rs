use super::generations::TargetBudget;
use super::generations::TargetGenerations;
use super::layout::{Caps, CoreTargetLayout, FormatSupport, Input, Key, Missing, TextureSpec};
use super::*;
use std::cell::RefCell;
use std::rc::Rc;
use wgpu::{TextureFormat as Format, TextureUsages as Usage};

pub(super) fn caps() -> Caps {
    let formats = [
        Format::R8Unorm,
        Format::Rg8Unorm,
        Format::R16Float,
        Format::Rgba8Unorm,
        Format::R32Float,
        Format::R32Uint,
        Format::Depth32Float,
        Format::Rgba16Float,
    ];
    Caps {
        max_dimension_2d: 16384,
        max_array_layers: 2048,
        formats: formats.map(|format| FormatSupport {
            format,
            features: wgpu::TextureFormatFeatures {
                allowed_usages: Usage::RENDER_ATTACHMENT | Usage::TEXTURE_BINDING | Usage::COPY_SRC | Usage::COPY_DST,
                flags: wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X2
                    | wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X4
                    | wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X8
                    | wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X16,
            },
        }),
    }
}
pub(super) fn input(scene: [u32; 2]) -> Input {
    Input {
        scene,
        samples: 1,
        ssr: false,
        ao_enabled: false,
        legacy_ao: false,
        gtao: false,
        contact: false,
        reflection_scales: [0.0; 3],
    }
}
pub(super) fn layout(scene: [u32; 2]) -> CoreTargetLayout {
    CoreTargetLayout::checked(input(scene), &caps()).unwrap()
}
fn get(layout: &CoreTargetLayout, key: Key) -> TextureSpec {
    *layout.specs().find(|spec| spec.key == key).unwrap()
}

#[test]
fn core_targets_caps_and_every_supported_sample_count_match_independent_payload_math() {
    for [w, h] in [[320, 180], [480, 270], [640, 360]] {
        let base = layout([w, h]);
        let n = u64::from(w) * u64::from(h);
        assert_eq!(base.accounted_payload(), PayloadCost(25 * n));
        assert_eq!(base.specs().count(), 5);
        assert_eq!(base.missing(), Missing::default());
        assert_eq!(base.reservable_core_payload(), Ok(PayloadCost(25 * n)));
        for samples in [1, 2, 4, 8, 16] {
            let mut args = input([w, h]);
            args.samples = samples;
            args.ssr = true;
            args.ao_enabled = true;
            args.legacy_ao = true;
            let p = CoreTargetLayout::checked(args, &caps()).unwrap();
            // Resolved base25N, optional multisampled HDR/depth12*s*N,
            // Gbuffer4N plus MSAA4*s*N, AO depth4N plus two R8 half.
            let msaa = if samples > 1 { 16 * u64::from(samples) * n } else { 0 };
            let half = u64::from(w.div_ceil(2)) * u64::from(h.div_ceil(2));
            assert_eq!(p.accounted_payload(), PayloadCost(33 * n + msaa + 2 * half));
            let depth = get(&p, Key::AoDepth);
            assert_eq!(
                depth.usage,
                Usage::RENDER_ATTACHMENT | Usage::TEXTURE_BINDING | Usage::COPY_DST | Usage::COPY_SRC
            );
            assert_eq!(get(&p, Key::SceneCopy).usage, Usage::TEXTURE_BINDING | Usage::COPY_DST);
            if samples > 1 {
                assert_eq!(get(&p, Key::ColorMsaa).samples, samples);
                assert_eq!(
                    get(&p, Key::DepthMsaa).usage,
                    Usage::RENDER_ATTACHMENT | Usage::TEXTURE_BINDING | Usage::COPY_SRC
                );
            }
        }
    }
}

#[test]
fn core_targets_preserve_odd_portrait_normalization_reflection_rounding_and_maximum18() {
    for [w, h] in [[0, 0], [1, 1], [181, 321], [319, 179]] {
        let mut args = input([w, h]);
        args.ao_enabled = true;
        args.legacy_ao = true;
        args.reflection_scales = [0.01, 0.75, 2.0];
        let p = CoreTargetLayout::checked(args, &caps()).unwrap();
        let [nw, nh] = [w.max(1), h.max(1)];
        assert_eq!(get(&p, Key::Color).size, [nw, nh, 1]);
        assert_eq!(get(&p, Key::Ao).size, [nw.div_ceil(2), nh.div_ceil(2), 1]);
        let air = [((nw as f32 * 0.75) as u32).max(16), ((nh as f32 * 0.75) as u32).max(16), 1];
        assert_eq!(get(&p, Key::WaterAirColor).size, air);
        assert_eq!(get(&p, Key::MirrorColor).size, [nw.max(16), nh.max(16), 1]);
        assert_eq!(get(&p, Key::WaterReflectionDepth).usage, Usage::RENDER_ATTACHMENT);
        assert_eq!(get(&p, Key::WaterAirDepth).usage, Usage::RENDER_ATTACHMENT | Usage::TEXTURE_BINDING);
        assert_eq!(get(&p, Key::WaterReflectionColor).size, [16, 16, 1]);
        let expected = 29 * u64::from(nw) * u64::from(nh)
            + 2 * u64::from(nw.div_ceil(2)) * u64::from(nh.div_ceil(2))
            + 12 * (16 * 16 + u64::from(air[0]) * u64::from(air[1]) + u64::from(nw.max(16)) * u64::from(nh.max(16)));
        assert_eq!(p.accounted_payload(), PayloadCost(expected));
    }
    let mut args = input([640, 360]);
    args.samples = 16;
    args.ssr = true;
    args.ao_enabled = true;
    args.legacy_ao = true;
    args.gtao = true;
    args.contact = true;
    args.reflection_scales = [1.0, 0.75, 1.0];
    let p = CoreTargetLayout::checked(args, &caps()).unwrap();
    assert_eq!(p.specs().count(), 18);
    assert_eq!(p.accounted_payload(), PayloadCost(73_785_600));
    let mut keys = Vec::new();
    for spec in p.specs() {
        assert!(!keys.contains(&spec.key));
        keys.push(spec.key);
    }
    assert_eq!(p.missing(), Missing { gtao: true, contact: true });
    assert_eq!(p.reservable_core_payload(), Err(Error::MissingDependencies));
}

#[test]
fn core_targets_missing_nested_components_never_create_core_budget_reservation_or_stale_fallback() {
    let mut a = input([320, 180]);
    a.ao_enabled = true;
    a.gtao = true;
    let p = CoreTargetLayout::checked(a, &caps()).unwrap();
    assert_eq!(p.accounted_payload(), PayloadCost(29 * 320 * 180)); // shared AO depth counted once
    assert_eq!(p.reservable_core_payload(), Err(Error::MissingDependencies));
    a.legacy_ao = true;
    let alternate = CoreTargetLayout::checked(a, &caps()).unwrap();
    assert_eq!(alternate.accounted_payload(), PayloadCost(29 * 320 * 180 + 2 * 160 * 90));
    a.ao_enabled = false;
    a.contact = true;
    let contact = CoreTargetLayout::checked(a, &caps()).unwrap();
    assert_eq!(
        contact.missing(),
        Missing {
            gtao: false,
            contact: true
        }
    );
    let budget = TargetBudget::new(10_000).unwrap();
    let mut owner = TargetGenerations::<()>::new(budget.clone()).unwrap();
    owner.request(layout([1, 1])).unwrap();
    assert_eq!(owner.request(p), Err(Error::MissingDependencies));
    assert!(matches!(
        owner.admit(|_| panic!("invalid wanted must never allocate")),
        Err(Error::NoRequest)
    ));
    assert_eq!(budget.used(), 0);
}

#[test]
fn texture_payload_checks_mips_layers_samples_formats_limits_and_overflow_without_zero_cost() {
    let c = caps();
    let spec = TextureSpec {
        key: Key::Color,
        size: [8, 4, 3],
        mips: 4,
        samples: 1,
        format: Format::Rgba16Float,
        usage: Usage::TEXTURE_BINDING,
    };
    // 8*4 +4*2 +2*1 +1*1 =43 texels per layer, three layers,8bytes.
    assert_eq!(spec.payload(&c), Ok(PayloadCost(43 * 3 * 8)));
    for (format, bytes) in [
        (Format::R8Unorm, 1),
        (Format::Rg8Unorm, 2),
        (Format::R16Float, 2),
        (Format::Rgba8Unorm, 4),
        (Format::R32Float, 4),
        (Format::R32Uint, 4),
        (Format::Depth32Float, 4),
        (Format::Rgba16Float, 8),
    ] {
        assert_eq!(
            TextureSpec {
                size: [4, 2, 1],
                mips: 1,
                format,
                ..spec
            }
            .payload(&c),
            Ok(PayloadCost(8 * bytes))
        );
    }
    assert_eq!(
        TextureSpec {
            usage: Usage::empty(),
            ..spec
        }
        .payload(&c),
        Err(Error::Invalid)
    );
    let mut duplicate = caps();
    duplicate.formats[0].format = Format::Rgba16Float;
    assert_eq!(spec.payload(&duplicate), Err(Error::Limits));
    for bad in [
        TextureSpec { mips: 0, ..spec },
        TextureSpec { mips: 5, ..spec },
        TextureSpec { samples: 4, ..spec },
        TextureSpec { size: [0, 4, 1], ..spec },
        TextureSpec { samples: 3, ..spec },
    ] {
        assert_eq!(bad.payload(&c), Err(Error::Invalid));
    }
    assert_eq!(
        TextureSpec {
            format: Format::Depth24Plus,
            ..spec
        }
        .payload(&c),
        Err(Error::Format)
    );
    assert_eq!(
        TextureSpec {
            size: [16385, 1, 1],
            mips: 1,
            ..spec
        }
        .payload(&c),
        Err(Error::Limits)
    );
    assert_eq!(
        TextureSpec {
            usage: Usage::STORAGE_BINDING,
            ..spec
        }
        .payload(&c),
        Err(Error::Limits)
    );
    let mut low = caps();
    low.formats[7].features.flags = wgpu::TextureFormatFeatureFlags::empty();
    assert_eq!(
        TextureSpec {
            size: [8, 4, 1],
            mips: 1,
            samples: 4,
            ..spec
        }
        .payload(&low),
        Err(Error::Limits)
    );
    let mut huge = caps();
    huge.max_dimension_2d = u32::MAX;
    huge.max_array_layers = u32::MAX;
    assert_eq!(
        TextureSpec {
            size: [u32::MAX, u32::MAX, u32::MAX],
            mips: 1,
            ..spec
        }
        .payload(&huge),
        Err(Error::Overflow)
    );
    let mut sum = input([1 << 31, 1 << 29]);
    sum.samples = 1;
    assert_eq!(CoreTargetLayout::checked(sum, &huge), Err(Error::Overflow));
    for bad in [f32::NAN, f32::INFINITY] {
        let mut args = input([320, 180]);
        args.reflection_scales[0] = bad;
        assert_eq!(CoreTargetLayout::checked(args, &c), Err(Error::Invalid));
    }
}

#[derive(Default)]
struct Counts {
    made: usize,
    live: usize,
    peak: usize,
    dropped: Vec<(usize, u64)>,
}
struct Resource {
    id: usize,
    pixels: Vec<u8>,
    counts: Rc<RefCell<Counts>>,
    budget: TargetBudget,
}
impl Drop for Resource {
    fn drop(&mut self) {
        let mut c = self.counts.borrow_mut();
        c.live -= 1;
        c.dropped.push((self.id, self.budget.used()));
    }
}
fn resource(layout: &CoreTargetLayout, budget: &TargetBudget, counts: &Rc<RefCell<Counts>>) -> Resource {
    let bytes = layout.reservable_core_payload().unwrap().0;
    assert!(budget.used() >= bytes, "reserve before actual byte-buffer allocation");
    let id = {
        let mut c = counts.borrow_mut();
        let id = c.made;
        c.made += 1;
        c.live += 1;
        c.peak = c.peak.max(c.live);
        id
    };
    let pixels = vec![id as u8; bytes as usize];
    assert_eq!(pixels.len() as u64, bytes);
    Resource {
        id,
        pixels,
        counts: counts.clone(),
        budget: budget.clone(),
    }
}

#[test]
fn target_generations_matching_reuse_third_generation_busy_and_latest_request_coalescing() {
    let budget = TargetBudget::new(1000).unwrap();
    let counts = Rc::new(RefCell::new(Counts::default()));
    let mut owner = TargetGenerations::new(budget.clone()).unwrap();
    let a = layout([1, 1]);
    let b = layout([2, 1]);
    let c = layout([3, 1]);
    let d = layout([4, 1]);
    owner.request(a).unwrap();
    let first = owner.admit(|p| Ok(resource(p, &budget, &counts))).unwrap();
    assert!(first.retirement.is_none());
    let id = first.lease.id();
    let repeated = owner.admit(|_| panic!("same key must not allocate")).unwrap();
    assert_eq!(repeated.lease.id(), id);
    assert_eq!(counts.borrow().made, 1);
    drop(repeated);
    drop(first);
    owner.request(b).unwrap();
    let second = owner.admit(|p| Ok(resource(p, &budget, &counts))).unwrap();
    let retirement = second.retirement.unwrap();
    assert_eq!(retirement.id(), id);
    assert_ne!(second.lease.id(), id);
    assert_eq!(budget.used(), 75);
    owner.request(c).unwrap();
    assert!(matches!(
        owner.admit(|_| panic!("third generation must not allocate")),
        Err(Error::Busy)
    ));
    owner.request(d).unwrap();
    assert!(matches!(owner.admit(|_| panic!("latest metadata still waits")), Err(Error::Busy)));
    assert_eq!(counts.borrow().made, 2);
    owner.cancel();
    owner.poll();
    assert_eq!(budget.used(), 75, "cancel cannot retire unfinished work");
    assert!(matches!(
        owner.admit(|_| panic!("canceled wanted must not allocate")),
        Err(Error::NoRequest)
    ));
    owner.request(d).unwrap();
    retirement.complete();
    assert_eq!(budget.used(), 75, "completion signal alone does not evict a bundle before poll");
    owner.poll();
    assert_eq!(budget.used(), 50);
    let third = owner.admit(|p| Ok(resource(p, &budget, &counts))).unwrap();
    assert_eq!(budget.used(), 150, "latest100byte bundle replaces25/75byte pending metadata");
    assert_eq!(counts.borrow().made, 3);
    assert_eq!(counts.borrow().peak, 2);
    drop(second.lease);
    third.retirement.unwrap().complete();
    owner.poll();
    assert_eq!(budget.used(), 100);
    owner.cancel();
    assert!(matches!(owner.admit(|_| panic!("cancel has no allocation")), Err(Error::NoRequest)));
    drop(third.lease);
    drop(owner);
    assert_eq!(budget.used(), 0);
    assert_eq!(counts.borrow().live, 0);
}

#[test]
fn target_generations_completion_waits_for_handles_and_duplicate_callbacks_do_not_free_newer_work() {
    let budget = TargetBudget::new(1000).unwrap();
    let counts = Rc::new(RefCell::new(Counts::default()));
    let mut owner = TargetGenerations::new(budget.clone()).unwrap();
    owner.request(layout([1, 1])).unwrap();
    let first = owner.admit(|p| Ok(resource(p, &budget, &counts))).unwrap();
    let old_handle = first.lease.clone();
    drop(first);
    owner.request(layout([2, 1])).unwrap();
    let second = owner.admit(|p| Ok(resource(p, &budget, &counts))).unwrap();
    let callback = second.retirement.unwrap();
    let duplicate = callback.clone();
    callback.complete();
    owner.poll();
    assert_eq!(budget.used(), 75);
    drop(old_handle);
    owner.poll();
    assert_eq!(budget.used(), 75, "duplicate callback retains physical lease");
    duplicate.complete();
    owner.poll();
    assert_eq!(budget.used(), 50);
    owner.request(layout([3, 1])).unwrap();
    let third = owner.admit(|p| Ok(resource(p, &budget, &counts))).unwrap();
    owner.poll();
    assert_eq!(budget.used(), 125, "old callback cannot complete newer retirement");
    drop(second.lease);
    third.retirement.unwrap().complete();
    owner.poll();
    assert_eq!(budget.used(), 75);
    drop(third.lease);
    drop(owner);
    assert_eq!(budget.used(), 0);
}

#[test]
fn target_generations_failed_or_panicking_factory_rolls_back_new_credit_keeps_old_active() {
    let budget = TargetBudget::new(100).unwrap();
    let counts = Rc::new(RefCell::new(Counts::default()));
    let mut owner = TargetGenerations::new(budget.clone()).unwrap();
    owner.request(layout([1, 1])).unwrap();
    let first = owner.admit(|p| Ok(resource(p, &budget, &counts))).unwrap();
    let id = first.lease.id();
    owner.request(layout([4, 1])).unwrap();
    assert!(matches!(
        owner.admit(|_| panic!("old+new exceeds explicit100byte pool")),
        Err(Error::Capacity)
    ));
    assert_eq!(budget.used(), 25);
    owner.request(layout([2, 1])).unwrap();
    assert!(matches!(
        owner.admit(|p| {
            let partial = resource(p, &budget, &counts);
            assert_eq!(partial.pixels.len(), 50);
            drop(partial);
            Err(Error::Allocation)
        }),
        Err(Error::Allocation)
    ));
    assert_eq!(budget.used(), 25);
    assert_eq!(counts.borrow().dropped[0], (1, 75));
    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = owner.admit(|p| {
            let _partial = resource(p, &budget, &counts);
            panic!("synthetic construction unwind")
        });
    }));
    assert!(failed.is_err());
    assert_eq!(budget.used(), 25);
    owner.request(layout([1, 1])).unwrap();
    let reused = owner.admit(|_| panic!("failed construction must keep original")).unwrap();
    assert_eq!(reused.lease.id(), id);
    drop(reused);
    drop(first);
    drop(owner);
    assert_eq!(budget.used(), 0);
}

#[test]
fn target_generations_old_handles_and_callbacks_keep_credit_after_teardown_against_replacement_owner() {
    for limit in [100, 125] {
        let budget = TargetBudget::new(limit).unwrap();
        let counts = Rc::new(RefCell::new(Counts::default()));
        let mut owner = TargetGenerations::new(budget.clone()).unwrap();
        owner.request(layout([1, 1])).unwrap();
        let old = owner.admit(|p| Ok(resource(p, &budget, &counts))).unwrap();
        let old_id = old.lease.id();
        owner.request(layout([2, 1])).unwrap();
        let second = owner.admit(|p| Ok(resource(p, &budget, &counts))).unwrap();
        drop(second.lease);
        drop(owner);
        assert_eq!(budget.used(), 25, "old callback+handle keep first physical bundle");
        let mut replacement = TargetGenerations::new(budget.clone()).unwrap();
        replacement.request(layout([4, 1])).unwrap();
        let admitted = if limit == 100 {
            assert!(matches!(
                replacement.admit(|_| panic!("new owner must share retained credit")),
                Err(Error::Capacity)
            ));
            None
        } else {
            let new = replacement.admit(|p| Ok(resource(p, &budget, &counts))).unwrap();
            assert_eq!(budget.used(), 125);
            Some(new)
        };
        drop(old.lease);
        second.retirement.unwrap().complete();
        assert_eq!(
            budget.used(),
            if admitted.is_some() { 100 } else { 0 },
            "late old-owner completion cannot free the replacement bundle"
        );
        let new = match admitted {
            Some(new) => new,
            None => replacement.admit(|p| Ok(resource(p, &budget, &counts))).unwrap(),
        };
        assert_ne!(new.lease.id(), old_id);
        let reuse = replacement
            .admit(|_| panic!("late old callback cannot replace current allocation"))
            .unwrap();
        assert_eq!(reuse.lease.id(), new.lease.id());
        drop(reuse);
        assert_eq!(budget.used(), 100);
        drop(new.lease);
        drop(replacement);
        assert_eq!(budget.used(), 0);
        assert_eq!(counts.borrow().live, 0);
    }
}
