//! Native workspace effects stay on the App thread. The store only publishes
//! definitions to its captured profile; replies never apply geometry.
use super::{App, Renderer, Screen, Window, ui};
use crate::workspace::{
    controller::{Action, Command, Pending, Source, Status},
    model::{
        Background, Bars, CameraContext, CaptureGroups, Catalogue, Graphics, HudSet, LIMIT, Layout, Record, Startup, Viewport, valid_name,
    },
    store::{Lane, Request},
};
use uuid::Uuid;

impl App {
    fn workspace_source(&self) -> Option<Source> {
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
    pub(super) fn workspace_background(&self) -> Background {
        if !matches!(self.screen, Screen::World)
            || self.capture.is_some()
            || self.panels.snapshot
            || self.snapshots.capturing.is_some()
            || self.snapshots.ui.full_frame
            || self.snapshots.ui.freeze
            || (self.interface.hidden && !self.camera.mouselook())
            || self
                .workspaces
                .source
                .as_ref()
                .is_none_or(|s| Some(s) != self.workspace_source().as_ref())
        {
            return Background::Black {};
        }
        self.workspaces.background()
    }
    fn workspace_camera_allowed(&self) -> bool {
        matches!(self.screen, Screen::World)
            && self
                .workspaces
                .source
                .as_ref()
                .is_some_and(|s| Some(s) == self.workspace_source().as_ref())
            && self.camera.third_person()
            && self.camera.focus_point().is_none()
            && self.camera.free().is_none()
            && !self.world.rlva.camera_preset_locked(Uuid::nil())
            && self.world.rlva.camera_owner().is_none()
    }
    fn workspace_apply_settings(&mut self, layout: &Layout) {
        let before = self.settings.camera.clone();
        layout.settings(&mut self.settings, self.host_session.is_some());
        if let Some(camera) = &layout.camera {
            // Rollback and ordinary application use the same fresh native guards.
            // A new source or lock must never be overwritten by an old Preview.
            if self.workspaces.source.as_ref().is_none_or(|s| {
                s.agent != self.world.agent_id
                    || self.world.login.as_ref().is_none_or(|login| login.session_id != s.login)
                    || crate::session_host::transport::grid_key(&self.settings.login_uri())
                        .ok()
                        .is_none_or(|grid| grid.to_string() != s.grid)
            }) {
                self.settings.camera = before;
                return;
            }
            self.world.rlva.preserve_camera_settings(&mut self.settings.camera, &before);
            if camera.validate().is_ok() && self.workspace_camera_allowed() {
                // Preserve orbit/pitch and camera mode; only avatar-relative zoom
                // is part of this group. Never replay free/object/photo anchors.
                self.camera.set_zoom_fraction(camera.zoom);
            }
        }
    }
    fn workspace_capture(&mut self, groups: CaptureGroups) -> Option<Layout> {
        if groups.camera && !self.workspace_camera_allowed() {
            return None;
        }
        let folder = groups
            .inventory
            .then(|| self.inventory_ui.workspace_folder(&self.world.inventory))
            .flatten();
        let inventory_folder = folder.filter(|id| self.workspace_folder_allowed(*id));
        let huds = if groups.huds {
            self.workspaces.source.clone().and_then(|source| {
                crate::workspace::huds::worn(&self.world, &source, &mut self.workspace_huds.policy)
                    .and_then(|rows| HudSet::try_from(rows.into_iter().map(|r| r.reference).collect::<Vec<_>>()).ok())
            })
        } else {
            None
        };
        Some(Layout {
            huds,
            inventory_folder,
            background: groups.background.then(|| self.workspaces.background()),
            camera: groups
                .camera
                .then(|| CameraContext::capture(&self.settings.camera, self.camera.zoom_fraction())),
            graphics: groups.graphics.then(|| Graphics::capture(&self.settings)),
            geometry: ui::layout::workspace_capture(&self.egui_ctx),
            viewport: Viewport::from(&self.settings.world_viewport),
            bars: Bars {
                classic: self.settings.classic_bars,
                stretch: self.settings.place_bar_stretch,
                rights_text: self.settings.place_bar_rights_text,
            },
        })
    }
    fn workspace_capture_for_save(&mut self) -> Result<Layout, Status> {
        let groups = self
            .workspaces
            .catalogue
            .as_ref()
            .map_or(CaptureGroups::default(), |c| CaptureGroups {
                inventory: c.remember_folders,
                graphics: c.capture_graphics,
                camera: c.capture_camera,
                background: c.capture_background,
                huds: c.capture_huds,
            });
        let mut layout = self.workspace_capture(groups).ok_or(Status::CameraUnavailable)?;
        if groups.huds {
            let source = self.workspaces.source.as_ref().ok_or(Status::HudUnavailable)?;
            let selected = self
                .workspaces
                .catalogue
                .as_ref()
                .ok_or(Status::HudUnavailable)?
                .selected_huds
                .clone();
            let rows = crate::workspace::huds::resolve(&self.world, source, &mut self.workspace_huds.policy, &selected)
                .filter(|rows| rows.len() == selected.refs().len() && rows.iter().all(|r| r.worn))
                .ok_or(Status::HudUnavailable)?;
            layout.huds = Some(HudSet::try_from(rows.into_iter().map(|r| r.reference).collect::<Vec<_>>()).map_err(|_| Status::Invalid)?);
        }
        Ok(layout)
    }
    pub(super) fn workspace_cancel_preview(&mut self, restore_geometry: bool) {
        let Some(p) = self.workspaces.preview.take() else { return };
        self.workspaces.placement = None;
        self.workspace_apply_settings(&p.baseline);
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
            self.workspace_huds.retire();
            self.workspace_cancel_preview(same);
            self.workspaces.bind(source);
            self.workspace_ui.reset();
        }
        let permit = self.gfx.as_ref().is_some_and(|g| self.workspace_permit(&g.renderer, &g.window));
        if self.workspaces.permit != permit {
            self.workspaces.advance();
        }
        self.workspaces.permit = permit;
        let camera_revoked =
            self.workspaces.preview.as_ref().is_some_and(|p| p.baseline.camera.is_some()) && !self.workspace_camera_allowed();
        if (self.panels.settings || !permit || camera_revoked) && self.workspaces.preview.is_some() {
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
        if self.workspaces.needs_catalogue() {
            self.workspace_io(None, None);
        }
        if self.workspaces.pending.is_some() && self.workspaces.permit {
            self.egui_ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        self.workspace_startup();
        self.workspace_remember_last(false);
    }
    /// Called before World consumes login/disconnect, including exact
    /// same-source reauthentication. No old IO/UI capture can be readmitted.
    pub(super) fn workspace_source_boundary(&mut self) {
        self.workspace_huds.retire();
        self.workspace_cancel_preview(true);
        self.workspace_finish_io();
        self.workspace_remember_last(true);
        self.workspace_finish_io();
        // Only logout/reauthentication resets startup, not activation or focus.
        self.workspaces.startup_source = None;
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
    pub(super) fn workspace_open(&mut self) {
        self.workspaces.cancel_startup();
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
            Command::Rename(id, _)
            | Command::ReviewHuds(id)
            | Command::Update(id)
            | Command::Delete(id)
            | Command::Preview(id)
            | Command::Switch(id)
            | Command::Favorite(id)
            | Command::Startup(Startup::Named { workspace: id }) => Some(*id),
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
            self.workspace_huds.retire();
            if self.workspaces.preview.is_some() {
                self.workspace_cancel_preview(true);
            }
            return;
        }
        if matches!(action.command, Command::Retry) {
            self.workspace_huds.retire();
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
        if !matches!(action.command, Command::DecideHuds(..)) {
            self.workspace_huds.retire();
        }
        self.workspace_huds_prepare(renderer, window);
        self.workspaces.cancel_startup();
        let commit = matches!(&action.command, Command::Switch(_));
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
                let Some(current) = self.workspace_capture(layout.groups()) else {
                    self.workspaces.status = Status::CameraUnavailable;
                    return;
                };
                if current.validate().is_err() {
                    self.workspaces.status = Status::Invalid;
                    return;
                }
                if let Some(baseline) = self.workspaces.preview.as_ref().map(|p| p.baseline.clone()) {
                    ui::layout::workspace_restore(&self.egui_ctx);
                    self.workspace_apply_settings(&baseline);
                }
                let baseline = self.workspaces.begin_preview(layout, id, current, commit);
                self.settings.workspace_held = Some(baseline.clone());
                ui::layout::workspace_preview(&self.egui_ctx, Some(baseline.geometry));
                let layout = self.workspaces.placement.as_ref().expect("staged workspace").layout.clone();
                self.workspace_apply_settings(&layout);
                self.egui_ctx.stop_dragging();
                self.egui_ctx.input_mut(|i| i.pointer = Default::default());
            }
            Command::Previous => {
                let groups = self
                    .workspaces
                    .previous
                    .as_ref()
                    .map_or(CaptureGroups::default(), |(layout, _)| layout.groups());
                let Some(current) = self.workspace_capture(groups) else {
                    self.workspaces.status = Status::CameraUnavailable;
                    return;
                };
                if current.validate().is_err() {
                    self.workspaces.status = Status::Invalid;
                    return;
                }
                let Some(baseline) = self.workspaces.begin_previous(current) else {
                    return;
                };
                self.settings.workspace_held = Some(baseline.clone());
                ui::layout::workspace_preview(&self.egui_ctx, Some(baseline.geometry));
                let layout = self.workspaces.placement.as_ref().expect("staged workspace").layout.clone();
                self.workspace_apply_settings(&layout);
                self.egui_ctx.stop_dragging();
                self.egui_ctx.input_mut(|i| i.pointer = Default::default());
            }
            Command::Save(name) | Command::Rename(_, name) => {
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
                        let layout = match self.workspace_capture_for_save() {
                            Ok(layout) => layout,
                            Err(status) => {
                                self.workspaces.status = status;
                                return;
                            }
                        };
                        if layout.validate().is_err() {
                            self.workspaces.status = Status::Invalid;
                            return;
                        }
                        let id = Uuid::new_v4();
                        next.records.push(Record { id, name, layout });
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
            Command::Update(id) => {
                if self.workspaces.preview.is_some() {
                    return;
                }
                let layout = match self.workspace_capture_for_save() {
                    Ok(layout) => layout,
                    Err(status) => {
                        self.workspaces.status = status;
                        return;
                    }
                };
                let mut next = self.workspaces.catalogue.clone().expect("ready workspace catalogue");
                if next.update_layout(id, layout).is_err() {
                    self.workspaces.status = Status::Invalid;
                    return;
                }
                self.workspace_save_with_active(next, Some(id));
            }
            Command::Delete(id) => {
                if self.workspaces.preview.is_some() {
                    return;
                }
                let mut next = self.workspaces.catalogue.clone().expect("ready workspace catalogue");
                let before = next.records.len();
                next.records.retain(|r| r.id != id);
                next.remove_references(id);
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
            Command::Favorite(id) => {
                if self.workspaces.preview.is_some() || self.workspaces.record(id).is_none() {
                    return;
                }
                let mut next = self.workspaces.catalogue.clone().expect("ready workspace catalogue");
                if next.favorites.contains(&id) {
                    next.favorites.retain(|favorite| *favorite != id);
                } else {
                    next.favorites.push(id);
                }
                self.workspace_save_definitions(next);
            }
            Command::Startup(startup) => {
                if self.workspaces.preview.is_some() {
                    return;
                }
                let mut next = self.workspaces.catalogue.clone().expect("ready workspace catalogue");
                next.startup = startup;
                if startup == Startup::LastArrangement {
                    let layout = match self.workspace_capture_for_save() {
                        Ok(layout) => layout,
                        Err(status) => {
                            self.workspaces.status = status;
                            return;
                        }
                    };
                    next.last_layout = Some(layout);
                }
                self.workspace_save_definitions(next);
            }
            Command::ShowFavorites(show) => {
                if self.workspaces.preview.is_some() {
                    return;
                }
                let mut next = self.workspaces.catalogue.clone().expect("ready workspace catalogue");
                next.show_favorites = show;
                self.workspace_save_definitions(next);
            }
            Command::RememberFolders(remember) => {
                if self.workspaces.preview.is_some() {
                    return;
                }
                let mut next = self.workspaces.catalogue.clone().expect("ready workspace catalogue");
                next.remember_folders = remember;
                if !remember && let Some(last) = &mut next.last_layout {
                    last.inventory_folder = None;
                }
                self.workspace_save_definitions(next);
            }
            Command::Background(background) => {
                if self.workspaces.preview.is_some() {
                    return;
                }
                let mut next = self.workspaces.catalogue.clone().expect("ready workspace catalogue");
                next.background = background;
                self.workspace_save_definitions(next);
            }
            Command::CaptureBackground(capture) => {
                if self.workspaces.preview.is_some() {
                    return;
                }
                let mut next = self.workspaces.catalogue.clone().expect("ready workspace catalogue");
                next.capture_background = capture;
                if !capture && let Some(last) = &mut next.last_layout {
                    last.background = None;
                }
                self.workspace_save_definitions(next);
            }
            Command::CaptureCamera(capture) => {
                if self.workspaces.preview.is_some() {
                    return;
                }
                let mut next = self.workspaces.catalogue.clone().expect("ready workspace catalogue");
                next.capture_camera = capture;
                if !capture && let Some(last) = &mut next.last_layout {
                    last.camera = None;
                }
                self.workspace_save_definitions(next);
            }
            Command::CaptureGraphics(capture) => {
                if self.workspaces.preview.is_some() {
                    return;
                }
                let mut next = self.workspaces.catalogue.clone().expect("ready workspace catalogue");
                next.capture_graphics = capture;
                if !capture && let Some(last) = &mut next.last_layout {
                    last.graphics = None;
                }
                self.workspace_save_definitions(next);
            }
            Command::CaptureHuds(capture) => {
                if self.workspaces.preview.is_some() {
                    return;
                }
                let mut next = self.workspaces.catalogue.clone().expect("ready workspace catalogue");
                next.capture_huds = capture;
                if !capture && let Some(last) = &mut next.last_layout {
                    last.huds = None;
                }
                self.workspace_save_definitions(next);
            }
            Command::ClearHuds => {
                if self.workspaces.preview.is_some() {
                    return;
                }
                let mut next = self.workspaces.catalogue.clone().expect("ready workspace catalogue");
                next.selected_huds = HudSet::default();
                self.workspace_save_definitions(next);
            }
            Command::SelectHud(reference, selected) => {
                if self.workspaces.preview.is_some() {
                    return;
                }
                let Some(source) = self.workspaces.source.clone() else { return };
                let Some(rows) = crate::workspace::huds::worn(&self.world, &source, &mut self.workspace_huds.policy) else {
                    self.workspaces.status = Status::HudUnavailable;
                    return;
                };
                if !rows.iter().any(|r| r.reference == reference) {
                    return;
                }
                let mut next = self.workspaces.catalogue.clone().expect("ready workspace catalogue");
                let mut refs = next.selected_huds.refs().to_vec();
                refs.retain(|r| r.item != reference.item);
                if selected {
                    refs.push(reference);
                }
                let Ok(refs) = HudSet::try_from(refs) else { return };
                next.selected_huds = refs;
                self.workspace_save_definitions(next);
            }
            Command::ReviewHuds(id) => {
                if self.workspaces.preview.is_some() {
                    return;
                }
                if let Some(wanted) = self.workspaces.record(id).and_then(|r| r.layout.huds.clone()) {
                    self.workspace_huds_offer(&wanted, renderer, window);
                }
            }
            Command::DecideHuds(nonce, accepted) => {
                if self.workspaces.preview.is_some() {
                    return;
                }
                let Some(source) = self.workspaces.source.clone() else { return };
                self.workspace_huds.decide(&self.world, &source, nonce, accepted);
            }
            Command::Cancel | Command::Retry => {}
        }
        self.egui_ctx.request_repaint();
    }
    pub(super) fn workspace_after_pass(&mut self, renderer: &Renderer, window: &Window) {
        if self.workspaces.placement.is_none() {
            return;
        }
        if self.workspace_source() != self.workspaces.source {
            self.workspace_cancel_preview(false);
            return;
        }
        if !self.workspace_permit(renderer, window)
            || (self.workspaces.preview.as_ref().is_some_and(|p| p.baseline.camera.is_some()) && !self.workspace_camera_allowed())
        {
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
        } else if p.commit {
            let huds = p.layout.huds.clone();
            let background = p.layout.background;
            let folder_restored = self.workspace_restore_folder(p.layout.inventory_folder);
            self.workspaces.finish_placement(p);
            if let Some(huds) = huds {
                self.workspace_huds_offer(&huds, renderer, window);
            }
            if let Some(background) = background
                && let Some(mut catalogue) = self.workspaces.catalogue.clone()
                && catalogue.background != background
            {
                catalogue.background = background;
                self.workspace_save_definitions(catalogue);
            }
            if !folder_restored {
                self.workspaces.status = Status::FolderUnavailable;
            }
            self.settings.workspace_held = None;
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
        self.workspace_huds_tick(renderer, window);
    }
    pub(super) fn workspace_modified(&mut self) -> bool {
        let saved = self
            .workspaces
            .active
            .and_then(|id| self.workspaces.record(id))
            .map(|r| r.layout.clone());
        let Some(mut saved) = saved else {
            return false;
        };
        let graphics_match = saved
            .graphics
            .as_ref()
            .is_none_or(|graphics| graphics.matches_settings(&self.settings));
        // Ordinary frames compare the typed allowlist; no JSON decode/default
        // Settings construction belongs in the recurring modified indicator.
        saved.graphics = None;
        if !self.workspace_camera_allowed() {
            saved.camera = None;
        }
        !graphics_match
            || self
                .workspace_capture(saved.groups())
                .is_some_and(|current| !saved.matches(&current))
    }

    fn workspace_restore_folder(&mut self, folder: Option<Uuid>) -> bool {
        let Some(folder) = folder else {
            return true;
        };
        let allowed = self.workspace_folder_allowed(folder);
        if allowed {
            self.inventory_ui.restore_workspace_folder(&self.world.inventory, folder);
        }
        allowed
    }
    fn workspace_folder_allowed(&mut self, folder: Uuid) -> bool {
        let owner = Uuid::nil(); // The exact main Inventory owner, never a current extra window.
        if self.world.inventory.owner != self.world.agent_id
            || self.world.rlva.has("showinv")
            || !self.inventory_ui.jump_owner_visible(&self.egui_ctx, self.panels.inventory, owner)
        {
            return false;
        }
        {
            let (instance, read) = super::folder_jump::archive_read(&mut self.inventory_ui, &self.world, owner);
            let gate = ui::inventory::archive::State::ordinary(instance, read.as_ref(), self.world.inventory_archive_instance);
            read.as_ref().is_some_and(|read| read.passes(folder))
                && ui::inventory::folder_jump::current_folder(&self.world.inventory, folder, gate.mask())
        }
    }

    fn workspace_save_definitions(&mut self, next: Catalogue) {
        self.workspace_save_with_active(next, self.workspaces.active);
    }
    fn workspace_save_with_active(&mut self, mut next: Catalogue, active: Option<Uuid>) {
        let Some(revision) = (if self.workspaces.persisted {
            next.revision.checked_add(1)
        } else {
            Some(1)
        }) else {
            self.workspaces.exhausted = true;
            return;
        };
        next.revision = revision;
        if next.validate().is_ok() {
            self.workspace_io(Some(next), active);
        }
    }

    fn workspace_finish_io(&mut self) {
        if self.workspaces.pending.is_none() {
            return;
        }
        match self
            .workspaces
            .lane
            .as_ref()
            .map(|lane| lane.finish(std::time::Duration::from_millis(1500)))
        {
            Some(Ok(Some(reply))) => self.workspaces.accept_reply(reply),
            _ => log::warn!("workspace: final write did not finish before the login boundary"),
        }
    }

    fn workspace_startup(&mut self) {
        if !self.workspaces.startup_waiting {
            return;
        }
        if self.panels.settings || self.workspaces.status == Status::Error {
            self.workspaces.cancel_startup();
            return;
        }
        let Some((id, layout)) = self.workspaces.take_startup() else {
            return;
        };
        if layout.validate().is_err() {
            return;
        }
        let Some(current) = self.workspace_capture(layout.groups()) else {
            self.workspaces.status = Status::CameraUnavailable;
            return;
        };
        if current.validate().is_err() {
            self.workspaces.status = Status::Invalid;
            return;
        }
        let baseline = self.workspaces.begin_preview(layout, id.unwrap_or(Uuid::nil()), current, true);
        if id.is_none() {
            self.workspaces.placement.as_mut().expect("startup placement").active = None;
        }
        self.settings.workspace_held = Some(baseline.clone());
        ui::layout::workspace_preview(&self.egui_ctx, Some(baseline.geometry));
        let layout = self.workspaces.placement.as_ref().expect("staged workspace").layout.clone();
        self.workspace_apply_settings(&layout);
        self.egui_ctx.request_repaint();
    }

    fn workspace_remember_last(&mut self, force: bool) {
        if self.workspaces.pending.is_some()
            || self.workspaces.preview.is_some()
            || self.panels.settings
            || self.workspaces.placement.is_some()
            || self.workspaces.source.as_ref() != self.workspace_source().as_ref()
            || self
                .workspaces
                .catalogue
                .as_ref()
                .is_none_or(|c| c.startup != Startup::LastArrangement)
            || (!force && !self.workspaces.permit)
        {
            return;
        }
        let Ok(mut layout) = self.workspace_capture_for_save() else {
            return;
        };
        if layout.inventory_folder.is_none() && self.workspaces.catalogue.as_ref().is_some_and(|c| c.remember_folders) {
            // An unobserved/closed owner must not erase an earlier authorized
            // folder reference in this same account/grid's saved arrangement.
            layout.inventory_folder = self
                .workspaces
                .catalogue
                .as_ref()
                .and_then(|c| c.last_layout.as_ref())
                .and_then(|l| l.inventory_folder);
        }
        let now = std::time::Instant::now();
        let stable = self
            .workspaces
            .last_observed
            .as_ref()
            .is_some_and(|(old, since)| old == &layout && now.saturating_duration_since(*since) >= std::time::Duration::from_secs(2));
        if !stable && !force {
            if self.workspaces.last_observed.as_ref().is_none_or(|(old, _)| old != &layout) {
                self.workspaces.last_observed = Some((layout, now));
            }
            return;
        }
        if self
            .workspaces
            .catalogue
            .as_ref()
            .is_some_and(|c| c.last_layout.as_ref() == Some(&layout))
        {
            return;
        }
        let mut next = self.workspaces.catalogue.clone().expect("loaded startup settings");
        next.last_layout = Some(layout);
        self.workspace_save_definitions(next);
    }
    pub(super) fn workspace_huds_prepare(&mut self, renderer: &Renderer, window: &Window) {
        if !self.workspace_ui.open {
            // Closing the manager cancels future work even if its final paint
            // could not emit an action ticket. Idle frames need no HUD scan.
            self.workspace_huds.retire();
            return;
        }
        let authorized = self.workspace_permit(renderer, window)
            && !renderer.failed()
            && !renderer.device_lost()
            && renderer.stalled_for().is_none()
            && self.workspace_source() == self.workspaces.source
            && !self.panels.settings;
        self.workspace_huds
            .sync(&self.world, self.workspaces.source.as_ref(), self.workspaces.binding, authorized);
    }
    fn workspace_huds_offer(&mut self, wanted: &HudSet, renderer: &Renderer, window: &Window) {
        self.workspace_ui.open = true;
        self.workspace_huds_prepare(renderer, window);
        let Some(source) = self.workspaces.source.clone() else { return };
        self.workspace_huds.offer(&self.world, &source, wanted);
        // A switch may originate from favorites/startup while the manager is
        // closed. Make the explicit review discoverable; never auto-accept it.
        self.workspace_ui.open = true;
    }
    fn workspace_huds_tick(&mut self, renderer: &Renderer, window: &Window) {
        self.workspace_huds_prepare(renderer, window);
        if !self.workspace_huds.busy() {
            return;
        }
        self.egui_ctx.request_repaint_after(std::time::Duration::from_millis(100));
        let Some(batch) = self.workspace_huds.next(&self.world, self.appearance_ui.pending.is_some()) else {
            return;
        };
        let Some(source) = self.workspaces.source.clone() else {
            self.workspace_huds.retire();
            return;
        };
        let Ok(refs) = HudSet::try_from(batch.clone()) else {
            self.workspace_huds.refuse();
            return;
        };
        if crate::workspace::huds::resolve(&self.world, &source, &mut self.workspace_huds.policy, &refs)
            .is_none_or(|rows| rows.len() != batch.len())
        {
            self.workspace_huds.refuse();
            return;
        }
        // Keep Aurora's complete-COF/attachment-limit/RLVa plan and final send
        // guards. No direct rez, replacement, detach, fetching or copied assets.
        let action = crate::world::appearance::Action::WearItems {
            items: batch.iter().map(|r| r.item).collect(),
            replace: false,
            point: batch[0].point,
        };
        // Refuse incomplete COF/capacity before the native UI's normal error
        // recovery can fetch missing inventory. This feature never initiates
        // hidden metadata fetches to make a captured HUD available.
        if crate::world::appearance::plan(&self.world.inventory, action.clone(), &self.world.outfit_attachment_points()).is_err() {
            self.workspace_huds.dispatch_failed();
            return;
        }
        self.apply_appearance_action(action);
        if self.appearance_ui.pending.is_none() {
            self.workspace_huds.dispatch_failed();
        }
    }
}
