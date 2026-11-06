use super::*;
use glam::{Mat4, Vec3, Vec4};

fn ready(now: Instant) -> (Owner, Admission) {
    let mut owner = Owner::default();
    let fence = SessionCompletion {
        ticket: 5,
        frame: Some(8),
        surface_epoch: 1,
    };
    owner.fence_requested(fence.ticket);
    owner.fence_consumed(fence);
    (
        owner,
        Admission {
            epoch: 1,
            fence,
            observed_at: now,
            standby: true,
            visible: true,
            vision_allowed: true,
            gpu_safe: true,
            extra_gpu_bytes: GPU_RESERVE_BYTES,
        },
    )
}
fn request(admission: Admission, sequence: u64) -> Request {
    Request {
        admission,
        sequence,
        size: Size::Small,
        rate: Rate::Five,
    }
}
fn frame(key: Key) -> Frame {
    Frame {
        epoch: key.epoch,
        sequence: key.sequence,
        size: Size::Small,
        rgba: vec![40; Size::Small.rgba_bytes()],
        elapsed: Duration::from_millis(5),
    }
}

#[test]
fn admission_requires_fresh_visible_standby_vision_gpu_budget_and_exact_consumed_fence() {
    let now = Instant::now();
    let (owner, a) = ready(now);
    assert_eq!(owner.check(&request(a, 1), now), Ok(()));
    for denied in [
        Admission { standby: false, ..a },
        Admission { visible: false, ..a },
        Admission {
            vision_allowed: false,
            ..a
        },
        Admission { gpu_safe: false, ..a },
        Admission { epoch: 0, ..a },
    ] {
        assert_eq!(owner.check(&request(denied, 1), now), Err(Error::NotAdmitted));
    }
    assert_eq!(
        owner.check(
            &request(
                Admission {
                    extra_gpu_bytes: GPU_RESERVE_BYTES - 1,
                    ..a
                },
                1
            ),
            now
        ),
        Err(Error::Budget)
    );
    assert_eq!(
        owner.check(&request(a, 1), now + ADMISSION_AGE + Duration::from_nanos(1)),
        Err(Error::Stale)
    );
    assert_eq!(
        owner.check(
            &request(
                Admission {
                    observed_at: now + Duration::from_nanos(1),
                    ..a
                },
                1
            ),
            now
        ),
        Err(Error::Stale)
    );
    assert_eq!(
        owner.check(
            &request(
                Admission {
                    fence: SessionCompletion { ticket: 4, ..a.fence },
                    ..a
                },
                1
            ),
            now
        ),
        Err(Error::NotAdmitted)
    );
}

#[test]
fn normal_work_or_a_later_fence_invalidates_old_preview_authority() {
    let now = Instant::now();
    let (mut owner, a) = ready(now);
    owner.invalidate();
    assert_eq!(owner.authorize(&a, now), Err(Error::NotAdmitted));
    owner.fence_requested(6);
    owner.invalidate();
    owner.fence_consumed(SessionCompletion { ticket: 6, ..a.fence });
    assert!(owner.ready.is_none(), "work after a fence request cannot borrow its completion");
    owner.fence_requested(7);
    let proof = SessionCompletion { ticket: 7, ..a.fence };
    owner.fence_consumed(proof);
    assert_eq!(owner.authorize(&Admission { fence: proof, ..a }, now), Ok(()));
    owner.revision = u64::MAX;
    owner.invalidate();
    owner.fence_requested(8);
    owner.fence_consumed(SessionCompletion { ticket: 8, ..a.fence });
    assert_eq!(
        owner.authorize(
            &Admission {
                fence: SessionCompletion { ticket: 8, ..a.fence },
                ..a
            },
            now
        ),
        Err(Error::NotAdmitted)
    );
}

#[test]
fn initialization_needs_new_fresh_admission_without_consuming_frame_sequence() {
    let now = Instant::now();
    let (mut owner, a) = ready(now);
    owner.initializing();
    assert_eq!(owner.check(&request(a, 1), now), Err(Error::Busy));
    owner.receive(Reply::Warming);
    assert!(matches!(owner.next_job(), Some(Job::WarmPoll)));
    owner.receive(Reply::Warmed);
    let later = now + Duration::from_secs(2);
    assert_eq!(owner.check(&request(a, 1), later), Err(Error::Stale));
    assert_eq!(owner.check(&request(Admission { observed_at: later, ..a }, 1), later), Ok(()));
    assert_eq!(owner.sequence, 0);
}

#[test]
fn busy_requests_drop_and_only_the_latest_completed_frame_is_kept() {
    let now = Instant::now();
    let (mut owner, a) = ready(now);
    let first = request(a, 1);
    owner.submitted(&first, now);
    let key = owner.key(&first);
    assert_eq!(owner.check(&request(a, 2), now), Err(Error::Busy));
    owner.receive(Reply::Submitted(key, Box::default()));
    owner.receive(Reply::Pending(key));
    assert_eq!(owner.check(&request(a, 2), now + Duration::from_millis(100)), Err(Error::Busy));
    owner.receive(Reply::Ready(key, frame(key)));
    assert_eq!(
        owner.check(&request(Admission { epoch: 2, ..a }, 1), now + Duration::from_millis(100)),
        Err(Error::Busy),
        "policy changes cannot bypass physical rate limits"
    );
    let later = now + Duration::from_millis(200);
    let b = Admission { observed_at: later, ..a };
    let second = request(b, 2);
    assert_eq!(owner.check(&second, later), Ok(()));
    owner.submitted(&second, later);
    let key2 = owner.key(&second);
    owner.receive(Reply::Ready(key2, frame(key2)));
    assert_eq!(owner.take(&b, later).unwrap().unwrap().sequence, 2);
    assert!(owner.take(&b, later).unwrap().is_none());
    assert_eq!(owner.check(&second, later), Err(Error::Stale));
}

#[test]
fn cancellation_drops_late_frames_and_slow_gpu_readbacks_require_readmission() {
    let now = Instant::now();
    let (mut owner, a) = ready(now);
    let r = request(a, 1);
    owner.submitted(&r, now);
    let key = owner.key(&r);
    owner.invalidate();
    owner.receive(Reply::Ready(key, frame(key)));
    assert!(owner.latest.is_none());
    assert!(matches!(owner.next_job(), Some(Job::Release)));
    let (mut owner, a) = ready(now);
    let r = request(a, 1);
    owner.submitted(&r, now);
    let key = owner.key(&r);
    let mut slow = frame(key);
    slow.elapsed = ADMISSION_AGE + Duration::from_nanos(1);
    owner.receive(Reply::Ready(key, slow));
    assert_eq!(owner.take(&a, now).unwrap_err(), Error::Budget);
    assert!(owner.ready.is_none());
    assert!(owner.latest.is_none());
}

#[test]
fn sizes_rates_and_physical_headroom_are_bounded_and_unknown_is_denied() {
    assert_eq!(
        [Size::Small.dimensions(), Size::Medium.dimensions(), Size::Large.dimensions()],
        [(320, 180), (480, 270), (640, 360)]
    );
    assert_eq!(
        [
            Rate::Half.interval().as_millis(),
            Rate::One.interval().as_millis(),
            Rate::Two.interval().as_millis(),
            Rate::Five.interval().as_millis()
        ],
        [2000, 1000, 500, 200]
    );
    let m = crate::vram::VideoMemory {
        budget: 2 << 30,
        usage: 1 << 30,
        allocated: 1 << 30,
    };
    assert!(memory_safe(Some(m)));
    assert!(!memory_safe(None));
    assert!(!memory_safe(Some(crate::vram::VideoMemory { usage: m.budget, ..m })));
    assert!(!memory_safe(Some(crate::vram::VideoMemory { allocated: m.budget, ..m })));
    let low = settings();
    assert_eq!(low.aa, AntiAliasing::None);
    assert_eq!(low.shadow_resolution, 0);
    assert_eq!(low.probe_slots, 1);
    assert!(!low.occlusion);
}

#[test]
fn rgba_rows_drop_alignment_padding_and_bgra_is_swizzled_without_edge_stripes() {
    for size in [Size::Small, Size::Medium, Size::Large] {
        let (w, h) = size.dimensions();
        let row = row_pitch(w) as usize;
        let mut bytes = vec![0xee; row * h as usize];
        for y in 0..h as usize {
            for x in 0..w as usize {
                bytes[y * row + x * 4..y * row + x * 4 + 4].copy_from_slice(&[x as u8, y as u8, 7, 23]);
            }
        }
        let rgba = unpack(&bytes, size, wgpu::TextureFormat::Rgba8Unorm).unwrap();
        let bgra = unpack(&bytes, size, wgpu::TextureFormat::Bgra8Unorm).unwrap();
        assert_eq!(rgba.len(), size.rgba_bytes());
        for (index, pixel) in rgba.as_chunks::<4>().0.iter().enumerate() {
            let x = index % w as usize;
            let y = index / w as usize;
            assert_eq!(*pixel, [x as u8, y as u8, 7, 255]);
            assert_eq!(bgra[index * 4..index * 4 + 4], [7, y as u8, x as u8, 255]);
        }
        assert!(unpack(&bytes[..bytes.len() - 1], size, wgpu::TextureFormat::Rgba8Unorm).is_none());
    }
}

#[test]
fn oversized_secondary_world_lists_are_rejected_before_any_clone() {
    let mut lists = DrawLists::default();
    assert_eq!(lists.total(), 0);
    assert!(lists_ready(&lists));
    lists.blend_glow = vec![false; MAX_SCENE_SLOTS + 1];
    assert!(!lists_ready(&lists));
    lists.blend_glow.clear();
    lists.blend_double_sided = vec![false; MAX_SCENE_SLOTS + 1];
    assert!(!lists_ready(&lists));
    lists.blend_double_sided.clear();
    lists.blend_under_double_sided = vec![false; MAX_SCENE_SLOTS + 1];
    assert!(!lists_ready(&lists));
    lists.blend_under_double_sided.clear();
    lists.blend_depth_runs = vec![0; MAX_SCENE_SLOTS + 1];
    assert!(!lists_ready(&lists));
    lists.blend_depth_runs.clear();
    lists.water_ground.heights = vec![[0.0; 2]; 4097].into();
    assert!(!lists_ready(&lists));
    lists.water_ground.heights = vec![[0.0; 2]; 4096].into();
    assert!(lists_ready(&lists));
}

fn params() -> FrameParams {
    FrameParams {
        world_viewport: Some(crate::WorldViewport {
            x: 200,
            y: 100,
            width: 1600,
            height: 900,
        }),
        rlva_spheres: vec![crate::SphereEffect {
            origin: Vec3::ONE,
            mode: 1,
            extend: 1,
            params: [1.0; 4],
            range: [2.0; 4],
        }],
        rlva_resolution_divisor: 2,
        rlva_overlays: vec![crate::OverlayEffect {
            color: [0.0, 0.0, 0.0, 1.0],
            texture: [7, 0, 0, 0],
        }],
        selection_color: [1.0; 4],
        rez_gradient: [[1.0; 4]; 3],
        selection_pulse: 1.0,
        selection_scale: 1.5,
        view: Mat4::IDENTITY,
        proj: glam::camera::rh::proj::directx::perspective_infinite_reverse(1.0, 2.0, 0.1),
        photo_tile: Mat4::IDENTITY,
        photo_scale: 1.0,
        photo_context: PhotoContext::None,
        camera_pos: Vec3::ZERO,
        near: 0.1,
        far: 256.0,
        fov_y: 1.0,
        time: 0.0,
        anim_at: None,
        tex_anim_at: None,
        sun_dir: Vec3::Z,
        sun_visible: 1.0,
        sun_color: Vec3::ONE,
        sky_zenith: Vec3::ONE,
        sky_horizon: Vec3::ONE,
        ground_color: Vec3::ONE,
        fog_color: Vec3::ONE,
        fog_density: 0.0,
        exposure: 1.0,
        exposure_lock: false,
        tonemap_mix: 0.7,
        sharpen: 0.0,
        glow: true,
        bloom: 1.0,
        water_height: 0.0,
        shadows: true,
        shadow_distance: 128.0,
        lights: Vec::new(),
        spot_shadows: [None; crate::projector_shadows::MAX_PROJECTOR_SHADOWS],
        clear_color: Vec4::ONE,
        sky: Default::default(),
        water: Default::default(),
        mirror: None,
        probes: crate::ProbeFrame {
            enabled: true,
            ..Default::default()
        },
        wireframe: true,
        debug_glow: true,
        blend_late_discard: false,
        all_lights: false,
        probe_walk_list: false,
        debug: Default::default(),
        dof: None,
        cutout: false,
        look: None,
    }
}

#[test]
fn small_world_strips_ui_hud_and_capture_work_but_preserves_vision_and_real_geometry() {
    let mut p = params();
    let original = p.clone();
    let cmd = crate::DrawCmd {
        index_count: 6,
        first_index: 0,
        base_vertex: 0,
        record: 1,
        bounds: [0.0; 4],
    };
    let mut lists = DrawLists::default();
    lists.opaque.push(cmd);
    lists.hud_opaque.push(cmd);
    lists.select_faces.push(cmd);
    lists.physics_shapes.push(cmd);
    lists.probe.push(crate::ShadowCaster {
        cmd,
        center: Vec3::ZERO,
        radius: 1.0,
        alpha: false,
    });
    let copied = world_lists(&lists);
    assert_eq!(copied.opaque.len(), 1);
    assert!(copied.hud_opaque.is_empty());
    assert!(copied.probe.is_empty());
    strip(&mut p, &mut lists, Size::Medium).unwrap();
    assert_eq!(lists.opaque.len(), 1);
    assert!(lists.hud_opaque.is_empty());
    assert!(lists.select_faces.is_empty());
    assert!(lists.physics_shapes.is_empty());
    assert!(lists.probe.is_empty());
    assert_eq!(p.world_viewport, None);
    assert_eq!(p.rlva_resolution_divisor, 2);
    assert_eq!(p.rlva_overlays[0].texture, [7, 0, 0, 0]);
    assert_eq!(p.rlva_spheres[0].origin, original.rlva_spheres[0].origin);
    assert_eq!(
        p.proj,
        glam::camera::rh::proj::directx::perspective_infinite_reverse(1.0, 16.0 / 9.0, 0.1)
    );
    assert!(!p.probes.enabled);
    assert!(!p.glow);
    assert!(!p.shadows);
    p.photo_tile = Mat4::from_scale(Vec3::splat(2.0));
    assert_eq!(strip(&mut p, &mut lists, Size::Small), Err(Error::SceneNotReady));
}

#[test]
#[ignore]
fn small_preview_targets_async_gpu_readback_matches_exact_size_and_rgba() {
    let _gpu = crate::test_gpu::lock();
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = crate::BACKENDS;
    descriptor.flags = wgpu::InstanceFlags::debugging();
    let instance = wgpu::Instance::new(descriptor);
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
        eprintln!("no GPU adapter: skipped");
        return;
    };
    eprintln!("preview GPU helper adapter: {:?}", adapter.get_info());
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("GPU device");
    for size in [Size::Small, Size::Medium, Size::Large] {
        for format in [wgpu::TextureFormat::Rgba8Unorm, wgpu::TextureFormat::Bgra8Unorm] {
            let (output, readback) = View::targets(&device, format, size);
            let (width, height) = size.dimensions();
            assert_eq!((output.width(), output.height()), (width, height));
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            {
                let view = output.create_view(&Default::default());
                let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 0.25,
                                g: 0.5,
                                b: 0.75,
                                a: 1.0,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
            }
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &output,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &readback,
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
            queue.submit([encoder.finish()]);
            let answer = Arc::new(AtomicU8::new(0));
            let mapped = answer.clone();
            readback.map_async(wgpu::MapMode::Read, .., move |r| {
                mapped.store(if r.is_ok() { 1 } else { 2 }, Ordering::Release)
            });
            let deadline = Instant::now() + Duration::from_secs(10);
            while answer.load(Ordering::Acquire) == 0 && Instant::now() < deadline {
                device.poll(wgpu::PollType::Poll).expect("GPU poll");
                std::thread::sleep(Duration::from_millis(2));
            }
            assert_eq!(answer.load(Ordering::Acquire), 1, "bounded asynchronous map");
            let bytes = readback.get_mapped_range(..).expect("mapped preview");
            let rgba = unpack(&bytes, size, format).expect("tightly packed RGBA");
            assert_eq!(rgba.len(), size.rgba_bytes());
            assert!(rgba.as_chunks::<4>().0.iter().all(|p| (i32::from(p[0]) - 64).abs() <= 1
                && (i32::from(p[1]) - 128).abs() <= 1
                && (i32::from(p[2]) - 191).abs() <= 1
                && p[3] == 255));
            drop(bytes);
            readback.unmap();
        }
    }
}
