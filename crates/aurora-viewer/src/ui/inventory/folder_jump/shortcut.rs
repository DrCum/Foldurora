//! One committed Inventory owner for the local Ctrl/Cmd+J chord. The native
//! adapter consumes an admitted press before egui and viewer movement/actions.

use crate::keybinds::{self, Input, KeyBindings, Mods};
use egui::{Id, LayerId};
use uuid::Uuid;

#[derive(Clone)]
struct Claim {
    owner: Uuid,
    layer: LayerId,
    frame: u64,
}

fn key() -> Id {
    Id::unique("inventory_folder_jump_owners")
}

pub(super) fn claim(ui: &egui::Ui, owner: Uuid) {
    if !ui.is_enabled() || ui.ctx().will_discard() {
        return;
    }
    let frame = ui.ctx().cumulative_frame_nr();
    let layer = ui.layer_id();
    ui.ctx().data_mut(|data| {
        let claims = data.get_temp_mut_or_default::<Vec<Claim>>(key());
        claims.retain(|claim| claim.owner != owner && claim.frame.saturating_add(1) >= frame);
        if claims.len() < 2 {
            claims.push(Claim { owner, layer, frame });
        }
    });
}

pub(super) fn owner(ctx: &egui::Context) -> Option<Uuid> {
    if egui::Popup::is_any_open(ctx) {
        return None;
    }
    let frame = ctx.cumulative_frame_nr();
    let claims = ctx.data(|data| data.get_temp::<Vec<Claim>>(key())).unwrap_or_default();
    let focused = ctx.memory(|memory| memory.focused());
    let layer = match focused {
        Some(widget) => ctx.read_response(widget).map(|response| response.layer_id),
        None => ctx.pointer_latest_pos().and_then(|position| ctx.layer_id_at(position)),
    }?;
    if !ctx.memory(|memory| memory.is_above_modal_layer(layer)) {
        return None;
    }
    claims
        .iter()
        .find(|claim| claim.frame.saturating_add(1) >= frame && claim.layer == layer)
        .map(|claim| claim.owner)
}

pub(super) fn available(bindings: &KeyBindings, input: &Input, mods: Mods) -> bool {
    mods.ctrl
        && !mods.shift
        && !mods.alt
        && bindings.triggered(input, mods).is_none()
        && !bindings.pass_wall_triggered(input, mods)
        && !bindings.map.iter().any(|(action, slots)| {
            keybinds::is_held(*action)
                && slots.iter().flatten().any(|binding| {
                    &binding.input == input && (!binding.ctrl || mods.ctrl) && (!binding.shift || mods.shift) && (!binding.alt || mods.alt)
                })
        })
}
