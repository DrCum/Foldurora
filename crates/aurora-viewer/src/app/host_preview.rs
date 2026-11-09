//! Native preview policy and producer. A validated Some bootstrap and explicit
//! typed demand are required. Inactive sampling additionally requires fresh
//! native Standby, visibility/admission and reduced render-owner resources.
//!
//! Legacy `FSSessionWorker::sharingAllowed` (fssessionworker.cpp:294, pinned
//! abbe3cb0) forbids names/location privacy. Aurora also applies its
//! shared locked-HUD rule (viewfinder.rs / World::rlva_has_locked_hud), shownearby,
//! unresolved RLVa visual masks, incomplete mute state,
//! teleport/loading/recovery and every photo setup. Explicit OpenMonitor binding
//! will own RememberActive demand in a later slice; no separate Remember button.

use super::{App, Gfx};
use crate::session_host::worker::{PreviewAuthority, PreviewScope};
use crate::world::World;
use aurora_render::WorldViewport;
use aurora_session::preview_image::{PreviewAccess, PreviewEpochs, PreviewPolicy, PreviewSource};
use std::time::Instant;
use uuid::Uuid;

mod demand;
mod live;
#[cfg(test)]
mod live_tests;
mod producer;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WorldPolicy {
    agent: Uuid,
    ready: bool,
    rlva_revision: u64,
    mute_version: u64,
    derender_generation: u64,
    privacy: bool,
    visual_mask: bool,
    locked_hud: bool,
}

impl WorldPolicy {
    fn observe(world: &World) -> Self {
        let enabled = world.rlva.enabled();
        let visuals = &world.rlva.visuals;
        Self {
            agent: world.agent_id,
            ready: !world.agent_id.is_nil()
                && world.main_region.is_some()
                && world.movement_complete
                && !world.teleporting
                && world.mutes.is_loaded(),
            rlva_revision: world.rlva.revision,
            mute_version: world.mutes.version,
            derender_generation: world.derender.generation,
            privacy: enabled && (world.rlva.has("shownames") || world.rlva.has("showloc") || world.rlva.has("shownearby")),
            // Raw registrations count even when textures have not resolved into
            // this frame's RLVa draw lists. Missing assets are never permission.
            visual_mask: enabled
                && (!visuals.overlays.is_empty()
                    || !visuals.spheres.is_empty()
                    || !visuals.overlay_order.is_empty()
                    || !visuals.sphere_order.is_empty()),
            locked_hud: world.rlva_has_locked_hud(),
        }
    }
    fn allows_retention(self) -> bool {
        self.ready && !self.privacy && !self.visual_mask && !self.locked_hud
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct PhotoPolicy {
    panel: bool,
    drawing: bool,
    capturing: bool,
    frozen: bool,
    scene: bool,
    full: bool,
    snapshot: bool,
    context: aurora_render::PhotoContext,
}

impl PhotoPolicy {
    fn observe_snapshots(snapshots: &super::snapshot::Snapshots, panel: bool, frozen: bool) -> Self {
        Self {
            panel,
            drawing: snapshots.drawing(),
            capturing: snapshots.capturing.is_some(),
            frozen,
            ..Self::default()
        }
    }

    fn allows_capture(self) -> bool {
        self == Self::default()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PhysicalStamp {
    instance: u64,
    device: u64,
    generation: u64,
    size: (u32, u32),
    surface: u64,
}

impl PhysicalStamp {
    fn valid(self) -> bool {
        self.instance != 0 && self.device != 0 && self.generation != 0 && self.surface != 0 && self.size.0 != 0 && self.size.1 != 0
    }
    fn witness(self) -> (u64, u64, u64) {
        (self.instance, self.device, self.generation)
    }
    fn render_domain(self) -> ((u64, u64, u64), (u32, u32)) {
        (self.witness(), self.size)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ActiveGrant {
    activation: u64,
    viewport: aurora_session::HostedViewport,
    boundary: aurora_render::viewport_capture::CaptureBoundary,
}
impl From<PreviewAuthority> for ActiveGrant {
    fn from(value: PreviewAuthority) -> Self {
        Self {
            activation: value.activation,
            viewport: value.viewport,
            boundary: value.boundary,
        }
    }
}

/// Small copied facts only. App supplies borrowed-out Gfx explicitly; missing
/// App.gfx during a frame is never interpreted as logout or renderer teardown.
struct NativePreviewFacts {
    scope: Option<PreviewScope>,
    authority: Option<PreviewAuthority>,
    inactive: Option<crate::session_host::worker::InactivePreviewAuthority>,
    world: WorldPolicy,
    physical: Option<PhysicalStamp>,
    viewport: Option<WorldViewport>,
    capture_generation: u64,
    capture_ready: bool,
    lifecycle_ready: bool,
    photo: PhotoPolicy,
    remembered: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Observation {
    source: Option<PreviewSource>,
    world: WorldPolicy,
    physical: Option<PhysicalStamp>,
    viewport: Option<WorldViewport>,
    active: Option<ActiveGrant>,
    inactive: Option<aurora_render::viewport_capture::CaptureBoundary>,
    capture_generation: u64,
    capture_ready: bool,
    retention_ready: bool,
    photo: PhotoPolicy,
    remembered: bool,
}

/// No pixels or unbounded collections. Export counters are local checked epochs;
/// surface is the actual checked renderer epoch within that render domain.
/// Neither wrapping World counters nor control FrameProof values are adopted.
#[derive(Default)]
pub(super) struct HostPreview {
    observed: Option<Observation>,
    policy: Option<PreviewPolicy>,
    revision: u64,
    privacy: u64,
    render: u64,
    viewport: u64,
    surface: u64,
    last_physical: Option<PhysicalStamp>,
    exhausted: bool,
    observed_at: Option<Instant>,
    producer: Option<Box<producer::Producer>>,
}

impl HostPreview {
    /// Return true when any old capture must be canceled. A normal park changes
    /// capture access/fences but keeps every export/history epoch stable.
    fn sync(&mut self, facts: NativePreviewFacts, now: Instant) -> bool {
        if self.exhausted || self.observed_at.is_some_and(|previous| now < previous) {
            self.exhausted = true;
            self.policy = None;
            return true;
        }
        self.observed_at = Some(now);
        let physical = facts.physical.filter(|stamp| stamp.valid());
        if physical
            .zip(self.last_physical)
            .is_some_and(|(next, old)| next.witness() == old.witness() && next.surface < old.surface)
        {
            // A missing observation, source change or local retire cannot
            // license a backward epoch in the same physical renderer domain.
            // Keep the last witness, deny all claims, and require a new policy
            // if that domain recovers or a genuinely new witness replaces it.
            self.policy = None;
            self.observed = None;
            return true;
        }
        let scope = facts.scope.filter(|scope| scope.expires_at > now);
        let source = scope.map(|scope| scope.source);
        // Active is optional while parked; the last acknowledged activation is
        // still the same source's monotonic watermark. Zero would regress the
        // host SourceImages policy and incorrectly discard retained history.
        let activation = self
            .policy
            .filter(|policy| Some(policy.source()) == source)
            .map_or(0, |policy| policy.access().activation);
        let active = facts
            .authority
            .filter(|authority| {
                Some(authority.scope.source) == source
                    && authority.scope.expires_at > now
                    && authority.activation != 0
                    && authority.activation >= activation
                    && physical.is_some_and(|physical| {
                        physical.device == authority.boundary.device_epoch()
                            && physical.size == authority.boundary.size()
                            && physical.surface == authority.boundary.surface_epoch()
                    })
                    && (authority.viewport.rect.width, authority.viewport.rect.height) == authority.boundary.size()
            })
            .map(ActiveGrant::from);
        let inactive = facts
            .inactive
            .filter(|authority| {
                active.is_none()
                    && Some(authority.scope.source) == source
                    && authority.scope.expires_at > now
                    && physical.is_some_and(|physical| {
                        physical.device == authority.boundary.device_epoch()
                            && physical.surface == authority.boundary.surface_epoch()
                            && physical.size == authority.boundary.size()
                    })
            })
            .map(|authority| authority.boundary);
        let retention_ready = source.is_some()
            && facts.lifecycle_ready
            && facts.world.allows_retention()
            && physical.zip(facts.viewport).is_some_and(|(physical, view)| {
                view.width != 0
                    && view.height != 0
                    && view.x.checked_add(view.width).is_some_and(|right| right <= physical.size.0)
                    && view.y.checked_add(view.height).is_some_and(|bottom| bottom <= physical.size.1)
            });
        let next = Observation {
            source,
            world: facts.world,
            physical,
            viewport: facts.viewport,
            active,
            inactive,
            capture_generation: facts.capture_generation,
            capture_ready: facts.capture_ready,
            retention_ready,
            photo: facts.photo,
            remembered: facts.remembered,
        };
        if self.observed == Some(next) {
            return false;
        }
        let old = self.observed;
        let cancel = self.policy.is_some_and(|policy| policy.access().can_capture())
            && old.is_some_and(|old| old.capture_generation == next.capture_generation);
        let source_changed = old.is_none_or(|old| old.source != next.source);
        let privacy_changed = old.is_none_or(|old| old.world != next.world || old.retention_ready != next.retention_ready);
        let physical_changed = source_changed
            || old.is_none_or(|old| old.physical.map(PhysicalStamp::render_domain) != next.physical.map(PhysicalStamp::render_domain));
        let surface_changed = physical.is_some_and(|physical| physical.surface != self.surface);
        let viewport_changed = source_changed || old.is_none_or(|old| old.viewport != next.viewport);
        // PreviewEpochs, including revision, invalidate already-admitted
        // history in SourceImages. Capture-only transitions use access fields
        // and renderer/native capture fences instead of an export epoch.
        let export_changed = source_changed || privacy_changed || physical_changed || viewport_changed || surface_changed;
        if (export_changed && !advance(&mut self.revision))
            || (privacy_changed && !advance(&mut self.privacy))
            || (physical_changed && !advance(&mut self.render))
            || (viewport_changed && !advance(&mut self.viewport))
        {
            self.exhausted = true;
            self.policy = None;
            self.observed = None;
            return true;
        }
        if let Some(physical) = physical {
            self.surface = physical.surface;
            self.last_physical = Some(physical);
        }
        // Before any actual boundary is known, surface stays zero and the
        // checked policy constructor below returns unavailable. A denied policy
        // may retain the last known surface, but cannot issue a capture claim.
        let capture = retention_ready
            && active.is_some()
            && facts.capture_ready
            && facts.capture_generation != 0
            && facts.photo.allows_capture()
            && facts.remembered;
        let access = PreviewAccess {
            capture_inactive: retention_ready
                && inactive.is_some()
                && facts.capture_ready
                && facts.capture_generation != 0
                && facts.photo.allows_capture()
                && facts.remembered,
            ready: retention_ready,
            active: retention_ready && active.is_some(),
            activation: active.map_or(activation, |grant| grant.activation),
            capture_active: capture,
            retain_last_active: retention_ready,
        };
        self.policy = source.and_then(|source| {
            let epochs = PreviewEpochs::new(self.revision, self.privacy, self.render, self.viewport, self.surface).ok()?;
            PreviewPolicy::new(source, epochs, access).ok()
        });
        self.observed = Some(next);
        cancel
    }

    pub(super) fn retire(&mut self) {
        if let Some(producer) = &mut self.producer {
            producer.retire();
        }
        // Even if the next observation repeats all values, it must earn new
        // epochs. There is no validity flag that can revive old image handles.
        self.policy = None;
        self.observed = None;
    }

    pub(super) fn capture_barrier(&self) {
        if let Some(producer) = &self.producer {
            producer.capture_barrier();
        }
    }
}

fn advance(value: &mut u64) -> bool {
    if let Some(next) = value.checked_add(1) {
        *value = next;
        true
    } else {
        false
    }
}

impl App {
    /// Runs before the parked return and before other GUI work even with no
    /// pending image. No new world frame is requested; only prior-origin
    /// capture completion or retirement may advance on its existing owner.
    pub(super) fn tick_host_preview(&mut self) {
        if self.has_preview_producer() {
            let mut gfx = self.gfx.take();
            self.run_preview_producer(gfx.as_mut(), aurora_render::PhotoContext::None, producer::Pass::Cpu);
            self.gfx = gfx;
            return;
        }
        if self.host_session.is_none() {
            self.host_preview.retire();
            return;
        }
        self.configure_host_chat_rules();
        let facts = self.host_preview_facts(self.gfx.as_ref(), aurora_render::PhotoContext::None, false);
        if self.host_preview.sync(facts, Instant::now())
            && let Some(gfx) = &mut self.gfx
        {
            gfx.renderer.invalidate_viewport_capture();
        }
    }

    pub(super) fn host_preview_frame(&mut self, gfx: &mut Gfx, photo_context: aurora_render::PhotoContext) {
        if self.run_preview_producer(Some(&mut *gfx), photo_context, producer::Pass::Observe) {
            return;
        }
        if self.host_session.is_none() {
            self.host_preview.retire();
            return;
        }
        self.configure_host_chat_rules();
        let facts = self.host_preview_facts(Some(gfx), photo_context, false);
        if self.host_preview.sync(facts, Instant::now()) {
            gfx.renderer.invalidate_viewport_capture();
        }
    }

    pub(super) fn host_preview_inactive(&mut self, gfx: &mut Gfx, photo: aurora_render::PhotoContext) {
        self.run_preview_producer(Some(gfx), photo, producer::Pass::BeforeInactiveRender);
    }

    pub(super) fn host_preview_before_render(
        &mut self,
        gfx: &mut Gfx,
        photo_context: aurora_render::PhotoContext,
        viewport: WorldViewport,
    ) {
        if !self.run_preview_producer(Some(&mut *gfx), photo_context, producer::Pass::BeforeRender(viewport)) {
            self.host_preview_frame(gfx, photo_context);
        }
    }

    fn host_preview_facts(&self, gfx: Option<&Gfx>, photo_context: aurora_render::PhotoContext, remembered: bool) -> NativePreviewFacts {
        let scope = self.host_session.as_ref().and_then(|bridge| {
            let scope = bridge.preview_scope()?;
            let actual = super::host_chat::source(self, bridge)?;
            (scope.source.worker() == actual.worker
                && scope.source.session() == actual.session
                && scope.source.lease_epoch() == actual.lease_epoch)
                .then_some(scope)
        });
        let inactive = scope.and_then(|_| {
            let gfx = gfx?;
            self.host_session.as_ref()?.inactive_preview_authority(&gfx.window, &gfx.renderer)
        });
        let authority = scope.and_then(|_| {
            let gfx = gfx?;
            self.host_session.as_ref()?.preview_authority(&gfx.window, &gfx.renderer)
        });
        // Retention observes the actual physical boundary independently of
        // capture-only XR/support exclusion. A requested XR/photo capture stop
        // cannot masquerade as source/device/surface loss and erase history.
        let boundary = gfx.and_then(|gfx| gfx.renderer.viewport_capture_physical_boundary());
        let physical = gfx
            .and_then(|gfx| gfx.renderer.viewport_capture_retention_guard())
            .zip(boundary)
            .filter(|(guard, boundary)| guard.current() && guard.device_epoch() == boundary.device_epoch())
            .map(|(guard, boundary)| PhysicalStamp {
                instance: guard.instance(),
                device: guard.device_epoch(),
                generation: guard.generation(),
                size: boundary.size(),
                surface: boundary.surface_epoch(),
            });
        let viewport = gfx.map(|gfx| self.world_viewport_for_size(gfx.renderer.size(), gfx.renderer.vr_engaged()));
        let photo = PhotoPolicy {
            scene: gfx.is_some_and(|gfx| gfx.renderer.capture_scene),
            full: gfx.is_some_and(|gfx| gfx.renderer.capture_request),
            snapshot: gfx.is_some_and(|gfx| gfx.renderer.snapshot_request.is_some()),
            context: photo_context,
            ..PhotoPolicy::observe_snapshots(&self.snapshots, self.panels.snapshot, self.photo_world_frozen())
        };
        NativePreviewFacts {
            scope,
            authority,
            inactive,
            world: WorldPolicy::observe(&self.world),
            physical,
            viewport,
            capture_generation: gfx.map_or(0, |gfx| gfx.renderer.viewport_capture_generation()),
            capture_ready: gfx.is_some_and(|gfx| !gfx.renderer.failed() && !gfx.renderer.device_lost() && !gfx.renderer.vr_engaged())
                && (self.host_submission_allowed() || inactive.is_some())
                && self.options_ui.capture.is_none()
                && !self.drawing_paused(),
            lifecycle_ready: !self.exit_prompt.open
                && !self.login_form.busy
                && self.gpu_stall.recover.is_none()
                && !self.gpu_stall.close
                && gfx.is_some_and(|gfx| !gfx.renderer.failed() && !gfx.renderer.device_lost()),
            photo,
            // Only a genuine current metadata-domain applied demand sets this.
            remembered,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_session::{AccountKey, GridKey, HostId, InstallationId, SessionId, SessionIdentity, WorkerId, WorkerIdentity};
    use std::sync::Arc;
    use std::time::Duration;

    pub(super) fn world() -> World {
        let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for event in crate::demo::events() {
            world.apply(event);
        }
        world.movement_complete = true;
        world.teleporting = false;
        world.mutes.on_server(aurora_net::MuteListSource::File(String::new()), None);
        world.rlva_configure(true);
        assert!(WorldPolicy::observe(&world).allows_retention());
        world
    }

    fn command(world: &mut World, source: Uuid, text: &str) {
        assert!(world.rlva.receive(&aurora_net::ChatMessage {
            from_name: "Preview test object".into(),
            source_id: source,
            owner_id: world.agent_id,
            source_type: aurora_net::ChatSourceType::Object,
            chat_type: aurora_net::ChatType::OwnerSay,
            position: glam::Vec3::ZERO,
            message: text.into(),
        }));
    }

    pub(super) fn facts(world: &World, now: Instant) -> NativePreviewFacts {
        let source = PreviewSource::new(
            WorkerIdentity {
                installation: InstallationId::from_bytes([1; 32]).unwrap(),
                host: HostId::from_bytes([2; 16]).unwrap(),
                worker: WorkerId::from_bytes([3; 16]).unwrap(),
            },
            SessionIdentity {
                session: SessionId::from_bytes([4; 16]).unwrap(),
                account: AccountKey::from_bytes([5; 32]).unwrap(),
                grid: GridKey::from_bytes([6; 32]).unwrap(),
                generation: 1,
            },
            7,
        )
        .unwrap();
        let scope = PreviewScope {
            source,
            expires_at: now + Duration::from_millis(aurora_session::LEASE_MILLIS),
        };
        let viewport = aurora_session::HostedViewport {
            host_window: 10,
            rect: aurora_session::PhysicalRect {
                x: 0,
                y: 0,
                width: 1280,
                height: 720,
            },
        };
        NativePreviewFacts {
            scope: Some(scope),
            inactive: None,
            authority: Some(PreviewAuthority {
                scope,
                activation: 8,
                viewport,
                boundary: aurora_render::viewport_capture::CaptureBoundary::new(41, 9, (1280, 720)).unwrap(),
            }),
            world: WorldPolicy::observe(world),
            physical: Some(PhysicalStamp {
                instance: 42,
                device: 41,
                generation: 1,
                size: (1280, 720),
                surface: 9,
            }),
            viewport: Some(WorldViewport::full((1280, 720))),
            capture_generation: 1,
            capture_ready: true,
            lifecycle_ready: true,
            photo: PhotoPolicy::default(),
            remembered: true,
        }
    }

    #[test]
    fn preview_native_world_policy_uses_actual_rlva_privacy_secure_aliases_and_unresolved_masks() {
        let mut world = world();
        let issuer = Uuid::from_u128(100);
        for restriction in ["shownames", "shownames_sec", "showloc", "shownearby"] {
            command(&mut world, issuer, &format!("@{restriction}=n"));
            let policy = WorldPolicy::observe(&world);
            assert!(policy.privacy, "{restriction}");
            assert!(!policy.allows_retention());
            command(&mut world, issuer, "@clear");
            assert!(WorldPolicy::observe(&world).allows_retention());
        }
        for restriction in ["setoverlay", "setsphere"] {
            command(&mut world, issuer, &format!("@{restriction}=n"));
            assert!(WorldPolicy::observe(&world).visual_mask);
            assert!(!WorldPolicy::observe(&world).allows_retention());
            if restriction == "setoverlay" {
                assert!(
                    world.rlva.visuals.overlays[&issuer].texture.is_nil(),
                    "unresolved texture still denies export"
                );
            }
            command(&mut world, issuer, "@clear");
        }
        world.rlva.visuals.overlay_order.push(issuer);
        assert!(
            !WorldPolicy::observe(&world).allows_retention(),
            "unresolved registration without resolved draw"
        );
        world.rlva_configure(false);
        assert!(WorldPolicy::observe(&world).allows_retention());
    }

    #[test]
    fn preview_native_world_policy_uses_loaded_mutes_derender_witness_and_actual_world_readiness() {
        let mut world = world();
        let original = WorldPolicy::observe(&world);
        world.mutes.reset(world.agent_id);
        assert!(!WorldPolicy::observe(&world).allows_retention());
        world.mutes.on_server(aurora_net::MuteListSource::File(String::new()), None);
        assert!(WorldPolicy::observe(&world).allows_retention());
        world
            .mutes
            .add(
                crate::world::mutes::Mute {
                    id: Uuid::from_u128(105),
                    name: "Synthetic resident".into(),
                    kind: crate::world::mutes::MuteType::Agent,
                    flags: 0,
                },
                crate::world::mutes::flag::TEXT_CHAT,
            )
            .unwrap();
        assert_ne!(WorldPolicy::observe(&world).mute_version, original.mute_version);
        world.derender.add(
            Uuid::from_u128(106),
            crate::world::derender::Entry {
                name: String::new(),
                region: String::new(),
                kind: crate::world::derender::Kind::Object,
                date: 0,
                permanent: false,
            },
        );
        assert_ne!(WorldPolicy::observe(&world).derender_generation, original.derender_generation);
        world.mutes.state = crate::world::mutes::LoadState::Degraded;
        assert!(
            WorldPolicy::observe(&world).allows_retention(),
            "same cached-mute readiness contract as native chat"
        );
        world.teleporting = true;
        assert!(!WorldPolicy::observe(&world).allows_retention());
        world.teleporting = false;
        world.movement_complete = false;
        assert!(!WorldPolicy::observe(&world).allows_retention());
        world.movement_complete = true;
        world.main_region = None;
        assert!(!WorldPolicy::observe(&world).allows_retention());
    }

    #[test]
    fn preview_native_world_policy_uses_actual_temporary_point_and_folder_locked_huds() {
        use crate::world::inventory::{FetchState, Folder};
        use aurora_net::inventory::{InvFolder, InvItem};
        let mut world = world();
        let id = Uuid::from_u128(110);
        let issuer = Uuid::from_u128(111);
        let point = 31;
        assert!(world.avatar_lib.attach_points[&point].hud);
        let mut object = crate::demo::outfit_attachment(&aurora_net::AttachRequest {
            item_id: id,
            owner_id: world.agent_id,
            point,
            add: true,
            flags: 0,
            group_mask: 0,
            everyone_mask: 0,
            next_owner_mask: 0,
            name: String::new(),
            desc: String::new(),
        });
        let region = world.main_region.unwrap();
        world.objects.upsert(region, object.clone());
        assert!(WorldPolicy::observe(&world).allows_retention());
        command(&mut world, id, "@detach=n");
        assert!(WorldPolicy::observe(&world).locked_hud, "temporary HUD lacks inventory record");
        command(&mut world, id, "@clear");
        command(&mut world, issuer, "@remattach:center 2=n");
        assert!(WorldPolicy::observe(&world).locked_hud, "actual worn point lock");
        command(&mut world, issuer, "@clear");
        let root = Uuid::from_u128(120);
        let shared = Uuid::from_u128(121);
        let locked = Uuid::from_u128(122);
        world.inventory.root = root;
        for (folder, parent, name, children, items) in [
            (root, Uuid::nil(), "Inventory", vec![shared], vec![]),
            (shared, root, "#RLV", vec![locked], vec![]),
            (locked, shared, "Synthetic HUD", vec![], vec![id]),
        ] {
            world.inventory.folders.insert(
                folder,
                Folder {
                    info: InvFolder {
                        id: folder,
                        parent,
                        name: name.into(),
                        type_default: -1,
                        ..Default::default()
                    },
                    children,
                    items,
                    state: FetchState::Fetched,
                    library: false,
                },
            );
        }
        world.inventory.items.insert(
            id,
            InvItem {
                id,
                parent: locked,
                asset_type: 6,
                flags: u32::from(point),
                ..Default::default()
            },
        );
        command(&mut world, id, "@detachallthis=n");
        assert!(
            WorldPolicy::observe(&world).locked_hud,
            "shared folder lock uses native worn/inventory resolution"
        );
        object.state = 1_u8.rotate_right(4);
        world.objects.upsert(region, object);
        assert!(
            !WorldPolicy::observe(&world).locked_hud,
            "ordinary locked attachment does not forbid HUD export"
        );
    }

    #[test]
    fn preview_native_model_requires_explicit_demand_and_keeps_history_epochs_on_normal_park() {
        let world = world();
        let now = Instant::now();
        let mut model = HostPreview::default();
        let mut input = facts(&world, now);
        input.remembered = false;
        assert!(!model.sync(input, now));
        let initial = model.policy.unwrap();
        assert!(initial.access().active && initial.access().retain_last_active);
        assert!(
            !initial.access().capture_active,
            "normal login/active grant never silently subscribes"
        );
        let input = facts(&world, now);
        assert!(!model.sync(input, now));
        let active = model.policy.unwrap();
        assert!(active.access().capture_active);
        let mut parked = facts(&world, now);
        parked.authority = None;
        parked.capture_ready = false;
        parked.capture_generation += 1; // ordinary session fence has already revoked capture
        assert!(!model.sync(parked, now), "do not create a self-invalidating generation loop");
        let history = model.policy.unwrap();
        assert!(!history.access().active && !history.access().capture_active);
        assert!(history.access().retain_last_active);
        assert_eq!(history.epochs(), active.epochs());
        assert_eq!(history.access().activation, active.access().activation);
        let mut restored = facts(&world, now);
        restored.capture_generation = 2;
        assert!(!model.sync(restored, now));
        assert!(
            model.policy.unwrap().access().capture_active,
            "reactivation changes capture access, never retained export epochs"
        );
    }

    #[test]
    fn preview_native_model_observes_privacy_and_physical_loss_without_pending_capture_or_open_view() {
        let mut world = world();
        let now = Instant::now();
        let mut model = HostPreview::default();
        let mut input = facts(&world, now);
        input.remembered = false;
        model.sync(input, now);
        let old = model.policy.unwrap();
        command(&mut world, Uuid::from_u128(130), "@showloc=n");
        let mut input = facts(&world, now);
        input.remembered = false;
        model.sync(input, now);
        let denied = model.policy.unwrap();
        assert!(!denied.access().ready && !denied.access().retain_last_active);
        assert!(denied.epochs().privacy() > old.epochs().privacy());
        command(&mut world, Uuid::from_u128(130), "@clear");
        let mut input = facts(&world, now);
        input.physical.as_mut().unwrap().generation += 1;
        model.sync(input, now);
        let replaced = model.policy.unwrap();
        assert!(replaced.epochs().render() > denied.epochs().render());
        let mut input = facts(&world, now);
        input.physical = None;
        model.sync(input, now);
        assert!(!model.policy.unwrap().access().retain_last_active);
        model.retire();
        model.sync(facts(&world, now), now);
        assert!(model.policy.unwrap().epochs().render() > replaced.epochs().render());
    }

    #[test]
    fn preview_native_photo_adapter_reads_actual_snapshot_state_and_all_tiled_contexts() {
        let mut snapshots = super::super::snapshot::Snapshots::default();
        assert!(PhotoPolicy::observe_snapshots(&snapshots, false, false).allows_capture());
        for interface in [false, true] {
            snapshots.capturing = Some(aurora_render::SnapshotSpec {
                width: 480,
                height: 270,
                interface,
                huds: true,
                photo: (480, 270),
                world_viewport: None,
            });
            assert!(!PhotoPolicy::observe_snapshots(&snapshots, false, false).allows_capture());
        }
        snapshots.capturing = None;
        for context in [
            aurora_render::PhotoContext::None,
            aurora_render::PhotoContext::Record,
            aurora_render::PhotoContext::Use,
        ] {
            let photo = PhotoPolicy {
                context,
                ..PhotoPolicy::observe_snapshots(&snapshots, false, false)
            };
            assert_eq!(photo.allows_capture(), matches!(context, aurora_render::PhotoContext::None));
        }
        assert!(!PhotoPolicy::observe_snapshots(&snapshots, true, false).allows_capture());
        assert!(!PhotoPolicy::observe_snapshots(&snapshots, false, true).allows_capture());
    }

    #[test]
    fn preview_native_model_vetoes_every_photo_setup_and_lifecycle_loss_without_destroying_photo_history() {
        let world = world();
        let now = Instant::now();
        for case in 0..8 {
            let mut model = HostPreview::default();
            model.sync(facts(&world, now), now);
            let before = model.policy.unwrap();
            let mut input = facts(&world, now);
            match case {
                0 => input.photo.panel = true,
                1 => input.photo.drawing = true,
                2 => input.photo.capturing = true,
                3 => input.photo.frozen = true,
                4 => input.photo.scene = true,
                5 => input.photo.full = true,
                6 => input.photo.snapshot = true,
                7 => input.photo.context = aurora_render::PhotoContext::Record,
                _ => unreachable!(),
            }
            assert!(model.sync(input, now), "cancel old permission for photo case {case}");
            let after = model.policy.unwrap();
            assert!(!after.access().capture_active && after.access().retain_last_active);
            assert_eq!(after.epochs().privacy(), before.epochs().privacy());
            assert_eq!(after.epochs().render(), before.epochs().render());
        }
        for case in 0..4 {
            let mut input = facts(&world, now);
            match case {
                0 => input.lifecycle_ready = false, // exit/recovery
                1 => input.scope = None,            // source/account/grid/logout/lease
                2 => input.physical = None,
                3 => input.viewport = None,
                _ => unreachable!(),
            }
            let mut model = HostPreview::default();
            model.sync(input, now);
            assert!(model.policy.is_none_or(|policy| !policy.access().retain_last_active));
        }
    }

    #[test]
    fn preview_native_model_lease_and_checked_epochs_fail_closed_without_revival() {
        let world = world();
        let now = Instant::now();
        let mut model = HostPreview::default();
        let input = facts(&world, now);
        let expiry = input.scope.unwrap().expires_at;
        model.sync(input, now);
        model.sync(facts(&world, now), expiry);
        assert!(model.policy.is_none(), "lease expiry is observed without a pending frame");
        let now = expiry;
        model.sync(facts(&world, now), now);
        let old = model.policy.unwrap();
        let mut next = facts(&world, now);
        let source = next.scope.unwrap().source;
        let mut session = source.session();
        session.generation += 1;
        next.scope.as_mut().unwrap().source = PreviewSource::new(source.worker(), session, source.lease_epoch()).unwrap();
        model.sync(next, now);
        assert!(
            !model.policy.unwrap().access().active,
            "old native grant cannot authorize new source"
        );
        assert!(model.policy.unwrap().epochs().render() > old.epochs().render());
        model.revision = u64::MAX;
        model.sync(facts(&world, now), now);
        assert!(model.exhausted && model.policy.is_none());
        model.retire();
        model.sync(facts(&world, now), now);
        assert!(model.policy.is_none(), "teardown does not reset an exhausted epoch");
    }

    #[test]
    fn preview_native_model_rejects_invalid_geometry_foreign_boundary_and_clock_regression() {
        let world = world();
        let now = Instant::now();
        for view in [
            WorldViewport {
                x: u32::MAX,
                y: 0,
                width: 4,
                height: 4,
            },
            WorldViewport {
                x: 0,
                y: 0,
                width: 0,
                height: 720,
            },
            WorldViewport {
                x: 0,
                y: 719,
                width: 1280,
                height: 2,
            },
        ] {
            let mut model = HostPreview::default();
            let mut input = facts(&world, now);
            input.viewport = Some(view);
            model.sync(input, now);
            assert!(!model.policy.unwrap().access().ready);
        }
        let mut model = HostPreview::default();
        let mut input = facts(&world, now);
        input.physical.as_mut().unwrap().device += 1;
        model.sync(input, now);
        assert!(
            !model.policy.unwrap().access().capture_active,
            "actual grant cannot cross renderer replacement"
        );
        model.sync(facts(&world, now), now + Duration::from_millis(1));
        model.sync(facts(&world, now), now);
        assert!(
            model.exhausted && model.policy.is_none(),
            "future observation cannot revive with a regressed clock"
        );
    }

    fn admitted_history(
        world: &World,
        now: Instant,
    ) -> (
        HostPreview,
        aurora_session::preview_image::SourceImages,
        aurora_session::preview_image::ImageBudget,
        aurora_session::preview_image::AdmittedImage,
        aurora_session::preview_image::PreviewGeometry,
    ) {
        use aurora_session::preview_image::{HOST_IMAGE_BYTES, ImageBudget, PreviewGeometry, PreviewStamp, SourceImages};
        let mut native = HostPreview::default();
        native.sync(facts(world, now), now);
        let policy = native.policy.unwrap();
        let budget = ImageBudget::new(HOST_IMAGE_BYTES).unwrap();
        let mut images = SourceImages::new(policy.source(), budget.clone()).unwrap();
        images.sync(policy, 0).unwrap();
        let geometry = PreviewGeometry::new([1280, 720], [0, 0, 1280, 720], aurora_session::PreviewSize::Small).unwrap();
        let claim = images.request(1, 1, geometry, 10).unwrap();
        let stamp = PreviewStamp::new(claim, 1, policy.epochs().surface(), 1, geometry).unwrap();
        let historical = images.admit(stamp, budget.reserve(geometry).unwrap(), 11).unwrap();
        (native, images, budget, historical, geometry)
    }

    #[test]
    fn preview_native_policy_source_images_retains_admitted_history_across_capture_only_transitions() {
        use aurora_session::preview_image::{ImageError, PreviewStamp};
        let world = world();
        let now = Instant::now();
        for case in 0..6 {
            let (mut native, mut images, budget, historical, geometry) = admitted_history(&world, now);
            let before = native.policy.unwrap();
            let late_claim = images.request(1, 1, geometry, 20).unwrap();
            let late = PreviewStamp::new(late_claim, 2, before.epochs().surface(), 2, geometry).unwrap();
            let mut input = facts(&world, now);
            match case {
                0 => {
                    input.authority = None;
                    input.capture_ready = false;
                    input.capture_generation += 1;
                }
                1 => input.photo.context = aurora_render::PhotoContext::Record,
                2 => input.remembered = false,
                3 => {
                    input.capture_ready = false;
                    input.capture_generation += 1;
                }
                4 => input.authority.as_mut().unwrap().activation += 1,
                5 => {
                    // XR wanted/engaged rejects capture authority while the
                    // readonly desktop physical witness remains unchanged.
                    input.authority = None;
                    input.capture_ready = false;
                }
                _ => unreachable!(),
            }
            native.sync(input, now + Duration::from_millis(21));
            let after = native.policy.unwrap();
            assert_eq!(
                after.epochs(),
                before.epochs(),
                "capture-only case {case} must not export a purge epoch"
            );
            assert!(
                after.access().activation >= before.access().activation,
                "monotonic source watermark"
            );
            images.sync(after, 21).unwrap();
            assert!(historical.current(21), "held admitted history survives case {case}");
            assert_eq!(images.retained(21).unwrap().unwrap().stamp(), historical.stamp());
            assert!(
                matches!(images.admit(late, budget.reserve(geometry).unwrap(), 22), Err(ImageError::Stale)),
                "late old-grant/request completion rejected for case {case}"
            );
            let mut restored = facts(&world, now);
            restored.authority.as_mut().unwrap().activation = after.access().activation;
            restored.capture_generation += 2;
            native.sync(restored, now + Duration::from_millis(30));
            let restored = native.policy.unwrap();
            assert_eq!(restored.epochs(), before.epochs());
            images.sync(restored, 30).unwrap();
            let fresh = images.request(1, 1, geometry, 31).unwrap();
            assert_ne!(fresh, late_claim);
            assert!(
                matches!(images.admit(late, budget.reserve(geometry).unwrap(), 32), Err(ImageError::Stale)),
                "restoration never revives the old pending request for case {case}"
            );
            let fresh = PreviewStamp::new(fresh, 2, restored.epochs().surface(), 3, geometry).unwrap();
            assert!(images.admit(fresh, budget.reserve(geometry).unwrap(), 33).is_ok());
            assert!(historical.current(33));
        }
    }

    #[test]
    fn preview_native_policy_source_images_purges_source_privacy_and_physical_loss_without_revival() {
        use aurora_session::preview_image::{ImageError, PreviewStamp};
        let mut world = world();
        let now = Instant::now();
        let issuer = Uuid::from_u128(140);
        for case in 0..3 {
            let (mut native, mut images, budget, held, geometry) = admitted_history(&world, now);
            let before = native.policy.unwrap();
            let late_claim = images.request(1, 1, geometry, 20).unwrap();
            let late = PreviewStamp::new(late_claim, 2, before.epochs().surface(), 2, geometry).unwrap();
            let mut input = facts(&world, now);
            match case {
                0 => {
                    let source = input.scope.unwrap().source;
                    let mut session = source.session();
                    session.generation += 1;
                    input.scope.as_mut().unwrap().source = PreviewSource::new(source.worker(), session, source.lease_epoch()).unwrap();
                }
                1 => {
                    command(&mut world, issuer, "@showloc=n");
                    input.world = WorldPolicy::observe(&world);
                }
                2 => input.physical = None,
                _ => unreachable!(),
            }
            native.sync(input, now + Duration::from_millis(21));
            let denied = native.policy.unwrap();
            assert_ne!(denied.epochs(), before.epochs());
            let result = images.sync(denied, 21);
            if case == 0 {
                assert_eq!(result, Err(ImageError::Stale));
            } else {
                result.unwrap();
            }
            assert!(!held.current(21), "case {case} revokes an externally held admitted handle");
            assert!(matches!(
                images.admit(late, budget.reserve(geometry).unwrap(), 22),
                Err(ImageError::Stale)
            ));
            if case == 1 {
                command(&mut world, issuer, "@clear");
            }
            let mut restored = facts(&world, now);
            if case == 2 {
                restored.physical.as_mut().unwrap().generation += 1;
            }
            native.sync(restored, now + Duration::from_millis(30));
            let result = images.sync(native.policy.unwrap(), 30);
            if case == 0 {
                assert_eq!(result, Err(ImageError::Stale));
            } else {
                result.unwrap();
            }
            assert!(!held.current(30), "case {case} restoration never revives the old handle");
            assert!(images.retained(30).unwrap().is_none());
        }
    }

    #[test]
    fn preview_native_activation_watermark_rejects_regression_without_export_epoch_change() {
        let world = world();
        let now = Instant::now();
        let mut native = HostPreview::default();
        native.sync(facts(&world, now), now);
        let before = native.policy.unwrap();
        let mut regressed = facts(&world, now);
        regressed.authority.as_mut().unwrap().activation -= 1;
        native.sync(regressed, now);
        let after = native.policy.unwrap();
        assert!(!after.access().active && !after.access().capture_active);
        assert_eq!(after.access().activation, before.access().activation);
        assert_eq!(after.epochs(), before.epochs());
        let mut generation = facts(&world, now);
        generation.capture_generation += 1;
        native.sync(generation, now);
        assert_eq!(
            native.policy.unwrap().epochs(),
            before.epochs(),
            "publication generation is a separate capture fence"
        );
        assert_eq!(native.observed.unwrap().capture_generation, 2);
    }

    #[test]
    fn preview_native_actual_surface9_parks_then_new_physical1_advances_render_and_never_revives_history() {
        use aurora_session::preview_image::{ImageError, PreviewStamp};
        let world = world();
        let now = Instant::now();
        let (mut native, mut images, budget, held9, geometry) = admitted_history(&world, now);
        let actual9 = native.policy.unwrap();
        assert_eq!(
            actual9.epochs().surface(),
            9,
            "publish the actual renderer surface, not a local counter"
        );
        assert_eq!(held9.stamp().surface_epoch(), 9);
        let mut parked = facts(&world, now);
        parked.authority = None;
        parked.capture_ready = false;
        parked.capture_generation = 2;
        native.sync(parked, now + Duration::from_millis(20));
        let parked = native.policy.unwrap();
        assert_eq!(parked.epochs(), actual9.epochs());
        images.sync(parked, 20).unwrap();
        assert!(held9.current(20));
        let mut active9 = facts(&world, now);
        active9.capture_generation = 2;
        native.sync(active9, now + Duration::from_millis(21));
        images.sync(native.policy.unwrap(), 21).unwrap();
        let pending9 = images.request(1, 1, geometry, 22).unwrap();
        let late9 = PreviewStamp::new(pending9, 2, 9, 2, geometry).unwrap();
        let replacement = || {
            let mut next = facts(&world, now);
            next.physical = Some(PhysicalStamp {
                instance: 43,
                device: 42,
                generation: 1,
                size: (1280, 720),
                surface: 1,
            });
            next.authority.as_mut().unwrap().activation = 9;
            next.authority.as_mut().unwrap().boundary = aurora_render::viewport_capture::CaptureBoundary::new(42, 1, (1280, 720)).unwrap();
            next.capture_generation = 3;
            next
        };
        native.sync(replacement(), now + Duration::from_millis(23));
        let actual1 = native.policy.unwrap();
        assert_eq!(actual1.epochs().surface(), 1);
        assert!(actual1.epochs().render() > actual9.epochs().render());
        images.sync(actual1, 23).unwrap();
        assert!(!held9.current(23));
        assert!(images.retained(23).unwrap().is_none());
        assert!(matches!(
            images.admit(late9, budget.reserve(geometry).unwrap(), 24),
            Err(ImageError::Stale)
        ));
        let fresh = images.request(1, 1, geometry, 25).unwrap();
        assert_eq!(fresh.epochs().surface(), 1);
        assert!(
            PreviewStamp::new(fresh, 3, 9, 3, geometry).is_err(),
            "old actual surface cannot stamp the new claim"
        );
        let stamp1 = PreviewStamp::new(fresh, 3, 1, 3, geometry).unwrap();
        let held1 = images.admit(stamp1, budget.reserve(geometry).unwrap(), 26).unwrap();
        assert!(held1.current(26));
        assert!(!held9.current(26));
        assert_eq!(images.sync(actual9, 27), Err(ImageError::Stale));
        native.sync(replacement(), now + Duration::from_millis(28));
        images.sync(native.policy.unwrap(), 28).unwrap();
        assert!(
            !held9.current(28) && !held1.current(28),
            "stale policy restoration never revives old held images"
        );
    }

    #[test]
    fn preview_native_same_witness_surface_regression_is_denied_across_unknown_and_retire() {
        let world = world();
        let now = Instant::now();
        let mut direct = HostPreview::default();
        direct.sync(facts(&world, now), now);
        let render = direct.render;
        let mut backwards = facts(&world, now);
        backwards.physical.as_mut().unwrap().surface = 1;
        backwards.authority.as_mut().unwrap().boundary = aurora_render::viewport_capture::CaptureBoundary::new(41, 1, (1280, 720)).unwrap();
        direct.sync(backwards, now);
        assert!(direct.policy.is_none());
        assert_eq!(direct.render, render, "a raw regression cannot mint a new render domain for itself");
        let (mut native, mut images, _, held, _) = admitted_history(&world, now);
        let mut unknown = facts(&world, now);
        unknown.physical = None;
        native.sync(unknown, now + Duration::from_millis(20));
        let denied = native.policy.unwrap();
        assert!(!denied.access().capture_active && !denied.access().retain_last_active);
        assert_eq!(
            denied.epochs().surface(),
            9,
            "denied state may carry only the last known actual boundary"
        );
        images.sync(denied, 20).unwrap();
        assert!(!held.current(20));
        for millis in [21, 22] {
            let mut backwards = facts(&world, now);
            backwards.physical.as_mut().unwrap().surface = 1;
            backwards.authority.as_mut().unwrap().boundary =
                aurora_render::viewport_capture::CaptureBoundary::new(41, 1, (1280, 720)).unwrap();
            assert!(native.sync(backwards, now + Duration::from_millis(millis)));
            assert!(
                native.policy.is_none(),
                "same witness cannot reset actual surface, even after a missing observation/local render bump"
            );
            assert_eq!(native.surface, 9);
            native.retire();
        }
        // A source owner must retire history on unavailable output; no claim is
        // minted from an invented epoch while the native boundary is unknown.
        images.retire();
        assert!(!held.current(22));
    }

    #[test]
    fn preview_native_unknown_boundary_never_fabricates_surface_authority() {
        let world = world();
        let now = Instant::now();
        let mut native = HostPreview::default();
        for surface in [None, Some(0)] {
            let mut unknown = facts(&world, now);
            if let Some(surface) = surface {
                unknown.physical.as_mut().unwrap().surface = surface;
            } else {
                unknown.physical = None;
            }
            native.sync(unknown, now);
            assert!(native.policy.is_none());
            assert_eq!(native.surface, 0);
        }
        native.sync(facts(&world, now), now);
        let policy = native.policy.unwrap();
        assert_eq!(policy.epochs().surface(), 9);
        assert!(policy.access().capture_active);
        let mut wrong_grant = facts(&world, now);
        wrong_grant.authority.as_mut().unwrap().boundary =
            aurora_render::viewport_capture::CaptureBoundary::new(41, 8, (1280, 720)).unwrap();
        native.sync(wrong_grant, now);
        assert!(
            !native.policy.unwrap().access().capture_active,
            "grant and actual physical surface must agree"
        );
        assert_eq!(native.policy.unwrap().epochs(), policy.epochs());
    }
}
