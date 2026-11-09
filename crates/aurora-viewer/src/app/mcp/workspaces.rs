// SPDX-License-Identifier: LGPL-2.1-or-later
//! Native named arrangements, after Folderstorm fsworkspacelistener.cpp
//! at abbe3cb0ca66ee2484f277219f6a27706fab7fc6. Preview is a read-only
//! diagram; mutation uses the existing staged native Controller.
use super::*;
use crate::workspace::{
    context::Hud,
    controller::Ticket,
    model::{Kind, Layout},
};
use std::time::{Duration, Instant};
#[derive(Default)]
pub(super) struct State {
    operations: BTreeMap<Uuid, Work>,
    latest: Option<Uuid>,
}
struct Work {
    capture: Capture,
    ticket: Ticket,
    started: Instant,
    result: Value,
    finished: bool,
    applied: usize,
    adjusted: usize,
    huds: Vec<Hud>,
    submitted: BTreeSet<Uuid>,
    proofs: BTreeMap<Uuid, Value>,
}
pub(super) fn supports(t: Tool) -> bool {
    matches!(
        t,
        Tool::WorkspaceList | Tool::WorkspaceStatus | Tool::WorkspacePreview | Tool::WorkspaceApply | Tool::WorkspacePrevious
    )
}
fn keys(c: &Capture, keys: &[&str]) -> Result<(), &'static str> {
    if c.arguments
        .keys()
        .any(|k| !keys.contains(&k.as_str()) && !matches!(k.as_str(), "viewer_session" | "viewer_pid" | "request_id"))
    {
        Err("invalid_args")
    } else {
        Ok(())
    }
}
fn target(app: &App, c: &Capture) -> Result<(Uuid, Kind), &'static str> {
    let id = uuid(&c.arguments, "id")?;
    let kind = match string(&c.arguments, "kind", 9)? {
        "workspace" => Kind::Workspace,
        "layout" => Kind::Layout,
        _ => return Err("invalid_workspace_kind"),
    };
    let r = app.workspaces.record(id).filter(|r| r.kind == kind).ok_or("workspace_not_found")?;
    r.layout.validate().map_err(|_| "invalid_native_workspace")?;
    Ok((id, kind))
}
pub(super) fn preparation(app: &mut App, assistant: &Assistant, c: &Capture) -> Result<bool, &'static str> {
    assistant.runtime.continue_preparation(c, Class::Read, || ())?;
    app.workspace_ensure_catalogue()
}
pub(super) fn prepare(app: &App, _state: &State, c: &Capture) -> Result<Vec<Class>, &'static str> {
    if app.workspace_source() != app.workspaces.source || app.workspaces.source.is_none() || app.workspaces.catalogue.is_none() {
        return Err("native_workspace_catalogue_unavailable");
    }
    match c.tool {
        Tool::WorkspaceList | Tool::WorkspaceStatus => keys(c, &[])?,
        Tool::WorkspacePreview | Tool::WorkspaceApply => {
            keys(c, &["kind", "id"])?;
            target(app, c)?;
        }
        Tool::WorkspacePrevious => {
            keys(c, &[])?;
            if app.workspaces.previous.is_none() {
                return Err("workspace_previous_unavailable");
            }
        }
        _ => return Err("unsupported_capability"),
    }
    if !c.tool.definition().read_only
        && (!app.workspace_assistant_allowed()
            || app.workspaces.pending.is_some()
            || app.workspaces.preview.is_some()
            || app.workspaces.assistant.is_some())
    {
        return Err("native_workspace_busy_or_unavailable");
    }
    let mut classes = vec![Class::Read];
    if !c.tool.definition().read_only {
        let definition = layout(app, c)?;
        if !app.workspace_context_allowed(&definition.context) {
            return Err("native_workspace_context_restricted");
        }
        if definition.context.camera.is_some() {
            classes.push(Class::Camera);
        }
        if !definition.context.huds.is_empty() {
            classes.push(Class::Wear);
            for hud in &definition.context.huds {
                hud_proof(app, hud)?;
            }
        }
    }
    Ok(classes)
}
fn hud_proof(app: &App, hud: &Hud) -> Result<Value, &'static str> {
    use crate::world::inventory::actions as rules;
    let inv = &app.world.inventory;
    let item = inv
        .items
        .get(&hud.item)
        .filter(|i| {
            i.owner == app.world.agent_id
                && !i.group_owned
                && i.asset_type == 6
                && rules::under(inv, i.id, inv.root)
                && !rules::library(inv, i.id)
                && !rules::in_type(inv, i.id, 14)
        })
        .ok_or("native_workspace_hud_unavailable")?;
    Ok(
        json!({"item":item.id,"owner":item.owner,"parent":item.parent,"asset":item.asset_id,"permissions":item.owner_mask,"point":hud.point}),
    )
}
fn layout(app: &App, c: &Capture) -> Result<Layout, &'static str> {
    if c.tool == Tool::WorkspacePrevious {
        return app
            .workspaces
            .previous
            .as_ref()
            .map(|(l, _)| l.clone())
            .ok_or("workspace_previous_unavailable");
    }
    Ok(app
        .workspaces
        .record(target(app, c)?.0)
        .ok_or("workspace_not_found")?
        .layout
        .clone())
}
pub(super) fn evidence(app: &App, state: &State, c: &Capture) -> Result<Value, &'static str> {
    if c.tool.definition().read_only {
        return Ok(Value::Null);
    }
    prepare(app, state, c)?;
    let definition = layout(app, c)?;
    let huds = definition
        .context
        .huds
        .iter()
        .map(|h| Ok(json!({"source":hud_proof(app,h)?,"worn":app.world.worn_attachment_points().get(&h.item)==Some(&h.point)})))
        .collect::<Result<Vec<_>, &'static str>>()?;
    Ok(
        json!({"instance":app.workspaces.instance,"binding":app.workspaces.binding,"revision":app.workspaces.revision,"definition":definition,"huds":huds,"previous_active":app.workspaces.previous.as_ref().and_then(|(_,id)|*id)}),
    )
}
fn status(app: &App, state: &State, c: &Capture) -> Value {
    let active = app.workspaces.active.and_then(|id| app.workspaces.record(id));
    let modified = active.is_some_and(|r| {
        (r.kind == Kind::Workspace && r.layout.geometry != crate::ui::layout::workspace_snapshot(&app.egui_ctx))
            || r.layout.viewport != (&app.settings.world_viewport).into()
            || r.layout.bars
                != crate::workspace::model::Bars {
                    classic: app.settings.classic_bars,
                    stretch: app.settings.place_bar_stretch,
                    rights_text: app.settings.place_bar_rights_text,
                }
            || app.workspace_capture_like(&r.layout.context).context.graphics != r.layout.context.graphics
            || app.workspace_capture_like(&r.layout.context).context.camera != r.layout.context.camera
            || r.layout
                .context
                .huds
                .iter()
                .any(|h| app.world.worn_attachment_points().get(&h.item) != Some(&h.point))
    });
    let report = state
        .latest
        .and_then(|id| state.operations.get(&id))
        .filter(|w| same(&w.capture, c))
        .map(|w| w.result.clone());
    json!({"workspace_tools":1,"available":app.workspaces.catalogue.is_some(),"can_switch":app.workspace_assistant_allowed() && app.workspaces.preview.is_none() && app.workspaces.pending.is_none() && app.workspaces.assistant.is_none(),"active":active.map(|r|r.id),"active_kind":active.map(|r|r.kind),"modified":modified,"has_previous":app.workspaces.previous.is_some(),"revision":app.workspaces.revision,"pending":app.workspaces.placement.is_some() || app.workspaces.assistant.is_some() || app.workspaces.native_huds.is_some(),"report":report,"native_hud_report":app.workspaces.native_hud_report})
}
fn same(a: &Capture, b: &Capture) -> bool {
    a.client == b.client && a.endpoint == b.endpoint && a.binding == b.binding && a.policy_revision == b.policy_revision
}
pub(super) fn execute(app: &mut App, state: &mut State, c: &Capture) -> Result<Value, &'static str> {
    prepare(app, state, c)?;
    if c.tool == Tool::WorkspaceList {
        let mut value = status(app, state, c);
        value["entries"] = json!(
            app.workspaces
                .catalogue
                .as_ref()
                .ok_or("native_workspace_catalogue_unavailable")?
                .records
                .iter()
                .map(|r| json!({"kind":r.kind,"id":r.id,"name":r.name,"favorite":r.favorite,"components":r.layout.components()}))
                .collect::<Vec<_>>()
        );
        return Ok(value);
    }
    if c.tool == Tool::WorkspaceStatus {
        return Ok(status(app, state, c));
    }
    let definition = layout(app, c)?;
    if c.tool == Tool::WorkspacePreview {
        let frame = app.egui_ctx.content_rect();
        let diagram = crate::ui::layout::workspace_diagram(&app.egui_ctx, &definition.geometry);
        return Ok(
            json!({"kind":c.arguments["kind"],"id":c.arguments["id"],"components":definition.components(),"frame":{"width":frame.width(),"height":frame.height()},"windows":diagram.iter().enumerate().filter_map(|(i,r)|r.map(|r|json!({"name":crate::workspace::model::Role::ALL[i].id(),"left":r.left(),"bottom":frame.bottom()-r.bottom(),"width":r.width(),"height":r.height()}))).collect::<Vec<_>>(),"chrome":{"viewport":definition.viewport,"bars":definition.bars},"hud_count":definition.context.huds.len(),"huds":definition.context.huds.iter().map(|h|json!({"name":h.name,"point":h.point,"worn":app.world.worn_attachment_points().get(&h.item)==Some(&h.point)})).collect::<Vec<_>>(),"graphics_preset":definition.context.graphics.as_ref().and_then(|g|g.active.as_deref()).unwrap_or(""),"camera_preset":"","graphics_context":definition.context.graphics.is_some(),"camera_context":definition.context.camera.is_some(),"unavailable_groups":["background"],"adjusted":crate::ui::layout::workspace_adjusted(&app.egui_ctx,&definition.geometry),"skipped":0,"missing_folders":0,"pending":false}),
        );
    }
    if state.operations.len() >= 32 {
        let Some(old) = state.operations.iter().find(|(_, w)| w.finished).map(|(id, _)| *id) else {
            return Err("workspace_operation_capacity");
        };
        state.operations.remove(&old);
    }
    let operation = Uuid::new_v4();
    let applied = 1
        + definition.geometry.iter().filter(|g| g.is_some()).count()
        + usize::from(definition.context.graphics.is_some())
        + usize::from(definition.context.camera.is_some());
    let adjusted = crate::ui::layout::workspace_adjusted(&app.egui_ctx, &definition.geometry);
    let proofs = definition
        .context
        .huds
        .iter()
        .map(|h| Ok((h.item, hud_proof(app, h)?)))
        .collect::<Result<BTreeMap<_, _>, &'static str>>()?;
    let mut ticket = app.workspaces.ticket().ok_or("native_workspace_source_unavailable")?;
    let selected = if c.tool == Tool::WorkspacePrevious {
        None
    } else {
        Some(target(app, c)?.0)
    };
    ticket.record = selected;
    app.workspace_assistant_begin(&ticket, selected, operation)?;
    let result = json!({"status":"submitted","accepted":true,"pending":true,"operation_id":operation,"components":definition.components(),"report":{"complete":false,"applied":0,"adjusted":0,"skipped":0}});
    state.operations.insert(
        operation,
        Work {
            capture: c.clone(),
            ticket,
            started: Instant::now(),
            result: result.clone(),
            finished: false,
            applied,
            adjusted,
            huds: definition.context.huds,
            submitted: BTreeSet::new(),
            proofs,
        },
    );
    state.latest = Some(operation);
    Ok(result)
}
pub(super) fn placement_current(app: &App, assistant: &Assistant, id: Uuid) -> bool {
    assistant.workspaces.operations.get(&id).is_some_and(|w| {
        !w.finished
            && w.started.elapsed() < Duration::from_secs(30)
            && binding_matches(app, assistant, &w.capture)
            && assistant.runtime.capture_current(&w.capture)
            && original_scope_current(app, assistant, &w.capture)
            && app.workspace_source().as_ref() == Some(&w.ticket.source)
            && !app.panels.settings
    })
}
pub(super) fn tick(app: &mut App, assistant: &mut Assistant) {
    let Some(id) = app.workspaces.assistant else { return };
    let settled = app.workspaces.preview.is_none() && app.workspaces.placement.is_none() && !app.workspaces.assistant_cancelled;
    let current = placement_current(app, assistant, id);
    if settled && current {
        let points = app.world.worn_attachment_points();
        let Some(work) = assistant.workspaces.operations.get_mut(&id) else {
            return;
        };
        let remaining: Vec<_> = work
            .huds
            .iter()
            .filter(|h| points.get(&h.item) != Some(&h.point))
            .cloned()
            .collect();
        if !remaining.is_empty() {
            work.result["pending"] = json!(true);
            work.result["report"] = json!({"complete":false,"applied":work.applied,"adjusted":work.adjusted,"skipped":0,"reason":"waiting_for_native_hud_observation"});
            if app.appearance_ui.pending.is_none()
                && let Some(hud) = remaining.iter().find(|h| !work.submitted.contains(&h.item))
            {
                if hud_proof(app, hud).ok().as_ref() != work.proofs.get(&hud.item) {
                    work.result["hud_error"] = json!("workspace_hud_source_changed");
                    work.submitted.insert(hud.item);
                } else {
                    let runtime = assistant.runtime.clone();
                    let result = runtime.continue_action(&work.capture, || {
                        inventory::workspace_hud(app, &mut assistant.inventory, &work.capture, hud.item, hud.point)
                    });
                    work.submitted.insert(hud.item);
                    if !matches!(result, Ok(Ok(_))) {
                        work.result["hud_error"] = json!("native_hud_submission_refused_or_unconfirmed");
                    }
                }
            }
            return;
        }
    }
    if !settled && !current {
        app.workspace_assistant_cancel(id);
    }
    if settled || !current || app.workspaces.assistant_cancelled {
        if let Some(work) = assistant.workspaces.operations.get_mut(&id) {
            let observed = work
                .huds
                .iter()
                .filter(|h| app.world.worn_attachment_points().get(&h.item) == Some(&h.point))
                .count();
            let success = settled && observed == work.huds.len();
            work.finished = true;
            work.result["status"] = json!(if success {
                "applied"
            } else if settled {
                "submitted_unknown"
            } else {
                "cancelled"
            });
            work.result["pending"] = json!(false);
            work.result["retry_safe"] = json!(false);
            work.result["report"] = json!({"complete":success || !settled,"applied":if settled{work.applied+observed}else{0},"adjusted":if settled{work.adjusted}else{0},"skipped":if settled{work.huds.len()-observed}else{1},"reason":if success{"native_layout_and_context_settled"}else if settled{"native_hud_outcome_unconfirmed"}else{"authority_or_native_layout_changed"}});
            let _ = assistant.runtime.complete(&work.capture, work.result.clone());
        }
        if app.workspaces.assistant == Some(id) {
            app.workspaces.assistant = None;
            app.workspaces.assistant_expected = None;
        }
    }
}
