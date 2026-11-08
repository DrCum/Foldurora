//! Manual stopped-profile copy front. Native files/defaults belong exclusively
//! to the existing physical worker; this model retains metadata and input proof.
use super::profile_copy::{Capture, Instance, Outcome, Phase, Selection, Source, State};
use super::slots::SlotId;
use super::transport::{HostError, HostSnapshot};
use crate::settings::profile_seed::{Families, Family};
use crate::theme::Palette;
use crate::ui::widgets;
use egui::emath::GuiRounding as _;
use egui::{Id, Key, Rect, RichText};

#[cfg(test)]
mod tests;

const MAX_CONTROLS: usize = 24;
const MAX_SOURCES: usize = 5;
const MAX_CONTROL_PARENTS: usize = 16;
const MAX_CONTROL_FRAMES: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Command {
    Inspect(Selection),
    Apply(Capture),
}
/// Shell must redeem this once after the entire Context::run_ui has settled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Action(u64);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Intent {
    Source(Selection, Source),
    Family(Selection, u8),
    Inspect(Selection),
    Apply(Capture),
    Close(Selection),
    Picker(Selection),
}
#[derive(Clone, Copy)]
struct Control {
    widget: Id,
    rect: Rect,
    layer: egui::LayerId,
    intent: Intent,
    registered: egui::WidgetRect,
    paint_clip: Rect,
    transform: Option<egui::emath::TSTransform>,
    frame_origin: Option<FrameOrigin>,
    frame: Option<FrameWitness>,
}

/// Pinned egui Frame reserves its background shape before the contents, but
/// registers the enclosing hover allocation afterwards (Frame::begin/end).
/// Capture the actual Frame stack marker, never infer structure from hover Sense.
#[derive(Clone, Copy, Debug, PartialEq)]
struct FrameOrigin {
    content: Id,
    parent: Id,
    root: Id,
    frame: egui::Frame,
    outer: [Option<FrameSeed>; MAX_CONTROL_FRAMES - 1],
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct FrameSeed {
    content: Id,
    parent: Id,
    frame: egui::Frame,
}
impl FrameOrigin {
    fn capture(ui: &egui::Ui, layer: egui::LayerId, popup_source: bool) -> Option<Self> {
        let mut frames = [None; MAX_CONTROL_FRAMES];
        let mut count = 0;
        for node in ui.stack().iter().take(MAX_CONTROL_PARENTS) {
            if node.kind() == Some(egui::UiKind::Frame) {
                if count == MAX_CONTROL_FRAMES || !node.frame().stroke.width.is_finite() || node.frame().stroke.width < 0.0 {
                    return None;
                }
                frames[count] = Some(FrameSeed {
                    content: node.unique_id,
                    parent: node.parent.as_ref()?.unique_id,
                    frame: *node.frame(),
                });
                count += 1;
            }
            if node.unique_id == layer.id {
                let expected = if popup_source { egui::UiKind::Menu } else { egui::UiKind::Modal };
                if node.kind() != Some(expected) {
                    return None;
                }
                let FrameSeed { content, parent, frame } = frames[0]?;
                return Some(Self {
                    content,
                    parent,
                    root: node.unique_id,
                    frame,
                    outer: std::array::from_fn(|index| frames[index + 1]),
                });
            }
        }
        None
    }
    fn frames(self) -> impl Iterator<Item = FrameSeed> {
        std::iter::once(FrameSeed {
            content: self.content,
            parent: self.parent,
            frame: self.frame,
        })
        .chain(self.outer.into_iter().flatten())
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct FrameWitness {
    allocation: egui::WidgetRect,
    outer: [Option<egui::WidgetRect>; MAX_CONTROL_FRAMES - 1],
    parents: [Option<egui::WidgetRect>; MAX_CONTROL_PARENTS],
}
impl FrameWitness {
    fn capture(widgets: &egui::WidgetRects, control: &Control) -> Option<Self> {
        let origin = control.frame_origin?;
        let mut parents = [None; MAX_CONTROL_PARENTS];
        let mut id = control.registered.parent_id;
        let mut reached_root = false;
        let mut reached_content = false;
        for index in 0..MAX_CONTROL_PARENTS {
            if parents_seen(control.registered.id, &parents[..index], id) {
                return None;
            }
            let parent = *widgets.get(id)?;
            if parent.layer_id != control.layer
                || !parent.enabled
                || !parent.visible
                || !finite_positive(parent.rect)
                || !finite_positive(parent.interact_rect)
            {
                return None;
            }
            if id == origin.content {
                if parent.parent_id != origin.parent {
                    return None;
                }
                reached_content = true;
            }
            parents[index] = Some(parent);
            if id == origin.root {
                reached_root = true;
                break;
            }
            id = parent.parent_id;
        }
        if !reached_root || !reached_content {
            return None;
        }
        let (layer, control_index) = widgets.order(control.widget)?;
        if layer != control.layer {
            return None;
        }
        let mut allocations = [None; MAX_CONTROL_FRAMES];
        for (index, frame) in origin.frames().enumerate() {
            if !parents
                .iter()
                .flatten()
                .any(|parent| parent.id == frame.content && parent.parent_id == frame.parent)
                || !frame.frame.stroke.width.is_finite()
                || frame.frame.stroke.width < 0.0
            {
                return None;
            }
            let content = widgets.get(frame.content)?;
            let parent = widgets.get(frame.parent)?;
            // ScrollArea itself adds an inner Frame::NONE. Every actual Frame
            // reserves its paint early and registers its allocation afterwards.
            // Ui::remember_min_rect stores the same min_rect used here, and
            // pinned Ui::allocate_rect rounds that outer rect with round_ui.
            let outer = frame.frame.outer_rect(content.rect).round_ui();
            let interact = outer.intersect(parent.interact_rect);
            if !finite_positive(outer)
                || !finite_positive(interact)
                || !parent.rect.contains_rect(outer)
                || !outer.contains_rect(control.rect)
            {
                return None;
            }
            // A self-parent Area root is early bookkeeping, not Frame::end.
            let mut matching = widgets.get_layer(layer).skip(control_index + 1).filter(|widget| {
                widget.parent_id == frame.parent
                    && widget.layer_id == control.layer
                    && widget.id != frame.content
                    && widget.enabled
                    && widget.visible
                    && widget.sense == egui::Sense::hover()
                    && finite_positive(widget.rect)
                    && finite_positive(widget.interact_rect)
                    && widget.rect == outer
                    && widget.interact_rect == interact
            });
            let allocation = *matching.next()?;
            if matching.next().is_some()
                || allocations[..index]
                    .iter()
                    .flatten()
                    .any(|previous: &egui::WidgetRect| previous.id == allocation.id)
            {
                return None;
            }
            allocations[index] = Some(allocation);
        }
        Some(Self {
            allocation: allocations[0]?,
            outer: std::array::from_fn(|index| allocations[index + 1]),
            parents,
        })
    }
    fn contains(self, id: Id) -> bool {
        self.allocation.id == id || self.outer.into_iter().flatten().any(|allocation| allocation.id == id)
    }
}
fn parents_seen(control: Id, parents: &[Option<egui::WidgetRect>], id: Id) -> bool {
    id == control || parents.iter().flatten().any(|parent| parent.id == id)
}

fn finite_positive(rect: Rect) -> bool {
    rect.is_finite() && rect.size().is_finite() && rect.is_positive()
}
fn finite_transform(transform: egui::emath::TSTransform) -> bool {
    transform.scaling.is_finite() && transform.scaling > 0.0 && transform.translation.x.is_finite() && transform.translation.y.is_finite()
}
fn transformed_rect(rect: Rect, transform: Option<egui::emath::TSTransform>) -> Option<Rect> {
    let transform = transform.unwrap_or(egui::emath::TSTransform::IDENTITY);
    if !finite_positive(rect) || !finite_transform(transform) || !finite_transform(transform.inverse()) {
        return None;
    }
    let global = transform * rect;
    finite_positive(global).then_some(global)
}
impl Control {
    fn geometry(&self, ctx: &egui::Context) -> bool {
        // This modal supports untransformed native UI coordinates only. Check
        // BOTH translations and the inverse before comparing identity: pinned
        // TSTransform::is_valid does not validate translation.y.
        let transform = self.transform.unwrap_or(egui::emath::TSTransform::IDENTITY);
        let current = ctx.layer_transform_to_global(self.layer);
        let current_transform = current.unwrap_or(egui::emath::TSTransform::IDENTITY);
        if !finite_transform(current_transform) || !finite_transform(current_transform.inverse()) {
            return false;
        }
        let viewport = ctx.content_rect();
        let Some(rect) = transformed_rect(self.registered.rect, self.transform) else {
            return false;
        };
        let Some(interact) = transformed_rect(self.registered.interact_rect, self.transform) else {
            return false;
        };
        let Some(clip) = transformed_rect(self.paint_clip, self.transform) else {
            return false;
        };
        let visible = rect.intersect(interact).intersect(clip).intersect(viewport);
        transform == egui::emath::TSTransform::IDENTITY
            && finite_positive(viewport)
            && finite_positive(visible)
            && visible == self.rect
            && current == self.transform
    }
    fn registry(&self, ctx: &egui::Context, committed: bool) -> bool {
        if !self.geometry(ctx) || !ctx.memory(|memory| memory.allows_interaction(self.layer)) {
            return false;
        }
        ctx.viewport(|viewport| {
            let widgets = if committed {
                &viewport.prev_pass.widgets
            } else {
                &viewport.this_pass.widgets
            };
            widgets.get(self.widget).is_some_and(|widget| {
                widget.enabled
                    && widget.visible
                    && widget.sense.senses_click()
                    && finite_positive(widget.rect)
                    && finite_positive(widget.interact_rect)
                    && *widget == self.registered
                    && widget.id == self.widget
                    && widget.layer_id == self.layer
            })
        })
    }
    fn committed(&self, ctx: &egui::Context) -> bool {
        if !self.registry(ctx, true) {
            return false;
        }
        // A later visible widget in this same layer can cover the
        // original even when focus and layer_id_at still name the old layer.
        let unoccluded = ctx.viewport(|viewport| {
            let widgets = &viewport.prev_pass.widgets;
            let Some(frame) = self.frame else {
                return false;
            };
            if FrameWitness::capture(widgets, self) != Some(frame) {
                return false;
            }
            let Some((layer, index)) = widgets.order(self.widget) else {
                return false;
            };
            layer == self.layer
                && !widgets
                    .get_layer(layer)
                    .skip(index + 1)
                    .any(|widget| !frame.contains(widget.id) && widget.visible && widget.interact_rect.intersect(self.rect).is_positive())
        });
        if !unoccluded {
            return false;
        }
        // Resolve actual final Area order outside memory/viewport closures;
        // never nest Context accessors while holding one of their locks.
        let above = ctx.memory(|memory| {
            memory
                .layer_ids()
                .skip_while(|layer| *layer != self.layer)
                .skip(1)
                .filter(|layer| memory.areas().is_visible(layer))
                .collect::<Vec<_>>()
        });
        for layer in above {
            if let Some(area) = egui::AreaState::load(ctx, layer.id) {
                let Some(rect) = transformed_rect(area.rect(), ctx.layer_transform_to_global(layer)) else {
                    return false;
                };
                if rect.intersect(self.rect).is_positive() {
                    return false;
                }
            }
        }
        ctx.layer_id_at(self.rect.center()) == Some(self.layer)
    }
}
#[derive(Clone, Copy)]
enum Trigger {
    Pointer,
    Key(Key),
    Accessibility,
}
#[derive(Clone, Copy)]
struct Gesture {
    control: Control,
    trigger: Trigger,
    frame: u64,
    index: usize,
}
#[derive(Clone, Copy)]
enum Release {
    Pointer(egui::Pos2),
    Key(Key),
    Accessibility,
}
struct Completed {
    nonce: u64,
    control: Control,
    release: Release,
    frame: u64,
    pass: u64,
    modal: egui::LayerId,
    focused: Option<Id>,
}
struct Dialog {
    selection: Selection,
    bits: u8,
}
struct Operation {
    selection: Selection,
    request: u64,
    state: Option<State>,
    apply_submitted: bool,
    cancel_requested: bool,
    cancel_queued: bool,
    uncertain: bool,
}
impl Operation {
    fn terminal(&self) -> bool {
        self.state.as_ref().is_some_and(|state| matches!(state.phase, Phase::Terminal(_)))
    }
}

pub(super) struct CopyUi {
    instance: Option<Instance>,
    incarnation: u64,
    nonce: u64,
    dialog: Option<Dialog>,
    operation: Option<Operation>,
    queue_error: Option<HostError>,
    controls: Vec<Control>,
    painting: Vec<Control>,
    painted: Option<(u64, u64, egui::LayerId)>,
    input_frame: Option<u64>,
    gesture: Option<Gesture>,
    completed: Option<Completed>,
    ime: bool,
    native_enter: super::chat_window::enter_release::EnterRelease,
    native_space: super::chat_window::enter_release::EnterRelease,
    rejected_keys: [bool; 2],
    native_modifiers: winit::keyboard::ModifiersState,
    modifier_transition: bool,
    native_focused: bool,
    alt_graph: bool,
    alt_graph_before_focus: bool,
}
impl Default for CopyUi {
    fn default() -> Self {
        let mut native_enter = super::chat_window::enter_release::EnterRelease::default();
        let mut native_space = super::chat_window::enter_release::EnterRelease::default();
        native_enter.focused(true);
        native_space.focused(true);
        Self {
            instance: Instance::fresh(),
            incarnation: 0,
            nonce: 0,
            dialog: None,
            operation: None,
            queue_error: None,
            controls: Vec::with_capacity(MAX_CONTROLS),
            painting: Vec::with_capacity(MAX_CONTROLS),
            painted: None,
            input_frame: None,
            gesture: None,
            completed: None,
            ime: false,
            native_enter,
            native_space,
            rejected_keys: [false; 2],
            native_modifiers: winit::keyboard::ModifiersState::empty(),
            modifier_transition: false,
            native_focused: true,
            alt_graph: false,
            alt_graph_before_focus: false,
        }
    }
}

impl CopyUi {
    pub fn native_event(&mut self, event: &winit::event::WindowEvent) {
        match event {
            winit::event::WindowEvent::Focused(focused) => {
                if self.native_focused != *focused {
                    if *focused {
                        self.alt_graph = self.alt_graph_before_focus;
                    }
                    self.alt_graph_before_focus = false;
                    self.native_focused = *focused;
                }
                self.native_enter.focused(*focused);
                self.native_space.focused(*focused);
                if !*focused {
                    self.cancel_gestures();
                }
            }
            winit::event::WindowEvent::KeyboardInput { event, is_synthetic, .. } => {
                self.native_key(&event.logical_key, event.state, event.repeat, *is_synthetic);
            }
            winit::event::WindowEvent::ModifiersChanged(modifiers) => {
                let next = modifiers.state();
                if self.native_modifiers != next {
                    self.native_modifiers = next;
                    self.modifier_transition = true;
                    self.cancel_gestures();
                }
            }
            _ => {}
        }
    }
    fn native_key(&mut self, key: &winit::keyboard::Key, state: winit::event::ElementState, repeat: bool, synthetic: bool) {
        use winit::keyboard::{Key as NativeKey, NamedKey};
        if matches!(key, NativeKey::Named(NamedKey::AltGraph)) {
            if state == winit::event::ElementState::Pressed {
                self.alt_graph = true;
                if !self.native_focused {
                    self.alt_graph_before_focus = true;
                }
            } else if !synthetic {
                self.alt_graph = false;
                self.alt_graph_before_focus = false;
            }
            self.modifier_transition = true;
            self.cancel_gestures();
            return;
        }
        if matches!(
            key,
            NativeKey::Named(NamedKey::Alt | NamedKey::Control | NamedKey::Shift | NamedKey::Super | NamedKey::Meta)
        ) {
            self.modifier_transition = true;
            self.cancel_gestures();
        }
        let (guard, index) = match key {
            NativeKey::Named(NamedKey::Enter) => (&mut self.native_enter, 0),
            NativeKey::Named(NamedKey::Space) => (&mut self.native_space, 1),
            NativeKey::Character(value) if value.as_str() == " " => (&mut self.native_space, 1),
            _ => return,
        };
        if guard.suppress(&NativeKey::Named(NamedKey::Enter), state, repeat, synthetic, self.native_modifiers) {
            // Keep a denied down rejected for its whole RawInput batch even
            // if a later native up/fresh down precedes the next paint.
            self.rejected_keys[index] = true;
        }
    }
    fn key_allowed(&self, key: Key) -> bool {
        if !self.modifiers_plain() {
            return false;
        }
        match key {
            Key::Enter => !self.rejected_keys[0],
            Key::Space => !self.rejected_keys[1],
            _ => false,
        }
    }
    fn modifiers_plain(&self) -> bool {
        self.native_modifiers.is_empty() && !self.alt_graph && !self.modifier_transition
    }
    pub fn is_open(&self) -> bool {
        self.dialog.is_some()
    }
    pub fn cancel_gestures(&mut self) {
        self.controls.clear();
        self.painting.clear();
        self.painted = None;
        self.gesture = None;
        self.completed = None;
    }
    fn selection(&self) -> Option<Selection> {
        self.dialog.as_ref().map(|dialog| dialog.selection)
    }
    fn busy(&self) -> bool {
        self.operation.as_ref().is_some_and(|operation| !operation.terminal())
    }
    fn revoke_operation(&mut self) {
        if let Some(operation) = &mut self.operation
            && !operation.terminal()
        {
            operation.cancel_requested = true;
        }
        self.cancel_gestures();
    }
    pub fn close(&mut self) {
        self.revoke_operation();
        self.dialog = None;
    }
    pub fn open(&mut self, target: SlotId, snapshot: &HostSnapshot) {
        if !stopped(snapshot, target) || snapshot.stopped {
            return;
        }
        self.close();
        let (Some(instance), Some(incarnation)) = (self.instance, self.incarnation.checked_add(1)) else {
            return;
        };
        self.incarnation = incarnation;
        self.dialog = Some(Dialog {
            selection: Selection {
                instance,
                incarnation,
                target,
                source: Source::MainSaved,
                families: Families::new(15).unwrap_or_else(|_| unreachable!("fixed four-family mask")),
            },
            bits: 15,
        });
        self.queue_error = None;
    }
    fn change(&mut self, source: Option<Source>, bits: Option<u8>) {
        self.revoke_operation();
        let Some(next) = self.incarnation.checked_add(1) else {
            self.dialog = None;
            return;
        };
        self.incarnation = next;
        if let Some(dialog) = &mut self.dialog {
            dialog.selection.incarnation = next;
            if let Some(source) = source {
                dialog.selection.source = source;
            }
            if let Some(bits) = bits {
                dialog.bits = bits;
            }
            // An empty group set has no submit authority. Preserve a valid
            // placeholder only inside the selection; Inspect also checks bits.
            if let Ok(families) = Families::new(dialog.bits) {
                dialog.selection.families = families;
            }
        }
        self.queue_error = None;
    }
    pub fn sync(&mut self, snapshot: &HostSnapshot) -> bool {
        let Some(operation) = &mut self.operation else { return false };
        let before = (operation.state.clone(), operation.uncertain);
        let uncertain = operation.uncertain
            || operation
                .state
                .as_ref()
                .is_some_and(|state| matches!(state.phase, Phase::Terminal(Outcome::Uncertain)));
        if uncertain
            && let Some(state) = &snapshot.copy_preferences
            && state.capture.selection == operation.selection
            && state.capture.operation == operation.request
            && matches!(state.phase, Phase::Terminal(_))
            && operation.state.as_ref().is_none_or(|old| old.capture == state.capture)
        {
            operation.state = Some(state.clone());
            operation.uncertain = false;
        } else if !operation.terminal() {
            if let Some(state) = &snapshot.copy_preferences
                && state.capture.selection == operation.selection
                && state.capture.operation == operation.request
            {
                if operation.state.as_ref().is_none_or(|old| old.capture == state.capture) {
                    // A queued Apply cannot borrow an earlier Prepared snapshot.
                    if !(operation.apply_submitted && matches!(state.phase, Phase::Prepared(_))) {
                        operation.state = Some(state.clone());
                    }
                } else {
                    operation.uncertain = true;
                }
            } else if snapshot.stopped || snapshot.completed_request >= operation.request {
                // No acknowledged terminal for this operation: do not infer a
                // no-write outcome from a newer receipt or dead controller.
                operation.uncertain = true;
            }
        }
        if snapshot.stopped && !operation.terminal() {
            operation.uncertain = true;
        }
        let changed = before != (operation.state.clone(), operation.uncertain);
        if changed {
            self.cancel_gestures();
        }
        changed
    }
    pub fn cancellation(&self) -> Option<(Selection, u64)> {
        self.operation
            .as_ref()
            .filter(|operation| operation.cancel_requested && !operation.cancel_queued && !operation.terminal())
            .map(|operation| (operation.selection, operation.request))
    }
    pub fn cancellation_queued(&mut self, selection: Selection, request: u64) {
        if let Some(operation) = &mut self.operation
            && operation.selection == selection
            && operation.request == request
        {
            operation.cancel_queued = true;
        }
    }
    pub fn queued(&mut self, command: Command, result: Result<u64, HostError>) {
        self.cancel_gestures();
        self.queue_error = result.err();
        let Ok(request) = result else {
            if matches!(command, Command::Apply(_)) {
                self.revoke_operation();
            }
            return;
        };
        match command {
            Command::Inspect(selection) => {
                self.operation = Some(Operation {
                    selection,
                    request,
                    state: None,
                    apply_submitted: false,
                    cancel_requested: false,
                    cancel_queued: false,
                    uncertain: false,
                })
            }
            Command::Apply(capture) => {
                if let Some(operation) = &mut self.operation
                    && operation.state.as_ref().is_some_and(|state| state.capture == capture)
                {
                    operation.apply_submitted = true;
                    if let Some(state) = &mut operation.state {
                        state.phase = Phase::Applying;
                    }
                }
            }
        }
    }
    fn valid(&self, intent: Intent, snapshot: &HostSnapshot) -> bool {
        if snapshot.stopped {
            return false;
        }
        match intent {
            Intent::Source(selection, source) => {
                self.selection() == Some(selection)
                    && source.slot() != Some(selection.target)
                    && source.slot().is_none_or(|slot| stopped(snapshot, slot))
            }
            Intent::Family(selection, _) | Intent::Close(selection) | Intent::Picker(selection) => self.selection() == Some(selection),
            Intent::Inspect(selection) => {
                self.selection() == Some(selection)
                    && !self.busy()
                    && self.dialog.as_ref().is_some_and(|dialog| dialog.bits != 0)
                    && stopped(snapshot, selection.target)
                    && selection.source.slot().is_none_or(|slot| stopped(snapshot, slot))
            }
            Intent::Apply(capture) => {
                self.selection() == Some(capture.selection)
                    && self.operation.as_ref().is_some_and(|operation| {
                        !operation.uncertain
                            && !operation.apply_submitted
                            && !operation.cancel_requested
                            && operation
                                .state
                                .as_ref()
                                .is_some_and(|state| state.capture == capture && matches!(state.phase, Phase::Prepared(_)))
                    })
                    && snapshot
                        .copy_preferences
                        .as_ref()
                        .is_some_and(|state| state.capture == capture && matches!(state.phase, Phase::Prepared(_)))
            }
        }
    }
    /// Final authority check after the WHOLE egui run. Taking the proof first
    /// makes discarded/late-focus/modal rejection final, with no queued retry.
    pub fn take_action(&mut self, action: Action, ctx: &egui::Context, snapshot: &HostSnapshot, other_modal: bool) -> Option<Command> {
        let completed = self.completed.take()?;
        let live = !other_modal
            && !self.ime
            && self.modifiers_plain()
            && self.is_open()
            && !ctx.will_discard()
            && completed.nonce == action.0
            && completed.frame.checked_add(1) == Some(ctx.cumulative_frame_nr())
            && completed.pass.checked_add(1) == Some(ctx.cumulative_pass_nr())
            && ctx.input(|input| {
                input.focused
                    && input.modifiers.is_none()
                    && !input.events.iter().any(|event| match event {
                        egui::Event::WindowFocused(_) | egui::Event::Ime(_) => true,
                        egui::Event::ModifiersChanged(modifiers) | egui::Event::Key { modifiers, .. } => !modifiers.is_none(),
                        _ => false,
                    })
            })
            && ctx.memory(|memory| memory.top_modal_layer()) == Some(completed.modal)
            && ctx.memory(|memory| memory.focused()) == completed.focused
            && completed.control.committed(ctx)
            && ctx.content_rect().contains_rect(completed.control.rect)
            && self.valid(completed.control.intent, snapshot)
            && (!egui::Popup::is_any_open(ctx) || matches!(completed.control.intent, Intent::Source(_, _) | Intent::Picker(_)))
            && match completed.release {
                Release::Pointer(pos) => completed.control.rect.contains(pos) && ctx.layer_id_at(pos) == Some(completed.control.layer),
                Release::Key(key) => self.key_allowed(key) && ctx.memory(|memory| memory.focused()) == Some(completed.control.widget),
                Release::Accessibility => ctx.memory(|memory| memory.focused()) == Some(completed.control.widget),
            };
        if !live {
            self.cancel_gestures();
            return None;
        }
        match completed.control.intent {
            Intent::Source(_, source) => {
                egui::Popup::close_all(ctx);
                self.change(Some(source), None);
                None
            }
            Intent::Family(_, bits) => {
                self.change(None, Some(bits));
                None
            }
            Intent::Close(_) => {
                egui::Popup::close_all(ctx);
                self.close();
                None
            }
            Intent::Inspect(selection) => Some(Command::Inspect(selection)),
            Intent::Apply(capture) => Some(Command::Apply(capture)),
            Intent::Picker(_) => None,
        }
    }
    pub fn finish_frame(&mut self, ctx: &egui::Context) {
        if self.is_open()
            && !self.ime
            && self.modifiers_plain()
            && !ctx.will_discard()
            && ctx.input(|input| input.focused)
            && self.painted.is_some_and(|(frame, pass, modal)| {
                frame.checked_add(1) == Some(ctx.cumulative_frame_nr())
                    && pass.checked_add(1) == Some(ctx.cumulative_pass_nr())
                    && ctx.memory(|memory| memory.top_modal_layer()) == Some(modal)
            })
        {
            self.painting.retain(|control| control.committed(ctx));
            std::mem::swap(&mut self.controls, &mut self.painting);
            self.painting.clear();
        } else {
            self.cancel_gestures();
        }
        self.completed = None;
        self.rejected_keys = [false; 2];
        self.modifier_transition = false;
    }
    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, snapshot: &HostSnapshot, blocked: bool) -> Option<Action> {
        self.sync(snapshot);
        self.completed = None;
        let Some(selection) = self.selection() else {
            self.cancel_gestures();
            return None;
        };
        ctx.input(|input| {
            for event in &input.events {
                match event {
                    egui::Event::Ime(egui::ImeEvent::Preedit { text, .. }) => self.ime = !text.is_empty(),
                    egui::Event::Ime(egui::ImeEvent::Commit(_)) | egui::Event::WindowFocused(false) => self.ime = false,
                    _ => {}
                }
            }
        });
        let raw_modifiers = ctx.input(|input| {
            !input.modifiers.is_none()
                || input.events.iter().any(|event| match event {
                    egui::Event::ModifiersChanged(modifiers) | egui::Event::Key { modifiers, .. } => !modifiers.is_none(),
                    _ => false,
                })
        });
        if raw_modifiers {
            self.modifier_transition = true;
            self.cancel_gestures();
        }
        let focused = !self.ime
            && self.modifiers_plain()
            && ctx.input(|input| {
                input.focused
                    && !input
                        .events
                        .iter()
                        .any(|event| matches!(event, egui::Event::WindowFocused(_) | egui::Event::Ime(_)))
            });
        if blocked || !focused || snapshot.stopped {
            self.cancel_gestures();
        }
        let live = !blocked && focused && !snapshot.stopped;
        self.capture_press(ctx, snapshot, live);
        self.painting.clear();
        let width = (ctx.content_rect().width() - 52.0).clamp(1.0, 420.0);
        let height = (ctx.content_rect().height() - 190.0).clamp(24.0, 440.0);
        let mut candidate = None;
        let shown = widgets::Modal::new(Id::unique(("host_profile_copy", selection.instance, selection.incarnation))).show(ctx, |ui| {
            ui.set_width(width);
            ui.set_max_width(width);
            ui.label(RichText::new(aurora_i18n::tr!("host-profile-copy-title")).strong());
            let scroll = egui::ScrollArea::vertical()
                .id_salt("host_copy_body")
                .max_height(height)
                .min_scrolled_height(height)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.set_max_width(width);
                    ui.label(aurora_i18n::tr!(
                        "host-profile-copy-destination",
                        destination = caption(snapshot, selection.target)
                    ));
                    ui.label(aurora_i18n::tr!("host-profile-copy-source"));
                    let source_name = match selection.source {
                        Source::MainSaved => aurora_i18n::tr!("host-profile-copy-main").to_owned(),
                        Source::Slot(slot) => caption(snapshot, slot),
                    };
                    let picker_width = ui.available_width().min(width);
                    let combo = ui
                        .allocate_ui_with_layout(egui::vec2(picker_width, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                            ui.set_min_width(picker_width);
                            ui.set_max_width(picker_width);
                            egui::ComboBox::from_id_salt(("copy_source", selection.incarnation))
                                .width(picker_width)
                                .truncate()
                                .selected_text(egui::RichText::new(source_name))
                                .show_ui(ui, |ui| {
                                    let inner = (picker_width - egui::Frame::popup(ui.style()).total_margin().sum().x).max(1.0);
                                    ui.set_max_width(ui.available_width().min(inner));
                                    #[cfg(test)]
                                    ctx.data_mut(|data| {
                                        data.insert_temp(Id::unique("host_copy_popup"), ui.layer_id().id);
                                    });
                                    for source in sources(snapshot, selection.target) {
                                        let label = match source {
                                            Source::MainSaved => aurora_i18n::tr!("host-profile-copy-main").to_owned(),
                                            Source::Slot(slot) => caption(snapshot, slot),
                                        };
                                        let response = ui.add(egui::Button::new(label).selected(selection.source == source).wrap());
                                        self.control(ui, response, Intent::Source(selection, source), snapshot, live, &mut candidate);
                                    }
                                })
                        })
                        .inner;
                    self.control(ui, combo.response, Intent::Picker(selection), snapshot, live, &mut candidate);
                    if selection.source == Source::MainSaved {
                        ui.label(aurora_i18n::tr!("host-profile-copy-main-help"));
                    }
                    ui.label(aurora_i18n::tr!("host-profile-copy-selected-help"));
                    let bits = self.dialog.as_ref().map_or(0, |dialog| dialog.bits);
                    for (index, family) in Family::ALL.into_iter().enumerate() {
                        let mask = 1 << index;
                        let response = ui
                            .push_id(("copy_family", index, selection.incarnation), |ui| {
                                widgets::checkbox_value(ui, p, bits & mask != 0, family_name(family))
                            })
                            .inner;
                        self.control(ui, response, Intent::Family(selection, bits ^ mask), snapshot, live, &mut candidate);
                    }
                    if bits == 0 {
                        ui.label(aurora_i18n::tr!("host-profile-copy-choose"));
                    }
                    self.status(ui, snapshot);
                });
            #[cfg(test)]
            ctx.data_mut(|data| {
                data.insert_temp(Id::unique("host_copy_scroll"), scroll.inner_rect);
            });
            #[cfg(not(test))]
            let _ = scroll;
            ui.horizontal_wrapped(|ui| {
                for (index, (label, intent)) in [
                    (aurora_i18n::tr!("host-profile-copy-inspect"), Some(Intent::Inspect(selection))),
                    (
                        aurora_i18n::tr!("common-apply"),
                        self.operation
                            .as_ref()
                            .and_then(|operation| operation.state.as_ref())
                            .map(|state| Intent::Apply(state.capture)),
                    ),
                    (
                        aurora_i18n::text(if self.operation.as_ref().is_some_and(Operation::terminal) {
                            "common-close"
                        } else {
                            "common-cancel"
                        }),
                        Some(Intent::Close(selection)),
                    ),
                ]
                .into_iter()
                .enumerate()
                {
                    let enabled = intent.is_some_and(|intent| self.valid(intent, snapshot)) && live;
                    let response = if matches!(intent, Some(Intent::Inspect(_) | Intent::Apply(_))) {
                        ui.add_enabled_ui(enabled, |ui| widgets::primary_button_sized(ui, p, label, egui::vec2(0.0, 30.0)))
                            .inner
                    } else {
                        ui.add_enabled(enabled, egui::Button::new(label).wrap())
                    };
                    #[cfg(test)]
                    ctx.data_mut(|data| {
                        data.insert_temp(Id::unique(("host_copy_footer", index)), response.rect);
                    });
                    #[cfg(not(test))]
                    let _ = index;
                    if let Some(intent) = intent {
                        self.control(ui, response, intent, snapshot, live, &mut candidate);
                    }
                }
            });
        });
        #[cfg(test)]
        ctx.data_mut(|data| {
            data.insert_temp(Id::unique("host_copy_modal"), shown.response.rect);
        });
        // Backdrop/Escape dismissal is local presentation only. Keep the exact
        // operation cancellation latch for Shell's bounded try-only polling.
        if shown.should_close() {
            self.close();
            ctx.request_repaint();
            return None;
        }
        if ctx.will_discard() {
            self.completed = None;
            self.gesture = None;
            return None;
        }
        // Frame::end and all enclosing Ui drops have now finalized the actual
        // structural allocation and parent registry. Do not guess it in paint.
        ctx.viewport(|viewport| {
            for control in &mut self.painting {
                control.frame = FrameWitness::capture(&viewport.this_pass.widgets, control);
            }
            if let Some((control, _)) = &mut candidate {
                control.frame = FrameWitness::capture(&viewport.this_pass.widgets, control);
            }
        });
        self.painted = Some((ctx.cumulative_frame_nr(), ctx.cumulative_pass_nr(), shown.response.layer_id));
        let candidate = candidate.and_then(|(control, release)| {
            if control.frame.is_some()
                && self
                    .gesture
                    .is_some_and(|gesture| gesture.control.frame_origin == control.frame_origin && gesture.control.frame == control.frame)
            {
                Some((control, release))
            } else {
                self.gesture = None;
                None
            }
        });
        if let Some((control, release)) = candidate {
            self.gesture = None;
            let nonce = self.nonce.checked_add(1)?;
            self.nonce = nonce;
            self.completed = Some(Completed {
                nonce,
                control,
                release,
                frame: ctx.cumulative_frame_nr(),
                pass: ctx.cumulative_pass_nr(),
                modal: shown.response.layer_id,
                focused: ctx.memory(|memory| memory.focused()),
            });
            return Some(Action(nonce));
        }
        if ctx.input(|input| {
            input.events.iter().any(|event| {
                matches!(
                    event,
                    egui::Event::PointerButton { pressed: false, .. } | egui::Event::Key { pressed: false, .. }
                )
            })
        }) {
            self.gesture = None;
        }
        None
    }
    fn status(&self, ui: &mut egui::Ui, snapshot: &HostSnapshot) {
        if let Some(error) = self.queue_error {
            ui.label(match error {
                HostError::Busy => aurora_i18n::tr!("host-busy"),
                HostError::Stopped => aurora_i18n::tr!("host-stopped"),
                HostError::Unsupported | HostError::Unavailable => aurora_i18n::tr!("host-profile-copy-unavailable"),
            });
        }
        let Some(operation) = &self.operation else { return };
        let target = caption(snapshot, operation.selection.target);
        if self
            .selection()
            .is_some_and(|selection| selection.target != operation.selection.target)
        {
            ui.label(aurora_i18n::tr!("host-profile-copy-destination", destination = target.clone()));
        }
        if operation.uncertain {
            ui.label(aurora_i18n::tr!("host-profile-copy-uncertain", destination = target));
            return;
        }
        let Some(state) = &operation.state else {
            ui.label(aurora_i18n::tr!("host-profile-copy-inspecting"));
            return;
        };
        if operation.cancel_requested && !operation.terminal() {
            ui.label(aurora_i18n::tr!("host-profile-copy-cancel-requested"));
        }
        if self.selection() != Some(operation.selection) && !operation.terminal() {
            ui.label(aurora_i18n::tr!("host-profile-copy-retiring"));
        }
        let key = match &state.phase {
            Phase::Inspecting => "host-profile-copy-inspecting",
            Phase::Prepared(summary) => {
                if self.selection() == Some(operation.selection) && !operation.cancel_requested {
                    for item in summary.iter().take(4) {
                        ui.label(aurora_i18n::tr!(
                            "host-profile-copy-summary",
                            family = family_name(item.family),
                            fields = item.fields as i64,
                            changed = item.changed as i64,
                            defaulted = item.defaulted as i64
                        ));
                    }
                }
                if operation.cancel_requested || self.selection() != Some(operation.selection) {
                    "host-profile-copy-retiring"
                } else {
                    "host-profile-copy-prepared"
                }
            }
            Phase::Applying => "host-profile-copy-applying",
            Phase::Terminal(Outcome::Applied) => {
                ui.label(aurora_i18n::tr!("host-profile-copy-applied", destination = target));
                return;
            }
            Phase::Terminal(Outcome::Uncertain) => {
                ui.label(aurora_i18n::tr!("host-profile-copy-uncertain", destination = target));
                return;
            }
            Phase::Terminal(Outcome::CancelledBeforeCommit) => "host-profile-copy-canceled",
            Phase::Terminal(Outcome::Expired) => "host-profile-copy-expired",
            Phase::Terminal(Outcome::Changed) => "host-profile-copy-changed",
            Phase::Terminal(Outcome::Refused(super::profile_copy::Failure::Busy)) => "host-busy",
            Phase::Terminal(Outcome::Refused(super::profile_copy::Failure::Stopped)) => "host-stopped",
            Phase::Terminal(Outcome::Refused(super::profile_copy::Failure::Projection(_))) => "host-profile-copy-invalid-file",
            Phase::Terminal(Outcome::Refused(_)) => "host-profile-copy-failed",
        };
        ui.label(aurora_i18n::text(key));
    }
    fn control(
        &mut self,
        ui: &egui::Ui,
        response: egui::Response,
        intent: Intent,
        snapshot: &HostSnapshot,
        live: bool,
        candidate: &mut Option<(Control, Release)>,
    ) {
        #[cfg(test)]
        ui.ctx().data_mut(|data| {
            data.insert_temp(response.id.with("copy_rect"), response.rect);
        });
        let Some(registered) = ui.ctx().viewport(|viewport| viewport.this_pass.widgets.get(response.id).copied()) else {
            return;
        };
        let paint_clip = ui.clip_rect();
        let transform = ui.ctx().layer_transform_to_global(response.layer_id);
        let rect = response
            .rect
            .intersect(response.interact_rect)
            .intersect(paint_clip)
            .intersect(ui.ctx().content_rect());
        let control = Control {
            widget: response.id,
            rect,
            layer: response.layer_id,
            intent,
            registered,
            paint_clip,
            transform,
            frame_origin: FrameOrigin::capture(ui, response.layer_id, matches!(intent, Intent::Source(_, _))),
            frame: None,
        };
        #[cfg(test)]
        if matches!(intent, Intent::Source(_, _)) {
            // Record actual Source capture eligibility before any refusal.
            // This adds no Ui/ID scope and grants no input authority.
            let record = tests::SourceDiagnostic {
                control,
                stack: ui
                    .stack()
                    .iter()
                    .take(MAX_CONTROL_PARENTS)
                    .map(|node| tests::SourceStackNode {
                        id: node.unique_id,
                        parent: node.parent.as_ref().map(|parent| parent.unique_id),
                        kind: node.kind(),
                    })
                    .collect(),
                flags: [
                    live,
                    response.enabled(),
                    self.valid(intent, snapshot),
                    control.registry(ui.ctx(), false),
                    ui.ctx().memory(|memory| memory.allows_interaction(response.layer_id)),
                ],
                pass: ui.ctx().current_pass_index(),
            };
            ui.ctx().data_mut(|data| {
                let records = data.get_temp_mut_or_default::<Vec<tests::SourceDiagnostic>>(Id::unique("copy_fixture_source_diagnostics"));
                if records.len() < MAX_CONTROLS {
                    records.push(record);
                }
            });
        }
        if !live
            || !response.enabled()
            || response.rect != registered.rect
            || response.interact_rect != registered.interact_rect
            || response.sense != registered.sense
            || !self.valid(intent, snapshot)
            || !control.registry(ui.ctx(), false)
            || self.painting.len() >= MAX_CONTROLS
            || egui::Popup::is_any_open(ui.ctx()) && !matches!(intent, Intent::Source(_, _) | Intent::Picker(_))
        {
            return;
        }
        self.painting.push(control);
        let Some(gesture) = self.gesture.filter(|gesture| {
            gesture.control.widget == control.widget
                && gesture.control.intent == intent
                && gesture.control.layer == control.layer
                && gesture.control.rect == control.rect
                && gesture.control.registered == control.registered
                && gesture.control.paint_clip == control.paint_clip
                && gesture.control.transform == control.transform
        }) else {
            return;
        };
        let frame = ui.ctx().cumulative_frame_nr();
        let after = |index| gesture.frame != frame || index > gesture.index;
        let release = match gesture.trigger {
            Trigger::Pointer => ui.input(|input| input.events.iter().enumerate().find_map(|(index,event)| match event {
                egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, .. } if after(index) => Some(*pos), _ => None,
            })).filter(|pos| rect.contains(*pos) && ui.ctx().layer_id_at(*pos) == Some(control.layer)).map(Release::Pointer),
            Trigger::Key(key) => (self.key_allowed(key) && response.has_focus() && ui.input(|input| input.modifiers.is_none() && input.events.iter().enumerate().any(|(index,event)| matches!(event, egui::Event::Key { key: released, pressed:false, modifiers, .. } if *released == key && modifiers.is_none() && after(index))))).then_some(Release::Key(key)),
            Trigger::Accessibility => (response.has_focus() && ui.input(|input| input.has_accesskit_action_request(response.id, egui::accesskit::Action::Click))).then_some(Release::Accessibility),
        };
        if let Some(release) = release
            && candidate.is_none()
            && !matches!(intent, Intent::Picker(_))
        {
            *candidate = Some((control, release));
        }
    }
    fn capture_press(&mut self, ctx: &egui::Context, snapshot: &HostSnapshot, live: bool) {
        let frame = ctx.cumulative_frame_nr();
        if self.input_frame == Some(frame) {
            return;
        }
        self.input_frame = Some(frame);
        if !live || self.gesture.is_some() {
            return;
        }
        let pointer = ctx.input(|input| {
            input.events.iter().enumerate().find_map(|(index, event)| match event {
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    ..
                } => Some((index, *pos)),
                _ => None,
            })
        });
        let focus = ctx.memory(|memory| memory.focused());
        let captured = if let Some((index, pos)) = pointer {
            self.controls
                .iter()
                .rev()
                .find(|control| control.rect.contains(pos) && ctx.layer_id_at(pos) == Some(control.layer))
                .map(|control| (index, Trigger::Pointer, *control))
        } else {
            let key = ctx.input(|input| {
                (!input
                    .events
                    .iter()
                    .any(|event| matches!(event, egui::Event::Key { key: Key::Tab, .. })))
                .then(|| {
                    input.events.iter().enumerate().find_map(|(index, event)| match event {
                        egui::Event::Key {
                            key,
                            pressed: true,
                            repeat: false,
                            modifiers,
                            ..
                        } if matches!(key, Key::Enter | Key::Space) && modifiers.is_none() => Some((index, *key)),
                        _ => None,
                    })
                })
                .flatten()
            });
            key.filter(|(_, key)| self.key_allowed(*key))
                .and_then(|(index, key)| {
                    self.controls
                        .iter()
                        .find(|control| Some(control.widget) == focus)
                        .map(|control| (index, Trigger::Key(key), *control))
                })
                .or_else(|| {
                    self.controls
                        .iter()
                        .find(|control| {
                            Some(control.widget) == focus
                                && ctx.input(|input| input.has_accesskit_action_request(control.widget, egui::accesskit::Action::Click))
                        })
                        .map(|control| (0, Trigger::Accessibility, *control))
                })
        };
        if let Some((index, trigger, control)) = captured.filter(|(_, _, control)| {
            self.valid(control.intent, snapshot)
                && control.committed(ctx)
                && (!egui::Popup::is_any_open(ctx) || matches!(control.intent, Intent::Source(_, _) | Intent::Picker(_)))
        }) {
            self.gesture = Some(Gesture {
                control,
                trigger,
                frame,
                index,
            });
        }
    }
}
fn stopped(snapshot: &HostSnapshot, slot: SlotId) -> bool {
    snapshot.slots.iter().any(|item| item.id == slot && item.available)
}
fn sources(snapshot: &HostSnapshot, target: SlotId) -> Vec<Source> {
    std::iter::once(Source::MainSaved)
        .chain(
            SlotId::all()
                .filter(|slot| *slot != target && stopped(snapshot, *slot))
                .map(Source::Slot),
        )
        .take(MAX_SOURCES)
        .collect()
}
fn caption(snapshot: &HostSnapshot, slot: SlotId) -> String {
    let base = aurora_i18n::tr!("host-profile-default", number = i64::from(slot.number()));
    snapshot
        .slots
        .iter()
        .find(|item| item.id == slot)
        .filter(|item| !item.label.as_str().is_empty())
        .map_or(base.clone(), |item| format!("{base} · {}", item.label.as_str()))
}
fn family_name(family: Family) -> &'static str {
    aurora_i18n::text(match family {
        Family::Graphics => "host-profile-copy-graphics",
        Family::Interface => "host-profile-copy-interface",
        Family::Colors => "host-profile-copy-colors",
        Family::Bindings => "host-profile-copy-bindings",
    })
}
