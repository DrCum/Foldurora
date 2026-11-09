//! Trusted native controls. Remote callers cannot reach enable/grant/resolve.
#[cfg(test)]
mod visual;
use super::*;
use crate::ui::widgets::{self, ActionKind, Floater};
use aurora_mcp::assistant::permissions::{DEFINITIONS, Preset};

pub(super) fn show(
    ctx: &egui::Context,
    p: &crate::theme::Palette,
    state: &mut Assistant,
    config: &mut aurora_mcp::assistant::permissions::Configuration,
    account: &str,
) -> bool {
    if !state.open {
        return false;
    }
    let mut open = true;
    let mut changed = false;
    let snapshot = state.runtime.snapshot();
    let mut floater = Floater::new(
        "local_assistant",
        aurora_i18n::tr!("assistant-title"),
        egui::pos2(64., 84.),
        egui::vec2(560., 660.),
    );
    floater.min_size = egui::vec2(400., 380.);
    floater.show(ctx, p, &mut open, |ui| {
        ui.label(egui::RichText::new(account).strong());
        ui.label(aurora_i18n::tr!("assistant-help"));
        if let Some(installation) = state.io.snapshot().installation {
            let command = format!("foldurora-mcp --installation {installation}");
            ui.add(egui::Label::new(egui::RichText::new(&command).monospace()).selectable(true));
            if ui.button(aurora_i18n::tr!("assistant-copy-command")).clicked() {
                ctx.copy_text(command);
            }
        }
        let mut enabled = snapshot.enabled || state.wanted.is_some();
        if ui
            .add_enabled_ui(state.binding.is_some(), |ui| {
                widgets::checkbox(ui, p, &mut enabled, aurora_i18n::tr!("assistant-enable"))
            })
            .inner
            .changed()
        {
            if enabled {
                if let Some(binding) = state.binding
                    && let Some(token) = state.io.start(state.runtime.clone())
                {
                    state.wanted = Some((token, binding));
                }
            } else {
                state.disable();
            }
        }
        if state.io.snapshot().failed {
            ui.colored_label(p.warn, aurora_i18n::tr!("assistant-unavailable"));
        }
        if state.binding.is_none() {
            ui.label(aurora_i18n::tr!("assistant-no-session"));
        }
        let mut paused = snapshot.paused;
        if ui
            .add_enabled_ui(enabled, |ui| {
                widgets::checkbox(ui, p, &mut paused, aurora_i18n::tr!("assistant-pause"))
            })
            .inner
            .changed()
        {
            state.runtime.pause(paused);
        }
        ui.horizontal_wrapped(|ui| {
            if ui.button(aurora_i18n::tr!("assistant-readonly")).clicked() {
                config.permissions.preset(Preset::ReadOnly);
                changed = true;
            }
            if ui.button(aurora_i18n::tr!("assistant-ask-changes")).clicked() {
                config.permissions.preset(Preset::AskBeforeChanges);
                changed = true;
            }
        });
        ui.separator();
        ui.label(egui::RichText::new(aurora_i18n::tr!("assistant-requests")).strong());
        egui::ScrollArea::vertical()
            .id_salt("assistant-approvals")
            .max_height(320.)
            .show(ui, |ui| {
                for approval in snapshot.approvals.iter().take(16) {
                    ui.push_id(&approval.capture.request_id, |ui| {
                        ui.label(approval.capture.tool.definition().name);
                        // Every frozen byte remains reviewable, including the
                        // tail of long chat/content. This is display-only text.
                        let details = serde_json::to_string_pretty(&approval.capture.arguments).unwrap_or_default();
                        let arguments = egui::ScrollArea::both()
                            .id_salt("frozen-arguments")
                            .max_height(140.)
                            .show(ui, |ui| {
                                ui.add(egui::Label::new(egui::RichText::new(details).monospace()).selectable(true));
                            });
                        #[cfg(test)]
                        ui.ctx().data_mut(|data| {
                            data.insert_temp(
                                egui::Id::unique(("assistant_arguments_scroll", &approval.capture.request_id)),
                                arguments.id,
                            );
                            data.insert_temp(
                                egui::Id::unique(("assistant_arguments_rect", &approval.capture.request_id)),
                                arguments.inner_rect.intersect(ui.clip_rect()),
                            );
                            data.insert_temp(
                                egui::Id::unique(("assistant_arguments_content", &approval.capture.request_id)),
                                arguments.content_size,
                            );
                            data.insert_temp(
                                egui::Id::unique(("assistant_arguments_viewport", &approval.capture.request_id)),
                                arguments.inner_rect.size(),
                            );
                        });
                        #[cfg(not(test))]
                        let _ = arguments;
                        if let Some(evidence) = state.evidence.get(&approval.capture.request_id) {
                            let evidence_header =
                                egui::CollapsingHeader::new(aurora_i18n::tr!("assistant-native-evidence")).show(ui, |ui| {
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(serde_json::to_string_pretty(evidence).unwrap_or_default()).monospace(),
                                        )
                                        .selectable(true),
                                    );
                                });
                            #[cfg(test)]
                            ui.ctx().data_mut(|data| {
                                data.insert_temp(
                                    egui::Id::unique(("assistant_evidence", &approval.capture.request_id)),
                                    evidence_header.header_response.rect,
                                )
                            });
                            #[cfg(not(test))]
                            let _ = evidence_header;
                        }
                        ui.horizontal_wrapped(|ui| {
                            // Fixture-only visibility evidence; do not infer the
                            // painted clip from a button's full layout bounds.
                            #[cfg(test)]
                            ui.ctx().data_mut(|data| {
                                data.insert_temp(
                                    egui::Id::unique(("assistant_action_clip", &approval.capture.request_id)),
                                    ui.clip_rect(),
                                )
                            });
                            let deny = widgets::action_button(
                                ui,
                                p,
                                "x",
                                aurora_i18n::tr!("assistant-deny"),
                                ActionKind::Flat,
                                egui::vec2(120., 28.),
                            );
                            #[cfg(test)]
                            ui.ctx().data_mut(|data| {
                                data.insert_temp(egui::Id::unique(("assistant_deny", &approval.capture.request_id)), deny.rect)
                            });
                            if deny.clicked() {
                                let _ = state.runtime.resolve(&approval.capture.request_id, false);
                            }
                            let allow = widgets::action_button(
                                ui,
                                p,
                                "check",
                                aurora_i18n::tr!("assistant-allow-once"),
                                ActionKind::Flat,
                                egui::vec2(150., 28.),
                            );
                            #[cfg(test)]
                            ui.ctx().data_mut(|data| {
                                data.insert_temp(egui::Id::unique(("assistant_allow", &approval.capture.request_id)), allow.rect)
                            });
                            if allow.clicked() {
                                let _ = state.runtime.resolve(&approval.capture.request_id, true);
                            }
                        });
                    });
                }
            });
        ui.label(aurora_i18n::tr!("assistant-private-read-help"));
        egui::ScrollArea::vertical()
            .id_salt("assistant-permissions")
            .max_height(if snapshot.approvals.is_empty() { 300. } else { 120. })
            .show(ui, |ui| {
                for def in DEFINITIONS {
                    ui.push_id(def.id, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(permission_name(def.class));
                            let mut level = config.permissions.level(def.class);
                            egui::ComboBox::from_id_salt("level")
                                .selected_text(level_name(level))
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut level, Level::Never, level_name(Level::Never));
                                    if def.ask {
                                        ui.selectable_value(&mut level, Level::Ask, level_name(Level::Ask));
                                    }
                                    ui.selectable_value(&mut level, Level::Allow, level_name(Level::Allow));
                                });
                            if level != config.permissions.level(def.class) {
                                let _ = config.permissions.set(def.class, level);
                                changed = true;
                            }
                        })
                    });
                }
            });
    });
    state.open = open;
    changed
}
fn level_name(level: Level) -> &'static str {
    match level {
        Level::Never => aurora_i18n::tr!("assistant-never"),
        Level::Ask => aurora_i18n::tr!("assistant-ask"),
        Level::Allow => aurora_i18n::tr!("assistant-allow"),
    }
}
fn permission_name(class: Class) -> &'static str {
    match class {
        Class::Read => aurora_i18n::tr!("assistant-perm-read"),
        Class::Camera => aurora_i18n::tr!("assistant-perm-camera"),
        Class::Create => aurora_i18n::tr!("assistant-perm-create"),
        Class::Edit => aurora_i18n::tr!("assistant-perm-edit"),
        Class::Move => aurora_i18n::tr!("assistant-perm-move"),
        Class::Trash => aurora_i18n::tr!("assistant-perm-trash"),
        Class::NoCopy => aurora_i18n::tr!("assistant-perm-nocopy"),
        Class::Wear => aurora_i18n::tr!("assistant-perm-wear"),
        Class::Workspace => aurora_i18n::tr!("assistant-perm-workspace"),
        Class::Links => aurora_i18n::tr!("assistant-perm-links"),
        Class::ChatRead => aurora_i18n::tr!("assistant-perm-chat_read"),
        Class::ChatState => aurora_i18n::tr!("assistant-perm-chat_state"),
        Class::ChatSendIM => aurora_i18n::tr!("assistant-perm-chat_send_im"),
        Class::ChatSendGroup => aurora_i18n::tr!("assistant-perm-chat_send_group"),
        Class::ChatSendConference => aurora_i18n::tr!("assistant-perm-chat_send_conference"),
        Class::ChatSendNearby => aurora_i18n::tr!("assistant-perm-chat_send_nearby"),
        Class::FriendsRead => aurora_i18n::tr!("assistant-perm-friends_read"),
        Class::ResidentSearch => aurora_i18n::tr!("assistant-perm-resident_search"),
        Class::ProfileRead => aurora_i18n::tr!("assistant-perm-profile_read"),
        Class::NotificationsRead => aurora_i18n::tr!("assistant-perm-notifications_read"),
        Class::NotificationReview => aurora_i18n::tr!("assistant-perm-notification_review"),
        Class::WorldRead => aurora_i18n::tr!("assistant-perm-world_read"),
        Class::GestureRead => aurora_i18n::tr!("assistant-perm-gesture_read"),
        Class::GesturePlay => aurora_i18n::tr!("assistant-perm-gesture_play"),
        Class::PoseRead => aurora_i18n::tr!("assistant-perm-pose_read"),
        Class::PoseEdit => aurora_i18n::tr!("assistant-perm-pose_edit"),
        Class::PoseSave => aurora_i18n::tr!("assistant-perm-pose_save"),
        Class::PoseShare => aurora_i18n::tr!("assistant-perm-pose_share"),
        Class::ContentEdit => aurora_i18n::tr!("assistant-perm-content_edit"),
        Class::PreferencesRead => aurora_i18n::tr!("assistant-perm-preferences_read"),
        Class::PreferencesWrite => aurora_i18n::tr!("assistant-perm-preferences_write"),
        Class::BuildEdit => aurora_i18n::tr!("assistant-perm-build_edit"),
        Class::BuildRez => aurora_i18n::tr!("assistant-perm-build_rez"),
        Class::InventoryPermissions => aurora_i18n::tr!("assistant-perm-inventory_permissions"),
        Class::NavigationMove => aurora_i18n::tr!("assistant-perm-navigation_move"),
        Class::NavigationTeleport => aurora_i18n::tr!("assistant-perm-navigation_teleport"),
        Class::Upload => aurora_i18n::tr!("assistant-perm-upload"),
        Class::TaskInventoryRead => aurora_i18n::tr!("assistant-perm-task_inventory_read"),
        Class::ScriptDebugRead => aurora_i18n::tr!("assistant-perm-script_debug_read"),
        Class::TaskInventoryWrite => aurora_i18n::tr!("assistant-perm-task_inventory_write"),
        Class::TaskInventoryRemove => aurora_i18n::tr!("assistant-perm-task_inventory_remove"),
        Class::ScriptRun => aurora_i18n::tr!("assistant-perm-script_run"),
        Class::ObjectTake => aurora_i18n::tr!("assistant-perm-object_take"),
        Class::ObjectInteract => aurora_i18n::tr!("assistant-perm-object_interact"),
        Class::ScriptTest => aurora_i18n::tr!("assistant-perm-script_test"),
    }
}
