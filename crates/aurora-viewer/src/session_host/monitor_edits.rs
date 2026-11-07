//! Delivered old-context editing only. Every pass names the exact canonical
//! surface/source; no selection, platform output, Send or Read is performed.
use super::chat_ui::{self, InputMode};
use super::chat_window::{edit_input, edit_prefix};
use super::monitor_model::{Binding, View};
use super::shared_chat::SharedChat;
use crate::theme::Palette;

pub(super) fn close(context: &egui::Context, raw: egui::RawInput, view: View, binding: Binding, palette: &Palette, chat: &mut SharedChat) {
    let mut raw = raw;
    let composer = chat.composer_rect_for(view.surface(), context);
    if let Some(prefix) = edit_prefix(&mut raw, composer, egui::Popup::is_any_open(context)) {
        let mut output = context.run_ui(prefix, |ui| {
            let _ = chat.show_bound_for(view.surface(), binding.source.chat(), ui, palette, InputMode::Closing);
        });
        output.textures_delta.clear();
    }
    let mut output = context.run_ui(edit_input(raw), |ui| {
        let _ = chat.show_bound_for(view.surface(), binding.source.chat(), ui, palette, InputMode::Closing);
    });
    output.textures_delta.clear();
    chat.surface_closed(view.surface());
    chat_ui::cancel_input(context);
}
