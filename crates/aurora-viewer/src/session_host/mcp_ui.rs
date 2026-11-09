//! Trusted native host permission and approval controls; never served over MCP.
use super::mcp::{UiCommand, UiState};
use crate::theme::Palette;
use crate::ui::widgets::{self, ActionKind};
use aurora_mcp::authority::{ApprovalDecision, AuthorityLevel, PermissionLevels};
use aurora_mcp::wire::{HostAction, PermissionClass};

fn class_label(class: PermissionClass) -> &'static str {
    match class {
        PermissionClass::Metadata => aurora_i18n::tr!("host-mcp-metadata"),
        PermissionClass::Activate => aurora_i18n::tr!("host-mcp-activate"),
        PermissionClass::Background => aurora_i18n::tr!("host-mcp-background"),
        PermissionClass::Monitors => aurora_i18n::tr!("host-mcp-monitors"),
        PermissionClass::OpenLogin => aurora_i18n::tr!("host-mcp-open-login"),
        PermissionClass::Restart => aurora_i18n::tr!("host-mcp-restart"),
        PermissionClass::Logout => aurora_i18n::tr!("host-mcp-logout"),
        PermissionClass::Detach => aurora_i18n::tr!("host-mcp-detach"),
    }
}

fn levels(ui: &mut egui::Ui, p: &Palette, permissions: &mut PermissionLevels) -> bool {
    let mut changed = false;
    egui::Grid::new("host_mcp_permissions").num_columns(1).show(ui, |ui| {
        for class in PermissionClass::ALL {
            ui.label(class_label(class));
            ui.end_row();
            let old = permissions.get(class);
            let mut selected = old;
            ui.horizontal_wrapped(|ui| {
                for (value, key) in [
                    (AuthorityLevel::Never, "host-mcp-never"),
                    (AuthorityLevel::Ask, "host-mcp-ask"),
                    (AuthorityLevel::Allow, "host-mcp-allow"),
                ] {
                    if widgets::radio(
                        ui,
                        p,
                        selected == value,
                        match key {
                            "host-mcp-never" => aurora_i18n::tr!("host-mcp-never"),
                            "host-mcp-ask" => aurora_i18n::tr!("host-mcp-ask"),
                            _ => aurora_i18n::tr!("host-mcp-allow"),
                        },
                    )
                    .clicked()
                    {
                        selected = value;
                    }
                }
            });
            if selected != old {
                permissions.set(class, selected);
                changed = true;
            }
            ui.end_row();
        }
    });
    changed
}

pub(super) fn show(ui: &mut egui::Ui, p: &Palette, state: &UiState) -> Option<UiCommand> {
    let mut command = None;
    widgets::organization_header(ui, p, aurora_i18n::tr!("host-mcp-title"));
    ui.label(aurora_i18n::tr!("host-mcp-help"));
    if !state.pending.is_empty() {
        ui.separator();
        widgets::organization_header(ui, p, aurora_i18n::tr!("host-mcp-pending"));
    }
    for approval in state.pending.iter().take(16) {
        ui.push_id(&approval.request_id, |ui| {
            widgets::surface(ui, p, |ui| {
                let class = approval.action.as_ref().map_or(PermissionClass::Metadata, |action| action.class());
                ui.label(class_label(class));
                if let Some(action) = &approval.action {
                    let detail = match action {
                        HostAction::Activate => aurora_i18n::tr!("host-mcp-activate"),
                        HostAction::BackgroundMode { .. } => aurora_i18n::tr!("host-mcp-background"),
                        HostAction::OpenLogin => aurora_i18n::tr!("host-mcp-open-login"),
                        HostAction::Restart => aurora_i18n::tr!("host-mcp-restart"),
                        HostAction::Logout => aurora_i18n::tr!("host-mcp-logout"),
                        HostAction::Detach => aurora_i18n::tr!("host-mcp-detach"),
                        HostAction::MonitorOpen => aurora_i18n::tr!("host-mcp-monitor-open"),
                        HostAction::MonitorClose { .. } => aurora_i18n::tr!("host-mcp-monitor-close"),
                        HostAction::MonitorsOpenAll => aurora_i18n::tr!("host-mcp-monitors-open-all"),
                        HostAction::MonitorsArrange => aurora_i18n::tr!("host-mcp-monitors-arrange"),
                        HostAction::MonitorCycleConfigure { .. } => aurora_i18n::tr!("host-mcp-cycle-configure"),
                        HostAction::MonitorCyclePause => aurora_i18n::tr!("host-mcp-cycle-pause"),
                        HostAction::MonitorCycleNext => aurora_i18n::tr!("host-mcp-cycle-next"),
                    };
                    ui.label(detail);
                    match action {
                        HostAction::MonitorClose { monitor } => {
                            ui.label(aurora_i18n::tr!("host-mcp-monitor-number", number = i64::from(*monitor)));
                        }
                        HostAction::MonitorCycleConfigure { interval_seconds, playing } => {
                            ui.label(aurora_i18n::tr!("host-mcp-cycle-interval", seconds = i64::from(*interval_seconds)));
                            ui.label(if *playing {
                                aurora_i18n::tr!("host-mcp-cycle-playing")
                            } else {
                                aurora_i18n::tr!("host-mcp-cycle-paused")
                            });
                        }
                        HostAction::BackgroundMode { mode } => {
                            ui.label(match mode {
                                aurora_mcp::wire::BackgroundMode::Warm => aurora_i18n::tr!("host-mcp-warm"),
                                aurora_mcp::wire::BackgroundMode::Economy => aurora_i18n::tr!("host-mcp-economy"),
                            });
                        }
                        _ => {}
                    }
                }
                for ticket in &approval.profiles {
                    if let Some(profile) = state.profiles.iter().find(|profile| profile.ticket == *ticket) {
                        ui.label(format!("{} · {}", ticket.profile.number(), profile.label.as_str()));
                    }
                }
                ui.label(aurora_i18n::tr!("host-mcp-approval-help"));
                ui.horizontal(|ui| {
                    // Deny comes first and no default/Enter approval is bound.
                    let deny = widgets::action_button(
                        ui,
                        p,
                        "x",
                        aurora_i18n::tr!("host-mcp-deny"),
                        ActionKind::Flat,
                        egui::vec2(100.0, 28.0),
                    );
                    #[cfg(test)]
                    ui.ctx()
                        .data_mut(|data| data.insert_temp(egui::Id::unique(("host_mcp_deny", &approval.request_id)), deny.rect));
                    if deny.clicked() {
                        command = Some(UiCommand::Resolve(approval.request_id.clone(), ApprovalDecision::Deny));
                    }
                    let allow = widgets::action_button(
                        ui,
                        p,
                        "check",
                        aurora_i18n::tr!("host-mcp-approve"),
                        ActionKind::Flat,
                        egui::vec2(100.0, 28.0),
                    );
                    #[cfg(test)]
                    ui.ctx()
                        .data_mut(|data| data.insert_temp(egui::Id::unique(("host_mcp_allow", &approval.request_id)), allow.rect));
                    if allow.clicked() {
                        command = Some(UiCommand::Resolve(approval.request_id.clone(), ApprovalDecision::AllowOnce));
                    }
                });
            });
        });
    }

    if let Some(installation) = state.installation {
        ui.label(aurora_i18n::tr!("host-mcp-client-config"));
        ui.label(installation.to_string());
        if widgets::flat_button(ui, p, aurora_i18n::tr!("host-mcp-copy-args")).clicked() {
            ui.ctx().copy_text(format!("--installation {installation}"));
        }
    }
    let mut preferences = state.preferences;
    if widgets::switch_labeled(ui, p, &mut preferences.enabled, aurora_i18n::tr!("host-mcp-enabled")).changed() {
        command = Some(UiCommand::Configure(preferences));
    }
    if state.error {
        ui.colored_label(p.danger, aurora_i18n::tr!("host-mcp-unavailable"));
    }
    #[cfg(test)]
    ui.ctx()
        .data_mut(|data| data.insert_temp(egui::Id::unique("host_mcp_policy_start"), ui.cursor().min.y));
    ui.push_id("host_policy", |ui| {
        if levels(ui, p, &mut preferences.permissions) {
            command = Some(UiCommand::Configure(preferences));
        }
    });
    ui.add_space(8.0);
    ui.label(aurora_i18n::tr!("host-mcp-profile-help"));
    for profile in &state.profiles {
        ui.push_id((profile.ticket.profile.number(), profile.ticket.revision), |ui| {
            ui.separator();
            ui.label(format!("{} · {}", profile.ticket.profile.number(), profile.label.as_str()));
            let mut opted_in = profile.permissions.is_some();
            if widgets::switch_labeled(ui, p, &mut opted_in, aurora_i18n::tr!("host-mcp-profile-enabled")).changed() {
                command = Some(if opted_in {
                    let mut permissions = PermissionLevels::default();
                    permissions.set(PermissionClass::Metadata, AuthorityLevel::Ask);
                    UiCommand::Grant(profile.ticket, permissions)
                } else {
                    UiCommand::Revoke(profile.ticket.profile)
                });
            }
            if let Some(mut permissions) = profile.permissions
                && levels(ui, p, &mut permissions)
            {
                command = Some(UiCommand::Grant(profile.ticket, permissions));
            }
        });
    }
    command
}
