//! Native named geometry manager. Drawing never mutates the catalogue or World.
use crate::theme::Palette;
use crate::workspace::controller::{Action, Command, Controller, Status, Ticket};
use uuid::Uuid;
pub(crate) const WINDOW: &str = "named_workspaces";
#[derive(Default)]
pub(crate) struct WorkspaceUi {
    pub open: bool,
    selected: Option<Uuid>,
    name: String,
    delete: Option<(Ticket, Uuid)>,
    painted: Option<Ticket>,
}
impl WorkspaceUi {
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }
    pub(crate) fn show(&mut self, ctx: &egui::Context, p: &Palette, c: &Controller, modified: bool) -> Option<Action> {
        use aurora_i18n::tr;
        if !self.open {
            self.painted = None;
            return None;
        }
        if self.delete.as_ref().is_some_and(|(t, _)| !c.accepts(t)) {
            self.delete = None;
        }
        let current = c.ticket().map(|mut t| {
            t.record = self.selected;
            t
        });
        let ticket = self.painted.take().filter(|t| Some(t) == current.as_ref() && c.accepts(t));
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
                                if ui.selectable_label(self.selected == Some(r.id), &r.name).clicked() {
                                    self.selected = Some(r.id);
                                    self.name = r.name.clone();
                                    self.delete = None;
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
                    if ui
                        .add_enabled(enabled && c.preview.is_none(), egui::Button::new(tr!("workspaces-save")))
                        .clicked()
                    {
                        command = Some(Command::Save(self.name.trim().to_owned()));
                    }
                    if let Some(id) = self.selected.filter(|id| c.record(*id).is_some()) {
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
                        if ui.add_enabled(enabled, egui::Button::new(tr!("workspaces-switch"))).clicked() {
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
        self.open = open;
        if was_open && !open && c.preview.is_some() {
            command = Some(Command::Cancel);
        }
        self.painted = if open { current } else { None };
        let ticket = ticket?;
        let command = command?;
        let target = match &command {
            Command::Rename(id, _) | Command::Delete(id) | Command::Preview(id) | Command::Switch(id) => Some(*id),
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
            found = texts
                .iter()
                .rfind(|t| t.source == aurora_i18n::tr!("workspaces-manage"))
                .map(|t| t.rect.center());
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
