// SPDX-License-Identifier: LGPL-2.1-or-later
//! Read-only projection of the lead's native workspace catalogue. Never serialize
//! Inventory/HUD/image references or install the earlier alternate context model.
//! Native MCP mutation waits for an integrated final-effect/consent bridge.
use super::*;
use crate::workspace::model::Layout;

#[derive(Default)]
pub(super) struct State;
pub(super) fn supports(tool: Tool) -> bool {
    matches!(tool, Tool::WorkspaceList | Tool::WorkspaceStatus | Tool::WorkspacePreview)
}
fn current(app: &App) -> Result<(), &'static str> {
    let source = app.workspace_source().ok_or("native_workspace_source_unavailable")?;
    if app.workspaces.source.as_ref() != Some(&source) || !crate::workspace::images::source_current(&app.world, &source) {
        return Err("native_workspace_source_or_privacy_unavailable");
    }
    Ok(())
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
fn target(app: &App, c: &Capture) -> Result<Uuid, &'static str> {
    // Native records are named workspaces. Do not masquerade them as the
    // separate layout catalogue from the author's older implementation.
    if string(&c.arguments, "kind", 9)? != "workspace" {
        return Err("native_workspace_kind_unavailable");
    }
    let id = uuid(&c.arguments, "id")?;
    app.workspaces
        .record(id)
        .ok_or("workspace_not_found")?
        .layout
        .validate()
        .map_err(|_| "invalid_native_workspace")?;
    Ok(id)
}
pub(super) fn preparation(app: &mut App, assistant: &Assistant, c: &Capture) -> Result<bool, &'static str> {
    assistant.runtime.continue_preparation(c, Class::Read, || ())?;
    prepare(app, &assistant.workspaces, c)?;
    if app.workspaces.catalogue.is_some() {
        return Ok(true);
    }
    app.workspace_ensure_catalogue()
}
pub(super) fn prepare(app: &App, _state: &State, c: &Capture) -> Result<Vec<Class>, &'static str> {
    if !supports(c.tool) {
        return Err("native_workspace_mutation_unavailable");
    }
    current(app)?;
    match c.tool {
        Tool::WorkspaceList | Tool::WorkspaceStatus => keys(c, &[])?,
        Tool::WorkspacePreview => {
            keys(c, &["kind", "id"])?;
            // The first preparation may still need the passive catalogue read.
            if app.workspaces.catalogue.is_some() {
                target(app, c)?;
            }
        }
        _ => return Err("unsupported_capability"),
    }
    Ok(vec![Class::Read])
}
pub(super) fn evidence(app: &App, state: &State, c: &Capture) -> Result<Value, &'static str> {
    prepare(app, state, c)?;
    Ok(Value::Null)
}
fn projection(layout: &Layout) -> Value {
    // Explicit allowlist: saved UUIDs/paths/names/pixels are never Read authority.
    json!({"geometry":layout.geometry,"chrome":{"viewport":layout.viewport,"bars":layout.bars},"graphics_context":layout.graphics.is_some(),"camera_context":layout.camera.is_some(),"unavailable_groups":["hud_details","inventory_details","background_details","mcp_workspace_mutation"],"pending":false})
}
fn status(app: &App) -> Value {
    json!({"workspace_tools":1,"available":app.workspaces.catalogue.is_some(),"can_switch":false,"active":app.workspaces.active,"active_kind":"workspace","has_previous":app.workspaces.previous.is_some(),"revision":app.workspaces.revision,"pending":app.workspaces.pending.is_some(),"supported_kinds":["workspace"],"unavailable_groups":["mcp_workspace_mutation"]})
}
pub(super) fn execute(app: &mut App, state: &mut State, c: &Capture) -> Result<Value, &'static str> {
    prepare(app, state, c)?;
    if c.tool == Tool::WorkspaceStatus {
        return Ok(status(app));
    }
    let catalogue = app.workspaces.catalogue.as_ref().ok_or("native_workspace_catalogue_unavailable")?;
    if c.tool == Tool::WorkspaceList {
        let mut value = status(app);
        value["entries"] = json!(
            catalogue
                .records
                .iter()
                .map(|r| json!({"kind":"workspace","id":r.id,"name":r.name,"favorite":catalogue.favorites.contains(&r.id)}))
                .collect::<Vec<_>>()
        );
        return Ok(value);
    }
    let id = target(app, c)?;
    let mut value = projection(&app.workspaces.record(id).ok_or("workspace_not_found")?.layout);
    value["kind"] = json!("workspace");
    value["id"] = json!(id);
    Ok(value)
}
pub(super) fn tick(_app: &mut App, _assistant: &mut Assistant) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::model::{Background, Bars, HudRef, HudSet, ImageMode, LocalPicture, ManagedRef, Viewport};
    #[test]
    fn mcp_workspace_projection_never_serializes_private_hud_inventory_or_managed_image_references() {
        let secret = Uuid::new_v4();
        let account = Uuid::new_v4();
        let grid = "b".repeat(64);
        let layout = Layout {
            geometry: [None; 4],
            viewport: Viewport::from(&crate::world_viewport::ViewportSettings::default()),
            bars: Bars {
                classic: false,
                stretch: false,
                rights_text: false,
            },
            inventory_folder: Some(secret),
            inventory_expanded: None,
            additional_inventory: None,
            graphics: None,
            camera: None,
            background: Some(
                Background::Black {}
                    .with_local(
                        None,
                        LocalPicture {
                            reference: ManagedRef::new(secret, account, grid.clone()).unwrap(),
                            mode: ImageMode::Fill,
                            brightness: 255,
                        },
                    )
                    .unwrap(),
            ),
            huds: Some(HudSet::try_from(vec![HudRef { item: secret, point: 31 }]).unwrap()),
        };
        let before = serde_json::to_vec(&layout).unwrap();
        let value = projection(&layout);
        let text = value.to_string();
        for forbidden in [secret.to_string(), account.to_string(), grid] {
            assert!(!text.contains(&forbidden), "private saved context must not enter MCP Read output");
        }
        assert_eq!(value["pending"], false);
        assert_eq!(serde_json::to_vec(&layout).unwrap(), before);
    }
    #[test]
    fn mcp_workspace_native_capabilities_refuse_unintegrated_mutation_even_when_sdk_discovers_it() {
        for t in [Tool::WorkspaceList, Tool::WorkspaceStatus, Tool::WorkspacePreview] {
            assert!(supports(t));
            assert!(t.definition().read_only);
        }
        for t in [Tool::WorkspaceApply, Tool::WorkspacePrevious] {
            assert!(!supports(t));
            assert!(!t.definition().read_only);
        }
    }
}
