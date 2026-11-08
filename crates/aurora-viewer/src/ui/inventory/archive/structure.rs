//! Exact bounded witnesses for the late structural hover allocations of actual
//! enclosing egui Frames. Their paint was reserved before the contents; every
//! other later visible widget remains a possible occluder.
use super::{Control, finite_positive};
use egui::emath::GuiRounding as _;
use egui::{Id, LayerId, UiKind, WidgetRect, WidgetRects};

const MAX_PARENTS: usize = 32;
const MAX_FRAMES: usize = 8;
#[derive(Clone, Copy, Debug, PartialEq)]
struct FrameOrigin {
    content: Id,
    parent: Id,
    frame: egui::Frame,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Origin {
    area: Id,
    kind: UiKind,
    frames: [Option<FrameOrigin>; MAX_FRAMES],
    links: [Option<(Id, Id)>; MAX_PARENTS],
}
impl Origin {
    #[cfg(test)]
    pub(super) fn diagnostic(self, widgets: &WidgetRects, control: &Control) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        let _ = writeln!(
            out,
            "origin={self:?} final_links={:?}",
            parent_links(widgets, control.registered, self.area, control.layer)
        );
        for (id, _) in self.links.iter().flatten() {
            let _ = writeln!(out, "parent {id:?}: {:?}", widgets.get(*id));
        }
        for frame in self.frames.iter().flatten() {
            let content = widgets.get(frame.content);
            let parent = widgets.get(frame.parent);
            let outer = content.map(|content| frame.frame.outer_rect(content.rect).round_ui());
            let interact = outer.zip(parent).map(|(outer, parent)| outer.intersect(parent.interact_rect));
            let _ = writeln!(
                out,
                "frame={frame:?} content={content:?} parent={parent:?} outer={outer:?} interact={interact:?}"
            );
            for candidate in widgets
                .get_layer(control.layer)
                .filter(|widget| widget.parent_id == frame.parent)
                .take(16)
            {
                let _ = writeln!(out, "frame_sibling {:?} order={:?}", candidate, widgets.order(candidate.id));
            }
        }
        let local = control
            .registered
            .rect
            .intersect(control.registered.interact_rect)
            .intersect(control.paint_clip);
        if let Some((layer, index)) = widgets.order(control.widget) {
            for cover in widgets
                .get_layer(layer)
                .skip(index + 1)
                .filter(|widget| widget.visible && widget.interact_rect.intersect(local).is_positive())
                .take(32)
            {
                let _ = writeln!(
                    out,
                    "late_intersection {:?} witnessed={}",
                    cover,
                    control.structure.is_some_and(|witness| witness.contains(cover.id))
                );
            }
        }
        out
    }

    pub(super) fn capture(ui: &egui::Ui, registered: WidgetRect) -> Option<Self> {
        let layer = registered.layer_id;
        let mut frames = [None; MAX_FRAMES];
        let mut count = 0;
        let mut area = None;
        // The actual popup Area and Frame stack tags establish structure.
        // A hover sense, shape size or guessed generated ID does not.
        for node in ui.stack().iter().take(MAX_PARENTS) {
            if node.kind() == Some(UiKind::Frame) {
                if count == MAX_FRAMES {
                    return None;
                }
                frames[count] = Some(FrameOrigin {
                    content: node.unique_id,
                    parent: node.parent.as_ref()?.unique_id,
                    frame: *node.frame(),
                });
                count += 1;
            }
            if node.unique_id == layer.id {
                let kind = node.kind()?;
                if !matches!(kind, UiKind::Menu | UiKind::Popup) {
                    return None;
                }
                area = Some((node.unique_id, kind));
                break;
            }
        }
        let (area, kind) = area?;
        if count == 0 {
            return None;
        }
        let links = ui
            .ctx()
            .viewport(|viewport| parent_links(&viewport.this_pass.widgets, registered, area, layer))?;
        for frame in frames.iter().flatten() {
            if !links
                .iter()
                .flatten()
                .any(|(id, parent)| *id == frame.content && *parent == frame.parent)
            {
                return None;
            }
        }
        Some(Self { area, kind, frames, links })
    }
}
fn parent_links(widgets: &WidgetRects, registered: WidgetRect, area: Id, layer: LayerId) -> Option<[Option<(Id, Id)>; MAX_PARENTS]> {
    let mut links = [None; MAX_PARENTS];
    let mut id = registered.parent_id;
    for index in 0..MAX_PARENTS {
        if id == registered.id || links[..index].iter().flatten().any(|(seen, _)| *seen == id) {
            return None;
        }
        let parent = widgets.get(id)?;
        if parent.layer_id != layer {
            return None;
        }
        links[index] = Some((id, parent.parent_id));
        if id == area {
            return Some(links);
        }
        id = parent.parent_id;
    }
    None
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Witness {
    parents: [Option<WidgetRect>; MAX_PARENTS],
    allocations: [Option<WidgetRect>; MAX_FRAMES],
}
impl Witness {
    pub(super) fn capture(widgets: &WidgetRects, control: &Control) -> Option<Self> {
        let origin = control.structure_origin;
        if parent_links(widgets, control.registered, origin.area, control.layer)? != origin.links {
            return None;
        }
        let mut parents = [None; MAX_PARENTS];
        for (index, (id, expected_parent)) in origin.links.iter().flatten().enumerate() {
            let parent = *widgets.get(*id)?;
            if parent.parent_id != *expected_parent
                || parent.layer_id != control.layer
                || !parent.enabled
                || !parent.visible
                || !finite_positive(parent.rect)
                || !finite_positive(parent.interact_rect)
            {
                return None;
            }
            parents[index] = Some(parent);
        }
        let local = control
            .registered
            .rect
            .intersect(control.registered.interact_rect)
            .intersect(control.paint_clip);
        if !finite_positive(local) {
            return None;
        }
        let mut allocations = [None; MAX_FRAMES];
        for (index, frame) in origin.frames.iter().flatten().enumerate() {
            let content = widgets.get(frame.content)?;
            let parent = widgets.get(frame.parent)?;
            let outer = frame.frame.outer_rect(content.rect).round_ui();
            let interact = outer.intersect(parent.interact_rect);
            if !finite_positive(outer) || !finite_positive(interact) || !parent.rect.contains_rect(outer) || !outer.contains_rect(local) {
                return None;
            }
            let (layer, control_index) = widgets.order(control.widget)?;
            if layer != control.layer {
                return None;
            }
            let mut matches = widgets.get_layer(control.layer).filter(|widget| {
                widget.parent_id == frame.parent
                    && widget.layer_id == control.layer
                    && widget.id != frame.content
                    && widget.enabled
                    && widget.visible
                    && widget.sense == egui::Sense::hover()
                    && widget.rect == outer
                    && widget.interact_rect == interact
                    && widgets
                        .order(widget.id)
                        .is_some_and(|(layer, index)| layer == control.layer && index > control_index)
            });
            let allocation = *matches.next()?;
            if matches.next().is_some()
                || allocations[..index]
                    .iter()
                    .flatten()
                    .any(|previous: &WidgetRect| previous.id == allocation.id)
            {
                return None;
            }
            allocations[index] = Some(allocation);
        }
        Some(Self { parents, allocations })
    }
    #[cfg(test)]
    pub(super) fn allocation_count(self) -> usize {
        self.allocations.iter().flatten().count()
    }
    pub(super) fn contains(self, id: Id) -> bool {
        self.allocations.iter().flatten().any(|allocation| allocation.id == id)
    }
}
