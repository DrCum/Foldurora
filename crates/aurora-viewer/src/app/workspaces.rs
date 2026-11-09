//! Native workspace effects stay on the App thread. The store only publishes
//! definitions to its captured profile; replies never apply geometry.
use super::{App, Renderer, Screen, Window, ui};
use crate::workspace::{
    controller::{Action, Command, Pending, Source, Status},
    model::{Bars, CaptureGroups, Catalogue, Graphics, LIMIT, Layout, Record, Startup, Viewport, valid_name},
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
    fn workspace_capture(&mut self, groups: CaptureGroups) -> Layout {
        let folder = groups
            .inventory
            .then(|| self.inventory_ui.workspace_folder(&self.world.inventory))
            .flatten();
        let inventory_folder = folder.filter(|id| self.workspace_folder_allowed(*id));
        Layout {
            inventory_folder,
            graphics: groups.graphics.then(|| Graphics::capture(&self.settings)),
            geometry: ui::layout::workspace_capture(&self.egui_ctx),
            viewport: Viewport::from(&self.settings.world_viewport),
            bars: Bars {
                classic: self.settings.classic_bars,
                stretch: self.settings.place_bar_stretch,
                rights_text: self.settings.place_bar_rights_text,
            },
        }
    }
    fn workspace_capture_for_save(&mut self) -> Layout {
        let groups = self
            .workspaces
            .catalogue
            .as_ref()
            .map_or(CaptureGroups::default(), |c| CaptureGroups {
                inventory: c.remember_folders,
                graphics: c.capture_graphics,
            });
        self.workspace_capture(groups)
    }
    pub(super) fn workspace_cancel_preview(&mut self, restore_geometry: bool) {
        let Some(p) = self.workspaces.preview.take() else { return };
        self.workspaces.placement = None;
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
        let permit = self.gfx.as_ref().is_some_and(|g| self.workspace_permit(&g.renderer, &g.window));
        if self.workspaces.permit != permit {
            self.workspaces.advance();
        }
        self.workspaces.permit = permit;
        if (self.panels.settings || !permit) && self.workspaces.preview.is_some() {
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
        if (self.workspace_ui.open || self.workspaces.startup_waiting)
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
        self.workspace_startup();
        self.workspace_remember_last(false);
    }
    /// Called before World consumes login/disconnect, including exact
    /// same-source reauthentication. No old IO/UI capture can be readmitted.
    pub(super) fn workspace_source_boundary(&mut self) {
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
                let current = self.workspace_capture(layout.groups());
                if current.validate().is_err() {
                    self.workspaces.status = Status::Invalid;
                    return;
                }
                if let Some(preview) = &self.workspaces.preview {
                    ui::layout::workspace_restore(&self.egui_ctx);
                    preview.baseline.settings(&mut self.settings, self.host_session.is_some());
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
                self.egui_ctx.stop_dragging();
                self.egui_ctx.input_mut(|i| i.pointer = Default::default());
            }
            Command::Previous => {
                let groups = self
                    .workspaces
                    .previous
                    .as_ref()
                    .map_or(CaptureGroups::default(), |(layout, _)| layout.groups());
                let current = self.workspace_capture(groups);
                if current.validate().is_err() {
                    self.workspaces.status = Status::Invalid;
                    return;
                }
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
                        let layout = self.workspace_capture_for_save();
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
                    next.last_layout = Some(self.workspace_capture_for_save());
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
        } else if p.commit {
            let folder_restored = self.workspace_restore_folder(p.layout.inventory_folder);
            self.workspaces.finish_placement(p);
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
        !graphics_match || !saved.matches(&self.workspace_capture(saved.groups()))
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

    fn workspace_save_definitions(&mut self, mut next: Catalogue) {
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
            self.workspace_io(Some(next), self.workspaces.active);
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
        let current = self.workspace_capture(layout.groups());
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
        self.workspaces
            .placement
            .as_ref()
            .expect("startup placement")
            .layout
            .settings(&mut self.settings, self.host_session.is_some());
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
        let mut layout = self.workspace_capture_for_save();
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
}
