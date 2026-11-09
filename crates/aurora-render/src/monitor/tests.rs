use super::*;

fn config(size: CaptureCap, rate: Rate) -> Config {
    Config { size, rate }
}

#[test]
fn monitor_cadences_drop_missed_ticks_without_catchup() {
    for (rate, millis) in [(Rate::Half, 2000), (Rate::One, 1000), (Rate::Two, 500), (Rate::Five, 200)] {
        let mut lane = Lane::<u32>::default();
        let start = Instant::now();
        lane.configure(Some(config(CaptureCap::Small, rate)));
        let (_, _, done) = lane.begin(start).unwrap();
        assert!(
            lane.begin(start + Duration::from_secs(60)).is_none(),
            "one in flight even when overdue"
        );
        done.finish(Some(1));
        lane.poll();
        assert!(lane.begin(start + Duration::from_millis(millis - 1)).is_none());
        let late = start + Duration::from_secs(60);
        let (_, _, done) = lane.begin(late).unwrap();
        done.finish(Some(2));
        lane.poll();
        assert!(lane.begin(late).is_none(), "never replay missed ticks");
        assert_eq!(lane.take_latest(), Some(2), "slow readers receive only newest completion");
        assert!(lane.take_latest().is_none());
    }
}

#[test]
fn monitor_hidden_minimized_closed_and_promotion_keep_drain_reservation() {
    for _reason in ["hidden", "minimized", "closed", "promoting", "permission revoked"] {
        let mut lane = Lane::<u32>::default();
        let start = Instant::now();
        let cfg = config(CaptureCap::Small, Rate::Five);
        lane.configure(Some(cfg));
        let (_, epoch, done) = lane.begin(start).unwrap();
        lane.retire();
        assert!(!epoch.current());
        assert!(lane.due(start + Duration::from_secs(60)).is_none());
        lane.configure(Some(cfg));
        assert!(
            lane.begin(start + Duration::from_secs(60)).is_none(),
            "reopen cannot erase GPU reservation"
        );
        done.finish(Some(99));
        lane.poll();
        assert_eq!(lane.take_latest(), None, "late old completion cannot revive image");
        assert!(lane.begin(start + Duration::from_secs(60)).is_some());
    }
}

#[test]
fn monitor_resize_invalidates_old_frames_and_does_not_accelerate_rate() {
    let mut lane = Lane::<u32>::default();
    let start = Instant::now();
    lane.configure(Some(config(CaptureCap::Large, Rate::Half)));
    let (_, epoch, done) = lane.begin(start).unwrap();
    done.finish(Some(1));
    lane.poll();
    lane.configure(Some(config(CaptureCap::Small, Rate::Five)));
    assert!(!epoch.current());
    assert_eq!(lane.take_latest(), None);
    assert!(lane.begin(start + Duration::from_millis(200)).is_none());
    assert!(lane.begin(start + Duration::from_secs(2)).is_some());
}

#[test]
fn monitor_character_lifetimes_and_close_are_isolated() {
    let start = Instant::now();
    let mut first = Lane::<u32>::default();
    let mut second = Lane::<u32>::default();
    for lane in [&mut first, &mut second] {
        lane.configure(Some(config(CaptureCap::Small, Rate::One)));
    }
    let (_, a, done_a) = first.begin(start).unwrap();
    let (_, b, done_b) = second.begin(start).unwrap();
    drop(first);
    assert!(!a.current());
    assert!(b.current());
    done_a.finish(Some(1));
    done_b.finish(Some(2));
    second.poll();
    assert_eq!(second.take_latest(), Some(2));
}

#[test]
fn monitor_cancelled_before_submission_and_after_callback_never_delivers() {
    for cancel_before in [true, false] {
        let mut lane = Lane::<u32>::default();
        lane.configure(Some(config(CaptureCap::Small, Rate::One)));
        let (_, _, done) = lane.begin(Instant::now()).unwrap();
        if cancel_before {
            lane.retire();
        }
        done.finish(Some(7));
        if !cancel_before {
            lane.retire();
        }
        lane.poll();
        assert_eq!(lane.take_latest(), None);
        assert!(!lane.pending());
    }
}

#[test]
fn monitor_busy_worker_does_not_prepare_scene_or_consume_uploads() {
    use crate::render_thread::handoff;
    use std::cell::Cell;
    let (owner, worker) = handoff::<u32, u32>();
    let built = Cell::new(0);
    assert!(
        owner
            .try_submit_with(|| {
                built.set(built.get() + 1);
                1
            })
            .unwrap()
            .is_some()
    );
    assert!(
        owner
            .try_submit_with(|| {
                built.set(999);
                2
            })
            .unwrap()
            .is_none()
    );
    let (job, _) = worker.next().unwrap();
    assert_eq!(job, 1);
    assert!(
        owner
            .try_submit_with(|| {
                built.set(999);
                2
            })
            .unwrap()
            .is_none()
    );
    worker.complete(3);
    let submitted = owner
        .try_submit_with(|| {
            built.set(built.get() + 1);
            4
        })
        .unwrap()
        .unwrap();
    assert_eq!(submitted.previous, Some(3));
    assert_eq!(built.get(), 2);
    drop(worker);
    assert!(
        owner
            .try_submit_with(|| {
                built.set(999);
                5
            })
            .is_err()
    );
    assert_eq!(built.get(), 2);
}

#[test]
fn monitor_small_gpu_targets_are_persistent_bounded_and_have_no_readback_usage() {
    let _gpu = crate::test_gpu::lock();
    let instance = wgpu::Instance::default();
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).expect("Monitor test requires a GPU adapter");
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
    for cap in [CaptureCap::Small, CaptureCap::Medium, CaptureCap::Large] {
        let targets = Targets::new(&device, cap.dimensions(), wgpu::TextureFormat::Rgba8Unorm);
        let first = targets.acquire().unwrap();
        let second = targets.acquire().unwrap();
        assert!(targets.acquire().is_none());
        assert!(!targets.idle());
        assert_eq!((first.width(), first.height()), cap.dimensions());
        assert_eq!(first.sample_count(), 1);
        assert!(
            !first
                .usage()
                .intersects(wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST)
        );
        let identity = Arc::as_ptr(&first);
        let view = first.create_view(&Default::default());
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Monitor bounded target test"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::GREEN),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
        }
        queue.submit([encoder.finish()]);
        let epoch = Epoch::new();
        let done = Arc::new(Completion::new(epoch.clone()));
        let callback = done.clone();
        let lost = Arc::new(AtomicBool::new(false));
        let frame = Frame {
            texture: first,
            epoch: epoch.clone(),
            lost: lost.clone(),
            index: 4,
            stats: Default::default(),
        };
        queue.on_submitted_work_done(move || callback.finish(Some(frame)));
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap(); // Test only, never the preview API.
        assert!(done.done.load(Ordering::Acquire));
        let frame = done.result.lock().take().unwrap();
        assert_eq!(frame.index(), 4);
        assert!(frame.texture().is_some());
        lost.store(true, Ordering::Release);
        assert!(frame.texture().is_none(), "device loss invalidates already-delivered leases");
        lost.store(false, Ordering::Release); // Independent epoch revocation check on this test-only flag.
        epoch.revoke();
        assert!(frame.texture().is_none());
        drop(frame);
        drop(second);
        assert!(targets.idle());
        assert_eq!(
            Arc::as_ptr(&targets.acquire().unwrap()),
            identity,
            "same-size frames reuse real texture"
        );
    }
}

/// A real world packet through the production queue owner, not a mocked draw.
/// Run under Xvfb on Linux; lack of display/adapter is a failure, not a skip.
#[test]
#[cfg(any(target_os = "linux", target_os = "windows"))]
#[allow(deprecated)]
fn monitor_world_packets_keep_surface_size_and_never_prove_presentation() {
    #[cfg(target_os = "windows")]
    use winit::platform::windows::EventLoopBuilderExtWindows;
    #[cfg(target_os = "linux")]
    use winit::platform::x11::EventLoopBuilderExtX11;
    let _gpu = crate::test_gpu::lock();
    let mut builder = winit::event_loop::EventLoop::builder();
    #[cfg(target_os = "linux")]
    builder.with_x11();
    let event_loop = builder.with_any_thread(true).build().unwrap();
    let window = Arc::new(
        event_loop
            .create_window(
                winit::window::Window::default_attributes()
                    .with_inner_size(winit::dpi::PhysicalSize::new(1280, 720))
                    .with_visible(false),
            )
            .unwrap(),
    );
    let mut renderer = crate::Renderer::new(window.clone(), 1280, 720, false, "", None).unwrap();
    assert!(renderer.threaded());
    let original = renderer.session_boundary().unwrap();
    let vertices = [
        crate::Vertex::new([-0.5, -0.5, -2.0], [0.0, 0.0, 1.0], [0.0, 0.0]),
        crate::Vertex::new([0.5, -0.5, -2.0], [0.0, 0.0, 1.0], [1.0, 0.0]),
        crate::Vertex::new([0.0, 0.5, -2.0], [0.0, 0.0, 1.0], [0.5, 1.0]),
    ];
    let mesh = renderer.upload_mesh(&vertices, &[0, 1, 2]).unwrap();
    let record = renderer.records.alloc(crate::DrawRecord {
        base_color: [0.0, 1.0, 0.0, 1.0],
        ..Default::default()
    });
    let world = || {
        let mut lists = crate::DrawLists::default();
        lists.opaque.push(crate::DrawCmd {
            index_count: mesh.index_count,
            first_index: mesh.index_offset,
            base_vertex: mesh.vertex_offset as i32,
            record,
            bounds: [0.0; 4],
        });
        lists
    };

    for (index, cap) in [CaptureCap::Small, CaptureCap::Medium, CaptureCap::Large].into_iter().enumerate() {
        let cfg = config(cap, Rate::Five);
        renderer.configure_monitor_prototype(Some(cfg));
        let start = Instant::now();
        let deadline = start + Duration::from_secs(60);
        loop {
            let _ = renderer.poll_monitor_prototype();
            if renderer.try_render_monitor_prototype(Instant::now(), |_, size| {
                assert_eq!(size, cap.dimensions());
                let mut params = params();
                params.proj =
                    glam::camera::rh::proj::directx::perspective_infinite_reverse(params.fov_y, size.0 as f32 / size.1 as f32, params.near);
                (params, world())
            }) {
                break;
            }
            assert!(Instant::now() < deadline, "Monitor never admitted");
            std::thread::sleep(Duration::from_millis(1));
        }
        let frame = loop {
            if let Some(frame) = renderer.poll_monitor_prototype() {
                break frame;
            }
            assert!(Instant::now() < deadline, "world frame never completed");
            std::thread::sleep(Duration::from_millis(1));
        };
        assert!(frame.stats().draws > 0, "existing scene geometry actually drawn");
        assert!(!frame.stats().presented);
        let texture = frame.texture().unwrap();
        assert_eq!((texture.width(), texture.height()), cap.dimensions());
        assert_eq!(renderer.size(), (1280, 720));
        assert_eq!(renderer.frame_size(), (1280, 720));
        assert_eq!(renderer.session_boundary().unwrap().surface_epoch, original.surface_epoch);
        // Begin promotion through the existing production fence API. This
        // revokes the delivered lease and drains the same queue, but cannot
        // manufacture a foreground presentation from the preview's index.
        let ticket = index as u64 + 1;
        renderer.request_session_fence(ticket).unwrap();
        assert!(frame.texture().is_none());
        let proof = loop {
            if let Some(proof) = renderer.poll_session_fence(ticket).unwrap() {
                break proof;
            }
            assert!(Instant::now() < deadline, "promotion drain never completed");
            std::thread::sleep(Duration::from_millis(1));
        };
        assert_eq!(proof.frame, None, "offscreen frames never grant foreground input");
        assert_eq!(proof.surface_epoch, original.surface_epoch);
        assert!(!renderer.try_render_monitor_prototype(Instant::now(), |_, _| panic!("promotion revoked demand")));
        drop(frame);
    }
    // Only a real surface presentation can satisfy a fresh foreground boundary.
    window.set_visible(true);
    let before_foreground = renderer.session_boundary().unwrap();
    renderer.render(params(), &world(), None);
    renderer.request_session_fence(4).unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    let proof = loop {
        if let Some(proof) = renderer.poll_session_fence(4).unwrap() {
            break proof;
        }
        assert!(Instant::now() < deadline, "foreground presentation never completed");
        std::thread::sleep(Duration::from_millis(1));
    };
    assert!(proof.frame.is_some_and(|frame| frame + 1 > before_foreground.after_frame));
    assert_eq!(proof.surface_epoch, before_foreground.surface_epoch);
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
// Adapted from the preserved renderer fixture at a449faa7.
fn params() -> crate::FrameParams {
    use crate::{FrameParams, PhotoContext};
    use glam::{Mat4, Vec3, Vec4};
    FrameParams {
        world_viewport: None,
        rlva_spheres: Vec::new(),
        rlva_resolution_divisor: 1,
        rlva_overlays: Vec::new(),
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
        bloom: 0.0,
        water_height: 0.0,
        shadows: false,
        shadow_distance: 128.0,
        lights: Vec::new(),
        spot_shadows: [None; crate::projector_shadows::MAX_PROJECTOR_SHADOWS],
        clear_color: Vec4::ONE,
        sky: Default::default(),
        water: Default::default(),
        mirror: None,
        probes: crate::ProbeFrame {
            enabled: false,
            ..Default::default()
        },
        wireframe: false,
        debug_glow: false,
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
fn monitor_borrowed_queue_handle_never_closes_owner_but_cannot_outlive_its_authority() {
    let (owner, worker) = crate::render_thread::handoff::<u32, u32>();
    drop(owner.borrowed());
    assert!(owner.try_submit_with(|| 1).unwrap().is_some());
    let (job, _) = worker.next().unwrap();
    assert_eq!(job, 1);
    worker.complete(2);
    let borrowed = owner.borrowed();
    drop(owner);
    assert!(borrowed.try_submit_with(|| panic!("closed owner must not build")).is_err());
}
