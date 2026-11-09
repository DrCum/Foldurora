//! Native named geometry manager. Drawing never mutates the catalogue or World.
use crate::theme::Palette;
use crate::workspace::controller::{Action, Command, Controller, Status, Ticket};
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
    capture: crate::workspace::context::Capture,
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
            capture: Default::default(),
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
    pub(crate) fn show(&mut self, ctx: &egui::Context, p: &Palette, c: &Controller, modified: bool) -> Option<Action> {
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
            egui::vec2(540.0, 460.0),
        )
        .help(tr!("workspaces-help"))
        .show(ctx, p, &mut open, |ui| {
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
                };
                ui.label(status);
                if let Some(report) = &c.native_hud_report {
                    if c.native_huds.is_some() {
                        ui.label(tr!("workspaces-huds-pending"));
                    } else if !report.complete {
                        ui.label(tr!("workspaces-huds-unconfirmed"));
                    }
                }
                if let Some(r) = c.preview.as_ref().map(|p| p.candidate).or(c.active).and_then(|id| c.record(id)) {
                    ui.label(tr!(
                        "workspaces-current",
                        name = r.name.clone(),
                        marker = if modified { " *" } else { "" }
                    ));
                }
                egui::ScrollArea::vertical()
                    .id_salt("workspace_records")
                    .max_height(190.0)
                    .show(ui, |ui| {
                        if let Some(cat) = &c.catalogue {
                            for r in &cat.records {
                                let row = ui.selectable_label(self.selected == Some(r.id), &r.name);
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
                ui.horizontal_wrapped(|ui| {
                    ui.checkbox(&mut self.capture.graphics, tr!("workspaces-remember-graphics"));
                    ui.checkbox(&mut self.capture.camera, tr!("workspaces-remember-camera"));
                    ui.checkbox(&mut self.capture.huds, tr!("workspaces-remember-huds"));
                });
                ui.push_id(("workspace_selection_controls", selection_scope), |ui| {
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .add_enabled(enabled && c.preview.is_none(), egui::Button::new(tr!("workspaces-save")))
                            .clicked()
                        {
                            let name = self.name.trim().to_owned();
                            command = Some(if self.capture.graphics || self.capture.camera || self.capture.huds {
                                Command::SaveContext(name, self.capture)
                            } else {
                                Command::Save(name)
                            });
                        }
                        if ui
                            .add_enabled(enabled && c.preview.is_none(), egui::Button::new(tr!("workspaces-save-layout")))
                            .clicked()
                        {
                            command = Some(Command::SaveLayout(self.name.trim().to_owned()));
                        }
                        if let Some(id) = self.selected.filter(|id| c.record(*id).is_some()) {
                            if let Some(record) = c.record(id)
                                && ui
                                    .add_enabled(
                                        enabled && c.preview.is_none(),
                                        egui::Button::new(if record.favorite {
                                            tr!("workspaces-unfavorite")
                                        } else {
                                            tr!("workspaces-favorite")
                                        }),
                                    )
                                    .clicked()
                            {
                                command = Some(Command::Favorite(id, !record.favorite));
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
                            .add_enabled(
                                ticket.is_some() && (c.preview.is_some() || c.native_huds.is_some()),
                                egui::Button::new(tr!("workspaces-cancel")),
                            )
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
        self.open = open;
        if selection_scope != self.selection_epoch || self.selection_exhausted {
            self.painted = None;
            return None;
        }
        if was_open && !open && c.preview.is_some() {
            command = Some(Command::Cancel);
        }
        self.painted = if open { current } else { None };
        let ticket = ticket?;
        let command = command?;
        let target = match &command {
            Command::Rename(id, _) | Command::Delete(id) | Command::Preview(id) | Command::Switch(id) | Command::Favorite(id, _) => {
                Some(*id)
            }
            _ => None,
        };
        if target.is_some() && target != ticket.record {
            return None;
        }
        Some(Action { ticket, command })
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
            context: Default::default(),
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
            kind: crate::workspace::model::Kind::Workspace,
            favorite: false,
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
        let p = crate::theme::Theme::default().palette();
        let mut result = None;
        let texts = frames.run(events, |ui| {
            let action = st.show(ui.ctx(), &p, c, false);
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
                    kind: crate::workspace::model::Kind::Workspace,
                    favorite: false,
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
            kind: crate::workspace::model::Kind::Workspace,
            favorite: false,
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
}
