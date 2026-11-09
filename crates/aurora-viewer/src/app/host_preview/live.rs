//! Small world preparation, called only inside the admitted queue-owner slot.
//! Networking/agent/camera CPU ticks remain on the ordinary frame path. No UI,
//! window resize, photo, input dispatch or presentation proof is produced.
use super::App;
use crate::scene::CullView;
use crate::world::env::Environment;
use aurora_render::{DrawLists, FrameParams, PhotoContext, Renderer};
use glam::{Mat4, Vec4};
use std::time::{Duration, Instant};

impl App {
    pub(super) fn prepare_inactive_monitor(&mut self, renderer: &mut Renderer, size: (u32, u32)) -> (FrameParams, DrawLists) {
        let now = Instant::now();
        let aspect = size.0 as f32 / size.1 as f32;
        let view = self.camera.view();
        let proj = self.camera.proj(aspect);
        let cull = CullView::new(proj * view, self.camera.position, size.1 as f32, self.camera.fov_y);
        let slice = Duration::from_millis(1);
        self.scene.draw_distance = self.settings.draw_distance.min(128.0);
        self.scene.fov_y = self.camera.fov_y;
        self.scene.lod_factor = self.settings.lod_factor;
        self.scene.reflections = false;
        self.scene.max_avatars = self.settings.max_avatars.min(12) as usize;
        self.scene
            .textures
            .set_max_resolution(self.settings.max_texture_resolution.min(512));
        self.scene.agent_idx = self.world.objects.index_of_uuid(&self.world.agent_id);
        self.scene.synchronize_animations = self.settings.synchronize_animations;
        self.scene.sync_budget = slice;
        self.scene.set_debug_alpha(&mut self.world, None);
        self.scene.set_pass_view(&mut self.world, Default::default());
        self.scene.process_results(renderer, &self.net, slice);
        self.scene.update_foot_scope(&self.world, cull, now);
        let completed = self.scene.update_poses(&mut self.world, self.camera.position, now);
        for anim in completed {
            self.ao_runtime.finished(anim);
            let flags = self.world.agent.animation_stop_flags(anim, false, now);
            if flags != 0 {
                self.send(aurora_net::NetCommand::OneShotControl(flags));
            }
            self.send(aurora_net::NetCommand::AgentAnimation { anim, start: false });
        }
        let mut exceptions = self.settings.render_exceptions_map();
        for id in self.world.mutes.blocked_ids() {
            exceptions.insert(id, 3);
        }
        let friends = self.world.social.friends.iter().map(|friend| friend.id).collect();
        self.scene.update_complexity(
            &self.world,
            &crate::scene::complexity::Rules {
                max: self.settings.max_complexity,
                max_area: self.settings.max_attachment_area,
                mode: self.settings.complexity_mode,
                exceptions: &exceptions,
                friends: &friends,
            },
        );
        self.scene.set_first_person(
            self.camera
                .first_person_view()
                .then_some(self.settings.camera.show_avatar_in_mouselook),
        );
        self.scene.sync(renderer, &mut self.world, &cull);
        self.scene.update_portal_clipping(renderer, &self.world);
        self.scene.build_lists(renderer, &cull, false);
        let texture_budget = self.settings.texture_budget(renderer.info.vram_mb, renderer.info.integrated);
        self.scene.stream(
            renderer,
            &self.net,
            &self.world,
            texture_budget * 1024 * 1024,
            Duration::from_millis(16),
        );
        let (sky, water) = self.world.environment_frames(&mut self.panels.time_of_day);
        let mut env = Environment::from_eep(&sky, &water, self.world.cloud_scroll, self.scene.draw_distance)
            .with_colored_ambient(&sky, crate::world::env::colored_ambient_now(self.settings.colored_sky_ambient));
        let slots = self.scene.env_textures(renderer, env.textures);
        env.sky.cloud_texture = slots[0];
        env.sky.sun_texture = slots[1];
        env.sky.moon_texture = slots[2];
        env.water.normal_texture = slots[3];
        env.sky.rainbow_texture = slots[4];
        env.sky.halo_texture = slots[5];
        let fp = FrameParams {
            world_viewport: None,
            rlva_spheres: Vec::new(),
            rlva_resolution_divisor: self.world.rlva.resolution_divisor.unwrap_or(1),
            rlva_overlays: Vec::new(),
            selection_color: [0.0; 4],
            rez_gradient: [[0.0; 4]; 3],
            selection_pulse: 0.0,
            selection_scale: 1.0,
            view,
            proj,
            photo_tile: Mat4::IDENTITY,
            photo_scale: 1.0,
            photo_context: PhotoContext::None,
            camera_pos: self.camera.position,
            near: self.camera.near,
            far: self.scene.draw_distance,
            fov_y: self.camera.fov_y,
            time: self.start.elapsed().as_secs_f32(),
            anim_at: None,
            tex_anim_at: None,
            sun_dir: env.sun_dir,
            sun_visible: env.sun_visible,
            sun_color: env.sun_color,
            sky_zenith: env.zenith,
            sky_horizon: env.horizon,
            ground_color: env.ground,
            fog_color: env.fog_color,
            fog_density: env.fog_density,
            exposure: 1.0,
            exposure_lock: false,
            tonemap_mix: self.settings.tonemap_mix,
            sharpen: 0.0,
            glow: self.settings.glow,
            bloom: 0.0,
            water_height: self.world.main_water_height(),
            shadows: false,
            shadow_distance: 0.0,
            lights: Vec::new(),
            spot_shadows: [None; aurora_render::projector_shadows::MAX_PROJECTOR_SHADOWS],
            clear_color: Vec4::new(env.fog_color.x, env.fog_color.y, env.fog_color.z, 1.0),
            sky: env.sky,
            water: env.water,
            mirror: None,
            probes: Default::default(),
            wireframe: false,
            debug_glow: false,
            blend_late_discard: false,
            all_lights: false,
            probe_walk_list: false,
            debug: Default::default(),
            dof: None,
            cutout: false,
            look: None,
        };
        let mut lists = DrawLists::default();
        lists.copy_from(&self.scene.lists);
        (fp, lists)
    }
}
