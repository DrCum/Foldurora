//! Actual egui/model/channel cases, authored UNRUN. No fixture starts a worker
//! or reads a profile; Root owns all execution and native acceptance.
use super::*;
use crate::session_host::slots::{SlotLabel, SlotSummary};
use crate::session_host::transport::copy_fixture::{self, Request};
use aurora_session::{HostId, InstallationId};
use egui::{Event, Modifiers, PointerButton, Pos2, Vec2};

#[derive(Clone, Debug)]
pub(super) struct SourceStackNode {
    pub(super) id: Id,
    pub(super) parent: Option<Id>,
    pub(super) kind: Option<egui::UiKind>,
}
#[derive(Clone)]
pub(super) struct SourceDiagnostic {
    pub(super) control: Control,
    pub(super) response_rect: Rect,
    pub(super) response_interact_rect: Rect,
    pub(super) response_sense: egui::Sense,
    pub(super) painting_count: usize,
    pub(super) stack: Vec<SourceStackNode>,
    pub(super) flags: [bool; 5],
    pub(super) pass: usize,
}

fn rect_bits(rect: Rect) -> [u32; 4] {
    [
        rect.min.x.to_bits(),
        rect.min.y.to_bits(),
        rect.max.x.to_bits(),
        rect.max.y.to_bits(),
    ]
}

fn bounded_diagnostic(mut text: String) -> String {
    if text.len() > 65_536 {
        let mut end = 65_500;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str("\n[bounded diagnostic truncated]");
    }
    text
}

fn slot(index: u8) -> SlotId {
    SlotId::try_from(index).unwrap()
}
fn snapshot() -> HostSnapshot {
    HostSnapshot {
        slots: SlotId::all()
            .map(|id| SlotSummary {
                id,
                label: SlotLabel::new(&"W".repeat(64)).unwrap(),
                available: true,
            })
            .collect(),
        ..Default::default()
    }
}
fn pointer(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    }
}
fn key(key: Key, pressed: bool) -> Event {
    Event::Key {
        key,
        physical_key: Some(key),
        pressed,
        repeat: false,
        modifiers: Modifiers::NONE,
    }
}
#[derive(Clone, Copy, Debug)]
enum Late {
    None,
    Discard,
    OtherModal,
    Close,
    FocusLoss,
    MoveWidget,
    ReclipWidget,
    ResenseWidget,
    CoverWidget,
    CoverLayer,
    VisualWidgetCover,
    VisualLayerCover,
    FocusWidget,
    Transform(egui::emath::TSTransform),
    StructuralFrameMove,
    StructuralFrameSense,
    AmbiguousFrame,
    SameSizeVisualCover,
}
struct Frames {
    ctx: egui::Context,
    model: CopyUi,
    snapshot: HostSnapshot,
    size: Vec2,
    scale: f32,
    frame: u64,
    transform_layer: Option<egui::LayerId>,
    last_diagnostic: String,
}
impl Frames {
    fn new() -> Self {
        let ctx = egui::Context::default();
        crate::theme::Theme {
            panel_alpha: 255,
            ..Default::default()
        }
        .apply(&ctx, 1.0);
        let snapshot = snapshot();
        let mut model = CopyUi::default();
        model.open(slot(0), &snapshot);
        let mut frames = Self {
            ctx,
            model,
            snapshot,
            size: egui::vec2(760.0, 600.0),
            scale: 1.0,
            frame: 0,
            transform_layer: None,
            last_diagnostic: String::new(),
        };
        frames.show(vec![], Late::None);
        frames.show(vec![], Late::None);
        frames.settle();
        frames
    }
    fn settle(&mut self) {
        // The actual Area move rect can still use the previous size on the
        // second frame. Settle genuine empty layouts before minting any input;
        // production geometry and original-press checks remain exact.
        for _ in 0..8 {
            let previous = self.model.controls.clone();
            assert!(self.show(vec![], Late::None).is_none());
            if !previous.is_empty()
                && previous.len() == self.model.controls.len()
                && previous.iter().zip(&self.model.controls).all(|(old, current)| {
                    old.widget == current.widget
                        && old.intent == current.intent
                        && old.rect == current.rect
                        && old.registered == current.registered
                        && old.paint_clip == current.paint_clip
                        && old.transform == current.transform
                        && old.frame_origin == current.frame_origin
                        && old.frame == current.frame
                        && current.committed(&self.ctx)
                })
            {
                return;
            }
        }
        panic!("actual modal/control geometry did not settle within eight empty frames");
    }
    fn show(&mut self, events: Vec<Event>, late: Late) -> Option<Command> {
        if let Some(layer) = self.transform_layer.take() {
            // Restore only this fixture's previous late injection. The next
            // empty frame must remain silent even with valid geometry again.
            self.ctx.memory_mut(|memory| {
                memory.to_global.remove(&layer);
            });
        }
        self.ctx.data_mut(|data| {
            data.remove::<Vec<SourceDiagnostic>>(Id::unique("copy_fixture_source_diagnostics"));
        });
        self.frame += 1;
        let p = crate::theme::Theme::default().palette();
        let mut action = None;
        let mut raw = egui::RawInput {
            focused: true,
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
            time: Some(self.frame as f64 / 60.0),
            events,
            ..Default::default()
        };
        raw.viewports.entry(raw.viewport_id).or_default().native_pixels_per_point = Some(self.scale);
        let mut output = self.ctx.run_ui(raw, |ui| {
            action = action.take().or(self.model.show(ui.ctx(), &p, &self.snapshot, false));
            match late {
                Late::None => {}
                Late::Discard => ui.ctx().request_discard("copy fixture changes layout after the modal body"),
                Late::OtherModal => {
                    late_modal(ui.ctx());
                }
                Late::Close => self.model.close(),
                Late::FocusLoss => ui.ctx().input_mut(|input| input.focused = false),
                Late::Transform(_) => {}
                Late::MoveWidget
                | Late::ReclipWidget
                | Late::ResenseWidget
                | Late::CoverWidget
                | Late::CoverLayer
                | Late::VisualWidgetCover
                | Late::VisualLayerCover
                | Late::FocusWidget
                | Late::StructuralFrameMove
                | Late::StructuralFrameSense
                | Late::AmbiguousFrame
                | Late::SameSizeVisualCover => {
                    if let Some(completed) = &self.model.completed {
                        late_widget(ui, completed.control, late);
                    }
                }
            }
        });
        // This CPU fixture has no renderer; pinned TexturesDelta requires
        // explicit disposal when its uploads/frees are deliberately discarded.
        output.textures_delta.clear();
        if matches!(late, Late::OtherModal | Late::CoverLayer | Late::VisualLayerCover) {
            let control = self
                .model
                .completed
                .as_ref()
                .expect("original release before actual late coverage")
                .control;
            let layer = egui::LayerId::new(egui::Order::Foreground, late_layer_id(late));
            let above = self.ctx.memory(|memory| {
                let layers = memory.layer_ids().collect::<Vec<_>>();
                memory.areas().is_visible(&layer)
                    && matches!((layers.iter().position(|id| *id == control.layer), layers.iter().position(|id| *id == layer)), (Some(original), Some(cover)) if original < cover)
            });
            assert!(above, "late fixture must be genuinely visible above the original layer");
            let area = egui::AreaState::load(&self.ctx, layer.id).unwrap().rect();
            if matches!(late, Late::OtherModal) {
                assert!(!area.intersect(control.rect).is_positive(), "exercise the backdrop beyond its body");
            } else {
                assert!(area.contains_rect(control.rect), "actual late Area must cover the original control");
            }
            assert!(
                self.ctx
                    .viewport(|viewport| viewport.prev_pass.widgets.get_layer(layer).any(|widget| {
                        widget.visible && finite_positive(widget.interact_rect) && widget.interact_rect.contains_rect(control.rect)
                    })),
                "actual final visible widget/backdrop must cover the original control"
            );
        }
        if let Late::Transform(transform) = late {
            let control = self
                .model
                .completed
                .as_ref()
                .expect("original release before actual late transform")
                .control;
            // The public setter compares against identity and NaN-panics before
            // the final guard. Change the actual store AFTER the whole run.
            self.ctx.memory_mut(|memory| {
                memory.to_global.insert(control.layer, transform);
            });
            let actual = self.ctx.layer_transform_to_global(control.layer).unwrap();
            assert_eq!(actual.scaling.to_bits(), transform.scaling.to_bits());
            assert_eq!(actual.translation.x.to_bits(), transform.translation.x.to_bits());
            assert_eq!(actual.translation.y.to_bits(), transform.translation.y.to_bits());
            self.transform_layer = Some(control.layer);
        }
        let diagnostic = self.diagnostic(late, action);
        let command = action.and_then(|action| self.model.take_action(action, &self.ctx, &self.snapshot, false));
        self.last_diagnostic = bounded_diagnostic(format!(
            "completed_passes={}, discard={}, command={command:?}\n{diagnostic}",
            output.platform_output.num_completed_passes,
            self.ctx.will_discard(),
        ));
        self.model.finish_frame(&self.ctx);
        command
    }
    fn diagnostic(&self, late: Late, action: Option<Action>) -> String {
        let (focus, modal, layers) = self.ctx.memory(|memory| {
            (
                memory.focused(),
                memory.top_modal_layer(),
                memory.layer_ids().take(16).collect::<Vec<_>>(),
            )
        });
        let input = self.ctx.input(|input| {
            (
                input.focused,
                input.modifiers,
                input.events.iter().take(16).cloned().collect::<Vec<_>>(),
            )
        });
        let source = self
            .ctx
            .data(|data| data.get_temp::<Vec<SourceDiagnostic>>(Id::unique("copy_fixture_source_diagnostics")));
        let popup = self.ctx.data(|data| data.get_temp::<Id>(Id::unique("host_copy_popup")));
        let popup_widgets = self.ctx.viewport(|viewport| {
            layers
                .iter()
                .filter(|layer| Some(layer.id) == popup)
                .flat_map(|layer| viewport.prev_pass.widgets.get_layer(*layer).copied())
                .take(128)
                .collect::<Vec<_>>()
        });
        let areas = layers
            .iter()
            .map(|layer| (*layer, egui::AreaState::load(&self.ctx, layer.id).map(|area| area.rect())))
            .collect::<Vec<_>>();
        let describe = |control: &Control| {
            let current = self
                .ctx
                .viewport(|viewport| viewport.prev_pass.widgets.get(control.widget).copied());
            let final_frame = self
                .ctx
                .viewport(|viewport| FrameWitness::capture(&viewport.prev_pass.widgets, control));
            format!(
                "widget={:?}, layer={:?}, rect={:?}, registered={:?}, current={current:?}, origin={:?}, stored_frame={:?}, final_frame={final_frame:?}, registry={}, committed={}, hit={:?}",
                control.widget,
                control.layer,
                control.rect,
                control.registered,
                control.frame_origin,
                control.frame,
                control.registry(&self.ctx, true),
                control.committed(&self.ctx),
                self.ctx.layer_id_at(control.rect.center())
            )
        };
        let painting = self.model.painting.iter().take(MAX_CONTROLS).map(describe).collect::<Vec<_>>();
        let sources = source
            .unwrap_or_default()
            .iter()
            .map(|record| {
                let retained = self.model.painting.iter().find(|control| control.widget == record.control.widget);
                let final_frame = self
                    .ctx
                    .viewport(|viewport| FrameWitness::capture(&viewport.prev_pass.widgets, &record.control));
                let stored = retained.and_then(|control| control.frame);
                let covers = stored.map(|frame| {
                    self.ctx.viewport(|viewport| {
                        let widgets = &viewport.prev_pass.widgets;
                        widgets.order(record.control.widget).map(|(layer, index)| {
                            widgets
                                .get_layer(layer)
                                .skip(index + 1)
                                .filter(|widget| {
                                    !frame.contains(widget.id)
                                        && widget.visible
                                        && widget.interact_rect.intersect(record.control.rect).is_positive()
                                })
                                .take(8)
                                .map(|widget| {
                                    (
                                        widget.id.value(),
                                        widget.parent_id.value(),
                                        widget.rect,
                                        widget.interact_rect,
                                        widget.sense,
                                        widget.enabled,
                                        widget.visible,
                                    )
                                })
                                .collect::<Vec<_>>()
                        })
                    })
                });
                // Keep eligibility/commit stages ahead of bulky parent data.
                (
                    record.control.widget.value(),
                    (rect_bits(record.response_rect), rect_bits(record.control.registered.rect)),
                    (
                        rect_bits(record.response_interact_rect),
                        rect_bits(record.control.registered.interact_rect),
                    ),
                    (record.response_sense, record.control.registered.sense),
                    record.painting_count,
                    record.flags,
                    record.pass,
                    (
                        retained.is_some(),
                        stored.is_some(),
                        final_frame.is_some(),
                        stored.zip(final_frame).map(|(old, current)| old == current),
                    ),
                    retained.map(|control| (control.registry(&self.ctx, true), control.committed(&self.ctx))),
                    covers,
                    record
                        .stack
                        .iter()
                        .map(|node| (node.id.value(), node.parent.map(|parent| parent.value()), node.kind))
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>();
        let gesture = self
            .model
            .gesture
            .map(|gesture| (gesture.control.widget, gesture.frame, gesture.index));
        let completed = self
            .model
            .completed
            .as_ref()
            .map(|completed| (completed.control.widget, completed.focused, completed.frame, completed.pass));
        let completed_control = self.model.completed.as_ref().map(|completed| describe(&completed.control));
        format!(
            "late={late:?}, action={action:?}, frame={}, pass={}, input_frame={:?}, painted={:?}, focus={focus:?}, modal={modal:?}, gesture={gesture:?}, completed={completed:?}, native_focused={}, rejected={:?}, native_modifiers={:?}, transition={}, alt_graph={}, input={input:?}, areas={areas:?}\ncompleted_control={completed_control:?}\nSource records=(id, response/registered rect bits, response/registered interact bits, senses, paint_count, flags[live,enabled,valid,registry,allows], pass, retained/stored/final/equal, registry/committed, later_covers[id,parent,rect,interact,sense,enabled,visible], stack)={sources:#?}\npopup_widgets={popup_widgets:#?}\npainting={painting:#?}",
            self.ctx.cumulative_frame_nr(),
            self.ctx.cumulative_pass_nr(),
            self.model.input_frame,
            self.model.painted,
            self.model.native_focused,
            self.model.rejected_keys,
            self.model.native_modifiers,
            self.model.modifier_transition,
            self.model.alt_graph
        )
    }
    fn focus_with_tab(&mut self, control: Control) {
        for step in 0..=MAX_CONTROLS {
            if self.ctx.memory(|memory| memory.focused()) == Some(control.widget)
                && self
                    .model
                    .controls
                    .iter()
                    .any(|current| current.widget == control.widget && current.committed(&self.ctx))
            {
                self.settle();
                assert_eq!(self.ctx.memory(|memory| memory.focused()), Some(control.widget));
                assert!(
                    self.model
                        .controls
                        .iter()
                        .any(|current| current.widget == control.widget && current.committed(&self.ctx))
                );
                return;
            }
            if step == MAX_CONTROLS {
                break;
            }
            assert!(self.show(vec![key(Key::Tab, true)], Late::None).is_none());
            assert!(self.show(vec![key(Key::Tab, false)], Late::None).is_none());
        }
        panic!("actual enabled command did not receive bounded Tab focus: {}", self.last_diagnostic);
    }
    fn control(&self, predicate: impl Fn(Intent) -> bool) -> Control {
        self.model
            .controls
            .iter()
            .find(|control| predicate(control.intent))
            .copied()
            .unwrap_or_else(|| {
                let layers = self.ctx.memory(|memory| memory.layer_ids().collect::<Vec<_>>());
                let widgets = self.ctx.viewport(|viewport| {
                    layers
                        .iter()
                        .flat_map(|layer| viewport.prev_pass.widgets.get_layer(*layer).copied())
                        .take(256)
                        .collect::<Vec<_>>()
                });
                panic!(
                    "settled visible control: controls={}, frame={}, pass={}, painted={:?}, layers={layers:?}, widgets={widgets:#?}",
                    self.model.controls.len(),
                    self.ctx.cumulative_frame_nr(),
                    self.ctx.cumulative_pass_nr(),
                    self.model.painted,
                )
            })
    }
    fn click(&mut self, control: Control, late: Late) -> Option<Command> {
        self.warm_late(control, late);
        assert!(
            self.show(
                vec![Event::PointerMoved(control.rect.center()), pointer(control.rect.center(), true)],
                Late::None
            )
            .is_none()
        );
        self.show(vec![pointer(control.rect.center(), false)], late)
    }
    fn warm_late(&mut self, control: Control, late: Late) {
        if !matches!(late, Late::OtherModal | Late::CoverLayer | Late::VisualLayerCover) {
            return;
        }
        assert!(self.model.gesture.is_none());
        // A first Area pass is deliberately invisible sizing in pinned egui.
        // Warm the SAME real owner before minting input, then restore and
        // settle the original modal. No private focus or authority is minted.
        for _ in 0..2 {
            self.frame += 1;
            let mut raw = egui::RawInput {
                focused: true,
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
                time: Some(self.frame as f64 / 60.0),
                ..Default::default()
            };
            raw.viewports.entry(raw.viewport_id).or_default().native_pixels_per_point = Some(self.scale);
            let mut output = self.ctx.run_ui(raw, |ui| {
                if matches!(late, Late::OtherModal) {
                    late_modal(ui.ctx());
                } else {
                    late_widget(ui, control, late);
                }
            });
            output.textures_delta.clear();
        }
        assert!(self.show(vec![], Late::None).is_none());
        self.settle();
        let current = self.control(|intent| intent == control.intent);
        assert_eq!(current.registered, control.registered);
        assert_eq!(current.frame, control.frame);
        assert!(current.committed(&self.ctx));
    }
    fn inspect(&mut self) -> (Selection, u64) {
        let control = self.control(|intent| matches!(intent, Intent::Inspect(_)));
        let command = self.click(control, Late::None).unwrap_or_else(|| {
            let current = self.model.controls.iter().find(|current| current.widget == control.widget);
            let differences = control.frame.zip(current.and_then(|current| current.frame)).map(|(old, current)| {
                old.parents.into_iter().zip(current.parents).enumerate().filter(|(_, (old, current))| old != current).collect::<Vec<_>>()
            });
            panic!(
                "Inspect lost original input: original_rect={:?}, current_rect={:?}, original_frame={:?}, current_frame={:?}, parent_differences={differences:?}, controls={}",
                control.rect,
                current.map(|current| current.rect),
                control.frame_origin,
                current.and_then(|current| current.frame_origin),
                self.model.controls.len(),
            )
        });
        let Command::Inspect(selection) = command else { panic!("Inspect") };
        let (controller, inbox) = copy_fixture::controller(self.snapshot.clone());
        let request = controller.inspect_preferences(selection).unwrap();
        assert_eq!(inbox.take(), Some((request, Request::Inspect(selection))));
        assert!(inbox.take().is_none());
        self.model.queued(command, Ok(request));
        (selection, request)
    }
    fn prepare(&mut self) -> Capture {
        let (selection, request) = self.inspect();
        let capture = Capture {
            installation: InstallationId::from_bytes([11; 32]).unwrap(),
            controller: HostId::from_bytes([12; 16]).unwrap(),
            manager: Instance::fresh().unwrap(),
            operation: request,
            selection,
        };
        self.snapshot.completed_request = request;
        self.snapshot.copy_preferences = Some(State {
            capture,
            phase: Phase::Prepared(vec![crate::settings::profile_seed::Summary {
                family: Family::Interface,
                fields: 18,
                changed: 1,
                defaulted: 17,
            }]),
        });
        self.show(vec![], Late::None);
        self.show(vec![], Late::None);
        self.settle();
        capture
    }
}

fn late_layer_id(late: Late) -> Id {
    if matches!(late, Late::OtherModal) {
        Id::unique("copy_fixture_late_modal")
    } else {
        Id::unique("copy_guard_late_cover_area")
    }
}
fn late_modal(ctx: &egui::Context) {
    widgets::Modal::new(late_layer_id(Late::OtherModal)).show(ctx, |ui| {
        ui.label("synthetic modal");
    });
}
fn late_widget(ui: &mut egui::Ui, control: Control, change: Late) {
    if matches!(change, Late::AmbiguousFrame) {
        let frame = control.frame.unwrap();
        let original = frame
            .parents
            .iter()
            .flatten()
            .find(|parent| parent.id == frame.allocation.parent_id)
            .unwrap();
        ui.ctx().options_mut(|options| options.warn_on_id_clash = false);
        // Use a real Ui with the checked parent ID and preserve its exact
        // geometry. The second matching allocation makes structure ambiguous.
        let mut parent = egui::Ui::new(
            ui.ctx().clone(),
            original.id,
            egui::UiBuilder::new()
                .layer_id(original.layer_id)
                .max_rect(original.rect)
                .sense(original.sense),
        );
        parent.set_clip_rect(original.interact_rect);
        parent.set_min_size(original.rect.size());
        parent.interact(
            frame.allocation.rect,
            Id::unique("copy_guard_ambiguous_frame"),
            egui::Sense::hover(),
        );
        return;
    }
    if matches!(change, Late::CoverLayer | Late::VisualLayerCover) {
        let cover = control.rect.expand(2.0);
        egui::Area::new(late_layer_id(change))
            .order(egui::Order::Foreground)
            .interactable(matches!(change, Late::CoverLayer))
            .constrain(false)
            .fade_in(false)
            .default_size(cover.size())
            .fixed_pos(cover.min)
            .show(ui.ctx(), |ui| {
                ui.allocate_response(cover.size(), egui::Sense::click());
            });
        return;
    }
    // Same-id metadata turnover is deliberate here, using actual Ui::interact
    // registration and the final committed registry, not a mock response map.
    ui.ctx().options_mut(|options| options.warn_on_id_clash = false);
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("copy_guard_late_widget")
            .layer_id(control.layer)
            .max_rect(ui.ctx().content_rect()),
    );
    child.set_clip_rect(control.paint_clip);
    match change {
        Late::StructuralFrameMove => {
            let frame = control.frame.unwrap();
            child.interact(
                frame.allocation.rect.translate(egui::vec2(7.0, 0.0)),
                frame.allocation.id,
                frame.allocation.sense,
            );
        }
        Late::StructuralFrameSense => {
            let frame = control.frame.unwrap();
            child.interact(frame.allocation.rect, frame.allocation.id, egui::Sense::click());
        }
        Late::SameSizeVisualCover => {
            let frame = control.frame.unwrap();
            child.set_clip_rect(frame.allocation.interact_rect);
            child.interact(
                frame.allocation.rect,
                Id::unique("copy_guard_same_size_frame_cover"),
                egui::Sense::hover(),
            );
            child.painter().rect_filled(frame.allocation.rect, 0.0, egui::Color32::BLACK);
        }
        Late::MoveWidget => {
            child.interact(
                control.registered.rect.translate(egui::vec2(11.0, 0.0)),
                control.widget,
                control.registered.sense,
            );
        }
        Late::ReclipWidget => {
            child.set_clip_rect(Rect::from_min_max(
                control.rect.min,
                egui::pos2(control.rect.center().x, control.rect.max.y),
            ));
            child.interact(control.registered.rect, control.widget, control.registered.sense);
        }
        Late::ResenseWidget => {
            child.interact(
                control.registered.rect,
                control.widget,
                control.registered.sense | egui::Sense::drag(),
            );
        }
        Late::CoverWidget => {
            child.interact(
                control.registered.rect,
                Id::unique("copy_guard_late_same_layer_cover"),
                egui::Sense::click(),
            );
        }
        Late::VisualWidgetCover => {
            child.interact(
                control.registered.rect,
                Id::unique("copy_guard_late_visual_cover"),
                egui::Sense::hover(),
            );
        }
        Late::FocusWidget => {
            let mut text = String::new();
            child
                .put(
                    Rect::from_min_size(Pos2::ZERO, egui::vec2(80.0, 24.0)),
                    egui::TextEdit::singleline(&mut text).id(Id::unique("copy_guard_late_editor")),
                )
                .request_focus();
        }
        _ => {}
    }
}

#[test]
fn profile_copy_ui_starts_main_saved_all_four_checked_and_five_bounded_sources() {
    let mut frames = Frames::new();
    let dialog = frames.model.dialog.as_ref().unwrap();
    assert_eq!(dialog.bits, 15);
    assert_eq!(dialog.selection.source, Source::MainSaved);
    assert_eq!(dialog.selection.target, slot(0));
    assert_eq!(sources(&frames.snapshot, slot(0)).len(), 5);
    assert!(!sources(&frames.snapshot, slot(0)).contains(&Source::Slot(slot(0))));
    let required = sources(&frames.snapshot, slot(0)).len() + 1 + Family::ALL.len() + 3;
    assert!(required <= MAX_CONTROLS);
    let picker = frames.control(|intent| matches!(intent, Intent::Picker(_)));
    assert!(frames.click(picker, Late::None).is_none());
    frames.show(vec![], Late::None);
    let visible_sources = frames
        .model
        .controls
        .iter()
        .filter(|control| matches!(control.intent, Intent::Source(_, _)))
        .count();
    assert!((1..=MAX_SOURCES).contains(&visible_sources), "{}", frames.last_diagnostic);
    assert!(frames.model.controls.len() <= MAX_CONTROLS);
    frames.snapshot.slots[1].available = false;
    assert!(!sources(&frames.snapshot, slot(0)).contains(&Source::Slot(slot(1))));
    let before = frames.model.selection();
    frames.model.open(slot(1), &frames.snapshot);
    assert_eq!(frames.model.selection(), before);
}

#[test]
fn profile_copy_ui_actual_inspect_controller_inbox_and_prepared_apply_use_exact_capture() {
    let mut frames = Frames::new();
    let capture = frames.prepare();
    let control = frames.control(|intent| matches!(intent, Intent::Apply(_)));
    let command = frames.click(control, Late::None).unwrap();
    assert_eq!(command, Command::Apply(capture));
    let (controller, inbox) = copy_fixture::controller(frames.snapshot.clone());
    let request = controller.apply_preferences(capture).unwrap();
    assert_eq!(inbox.take(), Some((request, Request::Apply(capture))));
    frames.model.queued(command, Ok(request));
    frames.show(vec![], Late::None);
    assert!(matches!(
        frames.model.operation.as_ref().unwrap().state.as_ref().unwrap().phase,
        Phase::Applying
    ));
    assert!(
        !frames
            .model
            .controls
            .iter()
            .any(|control| matches!(control.intent, Intent::Apply(_))),
        "old Prepared cannot replay queued Apply"
    );
    assert!(inbox.take().is_none());
}

#[test]
fn profile_copy_ui_source_and_family_turnover_never_revive_apply_after_a_b_a() {
    for source_change in [false, true] {
        let mut frames = Frames::new();
        let capture = frames.prepare();
        let apply = frames.control(|intent| matches!(intent, Intent::Apply(_)));
        frames.show(
            vec![Event::PointerMoved(apply.rect.center()), pointer(apply.rect.center(), true)],
            Late::None,
        );
        let original = frames.model.selection().unwrap();
        if source_change {
            frames.model.change(Some(Source::Slot(slot(1))), None);
            frames.model.change(Some(original.source), None);
        } else {
            frames.model.change(None, Some(1));
            frames.model.change(None, Some(15));
        }
        assert_ne!(frames.model.selection(), Some(original));
        assert_eq!(frames.model.cancellation(), Some((capture.selection, capture.operation)));
        assert!(frames.show(vec![pointer(apply.rect.center(), false)], Late::None).is_none());
    }
}

#[test]
fn profile_copy_ui_final_whole_run_guard_rejects_late_discard_modal_removal_focus_and_other_modal() {
    for late in [Late::Discard, Late::Close, Late::FocusLoss, Late::OtherModal] {
        let mut frames = Frames::new();
        frames.prepare();
        let apply = frames.control(|intent| matches!(intent, Intent::Apply(_)));
        assert!(frames.click(apply, late).is_none(), "late={late:?}: {}", frames.last_diagnostic);
        assert!(frames.show(vec![], Late::None).is_none(), "rejected release has no deferred replay");
    }
}

#[test]
fn profile_copy_ui_controller_manager_turnover_and_clip_geometry_revoke_pointer_apply() {
    for manager_change in [false, true] {
        let mut frames = Frames::new();
        frames.prepare();
        let apply = frames.control(|intent| matches!(intent, Intent::Apply(_)));
        frames.show(
            vec![Event::PointerMoved(apply.rect.center()), pointer(apply.rect.center(), true)],
            Late::None,
        );
        if manager_change {
            let state = frames.snapshot.copy_preferences.as_mut().unwrap();
            state.capture.manager = Instance::fresh().unwrap();
            state.capture.controller = HostId::from_bytes([19; 16]).unwrap();
        } else {
            frames.size = egui::vec2(360.0, 453.0);
        }
        assert!(frames.show(vec![pointer(apply.rect.center(), false)], Late::None).is_none());
        assert!(frames.show(vec![], Late::None).is_none());
    }
}

#[test]
fn profile_copy_ui_modal_default_enter_cannot_turn_inspect_into_newly_enabled_apply() {
    let mut frames = Frames::new();
    let inspect = frames.control(|intent| matches!(intent, Intent::Inspect(_)));
    frames.ctx.memory_mut(|memory| memory.request_focus(inspect.widget));
    assert!(frames.show(vec![key(Key::Enter, true)], Late::None).is_none());
    let command = frames.show(vec![key(Key::Enter, false)], Late::None).unwrap();
    let Command::Inspect(selection) = command else { panic!("Inspect") };
    frames.model.queued(command, Ok(1));
    let capture = Capture {
        installation: InstallationId::from_bytes([21; 32]).unwrap(),
        controller: HostId::from_bytes([22; 16]).unwrap(),
        manager: Instance::fresh().unwrap(),
        operation: 1,
        selection,
    };
    frames.snapshot.copy_preferences = Some(State {
        capture,
        phase: Phase::Prepared(vec![]),
    });
    frames.snapshot.completed_request = 1;
    for _ in 0..3 {
        assert!(
            frames.show(vec![], Late::None).is_none(),
            "Modal synthetic primary click has no original Apply press"
        );
    }
}

#[test]
fn profile_copy_ui_space_and_accessibility_require_settled_original_enabled_control() {
    let mut frames = Frames::new();
    let capture = frames.prepare();
    let apply = frames.control(|intent| matches!(intent, Intent::Apply(_)));
    frames.ctx.memory_mut(|memory| memory.request_focus(apply.widget));
    assert!(frames.show(vec![key(Key::Space, true)], Late::None).is_none());
    assert_eq!(frames.show(vec![key(Key::Space, false)], Late::None), Some(Command::Apply(capture)));
    let mut accessible = Frames::new();
    let exact = accessible.prepare();
    let enabled = accessible.control(|intent| matches!(intent, Intent::Apply(_)));
    accessible.ctx.memory_mut(|memory| memory.request_focus(enabled.widget));
    accessible.show(vec![], Late::None);
    let click = egui::accesskit::ActionRequest {
        action: egui::accesskit::Action::Click,
        target_tree: egui::accesskit::TreeId::ROOT,
        target_node: enabled.widget.accesskit_id(),
        data: None,
    };
    assert_eq!(
        accessible.show(vec![Event::AccessKitActionRequest(click)], Late::None),
        Some(Command::Apply(exact))
    );
    // A fresh dialog has no settled Apply widget from which AccessKit can gain
    // authority, even when the old widget id is requested explicitly.
    frames.model.close();
    frames.model.open(slot(2), &frames.snapshot);
    let action = egui::accesskit::ActionRequest {
        action: egui::accesskit::Action::Click,
        target_tree: egui::accesskit::TreeId::ROOT,
        target_node: apply.widget.accesskit_id(),
        data: None,
    };
    assert!(frames.show(vec![Event::AccessKitActionRequest(action)], Late::None).is_none());
}

#[test]
fn profile_copy_ui_close_retains_cancel_on_full_real_channel_and_applied_original_receipt() {
    let mut frames = Frames::new();
    let capture = frames.prepare();
    let close = frames.control(|intent| matches!(intent, Intent::Close(_)));
    assert!(frames.click(close, Late::None).is_none());
    assert!(!frames.model.is_open());
    let (controller, inbox) = copy_fixture::controller(frames.snapshot.clone());
    for _ in 0..16 {
        controller.cancel_preferences(capture.selection, capture.operation).unwrap();
    }
    let exact = frames.model.cancellation().unwrap();
    assert_eq!(controller.cancel_preferences(exact.0, exact.1), Err(HostError::Busy));
    assert_eq!(
        frames.model.cancellation(),
        Some(exact),
        "queue-full cannot refund cancellation or original correlation"
    );
    for _ in 0..16 {
        assert!(
            matches!(inbox.take(),Some((_,Request::Cancel(selection,request))) if selection==capture.selection && request==capture.operation)
        );
    }
    let ticket = controller.cancel_preferences(exact.0, exact.1).unwrap();
    frames.model.cancellation_queued(exact.0, exact.1);
    assert_eq!(inbox.take(), Some((ticket, Request::Cancel(capture.selection, capture.operation))));
    assert!(frames.model.cancellation().is_none());
    frames.model.open(slot(2), &frames.snapshot);
    frames.snapshot.copy_preferences = Some(State {
        capture,
        phase: Phase::Terminal(Outcome::Applied),
    });
    frames.model.sync(&frames.snapshot);
    let operation = frames.model.operation.as_ref().unwrap();
    assert_eq!(operation.selection.target, slot(0));
    assert!(matches!(operation.state.as_ref().unwrap().phase, Phase::Terminal(Outcome::Applied)));
    assert_eq!(frames.model.selection().unwrap().target, slot(2));
}

#[test]
fn profile_copy_ui_missing_ack_is_uncertain_and_terminal_snapshot_repaints_without_new_ticket() {
    let mut frames = Frames::new();
    let (selection, request) = frames.inspect();
    frames.snapshot.completed_request = request;
    frames.model.sync(&frames.snapshot);
    assert!(frames.model.operation.as_ref().unwrap().uncertain);
    assert!(frames.model.busy(), "missing acknowledgement is not physical completion proof");
    frames.model.close();
    assert!(
        frames.model.cancellation().is_some(),
        "an uncertain local state cannot erase exact pending cancellation"
    );
    let original = Capture {
        installation: InstallationId::from_bytes([31; 32]).unwrap(),
        controller: HostId::from_bytes([32; 16]).unwrap(),
        manager: Instance::fresh().unwrap(),
        operation: request,
        selection,
    };
    frames.snapshot.copy_preferences = Some(State {
        capture: original,
        phase: Phase::Terminal(Outcome::Applied),
    });
    frames.model.sync(&frames.snapshot);
    assert!(!frames.model.operation.as_ref().unwrap().uncertain);
    frames.snapshot.copy_preferences = Some(State {
        capture: original,
        phase: Phase::Prepared(vec![]),
    });
    frames.model.sync(&frames.snapshot);
    assert!(
        matches!(
            frames.model.operation.as_ref().unwrap().state.as_ref().unwrap().phase,
            Phase::Terminal(Outcome::Applied)
        ),
        "late Prepared cannot erase original Applied"
    );
    let mut a = snapshot();
    let mut b = a.clone();
    let mut prepared = Frames::new();
    let capture = prepared.prepare();
    a.completed_request = capture.operation;
    b.completed_request = capture.operation;
    a.copy_preferences = Some(State {
        capture,
        phase: Phase::Prepared(vec![]),
    });
    b.copy_preferences = Some(State {
        capture,
        phase: Phase::Terminal(Outcome::Applied),
    });
    assert!(!crate::session_host::ui::same_snapshot(&a, &b));
    assert!(crate::session_host::ui::same_snapshot(&a, &a.clone()));
}

#[test]
fn profile_copy_ui_native_enter_space_snapshot_and_logical_reset_preserve_real_release_fence() {
    use winit::event::{ElementState, WindowEvent};
    use winit::keyboard::{Key as NativeKey, NamedKey};
    for native_key in [
        NativeKey::Named(NamedKey::Enter),
        NativeKey::Named(NamedKey::Space),
        NativeKey::Character(" ".into()),
    ] {
        for before_focus in [false, true] {
            let mut model = CopyUi::default();
            let key = if native_key == NativeKey::Named(NamedKey::Enter) {
                Key::Enter
            } else {
                Key::Space
            };
            model.native_key(&native_key, ElementState::Pressed, false, false);
            model.native_key(&native_key, ElementState::Released, false, true);
            model.native_event(&WindowEvent::Focused(false));
            if before_focus {
                model.native_key(&native_key, ElementState::Pressed, false, true);
            }
            model.native_event(&WindowEvent::Focused(true));
            if !before_focus {
                model.native_key(&native_key, ElementState::Pressed, false, true);
            }
            model.native_key(&native_key, ElementState::Pressed, false, false);
            assert!(!model.key_allowed(key));
            model.close();
            model.cancel_gestures();
            assert!(!model.key_allowed(key));
            let ctx = egui::Context::default();
            model.finish_frame(&ctx);
            model.native_key(&native_key, ElementState::Pressed, false, false);
            assert!(!model.key_allowed(key), "logical frame/reset cannot refund a physical held key");
            model.native_key(&native_key, ElementState::Released, false, false);
            model.finish_frame(&ctx);
            model.native_key(&native_key, ElementState::Pressed, false, false);
            assert!(model.key_allowed(key));
        }
    }
}

#[test]
fn profile_copy_ui_empty_groups_and_ime_preedit_have_no_inspect_authority() {
    let mut frames = Frames::new();
    frames.model.change(None, Some(0));
    frames.show(vec![], Late::None);
    assert!(
        !frames
            .model
            .controls
            .iter()
            .any(|control| matches!(control.intent, Intent::Inspect(_)))
    );
    frames.model.change(None, Some(15));
    frames.show(vec![], Late::None);
    frames.show(vec![], Late::None);
    let inspect = frames.control(|intent| matches!(intent, Intent::Inspect(_)));
    frames.ctx.memory_mut(|memory| memory.request_focus(inspect.widget));
    assert!(
        frames
            .show(
                vec![Event::Ime(egui::ImeEvent::Preedit {
                    text: "合成".into(),
                    active_range_chars: None
                })],
                Late::None
            )
            .is_none()
    );
    assert!(
        frames
            .show(vec![key(Key::Enter, true), key(Key::Enter, false)], Late::None)
            .is_none()
    );
    assert!(frames.model.operation.is_none());
}

#[test]
#[ignore = "Root executes separately for en/fr/ja via existing AURORA_COMPACT_HOST_VISUAL_LANGUAGE; actual font/layout acceptance"]
fn profile_copy_ui_actual_compact_modal_and_source_popup_layout_explicit_locale() {
    let locale = std::env::var("AURORA_COMPACT_HOST_VISUAL_LANGUAGE").unwrap();
    assert!(["en", "fr", "ja"].contains(&locale.as_str()));
    struct Restore(String);
    impl Drop for Restore {
        fn drop(&mut self) {
            aurora_i18n::select(&self.0, None);
        }
    }
    let _restore = Restore(aurora_i18n::language().to_owned());
    aurora_i18n::select(&locale, None);
    if locale == "ja" {
        assert!(
            crate::ui::fonts::fallback::wait()
                .iter()
                .any(|face| face.has('あ') && face.has('界'))
        );
    }
    for (physical, scale) in [
        (egui::vec2(1240.0, 800.0), 1.0),
        (egui::vec2(760.0, 600.0), 1.0),
        (egui::vec2(760.0, 600.0), 1.5),
        (egui::vec2(540.0, 680.0), 1.5),
    ] {
        let mut frames = Frames::new();
        frames.size = physical / scale;
        frames.scale = scale;
        crate::ui::fonts::apply(&frames.ctx, crate::ui::fonts::DEFAULT);
        if locale == "ja" {
            for item in &mut frames.snapshot.slots {
                item.label = SlotLabel::new("日本語".repeat(7).as_str()).unwrap();
            }
        }
        for _ in 0..3 {
            frames.show(vec![], Late::None);
        }
        let canvas = Rect::from_min_size(Pos2::ZERO, frames.size);
        let modal = frames
            .ctx
            .data(|data| data.get_temp::<Rect>(Id::unique("host_copy_modal")))
            .unwrap();
        assert!(canvas.expand(1.0).contains_rect(modal), "{locale} {physical:?}/{scale}: {modal:?}");
        for index in 0_usize..3 {
            let footer = frames
                .ctx
                .data(|data| data.get_temp::<Rect>(Id::unique(("host_copy_footer", index))))
                .unwrap();
            assert!(modal.contains_rect(footer) && canvas.contains_rect(footer));
        }
        let picker = frames.control(|intent| matches!(intent, Intent::Picker(_)));
        frames.click(picker, Late::None);
        frames.show(vec![], Late::None);
        assert_eq!(sources(&frames.snapshot, slot(0)).len(), 5);
        let visible_sources = frames
            .model
            .controls
            .iter()
            .filter(|control| matches!(control.intent, Intent::Source(_, _)))
            .count();
        assert!((1..=MAX_SOURCES).contains(&visible_sources));
        let popup_id = frames.ctx.data(|data| data.get_temp::<Id>(Id::unique("host_copy_popup"))).unwrap();
        let popup = frames.ctx.memory(|memory| memory.area_rect(popup_id)).unwrap();
        assert!(canvas.expand(1.0).contains_rect(popup), "source popup {popup:?} outside {canvas:?}");
        for control in &frames.model.controls {
            assert!(canvas.contains_rect(control.rect));
        }
        assert!(frames.model.controls.len() <= MAX_CONTROLS);
    }
}

#[derive(Clone, Copy, Debug)]
enum Modality {
    Pointer,
    Enter,
    Space,
    Accessibility,
}
fn release_with(frames: &mut Frames, control: Control, modality: Modality, late: Late) -> Option<Command> {
    if !matches!(modality, Modality::Pointer) {
        frames.warm_late(control, late);
    }
    match modality {
        Modality::Pointer => frames.click(control, late),
        Modality::Enter | Modality::Space => {
            frames.ctx.memory_mut(|memory| memory.request_focus(control.widget));
            frames.show(vec![], Late::None);
            let key = if matches!(modality, Modality::Enter) {
                Key::Enter
            } else {
                Key::Space
            };
            assert!(frames.show(vec![key_event(key, true, Modifiers::NONE)], Late::None).is_none());
            frames.show(vec![key_event(key, false, Modifiers::NONE)], late)
        }
        Modality::Accessibility => {
            frames.ctx.memory_mut(|memory| memory.request_focus(control.widget));
            frames.show(vec![], Late::None);
            frames.show(
                vec![Event::AccessKitActionRequest(egui::accesskit::ActionRequest {
                    action: egui::accesskit::Action::Click,
                    target_tree: egui::accesskit::TreeId::ROOT,
                    target_node: control.widget.accesskit_id(),
                    data: None,
                })],
                late,
            )
        }
    }
}
fn key_event(key: Key, pressed: bool, modifiers: Modifiers) -> Event {
    Event::Key {
        key,
        physical_key: Some(key),
        pressed,
        repeat: false,
        modifiers,
    }
}

#[test]
fn profile_copy_ui_actual_modal_and_popup_retain_unique_structural_frame_and_real_parent_chain() {
    let mut frames = Frames::new();
    assert!(!frames.model.controls.is_empty());
    for control in &frames.model.controls {
        let origin = control.frame_origin.unwrap();
        let frame = control.frame.unwrap();
        assert!(control.committed(&frames.ctx));
        assert_eq!(frame.allocation.parent_id, origin.parent);
        assert_ne!(
            frame.allocation.id, origin.content,
            "the allocation is a sibling, not the Frame content Ui"
        );
        assert!(frame.parents.iter().flatten().any(|parent| parent.id == origin.content));
        assert!(frame.parents.iter().flatten().any(|parent| parent.id == origin.root));
        assert_eq!(frame.allocation.sense, egui::Sense::hover());
        assert_eq!(origin.root, control.layer.id);
        assert_eq!(
            frames
                .ctx
                .viewport(|viewport| FrameWitness::capture(&viewport.prev_pass.widgets, control)),
            Some(frame)
        );
    }
    let picker = frames.control(|intent| matches!(intent, Intent::Picker(_)));
    assert!(frames.click(picker, Late::None).is_none());
    frames.show(vec![], Late::None);
    let sources = frames
        .model
        .controls
        .iter()
        .filter(|control| matches!(control.intent, Intent::Source(_, _)))
        .collect::<Vec<_>>();
    assert!(!sources.is_empty(), "{}", frames.last_diagnostic);
    assert!(sources.len() <= MAX_SOURCES);
    for control in sources {
        assert!(control.committed(&frames.ctx));
        assert_ne!(control.layer, picker.layer, "the source popup has its own actual Frame and Area");
        assert_eq!(control.frame_origin.unwrap().root, control.layer.id);
        assert_ne!(control.frame.unwrap().allocation.id, picker.frame.unwrap().allocation.id);
    }
}

#[test]
fn profile_copy_ui_actual_scroll_body_witnesses_both_frames_and_keeps_every_other_cover_ineligible() {
    let frames = Frames::new();
    for control in frames
        .model
        .controls
        .iter()
        .filter(|control| matches!(control.intent, Intent::Family(_, _) | Intent::Picker(_)))
    {
        let origin = control.frame_origin.unwrap();
        let witness = control.frame.unwrap();
        assert_eq!(
            origin.frames().count(),
            2,
            "ScrollArea Frame::NONE and outer Modal Frame are both real enclosing Frames"
        );
        assert_eq!(1 + witness.outer.iter().flatten().count(), 2);
        assert!(control.committed(&frames.ctx));
        for frame in origin.frames() {
            assert!(
                witness
                    .parents
                    .iter()
                    .flatten()
                    .any(|parent| parent.id == frame.content && parent.parent_id == frame.parent)
            );
        }
    }
    let inspect = frames.control(|intent| matches!(intent, Intent::Inspect(_)));
    assert_eq!(
        inspect.frame_origin.unwrap().frames().count(),
        1,
        "the footer is outside the ScrollArea"
    );
    for late in [
        Late::StructuralFrameMove,
        Late::StructuralFrameSense,
        Late::AmbiguousFrame,
        Late::SameSizeVisualCover,
    ] {
        let mut frames = Frames::new();
        let original = frames.control(|intent| matches!(intent, Intent::Family(_, _)));
        let selection = frames.model.selection().unwrap();
        assert!(frames.click(original, late).is_none());
        assert_eq!(
            frames.model.selection(),
            Some(selection),
            "a forged/moved frame or visual sibling cannot toggle the selected family"
        );
        assert_eq!(frames.model.dialog.as_ref().unwrap().bits, 15);
        assert!(!original.committed(&frames.ctx));
        assert!(frames.show(vec![], Late::None).is_none());
        assert_eq!(frames.model.selection(), Some(selection), "no deferred toggle on a later layout");
    }
}

#[test]
fn profile_copy_ui_structural_frame_turnover_ambiguity_and_same_size_visual_cover_revoke_every_modality() {
    for modality in [Modality::Pointer, Modality::Enter, Modality::Space, Modality::Accessibility] {
        for late in [
            Late::StructuralFrameMove,
            Late::StructuralFrameSense,
            Late::AmbiguousFrame,
            Late::SameSizeVisualCover,
        ] {
            let mut frames = Frames::new();
            frames.prepare();
            let original = frames.control(|intent| matches!(intent, Intent::Apply(_)));
            assert!(original.committed(&frames.ctx));
            assert!(release_with(&mut frames, original, modality, late).is_none());
            assert_eq!(
                frames
                    .ctx
                    .viewport(|viewport| viewport.prev_pass.widgets.get(original.widget).copied()),
                Some(original.registered),
                "the original button is unchanged; only its structural witness or a real visual cover invalidates it"
            );
            let witness = frames
                .ctx
                .viewport(|viewport| FrameWitness::capture(&viewport.prev_pass.widgets, &original));
            if matches!(late, Late::SameSizeVisualCover) {
                assert_eq!(
                    witness, original.frame,
                    "a different real hover cover cannot borrow the verified Frame exemption"
                );
            } else {
                assert_ne!(
                    witness, original.frame,
                    "a missing, changed or ambiguous structural witness must fail closed"
                );
            }
            assert!(!original.committed(&frames.ctx));
            assert!(
                frames.show(vec![], Late::None).is_none(),
                "a rejected proposal has no retry or reminted authority"
            );
            assert!(!frames.model.operation.as_ref().unwrap().apply_submitted);
        }
    }
    // A genuine Frame configuration change before release must also revoke
    // the original AccessKit gesture, even when the button geometry is intact.
    let mut frames = Frames::new();
    let capture = frames.prepare();
    let original = frames.control(|intent| matches!(intent, Intent::Apply(_)));
    frames.ctx.memory_mut(|memory| memory.request_focus(original.widget));
    frames.show(vec![], Late::None);
    let fill = frames.ctx.global_style().visuals.window_fill;
    frames.ctx.global_style_mut(|style| {
        style.visuals.window_fill = if fill == egui::Color32::BLACK {
            egui::Color32::WHITE
        } else {
            egui::Color32::BLACK
        };
    });
    let click = || {
        Event::AccessKitActionRequest(egui::accesskit::ActionRequest {
            action: egui::accesskit::Action::Click,
            target_tree: egui::accesskit::TreeId::ROOT,
            target_node: original.widget.accesskit_id(),
            data: None,
        })
    };
    assert!(frames.show(vec![click()], Late::None).is_none());
    assert!(
        frames.model.gesture.is_none(),
        "a witness mismatch consumes the old gesture for accessibility too"
    );
    assert_eq!(
        frames
            .ctx
            .viewport(|viewport| viewport.prev_pass.widgets.get(original.widget).copied()),
        Some(original.registered)
    );
    frames.ctx.global_style_mut(|style| style.visuals.window_fill = fill);
    assert!(
        frames.show(vec![], Late::None).is_none(),
        "restoring the Frame cannot replay the old gesture"
    );
    assert_eq!(
        frames.show(vec![click()], Late::None),
        Some(Command::Apply(capture)),
        "a fresh accessible intent can proceed"
    );
}

#[test]
fn profile_copy_ui_final_committed_turnover_and_coverage_reject_every_modality_without_replay() {
    for modality in [Modality::Pointer, Modality::Enter, Modality::Space, Modality::Accessibility] {
        for late in [
            Late::MoveWidget,
            Late::ReclipWidget,
            Late::ResenseWidget,
            Late::CoverWidget,
            Late::CoverLayer,
            Late::VisualWidgetCover,
            Late::VisualLayerCover,
            Late::FocusWidget,
        ] {
            let mut frames = Frames::new();
            frames.prepare();
            let original = frames.control(|intent| matches!(intent, Intent::Apply(_)));
            assert!(
                release_with(&mut frames, original, modality, late).is_none(),
                "modality={modality:?}, late={late:?}: {}",
                frames.last_diagnostic
            );
            let committed = frames
                .ctx
                .viewport(|viewport| viewport.prev_pass.widgets.get(original.widget).copied())
                .unwrap();
            match late {
                Late::MoveWidget => assert_ne!(committed.rect, original.registered.rect),
                Late::ReclipWidget => {
                    assert_ne!(committed.interact_rect, original.registered.interact_rect);
                    assert!(committed.interact_rect.is_positive());
                }
                Late::ResenseWidget => assert_ne!(committed.sense, original.registered.sense),
                Late::CoverWidget | Late::CoverLayer | Late::VisualWidgetCover | Late::VisualLayerCover => assert_eq!(
                    committed, original.registered,
                    "coverage alone must revoke a still-existing enabled widget"
                ),
                Late::FocusWidget => assert_eq!(
                    frames.ctx.memory(|memory| memory.focused()),
                    Some(Id::unique("copy_guard_late_editor"))
                ),
                _ => unreachable!(),
            }
            assert!(
                frames.show(vec![], Late::None).is_none(),
                "a rejected proposal cannot retry on an empty frame"
            );
            assert!(!frames.model.operation.as_ref().unwrap().apply_submitted);
        }
    }
}

#[test]
fn profile_copy_ui_visible_other_modal_backdrop_beyond_body_revokes_every_original_modality() {
    for modality in [Modality::Pointer, Modality::Enter, Modality::Space, Modality::Accessibility] {
        let mut frames = Frames::new();
        frames.prepare();
        let original = frames.control(|intent| matches!(intent, Intent::Apply(_)));
        // The helper checks actual final visibility/order and full backdrop
        // coverage, while also requiring that the modal BODY does not overlap.
        assert!(
            release_with(&mut frames, original, modality, Late::OtherModal).is_none(),
            "{modality:?}: {}",
            frames.last_diagnostic
        );
        assert_eq!(
            frames
                .ctx
                .viewport(|viewport| viewport.prev_pass.widgets.get(original.widget).copied()),
            Some(original.registered)
        );
        assert!(!frames.model.operation.as_ref().unwrap().apply_submitted);
        assert!(
            frames.show(vec![], Late::None).is_none(),
            "a hidden backdrop cannot replay the consumed proposal"
        );
    }
}

#[test]
fn profile_copy_ui_actual_late_transform_rejects_nonfinite_y_inverse_and_nonidentity_for_every_modality() {
    use egui::emath::TSTransform;
    let bad = [
        TSTransform::from_translation(egui::vec2(f32::INFINITY, 0.0)),
        TSTransform::from_translation(egui::vec2(0.0, f32::INFINITY)),
        TSTransform::from_translation(egui::vec2(0.0, f32::NAN)),
        TSTransform::from_scaling(0.0),
        TSTransform::from_scaling(f32::NAN),
        TSTransform::from_scaling(f32::from_bits(1)),
        TSTransform::from_scaling(f32::MAX),
        TSTransform::from_translation(egui::vec2(9.0, 3.0)),
        TSTransform::from_scaling(1.5),
    ];
    for modality in [Modality::Pointer, Modality::Enter, Modality::Space, Modality::Accessibility] {
        for transform in bad {
            let mut frames = Frames::new();
            frames.prepare();
            let original = frames.control(|intent| matches!(intent, Intent::Apply(_)));
            assert!(release_with(&mut frames, original, modality, Late::Transform(transform)).is_none());
            assert_eq!(
                frames
                    .ctx
                    .viewport(|viewport| viewport.prev_pass.widgets.get(original.widget).copied()),
                Some(original.registered),
                "reject transform change while original widget remains committed"
            );
            assert!(frames.show(vec![], Late::None).is_none());
            assert!(!frames.model.operation.as_ref().unwrap().apply_submitted);
        }
    }
    let rect = Rect::from_min_size(egui::pos2(1.0, 1.0), egui::vec2(90.0, 24.0));
    assert!(transformed_rect(rect, None).is_some());
    for invalid in [
        Rect::ZERO,
        Rect::EVERYTHING,
        Rect::from_min_max(egui::pos2(-f32::MAX, 0.0), egui::pos2(f32::MAX, 24.0)),
    ] {
        assert!(transformed_rect(invalid, None).is_none());
    }
}

#[test]
fn profile_copy_ui_final_registry_disabled_hidden_removed_and_layer_turnover_have_no_stale_fallback() {
    for fault in 0..5 {
        let mut frames = Frames::new();
        frames.prepare();
        let original = frames.control(|intent| matches!(intent, Intent::Apply(_)));
        assert!(original.committed(&frames.ctx));
        // A real subsequent committed layout replaces the registry. Keep the
        // original control unchanged; no later-layout proof is reminted.
        let _output = frames.ctx.run_ui(
            egui::RawInput {
                focused: true,
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, frames.size)),
                ..Default::default()
            },
            |ui| match fault {
                0 => {
                    frames
                        .model
                        .show(ui.ctx(), &crate::theme::Theme::default().palette(), &frames.snapshot, true);
                }
                1 => {
                    widgets::Modal::new(original.layer.id).show(ui.ctx(), |ui| {
                        ui.set_invisible();
                        ui.interact(original.registered.rect, original.widget, original.registered.sense);
                    });
                }
                2 => {}
                3 => {
                    egui::Area::new(Id::unique("copy_guard_replacement_layer"))
                        .order(egui::Order::Foreground)
                        .show(ui.ctx(), |ui| {
                            ui.interact(original.registered.rect, original.widget, original.registered.sense);
                        });
                }
                4 => {
                    widgets::Modal::new(original.layer.id).show(ui.ctx(), |ui| {
                        ui.interact(original.registered.rect, original.widget, egui::Sense::hover());
                    });
                }
                _ => unreachable!(),
            },
        );
        let committed = frames
            .ctx
            .viewport(|viewport| viewport.prev_pass.widgets.get(original.widget).copied());
        match fault {
            0 => assert!(!committed.unwrap().enabled),
            1 => assert!(!committed.unwrap().visible),
            2 => assert!(committed.is_none()),
            3 => assert_ne!(committed.unwrap().layer_id, original.layer),
            4 => assert!(!committed.unwrap().sense.senses_click()),
            _ => unreachable!(),
        }
        assert!(!original.committed(&frames.ctx));
        assert!(!frames.model.operation.as_ref().unwrap().apply_submitted);
    }
}

#[test]
fn profile_copy_ui_final_registry_guard_also_precedes_inspect_and_cancel_metadata() {
    for cancel in [false, true] {
        let mut frames = Frames::new();
        if cancel {
            frames.prepare();
        }
        let control = frames.control(|intent| {
            if cancel {
                matches!(intent, Intent::Close(_))
            } else {
                matches!(intent, Intent::Inspect(_))
            }
        });
        assert!(frames.click(control, Late::MoveWidget).is_none());
        assert!(frames.model.is_open());
        assert!(frames.model.cancellation().is_none());
        if !cancel {
            assert!(frames.model.operation.is_none());
        }
        assert!(frames.show(vec![], Late::None).is_none());
    }
    // The unchanged contract is one original release, never deferred replay.
    for modality in [Modality::Pointer, Modality::Enter, Modality::Space, Modality::Accessibility] {
        let mut frames = Frames::new();
        let exact = frames.prepare();
        let apply = frames.control(|intent| matches!(intent, Intent::Apply(_)));
        assert_eq!(release_with(&mut frames, apply, modality, Late::None), Some(Command::Apply(exact)));
        assert!(frames.show(vec![], Late::None).is_none());
    }
}

#[test]
fn profile_copy_ui_raw_modifier_up_and_same_batch_alt_aba_revoke_plain_key_gesture() {
    for key in [Key::Enter, Key::Space] {
        for modifiers in [
            Modifiers::ALT,
            Modifiers::CTRL,
            Modifiers::SHIFT,
            Modifiers::MAC_CMD,
            Modifiers::COMMAND,
        ] {
            let mut frames = Frames::new();
            frames.prepare();
            let apply = frames.control(|intent| matches!(intent, Intent::Apply(_)));
            frames.ctx.memory_mut(|memory| memory.request_focus(apply.widget));
            frames.show(vec![], Late::None);
            assert!(frames.show(vec![key_event(key, true, Modifiers::NONE)], Late::None).is_none());
            assert!(frames.show(vec![key_event(key, false, modifiers)], Late::None).is_none());
            assert!(frames.show(vec![], Late::None).is_none());
        }
        let mut frames = Frames::new();
        frames.prepare();
        let apply = frames.control(|intent| matches!(intent, Intent::Apply(_)));
        frames.ctx.memory_mut(|memory| memory.request_focus(apply.widget));
        frames.show(vec![], Late::None);
        frames.show(vec![key_event(key, true, Modifiers::NONE)], Late::None);
        assert!(
            frames
                .show(
                    vec![
                        Event::ModifiersChanged(Modifiers::ALT),
                        Event::ModifiersChanged(Modifiers::NONE),
                        key_event(key, false, Modifiers::NONE)
                    ],
                    Late::None
                )
                .is_none()
        );
        assert!(frames.show(vec![], Late::None).is_none());
    }
}

#[test]
fn profile_copy_ui_native_modifier_events_between_batches_preserve_physical_fence_and_require_fresh_plain_press() {
    use winit::event::{ElementState, WindowEvent};
    use winit::keyboard::{Key as NativeKey, ModifiersState, NamedKey};
    for native_key in [
        NativeKey::Named(NamedKey::Enter),
        NativeKey::Named(NamedKey::Space),
        NativeKey::Character(" ".into()),
    ] {
        for alt_graph in [false, true] {
            let mut frames = Frames::new();
            let exact = frames.prepare();
            let key = if native_key == NativeKey::Named(NamedKey::Enter) {
                Key::Enter
            } else {
                Key::Space
            };
            let apply = frames.control(|intent| matches!(intent, Intent::Apply(_)));
            frames.ctx.memory_mut(|memory| memory.request_focus(apply.widget));
            frames.show(vec![], Late::None);
            frames.model.native_key(&native_key, ElementState::Pressed, false, false);
            frames.show(vec![key_event(key, true, Modifiers::NONE)], Late::None);
            if alt_graph {
                frames
                    .model
                    .native_key(&NativeKey::Named(NamedKey::AltGraph), ElementState::Pressed, false, false);
            }
            let state = if alt_graph {
                ModifiersState::CONTROL | ModifiersState::ALT
            } else {
                ModifiersState::ALT
            };
            frames.model.native_event(&WindowEvent::ModifiersChanged(state.into()));
            frames
                .model
                .native_event(&WindowEvent::ModifiersChanged(ModifiersState::empty().into()));
            if alt_graph {
                frames
                    .model
                    .native_key(&NativeKey::Named(NamedKey::AltGraph), ElementState::Released, false, true);
                assert!(frames.model.alt_graph, "synthetic modifier release is not a physical release");
                frames
                    .model
                    .native_key(&NativeKey::Named(NamedKey::AltGraph), ElementState::Released, false, false);
            }
            assert!(frames.model.gesture.is_none());
            frames.show(vec![], Late::None);
            frames.show(vec![], Late::None);
            // An apparent fresh=false-repeat down without a real up remains
            // physically held despite modifier ABA or logical UI cancellation.
            frames.model.native_key(&native_key, ElementState::Pressed, false, false);
            assert!(!frames.model.key_allowed(key));
            assert!(frames.show(vec![key_event(key, true, Modifiers::NONE)], Late::None).is_none());
            frames.model.native_key(&native_key, ElementState::Released, false, false);
            assert!(frames.show(vec![key_event(key, false, Modifiers::NONE)], Late::None).is_none());
            frames.show(vec![], Late::None);
            // The refused transition temporarily disables Apply, so egui
            // legitimately drops its focus. After the real release, obtain
            // fresh real focus before the positive fresh native gesture.
            frames.focus_with_tab(apply);
            frames.model.native_key(&native_key, ElementState::Pressed, false, false);
            assert!(frames.show(vec![key_event(key, true, Modifiers::NONE)], Late::None).is_none());
            frames.model.native_key(&native_key, ElementState::Released, false, false);
            assert_eq!(
                frames.show(vec![key_event(key, false, Modifiers::NONE)], Late::None),
                Some(Command::Apply(exact)),
                "native_key={native_key:?}, alt_graph={alt_graph}: {}",
                frames.last_diagnostic
            );
        }
    }
}
