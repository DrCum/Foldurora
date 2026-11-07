//! Native workspace effects stay on the App thread. The store only publishes
//! definitions to its captured profile; replies never apply geometry.
use super::{App, Gfx, Screen, ui};
use crate::workspace::{
    controller::{Action, Command, Pending, Source, Status},
    model::{Bars, Catalogue, LIMIT, Layout, Record, Viewport, valid_name},
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
    fn workspace_permit(&self, gfx: &Gfx) -> bool {
        let size = gfx.window.inner_size();
        let canvas = self.egui_ctx.viewport_rect().size() * self.egui_ctx.pixels_per_point();
        if !matches!(self.screen, Screen::World)
            || self.workspace_source().is_none()
            || !self.focused
            || !gfx.window.has_focus()
            || gfx.window.is_minimized().unwrap_or(true)
            || gfx.window.is_visible() != Some(true)
            || size.width == 0
            || size.height == 0
            || (canvas.x - size.width as f32).abs() > 2.0
            || (canvas.y - size.height as f32).abs() > 2.0
            || self.capture.is_some()
            || self.panels.snapshot
            || self.snapshots.capturing.is_some()
            || self.snapshots.ui.full_frame
            || self.snapshots.ui.freeze
            || gfx.renderer.vr_engaged()
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
                .is_some_and(|(s, a)| b.layout_authorized(s, a, &gfx.window)),
            None => true,
        }
    }
    fn workspace_capture(&self) -> Layout {
        Layout {
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
        let permit = self.workspace_ui.open && self.gfx.as_ref().is_some_and(|g| self.workspace_permit(g));
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
        self.workspace_cancel_preview(true);
        self.workspaces.bind(None);
        self.workspace_ui.reset();
    }
    fn workspace_io(&mut self, next: Option<Catalogue>, active: Option<Uuid>) {
        if self.workspaces.pending.is_some() || self.workspaces.exhausted {
            return;
        }
        let Some(source) = &self.workspaces.source else { return };
        let Ok(directory) = crate::session_host::profile::WorkspaceDirectory::select(source.agent, &source.grid) else {
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
        if self.workspaces.preview.is_some() {
            self.workspace_cancel_preview(true);
        }
        self.panels.settings = false;
        self.settings.save();
        self.workspaces.advance();
        self.workspace_ui.open = true;
        self.egui_ctx.request_repaint();
    }
    pub(super) fn workspace_action(&mut self, action: Action, gfx: &Gfx) {
        let target = match &action.command {
            Command::Rename(id, _) | Command::Delete(id) | Command::Preview(id) | Command::Switch(id) => Some(*id),
            _ => None,
        };
        if target.is_some() && target != action.ticket.record {
            return;
        }
        if !self.workspaces.accepts(&action.ticket)
            || self.workspace_source().as_ref() != Some(&action.ticket.source)
            || !self.workspace_permit(gfx)
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
                let current = self.workspace_capture();
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
                self.egui_ctx.stop_dragging();
                self.egui_ctx.input_mut(|i| i.pointer = Default::default());
            }
            Command::Previous => {
                let current = self.workspace_capture();
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
                        let layout = self.workspace_capture();
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
            Command::Cancel | Command::Retry => {}
        }
        self.egui_ctx.request_repaint();
    }
    pub(super) fn workspace_after_pass(&mut self, gfx: &Gfx) {
        if self.workspaces.placement.is_none() {
            return;
        }
        if self.workspace_source() != self.workspaces.source {
            self.workspace_cancel_preview(false);
            return;
        }
        if !self.workspace_permit(gfx) {
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
            self.workspaces.finish_placement(p);
            self.settings.workspace_held = None;
            ui::layout::workspace_preview(&self.egui_ctx, None);
            self.layout_autosave.request(std::time::Instant::now());
            self.settings.save();
        }
        self.egui_ctx.request_repaint();
    }
    /// A Cancel captured by this pass wins over a pending Switch's last
    /// placement pass. Other gestures start after settling earlier placement.
    pub(super) fn workspace_after_ui(&mut self, action: Option<Action>, gfx: &Gfx) {
        if action.as_ref().is_some_and(|a| matches!(a.command, Command::Cancel)) {
            if let Some(action) = action {
                self.workspace_action(action, gfx);
            }
            self.workspace_after_pass(gfx);
        } else {
            self.workspace_after_pass(gfx);
            if let Some(action) = action {
                self.workspace_action(action, gfx);
            }
        }
    }
    pub(super) fn workspace_modified(&self) -> bool {
        self.workspaces
            .active
            .and_then(|id| self.workspaces.record(id))
            .is_some_and(|r| self.workspace_capture() != r.layout)
    }
}
