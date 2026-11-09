//! Native named geometry manager. Drawing never mutates the catalogue or World.
use crate::theme::Palette;
use crate::workspace::controller::{Action, Command, Controller, Status, Ticket};
use crate::workspace::model::{Background, Startup};
use uuid::Uuid;
pub(crate) const WINDOW: &str = "named_workspaces";
pub(crate) struct WorkspaceUi {
    pub open: bool,
    selected: Option<Uuid>,
    name: String,
    delete: Option<(Ticket, Uuid)>,
    painted: Option<Ticket>,
    selection_epoch: u64,
    selection_observed: Option<Uuid>,
    selection_exhausted: bool,
    favorites_painted: Option<Ticket>,
    background_edit: Option<[u8; 4]>,
}
impl Default for WorkspaceUi {
    fn default() -> Self {
        Self {
            open: false,
            selected: None,
            name: String::new(),
            delete: None,
            painted: None,
            selection_epoch: 1,
            selection_observed: None,
            selection_exhausted: false,
            favorites_painted: None,
            background_edit: None,
        }
    }
}
impl WorkspaceUi {
    fn selection_changed(&mut self) {
        if let Some(next) = self.selection_epoch.checked_add(1) {
            self.selection_epoch = next;
        } else {
            self.selection_exhausted = true;
        }
        self.selection_observed = self.selected;
        self.painted = None;
        self.delete = None;
    }
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }
    pub(crate) fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        c: &Controller,
        modified: bool,
        huds: &crate::workspace::huds::View,
    ) -> Option<Action> {
        use aurora_i18n::tr;
        if !self.open {
            self.painted = None;
            return None;
        }
        if self.selection_observed != self.selected {
            self.selection_changed();
        }
        let selection_scope = self.selection_epoch;
        if self.delete.as_ref().is_some_and(|(t, _)| !c.accepts(t)) {
            self.delete = None;
        }
        let current = c.ticket().map(|mut t| {
            t.record = self.selected;
            t
        });
        let ticket = self
            .painted
            .take()
            .filter(|t| !self.selection_exhausted && Some(t) == current.as_ref() && c.accepts(t));
        let enabled = ticket.is_some() && c.ready();
        let was_open = self.open;
        let mut open = self.open;
        let mut command = None;
        let screen = ctx.content_rect();
        super::widgets::Floater::new(
            WINDOW,
            tr!("workspaces-title"),
            screen.min + egui::vec2(60.0, 80.0),
            egui::vec2(540.0, 540.0),
        )
        .help(tr!("workspaces-help"))
        .show(ctx, p, &mut open, |ui| {
            egui::ScrollArea::vertical().id_salt("workspace_manager_body").show(ui, |ui| {
                ui.push_id((c.instance, c.binding, c.revision), |ui| {
                    ui.label(tr!("workspaces-help"));
                    if c.source.as_ref().is_some_and(|s| s.hosted.is_some()) {
                        ui.label(tr!("workspaces-hosted"));
                    }
                    let status = match if !c.permit && !matches!(c.status, Status::Error | Status::Loading | Status::Saving) {
                        Status::Unavailable
                    } else {
                        c.status
                    } {
                        Status::Unavailable => tr!("workspaces-unavailable"),
                        Status::Loading => tr!("workspaces-loading"),
                        Status::Saving => tr!("workspaces-saving"),
                        Status::Error => tr!("workspaces-error"),
                        Status::Invalid => tr!("workspaces-invalid"),
                        Status::Previewing => tr!("workspaces-previewing"),
                        Status::Ready => tr!("workspaces-ready"),
                        Status::FolderUnavailable => tr!("workspaces-folder-unavailable"),
                        Status::CameraUnavailable => tr!("workspaces-camera-unavailable"),
                        Status::HudUnavailable => tr!("workspaces-huds-unavailable"),
                    };
                    ui.label(status);
                    if let Some(r) = c.preview.as_ref().map(|p| p.candidate).or(c.active).and_then(|id| c.record(id)) {
                        ui.label(tr!(
                            "workspaces-current",
                            name = r.name.clone(),
                            marker = if modified { " *" } else { "" }
                        ));
                    }
                    if c.permit {
                        use crate::workspace::hud_lane::Progress;
                        let message = match huds.progress {
                            Progress::Idle => None,
                            Progress::Review => Some(tr!("workspaces-huds-review")),
                            Progress::Running => Some(tr!("workspaces-huds-running")),
                            Progress::Complete => Some(tr!("workspaces-huds-complete")),
                            Progress::Unavailable => Some(tr!("workspaces-huds-unavailable")),
                            Progress::Declined => Some(tr!("workspaces-huds-declined")),
                            Progress::Interrupted => Some(tr!("workspaces-huds-interrupted")),
                            Progress::TimedOut => Some(tr!("workspaces-huds-timeout")),
                            Progress::Failed => Some(tr!("workspaces-huds-failed")),
                        };
                        if let Some(message) = message {
                            ui.label(message);
                        }
                        if let Some((nonce, rows)) = &huds.review {
                            for row in rows {
                                ui.label(&row.name);
                            }
                            ui.push_id(("workspace_hud_review", nonce), |ui| {
                                ui.horizontal_wrapped(|ui| {
                                    if ui
                                        .add_enabled(enabled && c.preview.is_none(), egui::Button::new(tr!("workspaces-huds-accept")))
                                        .clicked()
                                    {
                                        command = Some(Command::DecideHuds(*nonce, true));
                                    }
                                    if ui.add_enabled(enabled, egui::Button::new(tr!("common-cancel"))).clicked() {
                                        command = Some(Command::DecideHuds(*nonce, false));
                                    }
                                });
                            });
                        }
                    }
                    if let Some(cat) = &c.catalogue {
                        super::widgets::collapsing(ui, p, "workspace_huds_selection", tr!("workspaces-huds-title"), false, |ui| {
                            ui.label(tr!("workspaces-huds-help"));
                            let mut capture = cat.capture_huds;
                            if ui
                                .add_enabled_ui(enabled && c.preview.is_none() && (huds.available || capture), |ui| {
                                    super::widgets::checkbox(ui, p, &mut capture, tr!("workspaces-capture-huds"))
                                })
                                .inner
                                .changed()
                            {
                                command = Some(Command::CaptureHuds(capture));
                            }
                            if !huds.available {
                                ui.label(tr!("workspaces-huds-unavailable"));
                            }
                            for row in &huds.worn {
                                ui.push_id(("workspace_hud", row.reference), |ui| {
                                    let mut selected = cat.selected_huds.contains(row.reference);
                                    if ui
                                        .add_enabled_ui(enabled && c.preview.is_none() && huds.available, |ui| {
                                            super::widgets::checkbox(ui, p, &mut selected, &row.name)
                                        })
                                        .inner
                                        .changed()
                                    {
                                        command = Some(Command::SelectHud(row.reference, selected));
                                    }
                                });
                            }
                            if ui
                                .add_enabled(
                                    enabled && c.preview.is_none() && !cat.selected_huds.is_empty(),
                                    egui::Button::new(tr!("workspaces-huds-clear")),
                                )
                                .clicked()
                            {
                                command = Some(Command::ClearHuds);
                            }
                        });
                        let mut background = cat.capture_background;
                        if ui
                            .add_enabled_ui(enabled && c.preview.is_none(), |ui| {
                                super::widgets::checkbox(ui, p, &mut background, tr!("workspaces-capture-background"))
                            })
                            .inner
                            .changed()
                        {
                            command = Some(Command::CaptureBackground(background));
                        }
                        ui.add_enabled_ui(enabled && c.preview.is_none(), |ui| {
                            ui.horizontal_wrapped(|ui| {
                                let color = self.background_edit.get_or_insert_with(|| match c.background() {
                                    Background::Solid { rgb: [r, g, b] } => [r, g, b, 255],
                                    Background::Black {} => {
                                        let mut rgba = p.raised.to_array();
                                        rgba[3] = 255;
                                        rgba
                                    }
                                });
                                super::widgets::color_button(ui, p, "workspace_background_color", color, false);
                                if ui.button(tr!("workspaces-background-solid")).clicked() {
                                    command = Some(Command::Background(Background::Solid {
                                        rgb: [color[0], color[1], color[2]],
                                    }));
                                }
                                if ui.button(tr!("workspaces-background-black")).clicked() {
                                    command = Some(Command::Background(Background::Black {}));
                                }
                            });
                        });
                        let mut show = cat.show_favorites;
                        let mut folders = cat.remember_folders;
                        let mut graphics = cat.capture_graphics;
                        let mut camera = cat.capture_camera;
                        if ui
                            .add_enabled_ui(enabled && c.preview.is_none(), |ui| {
                                super::widgets::checkbox(ui, p, &mut camera, tr!("workspaces-capture-camera"))
                            })
                            .inner
                            .changed()
                        {
                            command = Some(Command::CaptureCamera(camera));
                        }
                        if ui
                            .add_enabled_ui(enabled && c.preview.is_none(), |ui| {
                                super::widgets::checkbox(ui, p, &mut graphics, tr!("workspaces-capture-graphics"))
                            })
                            .inner
                            .changed()
                        {
                            command = Some(Command::CaptureGraphics(graphics));
                        }
                        if ui
                            .add_enabled_ui(enabled && c.preview.is_none(), |ui| {
                                super::widgets::checkbox(ui, p, &mut folders, tr!("workspaces-remember-folders"))
                            })
                            .inner
                            .changed()
                        {
                            command = Some(Command::RememberFolders(folders));
                        }
                        if ui
                            .add_enabled_ui(enabled && c.preview.is_none(), |ui| {
                                super::widgets::checkbox(ui, p, &mut show, tr!("workspaces-show-favorites"))
                            })
                            .inner
                            .changed()
                        {
                            command = Some(Command::ShowFavorites(show));
                        }
                        ui.horizontal_wrapped(|ui| {
                            ui.label(tr!("workspaces-startup"));
                            if let Startup::Named { workspace } = cat.startup
                                && let Some(record) = c.record(workspace)
                            {
                                ui.label(tr!("workspaces-startup-named", name = record.name.clone()));
                            }
                            if ui
                                .add_enabled(
                                    enabled && c.preview.is_none(),
                                    egui::Button::new(tr!("workspaces-startup-none")).selected(cat.startup == Startup::Disabled),
                                )
                                .clicked()
                            {
                                command = Some(Command::Startup(Startup::Disabled));
                            }
                            if ui
                                .add_enabled(
                                    enabled && c.preview.is_none(),
                                    egui::Button::new(tr!("workspaces-startup-last")).selected(cat.startup == Startup::LastArrangement),
                                )
                                .clicked()
                            {
                                command = Some(Command::Startup(Startup::LastArrangement));
                            }
                            if let Some(id) = self.selected.filter(|id| c.record(*id).is_some())
                                && ui
                                    .add_enabled(
                                        enabled && c.preview.is_none(),
                                        egui::Button::new(tr!("workspaces-startup-selected"))
                                            .selected(cat.startup == (Startup::Named { workspace: id })),
                                    )
                                    .clicked()
                            {
                                command = Some(Command::Startup(Startup::Named { workspace: id }));
                            }
                        });
                    }
                    egui::ScrollArea::vertical()
                        .id_salt("workspace_records")
                        .max_height(190.0)
                        .show(ui, |ui| {
                            if let Some(cat) = &c.catalogue {
                                for r in &cat.records {
                                    let label = if cat.favorites.contains(&r.id) {
                                        format!("★ {}", r.name)
                                    } else {
                                        r.name.clone()
                                    };
                                    let row = ui.selectable_label(self.selected == Some(r.id), label);
                                    #[cfg(test)]
                                    ui.ctx()
                                        .data_mut(|d| d.insert_temp(egui::Id::unique(("workspace_test_row", r.id)), row.id));
                                    if row.clicked() {
                                        self.selected = Some(r.id);
                                        self.name = r.name.clone();
                                        self.selection_changed();
                                    }
                                }
                            }
                        });
                    ui.add(
                        egui::TextEdit::singleline(&mut self.name)
                            .id_salt("workspace_name")
                            .char_limit(64)
                            .hint_text(tr!("workspaces-name")),
                    );
                    ui.push_id(("workspace_selection_controls", selection_scope), |ui| {
                        ui.horizontal_wrapped(|ui| {
                            if ui
                                .add_enabled(enabled && c.preview.is_none(), egui::Button::new(tr!("workspaces-save")))
                                .clicked()
                            {
                                command = Some(Command::Save(self.name.trim().to_owned()));
                            }
                            if let Some(id) = self.selected.filter(|id| c.record(*id).is_some()) {
                                if ui
                                    .add_enabled(enabled && c.preview.is_none(), egui::Button::new(tr!("workspaces-update")))
                                    .clicked()
                                {
                                    command = Some(Command::Update(id));
                                }
                                if ui
                                    .add_enabled(
                                        enabled && c.preview.is_none() && c.record(id).is_some_and(|r| r.layout.huds.is_some()),
                                        egui::Button::new(tr!("workspaces-huds-review-selected")),
                                    )
                                    .clicked()
                                {
                                    command = Some(Command::ReviewHuds(id));
                                }
                                let favorite = c.catalogue.as_ref().is_some_and(|c| c.favorites.contains(&id));
                                if ui
                                    .add_enabled(
                                        enabled && c.preview.is_none(),
                                        egui::Button::new(if favorite {
                                            tr!("workspaces-unfavorite")
                                        } else {
                                            tr!("workspaces-favorite")
                                        }),
                                    )
                                    .clicked()
                                {
                                    command = Some(Command::Favorite(id));
                                }
                                if ui
                                    .add_enabled(enabled && c.preview.is_none(), egui::Button::new(tr!("workspaces-rename")))
                                    .clicked()
                                {
                                    command = Some(Command::Rename(id, self.name.trim().to_owned()));
                                }
                                if ui
                                    .add_enabled(enabled && c.preview.is_none(), egui::Button::new(tr!("workspaces-delete")))
                                    .clicked()
                                {
                                    self.delete = ticket.clone().filter(|t| t.record == Some(id)).map(|t| (t, id));
                                }
                                if ui.add_enabled(enabled, egui::Button::new(tr!("workspaces-preview"))).clicked() {
                                    command = Some(Command::Preview(id));
                                }
                                let switch = ui.add_enabled(enabled, egui::Button::new(tr!("workspaces-switch")));
                                #[cfg(test)]
                                ui.ctx()
                                    .data_mut(|d| d.insert_temp(egui::Id::unique("workspace_test_switch"), switch.id));
                                if switch.clicked() {
                                    command = Some(Command::Switch(id));
                                }
                            }
                            if ui
                                .add_enabled(ticket.is_some() && c.preview.is_some(), egui::Button::new(tr!("workspaces-cancel")))
                                .clicked()
                            {
                                command = Some(Command::Cancel);
                            }
                            if ui
                                .add_enabled(
                                    enabled && c.preview.is_none() && c.previous.is_some(),
                                    egui::Button::new(tr!("workspaces-previous")),
                                )
                                .clicked()
                            {
                                command = Some(Command::Previous);
                            }
                            if ui
                                .add_enabled(
                                    ticket.is_some() && c.pending.is_none() && c.status == Status::Error,
                                    egui::Button::new(tr!("workspaces-retry")),
                                )
                                .clicked()
                            {
                                command = Some(Command::Retry);
                            }
                        });
                        if let Some(id) = self
                            .delete
                            .as_ref()
                            .filter(|(t, id)| c.accepts(t) && c.record(*id).is_some())
                            .map(|(_, id)| *id)
                        {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(tr!("workspaces-delete-confirm"));
                                if ui.add_enabled(enabled, egui::Button::new(tr!("workspaces-delete"))).clicked() {
                                    command = Some(Command::Delete(id));
                                    self.delete = None;
                                }
                                if ui.button(tr!("common-cancel")).clicked() {
                                    self.delete = None;
                                }
                            });
                        }
                    });
                });
            });
        });
        self.open = open;
        if selection_scope != self.selection_epoch || self.selection_exhausted {
            self.painted = None;
            return None;
        }
        if was_open && !open {
            command = Some(Command::Cancel);
        }
        self.painted = if open { current } else { None };
        let ticket = ticket?;
        let command = command?;
        let target = match &command {
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
        if target.is_some() && target != ticket.record {
            return None;
        }
        Some(Action { ticket, command })
    }

    pub(crate) fn favorites(&mut self, ctx: &egui::Context, c: &Controller) -> Option<Action> {
        let current = c.ticket();
        let ticket = self
            .favorites_painted
            .take()
            .filter(|t| Some(t) == current.as_ref() && c.accepts(t));
        let cat = c
            .catalogue
            .as_ref()
            .filter(|cat| cat.show_favorites && !cat.favorites.is_empty() && !self.open && c.ready() && c.preview.is_none())?;
        let mut selected = None;
        let bounds = super::layout::toolbar_rect(ctx);
        let area = egui::Area::new(egui::Id::unique("workspace_favorites_strip"))
            .pivot(egui::Align2::CENTER_TOP)
            .fixed_pos(bounds.center_top() + egui::vec2(0.0, 72.0))
            .show(ctx, |ui| {
                ui.set_clip_rect(bounds.intersect(ctx.content_rect()));
                ui.set_max_width((bounds.width() - 16.0).clamp(1.0, 800.0));
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        for id in &cat.favorites {
                            if let Some(record) = c.record(*id) {
                                ui.push_id((c.instance, c.binding, c.revision, id), |ui| {
                                    if ui
                                        .add_enabled(ticket.is_some(), egui::Button::new(&record.name).selected(c.active == Some(*id)))
                                        .clicked()
                                    {
                                        selected = Some(*id);
                                    }
                                });
                            }
                        }
                    });
                });
            });
        super::layout::toolbar_layer(ctx, area.response.layer_id);
        self.favorites_painted = current;
        let id = selected?;
        let mut ticket = ticket?;
        ticket.record = Some(id);
        Some(Action {
            ticket,
            command: Command::Switch(id),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::widgets::elide_tests::{Frames, Text};
    use crate::workspace::{
        controller::Source,
        model::{Bars, Catalogue, Layout, Record},
    };
    fn fixture() -> (Controller, WorkspaceUi) {
        let source = Source {
            agent: Uuid::new_v4(),
            login: Uuid::new_v4(),
            grid: "b".repeat(64),
            hosted: None,
        };
        let s = crate::settings::Settings::default();
        let layout = Layout {
            inventory_folder: None,
            graphics: None,
            camera: None,
            background: None,
            huds: None,
            geometry: [None; 4],
            viewport: (&s.world_viewport).into(),
            bars: Bars {
                classic: s.classic_bars,
                stretch: s.place_bar_stretch,
                rights_text: s.place_bar_rights_text,
            },
        };
        let id = Uuid::new_v4();
        let mut catalogue = Catalogue::empty(source.agent, source.grid.clone());
        catalogue.records.push(Record {
            id,
            name: "Desk".into(),
            layout,
        });
        (
            Controller {
                source: Some(source),
                permit: true,
                catalogue: Some(catalogue),
                status: Status::Ready,
                ..Default::default()
            },
            WorkspaceUi {
                open: true,
                selected: Some(id),
                name: "Desk".into(),
                ..Default::default()
            },
        )
    }
    fn draw(frames: &mut Frames, st: &mut WorkspaceUi, c: &Controller, events: Vec<egui::Event>) -> (Vec<Text>, Option<Action>) {
        draw_huds(frames, st, c, &Default::default(), events)
    }
    fn draw_huds(
        frames: &mut Frames,
        st: &mut WorkspaceUi,
        c: &Controller,
        huds: &crate::workspace::huds::View,
        events: Vec<egui::Event>,
    ) -> (Vec<Text>, Option<Action>) {
        let p = crate::theme::Theme::default().palette();
        let mut result = None;
        let texts = frames.run(events, |ui| {
            let action = st.show(ui.ctx(), &p, c, false, huds);
            if result.is_none() {
                result = action;
            }
        });
        (texts, result)
    }
    fn pointer(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        }
    }
    fn location(frames: &mut Frames, st: &mut WorkspaceUi, c: &Controller, label: &str) -> egui::Pos2 {
        let mut texts = Vec::new();
        for _ in 0..3 {
            texts = draw(frames, st, c, vec![]).0;
        }
        texts
            .iter()
            .rfind(|t| t.source == label)
            .expect("real native manager button is painted")
            .rect
            .center()
    }
    fn click(frames: &mut Frames, st: &mut WorkspaceUi, c: &Controller, label: &str) -> Option<Action> {
        let pos = location(frames, st, c, label);
        draw(frames, st, c, vec![egui::Event::PointerMoved(pos)]);
        assert!(draw(frames, st, c, vec![pointer(pos, true)]).1.is_none());
        draw(frames, st, c, vec![pointer(pos, false)]).1
    }
    #[test]
    fn native_workspace_actual_manager_emits_captured_save_rename_and_confirmed_delete_only() {
        let (c, mut st) = fixture();
        let id = st.selected.unwrap();
        let mut frames = Frames::new();
        st.name = "Second".into();
        let action = click(&mut frames, &mut st, &c, aurora_i18n::tr!("workspaces-save")).unwrap();
        assert!(matches!(action.command,Command::Save(ref n) if n=="Second"));
        let action = click(&mut frames, &mut st, &c, aurora_i18n::tr!("workspaces-rename")).unwrap();
        assert!(matches!(action.command,Command::Rename(i,ref n) if i==id && n=="Second"));
        assert_eq!(action.ticket.record, Some(id));
        assert!(click(&mut frames, &mut st, &c, aurora_i18n::tr!("workspaces-delete")).is_none());
        let action = click(&mut frames, &mut st, &c, aurora_i18n::tr!("workspaces-delete")).unwrap();
        assert!(matches!(action.command,Command::Delete(i) if i==id));
        assert_eq!(
            c.catalogue.as_ref().unwrap().records.len(),
            1,
            "drawing and clicking only emit typed requests"
        );
    }
    #[test]
    fn native_workspace_favorite_and_startup_controls_emit_current_source_tickets() {
        let (mut c, mut st) = fixture();
        let id = st.selected.unwrap();
        let mut frames = Frames::new();
        let favorite = click(&mut frames, &mut st, &c, aurora_i18n::tr!("workspaces-favorite")).unwrap();
        assert!(matches!(favorite.command, Command::Favorite(i) if i == id));
        assert_eq!(favorite.ticket.record, Some(id));
        assert_eq!(Some(favorite.ticket.source), c.source);
        let startup = click(&mut frames, &mut st, &c, aurora_i18n::tr!("workspaces-startup-selected")).unwrap();
        assert!(matches!(startup.command, Command::Startup(Startup::Named { workspace }) if workspace == id));
        assert_eq!(startup.ticket.record, Some(id));
        c.catalogue.as_mut().unwrap().favorites.push(id);
        c.advance();
        let favorite = click(&mut frames, &mut st, &c, aurora_i18n::tr!("workspaces-unfavorite")).unwrap();
        assert!(matches!(favorite.command, Command::Favorite(i) if i == id));
    }
    #[test]
    fn native_workspace_actual_preview_cancel_switch_and_previous_keep_original_capture() {
        let (mut c, mut st) = fixture();
        let id = st.selected.unwrap();
        let mut frames = Frames::new();
        let action = click(&mut frames, &mut st, &c, aurora_i18n::tr!("workspaces-preview")).unwrap();
        assert!(matches!(action.command,Command::Preview(i) if i==id));
        let baseline = c.record(id).unwrap().layout.clone();
        c.advance();
        c.begin_preview(baseline.clone(), id, baseline.clone(), false);
        c.placement = None;
        let action = click(&mut frames, &mut st, &c, aurora_i18n::tr!("workspaces-cancel")).unwrap();
        assert!(matches!(action.command, Command::Cancel));
        c.preview = None;
        c.advance();
        let action = click(&mut frames, &mut st, &c, aurora_i18n::tr!("workspaces-switch")).unwrap();
        assert!(matches!(action.command,Command::Switch(i) if i==id));
        c.previous = Some((baseline, None));
        c.advance();
        let action = click(&mut frames, &mut st, &c, aurora_i18n::tr!("workspaces-previous")).unwrap();
        assert!(matches!(action.command, Command::Previous));
    }
    #[test]
    fn native_workspace_queued_switch_release_cannot_rebind_to_new_login_before_replacement_paint() {
        let (mut c, mut st) = fixture();
        let mut frames = Frames::new();
        let pos = location(&mut frames, &mut st, &c, aurora_i18n::tr!("workspaces-switch"));
        draw(&mut frames, &mut st, &c, vec![egui::Event::PointerMoved(pos)]);
        draw(&mut frames, &mut st, &c, vec![pointer(pos, true)]);
        c.source.as_mut().unwrap().login = Uuid::new_v4();
        c.binding += 1;
        c.advance();
        assert!(draw(&mut frames, &mut st, &c, vec![pointer(pos, false)]).1.is_none());
        let action = click(&mut frames, &mut st, &c, aurora_i18n::tr!("workspaces-switch")).unwrap();
        assert_eq!(Some(action.ticket.source), c.source);
    }
    #[test]
    fn native_workspace_held_switch_cannot_follow_keyboard_or_accessibility_selection_even_a_b_a() {
        for accesskit in [false, true] {
            for return_to_a in [false, true] {
                let (mut c, mut st) = fixture();
                let a = st.selected.unwrap();
                let b = Uuid::new_v4();
                let layout = c.record(a).unwrap().layout.clone();
                c.catalogue.as_mut().unwrap().records.push(Record {
                    id: b,
                    name: "Other".into(),
                    layout,
                });
                let mut frames = Frames::new();
                let pos = location(&mut frames, &mut st, &c, aurora_i18n::tr!("workspaces-switch"));
                let old = frames
                    .ctx
                    .data(|d| d.get_temp::<egui::Id>(egui::Id::unique("workspace_test_switch")))
                    .unwrap();
                draw(&mut frames, &mut st, &c, vec![egui::Event::PointerMoved(pos)]);
                assert!(draw(&mut frames, &mut st, &c, vec![pointer(pos, true)]).1.is_none());
                let mut last = old;
                for target in std::iter::once(b).chain(return_to_a.then_some(a)) {
                    let widget = frames
                        .ctx
                        .data(|d| d.get_temp::<egui::Id>(egui::Id::unique(("workspace_test_row", target))))
                        .unwrap();
                    frames.ctx.memory_mut(|m| m.request_focus(widget));
                    if accesskit {
                        assert!(
                            draw(
                                &mut frames,
                                &mut st,
                                &c,
                                vec![egui::Event::AccessKitActionRequest(egui::accesskit::ActionRequest {
                                    action: egui::accesskit::Action::Click,
                                    target_tree: egui::accesskit::TreeId::ROOT,
                                    target_node: widget.accesskit_id(),
                                    data: None
                                })]
                            )
                            .1
                            .is_none()
                        );
                    } else {
                        for pressed in [true, false] {
                            assert!(
                                draw(
                                    &mut frames,
                                    &mut st,
                                    &c,
                                    vec![egui::Event::Key {
                                        key: egui::Key::Space,
                                        physical_key: None,
                                        pressed,
                                        repeat: false,
                                        modifiers: egui::Modifiers::NONE
                                    }]
                                )
                                .1
                                .is_none()
                            );
                        }
                    }
                    assert_eq!(st.selected, Some(target), "actual row selection while pointer stays held");
                    for _ in 0..4 {
                        assert!(draw(&mut frames, &mut st, &c, vec![]).1.is_none());
                    }
                    assert!(frames.ctx.input(|i| i.pointer.primary_down()));
                    let next = frames
                        .ctx
                        .data(|d| d.get_temp::<egui::Id>(egui::Id::unique("workspace_test_switch")))
                        .unwrap();
                    assert_ne!(next, old);
                    assert_ne!(next, last);
                    last = next;
                }
                assert!(
                    draw(&mut frames, &mut st, &c, vec![pointer(pos, false)]).1.is_none(),
                    "a settled replacement control cannot inherit the old press"
                );
                let expected = if return_to_a { a } else { b };
                let fresh = click(&mut frames, &mut st, &c, aurora_i18n::tr!("workspaces-switch")).unwrap();
                assert!(matches!(fresh.command,Command::Switch(id) if id==expected));
            }
        }
    }
    #[test]
    fn native_workspace_selection_epoch_exhaustion_keeps_real_controls_disabled_without_reuse() {
        let (mut c, mut st) = fixture();
        let a = st.selected.unwrap();
        let b = Uuid::new_v4();
        let layout = c.record(a).unwrap().layout.clone();
        c.catalogue.as_mut().unwrap().records.push(Record {
            id: b,
            name: "Other".into(),
            layout,
        });
        st.selection_observed = st.selected;
        st.selection_epoch = u64::MAX;
        let mut frames = Frames::new();
        location(&mut frames, &mut st, &c, aurora_i18n::tr!("workspaces-switch"));
        let row = frames
            .ctx
            .data(|d| d.get_temp::<egui::Id>(egui::Id::unique(("workspace_test_row", b))))
            .unwrap();
        assert!(
            draw(
                &mut frames,
                &mut st,
                &c,
                vec![egui::Event::AccessKitActionRequest(egui::accesskit::ActionRequest {
                    action: egui::accesskit::Action::Click,
                    target_tree: egui::accesskit::TreeId::ROOT,
                    target_node: row.accesskit_id(),
                    data: None
                })]
            )
            .1
            .is_none()
        );
        assert!(st.selection_exhausted);
        assert_eq!(st.selection_epoch, u64::MAX);
        assert!(click(&mut frames, &mut st, &c, aurora_i18n::tr!("workspaces-switch")).is_none());
    }
    #[test]
    fn native_workspace_preferences_layout_entry_uses_real_options_result_without_changing_layout() {
        let mut frames = Frames::new();
        let mut s = crate::settings::Settings::default();
        let before = s.world_viewport.clone();
        let mut st = super::super::options::OptionsUi::default();
        st.open(super::super::options::Page::Layout, 0);
        let p = crate::theme::Theme::default().palette();
        let icons = super::super::icons::Icons::default();
        let mut found = None;
        for _ in 0..3 {
            let texts = frames.run(vec![], |ui| {
                super::super::options::show(ui.ctx(), &p, &icons, &mut s, &mut st, &mut true);
            });
            // Cx::draw_row paints the control body before its identical,
            // passive row label. The last matching text is not the button.
            let matches: Vec<_> = texts.iter().filter(|t| t.source == aurora_i18n::tr!("workspaces-manage")).collect();
            assert_eq!(matches.len(), 2, "actual manager button and passive row label");
            let button = matches[0].rect;
            let label = matches[1].rect;
            assert!(button.left() > label.right(), "the control is to the right of its row label");
            found = Some(button.center());
        }
        let pos = found.expect("actual Preferences Layout manager entry");
        let mut open_manager = false;
        for event in [egui::Event::PointerMoved(pos), pointer(pos, true), pointer(pos, false)] {
            frames.run(vec![event], |ui| {
                open_manager |= super::super::options::show(ui.ctx(), &p, &icons, &mut s, &mut st, &mut true).open_workspaces;
            });
        }
        assert!(open_manager);
        assert_eq!(s.world_viewport, before);
    }
    #[test]
    fn native_workspace_update_is_a_source_bound_request_without_mutating_the_painted_record() {
        let (c, mut st) = fixture();
        let id = st.selected.unwrap();
        let original = c.catalogue.as_ref().unwrap().encode().unwrap();
        let mut frames = Frames::new();
        let update = click(&mut frames, &mut st, &c, aurora_i18n::tr!("workspaces-update")).unwrap();
        assert!(matches!(update.command, Command::Update(target) if target == id));
        assert_eq!(update.ticket.record, Some(id));
        assert_eq!(Some(update.ticket.source), c.source);
        assert_eq!(c.catalogue.as_ref().unwrap().encode().unwrap(), original);
    }
    #[test]
    fn native_workspace_hud_review_buttons_cannot_retarget_a_replaced_nonce() {
        let (c, mut st) = fixture();
        let mut frames = Frames::new();
        let mut huds = crate::workspace::huds::View {
            available: true,
            review: Some((
                Uuid::new_v4(),
                vec![crate::workspace::huds::HudRow {
                    reference: crate::workspace::model::HudRef {
                        item: Uuid::new_v4(),
                        point: 31,
                    },
                    name: "Visible owned HUD".into(),
                    worn: false,
                }],
            )),
            progress: crate::workspace::hud_lane::Progress::Review,
            ..Default::default()
        };
        let mut texts = Vec::new();
        for _ in 0..3 {
            texts = draw_huds(&mut frames, &mut st, &c, &huds, vec![]).0;
        }
        let pos = texts
            .iter()
            .find(|t| t.source == aurora_i18n::tr!("workspaces-huds-accept"))
            .unwrap()
            .rect
            .center();
        draw_huds(&mut frames, &mut st, &c, &huds, vec![egui::Event::PointerMoved(pos)]);
        assert!(draw_huds(&mut frames, &mut st, &c, &huds, vec![pointer(pos, true)]).1.is_none());
        let nonce = Uuid::new_v4();
        huds.review.as_mut().unwrap().0 = nonce;
        assert!(draw_huds(&mut frames, &mut st, &c, &huds, vec![pointer(pos, false)]).1.is_none());
        draw_huds(&mut frames, &mut st, &c, &huds, vec![]);
        draw_huds(&mut frames, &mut st, &c, &huds, vec![pointer(pos, true)]);
        let action = draw_huds(&mut frames, &mut st, &c, &huds, vec![pointer(pos, false)]).1.unwrap();
        assert!(matches!(action.command, Command::DecideHuds(n, true) if n == nonce));
        assert!(
            c.catalogue.as_ref().unwrap().selected_huds.is_empty(),
            "paint only emits a typed request"
        );
    }
}
