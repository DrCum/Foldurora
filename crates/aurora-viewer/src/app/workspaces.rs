//! Native workspace effects stay on the App thread. The store only publishes
//! definitions to its captured profile; replies never apply geometry.
use super::{App, Renderer, Screen, Window, ui};
use crate::workspace::{
    controller::{Action, Command, Pending, Source, Status},
    model::{Bars, Catalogue, Kind, LIMIT, Layout, Record, Viewport, valid_name},
    store::{Lane, Request},
};
use uuid::Uuid;

impl App {
    pub(super) fn workspace_native_huds_current(&self) -> bool {
        let Some(pending) = &self.workspaces.native_huds else {
            return false;
        };
        let current = self.workspace_source();
        let scope = match (&self.host_session, pending.scope) {
            (Some(bridge), Some(scope)) => bridge.native_action_current(scope),
            (None, None) => true,
            _ => false,
        };
        pending.accepts(
            current.as_ref(),
            self.focused
                && self.host_input_allowed()
                && matches!(self.screen, Screen::World)
                && !self.world.teleporting
                && !self.panels.settings
                && !self.exit_prompt.open,
            scope,
            std::time::Instant::now(),
        )
    }
    fn workspace_start_native_huds(&mut self, items: Vec<crate::workspace::context::Hud>, source: Source) -> bool {
        if items.is_empty() {
            return true;
        }
        let scope = self.host_session.as_ref().and_then(|b| b.native_action_scope());
        if self.host_session.is_some() && scope.is_none() {
            return false;
        }
        let deadline = scope.map_or_else(|| std::time::Instant::now() + std::time::Duration::from_secs(15), |s| s.deadline());
        self.workspaces.native_hud_report = Some(crate::workspace::controller::HudReport {
            complete: false,
            confirmed: 0,
            unconfirmed: items.len(),
            reason: "pending",
        });
        self.workspaces.native_huds = Some(crate::workspace::controller::NativeHuds {
            source,
            scope,
            deadline,
            items,
            submitted: Default::default(),
            request: None,
            item: None,
            starting: false,
        });
        true
    }
    pub(super) fn workspace_cancel_native_huds(&mut self, reason: &'static str) {
        let Some(pending) = self.workspaces.native_huds.take() else {
            return;
        };
        let points = self.world.worn_attachment_points();
        let confirmed = pending.items.iter().filter(|h| points.get(&h.item) == Some(&h.point)).count();
        if pending.request.is_some()
            && self
                .appearance_ui
                .pending
                .is_some_and(|(request, _)| Some(request) == pending.request)
        {
            self.appearance_ui.pending = None;
            self.appearance_ui.attachments.clear();
            self.appearance_ui.worn_before.clear();
        }
        self.workspaces.native_hud_report = Some(crate::workspace::controller::HudReport {
            complete: confirmed == pending.items.len(),
            confirmed,
            unconfirmed: pending.items.len() - confirmed,
            reason,
        });
    }
    /// Narrow final gate for this native queue's own COF -> attachment callback.
    /// Other human and assistant appearance operations retain their own owner.
    pub(super) fn workspace_native_hud_command(&mut self, command: &aurora_net::NetCommand) -> bool {
        let Some(pending) = &self.workspaces.native_huds else { return true };
        let relevant = match command {
            aurora_net::NetCommand::UpdateOutfit { request, .. } => pending.starting || pending.request == Some(*request),
            aurora_net::NetCommand::RezAttachments(items) => pending.item.is_some_and(|item| items.iter().any(|a| a.item_id == item)),
            _ => false,
        };
        if !relevant {
            return true;
        }
        if let aurora_net::NetCommand::UpdateOutfit { request, .. } = command
            && let Some(pending) = &mut self.workspaces.native_huds
            && pending.starting
        {
            pending.request = Some(*request);
        }
        if !self.workspace_native_huds_current() {
            self.workspace_cancel_native_huds("native_source_or_scope_retired");
            return false;
        }
        true
    }
    fn workspace_native_huds_tick(&mut self, renderer: &Renderer, window: &Window) {
        if self.workspaces.native_huds.is_none() {
            return;
        }
        if !self.workspace_native_huds_current() || !self.workspace_permit(renderer, window) {
            self.workspace_cancel_native_huds("native_source_or_scope_retired");
            return;
        }
        if self.workspaces.preview.is_some() || self.workspaces.placement.is_some() {
            return;
        }
        let points = self.world.worn_attachment_points();
        let remaining: Vec<_> = self
            .workspaces
            .native_huds
            .as_ref()
            .expect("native HUD queue")
            .items
            .iter()
            .filter(|h| points.get(&h.item) != Some(&h.point))
            .cloned()
            .collect();
        if remaining.is_empty() {
            self.workspace_cancel_native_huds("confirmed");
            return;
        }
        if self.appearance_ui.pending.is_some() {
            return;
        }
        let Some(hud) = remaining.into_iter().find(|h| {
            !self
                .workspaces
                .native_huds
                .as_ref()
                .expect("native HUD queue")
                .submitted
                .contains(&h.item)
        }) else {
            return;
        };
        let native = self.world.inventory.items.get(&hud.item).is_some_and(|i| {
            i.owner == self.world.agent_id
                && i.asset_type == 6
                && !i.group_owned
                && crate::world::inventory::actions::under(&self.world.inventory, i.id, self.world.inventory.root)
                && !crate::world::inventory::actions::library(&self.world.inventory, i.id)
                && !crate::world::inventory::actions::in_type(&self.world.inventory, i.id, 14)
        });
        let inv = &self.world.inventory;
        let cof_ready = inv
            .system_folder(46)
            .and_then(|id| inv.folders.get(&id).map(|f| (id, f)))
            .is_some_and(|(id, f)| {
                f.items.len() <= 128
                    && crate::world::appearance::complete(inv, id)
                    && f.items.iter().all(|id| {
                        crate::world::inventory::actions::original(inv, *id).is_some_and(|original| inv.items.contains_key(&original))
                    })
            });
        if !native || !cof_ready {
            self.workspace_cancel_native_huds("native_hud_item_changed");
            return;
        }
        let pending = self.workspaces.native_huds.as_mut().expect("native HUD queue");
        pending.submitted.insert(hud.item);
        pending.item = Some(hud.item);
        pending.request = None;
        pending.starting = true;
        self.apply_appearance_action(crate::world::appearance::Action::WearItems {
            items: vec![hud.item],
            replace: false,
            point: hud.point,
        });
        if let Some(pending) = &mut self.workspaces.native_huds {
            pending.starting = false;
            if pending.request.is_none() {
                self.workspace_cancel_native_huds("native_hud_submission_refused");
            }
        }
    }
    pub(super) fn workspace_huds(&self) -> Vec<crate::workspace::context::Hud> {
        let mut huds: Vec<_> = self
            .world
            .worn_attachment_points()
            .into_iter()
            .filter(|(_, point)| (31..=38).contains(point))
            .filter_map(|(item, point)| {
                let native = self
                    .world
                    .inventory
                    .items
                    .get(&item)
                    .filter(|i| i.owner == self.world.agent_id && i.asset_type == 6 && !i.group_owned)?;
                let mut name: String = native.name.chars().filter(|c| !c.is_control()).take(24).collect();
                if name.is_empty() {
                    name = "HUD".into();
                }
                Some(crate::workspace::context::Hud { item, name, point })
            })
            .take(64)
            .collect();
        huds.sort_by_key(|h| h.item);
        huds
    }
    pub(super) fn workspace_capture_like(&self, like: &crate::workspace::context::Context) -> Layout {
        let mut layout = self.workspace_capture();
        layout.context = crate::workspace::context::Context::capture_settings(&self.settings, like, self.camera.zoom_fraction());
        if !like.huds.is_empty() {
            layout.context.huds = self.workspace_huds();
        }
        layout
    }
    pub(super) fn workspace_context_allowed(&self, context: &crate::workspace::context::Context) -> bool {
        if context.graphics.is_some() && self.settings.photo_held.is_some() {
            return false;
        }
        if let Some(camera) = &context.camera {
            if self.world.rlva.camera_owner().is_some()
                || self.world.rlva.camera_preset_locked(Uuid::nil())
                || self.camera.flycam.is_some()
                || self.snapshots.ui.free.on
            {
                return false;
            }
            if self
                .world
                .rlva
                .camera_fov_limits(self.camera_aspect())
                .is_some_and(|(lo, hi)| camera.fov < lo || camera.fov > hi)
            {
                return false;
            }
        }
        true
    }
    fn workspace_stage_context(&mut self, before: &crate::workspace::context::Context, target: &crate::workspace::context::Context) {
        if target.graphics.is_none() && target.camera.is_none() {
            return;
        }
        self.settings.workspace_context_held = Some(crate::workspace::context::Held {
            before: before.clone(),
            expected: target.clone(),
        });
        target.write(&mut self.settings);
        if let Some(camera) = &target.camera {
            self.camera.set_zoom_fraction(camera.zoom);
        }
        self.workspaces.context_dirty = true;
    }
    pub(super) fn workspace_cleanup_context(&mut self) {
        let camera_allowed = self.world.rlva.camera_owner().is_none() && !self.world.rlva.camera_preset_locked(Uuid::nil());
        if let Some(held) = self.settings.workspace_context_held.take() {
            if camera_allowed
                && let (Some(before), Some(expected)) = (&held.before.camera, &held.expected.camera)
                && (self.camera.zoom_fraction() - expected.zoom).abs() < 1e-5
            {
                self.camera.set_zoom_fraction(before.zoom);
            }
            self.settings.workspace_context_held = held.restore(&mut self.settings, camera_allowed);
            self.workspaces.context_dirty = true;
        }
    }
    pub(super) fn workspace_render_context(&mut self, renderer: &mut Renderer, window: &Window) {
        if !self.workspaces.context_dirty || !self.workspace_permit(renderer, window) {
            return;
        }
        if let Some(operation) = self.workspaces.assistant
            && !self.workspaces.assistant_cancelled
            && !super::mcp::workspace_placement_current(self, operation)
        {
            return;
        }
        renderer.apply_settings(self.settings.render_settings());
        self.send(aurora_net::NetCommand::SetDrawDistance(self.settings.draw_distance));
        self.workspaces.context_dirty = false;
    }
    pub(super) fn workspace_source(&self) -> Option<Source> {
        if matches!(self.screen, Screen::Login) || self.world.agent_id.is_nil() {
            return None;
        }
        let login = self.world.login.as_ref()?.session_id;
        if login.is_nil() {
            return None;
        }
        let grid = match self
            .workspaces
            .source
            .as_ref()
            .filter(|s| s.agent == self.world.agent_id && s.login == login)
        {
            Some(s) => s.grid.clone(),
            None => crate::session_host::transport::grid_key(&self.settings.login_uri())
                .ok()?
                .to_string(),
        };
        let hosted = match &self.host_session {
            Some(b) => {
                let bound = aurora_session::chat::ChatSource::try_from(b.bound_session()?).ok()?;
                if bound.session.grid.to_string() != grid {
                    return None;
                }
                // Identity retention preserves Previous through a loading screen;
                // the effect gate still requires World and exact actual login.
                if matches!(self.screen, Screen::World) && super::host_chat::source(self, b) != Some(bound) {
                    return None;
                }
                Some((bound, b.layout_activation()))
            }
            None => None,
        };
        Some(Source {
            agent: self.world.agent_id,
            login,
            grid,
            hosted,
        })
    }
    fn workspace_permit(&self, renderer: &Renderer, window: &Window) -> bool {
        let size = window.inner_size();
        let canvas = self.egui_ctx.viewport_rect().size() * self.egui_ctx.pixels_per_point();
        if !matches!(self.screen, Screen::World)
            || self.workspace_source().is_none()
            || !self.focused
            || !window.has_focus()
            || window.is_minimized().unwrap_or(true)
            || window.is_visible() != Some(true)
            || size.width == 0
            || size.height == 0
            || (canvas.x - size.width as f32).abs() > 2.0
            || (canvas.y - size.height as f32).abs() > 2.0
            || self.capture.is_some()
            || self.panels.snapshot
            || self.snapshots.capturing.is_some()
            || self.snapshots.ui.full_frame
            || self.snapshots.ui.freeze
            || renderer.vr_engaged()
            || self.exit_prompt.open
            || self.login_form.busy
            || self.options_ui.capture.is_some()
            || self.options_ui.restore.is_some()
        {
            return false;
        }
        match &self.host_session {
            Some(b) => self
                .workspace_source()
                .and_then(|s| s.hosted)
                .is_some_and(|(s, a)| b.layout_authorized(s, a, window)),
            None => true,
        }
    }
    pub(super) fn workspace_capture(&self) -> Layout {
        Layout {
            context: Default::default(),
            geometry: ui::layout::workspace_capture(&self.egui_ctx),
            viewport: Viewport::from(&self.settings.world_viewport),
            bars: Bars {
                classic: self.settings.classic_bars,
                stretch: self.settings.place_bar_stretch,
                rights_text: self.settings.place_bar_rights_text,
            },
        }
    }
    pub(super) fn workspace_cancel_preview(&mut self, restore_geometry: bool) {
        self.workspace_cancel_native_huds("cancelled");
        let Some(p) = self.workspaces.preview.take() else { return };
        self.workspace_cleanup_context();
        self.workspaces.placement = None;
        if self.workspaces.assistant.is_some() {
            self.workspaces.assistant_cancelled = true;
        }
        self.workspaces.assistant_expected = None;
        p.baseline.settings(&mut self.settings, false);
        if restore_geometry {
            ui::layout::workspace_cancel_gesture(&self.egui_ctx);
            ui::layout::workspace_restore(&self.egui_ctx);
        }
        self.workspaces.active = p.active;
        self.settings.workspace_held = None;
        ui::layout::workspace_preview(&self.egui_ctx, None);
        self.workspaces.advance();
        self.workspaces.status = if self.workspaces.catalogue.is_some() {
            Status::Ready
        } else {
            Status::Unavailable
        };
        self.layout_autosave.request(std::time::Instant::now());
        self.settings.save();
    }
    pub(super) fn workspace_tick(&mut self) {
        let source = self.workspace_source();
        if self.workspaces.source != source {
            // Normal logout/login events call cancel before World changes. A
            // late mismatched source cannot restore old geometry into the new one.
            // Loss of host receipt/grant is not a different native World.
            // Cleanup may restore its own baseline without admitting an action.
            let same = self
                .workspaces
                .source
                .as_ref()
                .is_some_and(|s| s.agent == self.world.agent_id && self.world.login.as_ref().is_some_and(|l| l.session_id == s.login));
            self.workspace_cancel_preview(same);
            self.workspaces.bind(source);
            self.workspace_ui.reset();
        }
        let permit = self.workspace_ui.open && self.gfx.as_ref().is_some_and(|g| self.workspace_permit(&g.renderer, &g.window));
        if self.workspaces.permit != permit {
            self.workspaces.advance();
        }
        self.workspaces.permit = permit;
        if (self.panels.settings || !permit && self.workspaces.assistant.is_none()) && self.workspaces.preview.is_some() {
            self.workspace_cancel_preview(true);
        }
        let polled = self.workspaces.lane.as_ref().map_or(Ok(None), Lane::poll);
        if polled.is_err() {
            let current = self
                .workspaces
                .pending
                .as_ref()
                .is_some_and(|p| p.binding == self.workspaces.binding && self.workspaces.source.as_ref() == Some(&p.source));
            self.workspaces.lane = None;
            self.workspaces.pending = None;
            if current {
                self.workspaces.status = Status::Error;
                self.workspaces.catalogue = None;
                self.workspaces.advance();
            }
        }
        if let Ok(Some(reply)) = polled {
            self.workspaces.accept_reply(reply);
        }
        if self.workspace_ui.open
            && self.workspaces.permit
            && self.workspaces.pending.is_none()
            && self.workspaces.catalogue.is_none()
            && !matches!(self.workspaces.status, Status::Error)
        {
            self.workspace_io(None, None);
        }
        if self.workspaces.pending.is_some() && self.workspaces.permit {
            self.egui_ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }
    /// Called before World consumes login/disconnect, including exact
    /// same-source reauthentication. No old IO/UI capture can be readmitted.
    pub(super) fn workspace_source_boundary(&mut self) {
        self.workspace_cancel_native_huds("source_changed");
        super::mcp::preferences_native_opening(self);
        self.workspace_cancel_preview(true);
        self.workspaces.bind(None);
        self.workspace_ui.reset();
    }
    fn workspace_io(&mut self, next: Option<Catalogue>, active: Option<Uuid>) {
        if self.workspaces.pending.is_some() || self.workspaces.exhausted {
            return;
        }
        let Some(source) = &self.workspaces.source else { return };
        let Ok(directory) = crate::session_host::profile::NativeDirectory::select(
            crate::session_host::profile::NativeStoreKind::Workspaces,
            source.agent,
            &source.grid,
        ) else {
            self.workspaces.status = Status::Error;
            return;
        };
        if self.workspaces.lane.is_none() {
            self.workspaces.lane = Lane::new().ok();
        }
        let Some(op) = self.workspaces.operation.checked_add(1) else {
            self.workspaces.exhausted = true;
            return;
        };
        let writing = next.is_some();
        let expected = next.clone();
        let write = next.map(|n| {
            (
                if self.workspaces.persisted {
                    self.workspaces.catalogue.clone()
                } else {
                    None
                },
                n,
            )
        });
        let r = Request {
            source: source.clone(),
            operation: op,
            binding: self.workspaces.binding,
            directory,
            account: source.agent,
            grid: source.grid.clone(),
            write,
        };
        if self.workspaces.lane.as_ref().is_some_and(|l| l.submit(r)) {
            self.workspaces.operation = op;
            self.workspaces.pending = Some(Pending {
                source: source.clone(),
                expected,
                binding: self.workspaces.binding,
                operation: op,
                active,
                writing,
            });
            self.workspaces.status = if writing { Status::Saving } else { Status::Loading };
        } else {
            self.workspaces.status = Status::Error;
        }
    }
    /// Passive bounded catalogue demand: never opens the manager, modifies a
    /// layout, or writes definitions. The selected native profile owns the IO.
    pub(super) fn workspace_ensure_catalogue(&mut self) -> Result<bool, &'static str> {
        if self.workspace_source() != self.workspaces.source || self.workspaces.source.is_none() {
            return Err("native_workspace_source_not_ready");
        }
        if self.workspaces.catalogue.is_some() {
            return Ok(true);
        }
        if matches!(self.workspaces.status, Status::Error | Status::Invalid) {
            return Err("native_workspace_catalogue_unavailable");
        }
        if self.workspaces.pending.is_none() {
            self.workspace_io(None, None);
        }
        Ok(false)
    }
    pub(super) fn workspace_assistant_allowed(&self) -> bool {
        !self.panels.settings
            && self.settings.curated_held.is_none()
            && self.workspaces.native_huds.is_none()
            && self.gfx.as_ref().is_some_and(|g| self.workspace_permit(&g.renderer, &g.window))
    }
    pub(super) fn workspace_assistant_begin(
        &mut self,
        ticket: &crate::workspace::controller::Ticket,
        target: Option<Uuid>,
        operation: Uuid,
    ) -> Result<Layout, &'static str> {
        if !self.workspaces.source_accepts(ticket)
            || self.workspace_source().as_ref() != Some(&ticket.source)
            || !self.workspace_assistant_allowed()
            || self.workspaces.pending.is_some()
            || self.workspaces.preview.is_some()
            || self.workspaces.placement.is_some()
            || self.workspaces.assistant.is_some()
        {
            return Err("native_workspace_busy_or_stale_source");
        }
        let definition = if let Some(id) = target {
            self.workspaces.record(id).ok_or("workspace_not_found")?.layout.clone()
        } else {
            self.workspaces.previous.as_ref().ok_or("workspace_previous_unavailable")?.0.clone()
        };
        if !self.workspace_context_allowed(&definition.context) {
            return Err("native_workspace_context_restricted");
        }
        let current = self.workspace_capture_like(&definition.context);
        if !self.workspaces.advance() {
            return Err("native_workspace_revision_exhausted");
        }
        let baseline = if let Some(id) = target {
            let layout = self.workspaces.record(id).ok_or("workspace_not_found")?.layout.clone();
            layout.validate().map_err(|_| "invalid_native_workspace")?;
            self.workspaces.begin_preview(layout, id, current, true)
        } else {
            self.workspaces.begin_previous(current).ok_or("workspace_previous_unavailable")?
        };
        self.workspaces.assistant = Some(operation);
        self.workspaces.assistant_cancelled = false;
        self.settings.workspace_held = Some(baseline.clone());
        ui::layout::workspace_preview(&self.egui_ctx, Some(baseline.geometry));
        self.workspaces
            .placement
            .as_ref()
            .ok_or("native_workspace_not_staged")?
            .layout
            .settings(&mut self.settings, self.host_session.is_some());
        self.workspace_stage_context(&baseline.context, &definition.context);
        self.workspaces.assistant_expected = Some(self.workspace_capture());
        self.egui_ctx.stop_dragging();
        self.egui_ctx.input_mut(|i| i.pointer = Default::default());
        self.egui_ctx.request_repaint();
        Ok(baseline)
    }
    pub(super) fn workspace_assistant_cancel(&mut self, operation: Uuid) -> bool {
        if self.workspaces.assistant != Some(operation) {
            return false;
        }
        let current = self.workspace_capture();
        let expected = self.workspaces.assistant_expected.clone();
        if let Some(expected) = expected
            && let Some(preview) = self.workspaces.preview.as_mut()
        {
            macro_rules! keep_external {($($field:ident).+)=>{if current.$($field).+ != expected.$($field).+ {preview.baseline.$($field).+ = current.$($field).+;}};}
            keep_external!(viewport.enabled);
            keep_external!(viewport.restrict_ui);
            keep_external!(viewport.left);
            keep_external!(viewport.right);
            keep_external!(viewport.top);
            keep_external!(viewport.bottom);
            keep_external!(bars.classic);
            keep_external!(bars.stretch);
            keep_external!(bars.rights_text);
            let owned = std::array::from_fn(|i| current.geometry[i] == expected.geometry[i]);
            ui::layout::workspace_restore_mask(&self.egui_ctx, owned);
        }
        self.workspace_cancel_preview(true);
        true
    }
    pub(super) fn workspace_open(&mut self) {
        if self.workspaces.preview.is_some() {
            self.workspace_cancel_preview(true);
        }
        self.panels.settings = false;
        self.settings.save();
        self.workspaces.advance();
        self.workspace_ui.open = true;
        self.egui_ctx.request_repaint();
    }
    pub(super) fn workspace_action(&mut self, action: Action, renderer: &Renderer, window: &Window) {
        let target = match &action.command {
            Command::Rename(id, _) | Command::Delete(id) | Command::Preview(id) | Command::Switch(id) | Command::Favorite(id, _) => {
                Some(*id)
            }
            _ => None,
        };
        if target.is_some() && target != action.ticket.record {
            return;
        }
        if !self.workspaces.accepts(&action.ticket)
            || self.workspace_source().as_ref() != Some(&action.ticket.source)
            || !self.workspace_permit(renderer, window)
            || self.panels.settings
        {
            return;
        }
        if matches!(action.command, Command::Cancel) {
            self.workspace_cancel_native_huds("cancelled");
            if self.workspaces.preview.is_some() {
                self.workspace_cancel_preview(true);
            }
            return;
        }
        if matches!(action.command, Command::Retry) {
            if self.workspaces.pending.is_none() {
                self.workspaces.catalogue = None;
                self.workspaces.advance();
                self.workspace_io(None, None);
            }
            return;
        }
        if !self.workspaces.ready() || !self.workspaces.advance() {
            return;
        }
        let commit = matches!(&action.command, Command::Switch(_));
        let layout_only = matches!(&action.command, Command::SaveLayout(_));
        let capture_context = match &action.command {
            Command::SaveContext(_, options) => *options,
            _ => Default::default(),
        };
        let rename = match &action.command {
            Command::Rename(id, _) => Some(*id),
            _ => None,
        };
        match action.command {
            Command::Preview(id) | Command::Switch(id) => {
                let Some(layout) = self.workspaces.record(id).map(|r| r.layout.clone()) else {
                    return;
                };
                if layout.validate().is_err() {
                    return;
                }
                if !self.workspace_context_allowed(&layout.context) {
                    self.workspaces.status = Status::Invalid;
                    return;
                }
                let context = layout.context.clone();
                if commit && !self.workspace_start_native_huds(context.huds.clone(), action.ticket.source.clone()) {
                    return;
                }
                let current = self.workspace_capture_like(&context);
                if self.workspaces.preview.is_some() {
                    ui::layout::workspace_restore(&self.egui_ctx);
                }
                let baseline = self.workspaces.begin_preview(layout, id, current, commit);
                self.settings.workspace_held = Some(baseline.clone());
                ui::layout::workspace_preview(&self.egui_ctx, Some(baseline.geometry));
                self.workspaces
                    .placement
                    .as_ref()
                    .expect("staged workspace")
                    .layout
                    .settings(&mut self.settings, self.host_session.is_some());
                self.workspace_stage_context(&baseline.context, &context);
                self.egui_ctx.stop_dragging();
                self.egui_ctx.input_mut(|i| i.pointer = Default::default());
            }
            Command::Previous => {
                let Some(context) = self.workspaces.previous.as_ref().map(|(l, _)| l.context.clone()) else {
                    return;
                };
                if !self.workspace_context_allowed(&context) {
                    self.workspaces.status = Status::Invalid;
                    return;
                }
                if !self.workspace_start_native_huds(context.huds.clone(), action.ticket.source.clone()) {
                    return;
                }
                let current = self.workspace_capture_like(&context);
                let Some(baseline) = self.workspaces.begin_previous(current) else {
                    return;
                };
                self.settings.workspace_held = Some(baseline.clone());
                ui::layout::workspace_preview(&self.egui_ctx, Some(baseline.geometry));
                self.workspaces
                    .placement
                    .as_ref()
                    .expect("staged previous workspace")
                    .layout
                    .settings(&mut self.settings, self.host_session.is_some());
                self.workspace_stage_context(&baseline.context, &context);
                self.egui_ctx.stop_dragging();
                self.egui_ctx.input_mut(|i| i.pointer = Default::default());
            }
            Command::Save(name) | Command::SaveLayout(name) | Command::SaveContext(name, _) | Command::Rename(_, name) => {
                if self.workspaces.preview.is_some() || !valid_name(&name) {
                    self.workspaces.status = Status::Invalid;
                    return;
                }
                let mut next = self.workspaces.catalogue.clone().expect("ready workspace catalogue");
                if !next.name_available(&name, rename) {
                    self.workspaces.status = Status::Invalid;
                    return;
                }
                let mut active = self.workspaces.active;
                match rename {
                    None => {
                        if next.records.len() >= LIMIT {
                            self.workspaces.status = Status::Invalid;
                            return;
                        }
                        let mut layout = self.workspace_capture();
                        if capture_context.graphics {
                            let mut g = crate::workspace::context::Graphics::capture(&self.settings);
                            g.active = None;
                            layout.context.graphics = Some(g);
                        }
                        if capture_context.camera {
                            layout.context.camera = Some(crate::workspace::context::Camera::capture(
                                &self.settings,
                                self.camera.zoom_fraction(),
                            ));
                        }
                        if capture_context.huds {
                            layout.context.huds = self.workspace_huds();
                        }
                        if layout_only {
                            layout.geometry = [None; 4];
                        }
                        if layout.validate().is_err() {
                            self.workspaces.status = Status::Invalid;
                            return;
                        }
                        let id = Uuid::new_v4();
                        next.records.push(Record {
                            id,
                            name,
                            layout,
                            kind: if layout_only { Kind::Layout } else { Kind::Workspace },
                            favorite: false,
                        });
                        active = Some(id);
                    }
                    Some(id) => {
                        let Some(r) = next.records.iter_mut().find(|r| r.id == id) else {
                            return;
                        };
                        r.name = name;
                    }
                }
                let Some(revision) = (if self.workspaces.persisted {
                    next.revision.checked_add(1)
                } else {
                    Some(1)
                }) else {
                    self.workspaces.exhausted = true;
                    return;
                };
                next.revision = revision;
                self.workspace_io(Some(next), active);
            }
            Command::Delete(id) => {
                if self.workspaces.preview.is_some() {
                    return;
                }
                let mut next = self.workspaces.catalogue.clone().expect("ready workspace catalogue");
                let before = next.records.len();
                next.records.retain(|r| r.id != id);
                if next.records.len() == before {
                    return;
                }
                let Some(revision) = (if self.workspaces.persisted {
                    next.revision.checked_add(1)
                } else {
                    Some(1)
                }) else {
                    self.workspaces.exhausted = true;
                    return;
                };
                next.revision = revision;
                self.workspace_io(Some(next), self.workspaces.active.filter(|a| *a != id));
            }
            Command::Favorite(id, favorite) => {
                let mut next = self.workspaces.catalogue.clone().expect("ready workspace catalogue");
                let Some(record) = next.records.iter_mut().find(|r| r.id == id) else {
                    return;
                };
                record.favorite = favorite;
                let Some(revision) = next.revision.checked_add(u64::from(self.workspaces.persisted)) else {
                    return;
                };
                next.revision = revision;
                self.workspace_io(Some(next), self.workspaces.active);
            }
            Command::Cancel | Command::Retry => {}
        }
        self.egui_ctx.request_repaint();
    }
    pub(super) fn workspace_after_pass(&mut self, renderer: &Renderer, window: &Window) {
        self.workspace_native_huds_tick(renderer, window);
        if self.workspaces.placement.is_none() {
            return;
        }
        if let Some(operation) = self.workspaces.assistant
            && !super::mcp::workspace_placement_current(self, operation)
        {
            self.workspace_assistant_cancel(operation);
            return;
        }
        if self.workspace_source() != self.workspaces.source {
            self.workspace_cancel_preview(false);
            return;
        }
        if !self.workspace_permit(renderer, window) {
            self.workspace_cancel_preview(true);
            return;
        }
        let mut p = self.workspaces.placement.take().expect("workspace placement");
        let room = ui::layout::move_area(&self.egui_ctx);
        if p.passes == 0 || p.room != Some(room) {
            if p.passes >= 3 {
                self.workspace_cancel_preview(true);
                return;
            }
            if p.restore_absence {
                ui::layout::workspace_previous(&self.egui_ctx, &p.layout.geometry);
            } else {
                ui::layout::workspace_apply(&self.egui_ctx, &p.layout.geometry);
            }
            p.passes += 1;
            p.room = Some(room);
            self.workspaces.placement = Some(p);
            if self.workspaces.assistant.is_some() {
                self.workspaces.assistant_expected = Some(self.workspace_capture());
            }
        } else if p.commit {
            self.workspaces.finish_placement(p);
            self.settings.workspace_held = None;
            self.settings.workspace_context_held = None;
            ui::layout::workspace_preview(&self.egui_ctx, None);
            self.layout_autosave.request(std::time::Instant::now());
            self.settings.save();
        }
        self.egui_ctx.request_repaint();
    }
    /// A Cancel captured by this pass wins over a pending Switch's last
    /// placement pass. Other gestures start after settling earlier placement.
    pub(super) fn workspace_after_ui(&mut self, action: Option<Action>, renderer: &Renderer, window: &Window) {
        if action.as_ref().is_some_and(|a| matches!(a.command, Command::Cancel)) {
            if let Some(action) = action {
                self.workspace_action(action, renderer, window);
            }
            self.workspace_after_pass(renderer, window);
        } else {
            self.workspace_after_pass(renderer, window);
            if let Some(action) = action {
                self.workspace_action(action, renderer, window);
            }
        }
    }
    pub(super) fn workspace_modified(&self) -> bool {
        self.workspaces.active.and_then(|id| self.workspaces.record(id)).is_some_and(|r| {
            let mut current = self.workspace_capture_like(&r.layout.context);
            if r.kind == Kind::Layout {
                current.geometry = [None; 4];
            }
            current != r.layout
        })
    }
}
