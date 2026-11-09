// SPDX-License-Identifier: LGPL-2.1-or-later
//! Source-bound curated preferences, after Folderstorm fspreferenceslistener.cpp
//! at abbe3cb0ca66ee2484f277219f6a27706fab7fc6. The ordinary settings lifecycle
//! owns persistence; a preview never queues its draft values for saving.
use super::*;
use crate::settings::curated::{Cleanup, Field, Held, Id, Scalar, Values};
use std::time::{Duration, Instant};

#[derive(Default)]
pub(super) struct State {
    revision: u64,
    observed: Option<(Values, Values)>,
    transaction: Option<Transaction>,
    last: Cleanup,
    reason: &'static str,
    cleanup_source: Option<(Uuid, Uuid)>,
}
struct Transaction {
    id: Uuid,
    capture: Capture,
    deadline: Instant,
    active: bool,
}
pub(super) fn supports(t: Tool) -> bool {
    matches!(
        t,
        Tool::PreferencesSchema
            | Tool::PreferencesGet
            | Tool::PreferencesStatus
            | Tool::PreferencesPreview
            | Tool::PreferencesAccept
            | Tool::PreferencesCancel
    )
}
fn same(a: &Capture, b: &Capture) -> bool {
    a.client == b.client && a.endpoint == b.endpoint && a.binding == b.binding && a.policy_revision == b.policy_revision
}
fn owned_transaction<'a>(state: &'a State, c: &Capture, now: Instant) -> Result<&'a Transaction, &'static str> {
    let t = state
        .transaction
        .as_ref()
        .filter(|t| same(&t.capture, c) && uuid(&c.arguments, "transaction_id").ok() == Some(t.id))
        .ok_or("preference_transaction_not_owned")?;
    if t.deadline <= now {
        return Err("preference_preview_expired");
    }
    Ok(t)
}
fn revision(a: &serde_json::Map<String, Value>) -> Result<u64, &'static str> {
    let s = string(a, "expected_revision", 19)?;
    if s.is_empty() || s.starts_with('0') || !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err("invalid_preference_revision");
    }
    s.parse().map_err(|_| "invalid_preference_revision")
}
fn ids(c: &Capture) -> Result<Vec<Id>, &'static str> {
    let Some(v) = c.arguments.get("ids") else {
        return Ok(Id::ALL.to_vec());
    };
    let a = v
        .as_array()
        .filter(|a| !a.is_empty() && a.len() <= 16)
        .ok_or("invalid_preference_ids")?;
    let mut out = BTreeSet::new();
    for v in a {
        if !out.insert(Id::parse(v.as_str().ok_or("invalid_preference_ids")?).ok_or("unknown_preference")?) {
            return Err("duplicate_preference_id");
        }
    }
    Ok(out.into_iter().collect())
}
fn changes(c: &Capture) -> Result<BTreeMap<Id, Scalar>, &'static str> {
    let a = c
        .arguments
        .get("changes")
        .and_then(Value::as_object)
        .filter(|a| !a.is_empty() && a.len() <= 16)
        .ok_or("invalid_preference_changes")?;
    a.iter()
        .map(|(k, v)| {
            let id = Id::parse(k).ok_or("unknown_preference")?;
            Ok((id, id.input(v)?))
        })
        .collect()
}
fn keys(c: &Capture, k: &[&str]) -> Result<(), &'static str> {
    if c.arguments
        .keys()
        .any(|key| !k.contains(&key.as_str()) && !matches!(key.as_str(), "viewer_session" | "viewer_pid" | "request_id"))
    {
        Err("invalid_args")
    } else {
        Ok(())
    }
}
fn bounds(app: &App, id: Id) -> Option<(f64, f64)> {
    let (mut lo, mut hi) = id.bounds()?;
    if id == Id::CameraFov
        && let Some((a, b)) = app.world.rlva.camera_fov_limits(app.camera_aspect())
    {
        lo = lo.max(a as f64);
        hi = hi.min(b as f64);
    }
    Some((lo, hi))
}
fn available(app: &App, id: Id, cleanup: bool) -> Result<(), &'static str> {
    if !cleanup
        && (app.panels.settings
            || app.workspaces.preview.is_some()
            || app.workspaces.placement.is_some()
            || app.workspaces.inventory_pending.is_some()
            || app.workspace_local_images.busy()
            || app.workspace_huds.busy()
            || app.settings.workspace_held.is_some())
    {
        return Err("busy_native_preferences_or_workspace");
    }
    if id.camera()
        && (app.world.rlva.camera_owner().is_some()
            || app.world.rlva.camera_preset_locked(Uuid::nil())
            || app.camera.flycam.is_some()
            || app.snapshots.ui.free.on)
    {
        return Err("native_camera_restriction");
    }
    if id == Id::LookAtTargets && (app.world.rlva.has("shownames") || app.world.rlva.has("shownearby")) {
        return Err("native_name_restriction");
    }
    if id == Id::DrawDistance && app.settings.photo_held.is_some() {
        return Err("native_photo_quality_owns_graphics");
    }
    if id == Id::UiScale && std::env::var_os("AURORA_UI_SCALE").is_some() {
        return Err("native_ui_scale_override");
    }
    if id.active() && (!active_allowed(app) || app.gfx.is_none()) {
        return Err("requires_active_character");
    }
    Ok(())
}
pub(super) fn requires_active(state: &State, c: &Capture) -> bool {
    match c.tool {
        Tool::PreferencesPreview => changes(c).is_ok_and(|v| v.keys().any(|id| id.active())),
        Tool::PreferencesAccept | Tool::PreferencesCancel => state.transaction.as_ref().is_some_and(|t| same(&t.capture, c) && t.active),
        _ => false,
    }
}
pub(super) fn prepare(app: &App, state: &State, c: &Capture) -> Result<Vec<Class>, &'static str> {
    match c.tool {
        Tool::PreferencesSchema | Tool::PreferencesStatus => keys(c, &[])?,
        Tool::PreferencesGet => {
            keys(c, &["ids"])?;
            ids(c)?;
        }
        Tool::PreferencesPreview => {
            keys(c, &["changes", "expected_revision", "ttl_seconds"])?;
            if state.transaction.is_some() || app.settings.curated_held.is_some() {
                return Err("preferences_preview_busy");
            }
            if revision(&c.arguments)? != state.revision.max(1) {
                return Err("preference_revision_conflict");
            }
            if c.arguments
                .get("ttl_seconds")
                .map_or(Some(120), Value::as_u64)
                .is_none_or(|s| !(1..=300).contains(&s))
            {
                return Err("invalid_preferences_ttl");
            }
            for (id, v) in changes(c)? {
                available(app, id, false)?;
                if let Scalar::Number(v) = v
                    && bounds(app, id).is_none_or(|(lo, hi)| v < lo - 1e-7 || v > hi + 1e-7)
                {
                    return Err("native_preference_range");
                }
            }
        }
        Tool::PreferencesAccept | Tool::PreferencesCancel => {
            keys(c, &["transaction_id", "expected_revision"])?;
            if revision(&c.arguments)? != state.revision.max(1) {
                return Err("preference_revision_conflict");
            }
            owned_transaction(state, c, Instant::now())?;
            let current = Values::capture(&app.settings);
            let held = app.settings.curated_held.as_ref().ok_or("preference_preview_unavailable")?;
            for (id, f) in &held.0 {
                available(app, *id, c.tool == Tool::PreferencesCancel)?;
                if c.tool == Tool::PreferencesAccept && (f.external || current.get(*id) != f.expected) {
                    return Err("preference_draft_changed_externally");
                }
            }
        }
        _ => return Err("unsupported_capability"),
    }
    Ok(vec![Class::PreferencesRead])
}
fn observe(app: &mut App, state: &mut State) {
    let current = Values::capture(&app.settings);
    if let Some(held) = &mut app.settings.curated_held {
        held.observe(&current);
    }
    let saved = app.settings.curated_saved.get(&app.settings);
    let next = (current, saved);
    if state.observed.as_ref() != Some(&next) {
        state.revision = state.revision.max(1).saturating_add(u64::from(state.observed.is_some()));
        state.observed = Some(next);
    }
}
fn cleanup(app: &mut App, state: &mut State, reason: &'static str) {
    let before = Values::capture(&app.settings);
    let same_native = state
        .cleanup_source
        .is_some_and(|(agent, login)| app.world.agent_id == agent && app.world.login.as_ref().is_some_and(|l| l.session_id == login));
    let allowed: BTreeSet<_> = Id::ALL
        .into_iter()
        .filter(|id| same_native && available(app, *id, true).is_ok())
        .collect();
    if let Some(held) = app.settings.curated_held.take() {
        let (residual, last) = held.restore(&mut app.settings, |id| allowed.contains(&id));
        app.settings.curated_held = residual;
        state.last = last;
    }
    state.transaction = None;
    state.reason = reason;
    app.curated_effects(&before);
    observe(app, state);
}
pub(super) fn evidence(app: &App, state: &State, c: &Capture) -> Result<Value, &'static str> {
    if c.tool.definition().read_only {
        return Ok(Value::Null);
    }
    prepare(app, state, c)?;
    let current = Values::capture(&app.settings);
    let saved = app.settings.curated_saved.get(&app.settings);
    Ok(
        json!({"revision":state.revision.max(1).to_string(),"transaction":state.transaction.as_ref().map(|t|t.id),"values":Id::ALL.into_iter().map(|id|(id.key(),json!({"effective":current.get(id),"saved":saved.get(id)}))).collect::<BTreeMap<_,_>>(),"workspace_binding":app.workspaces.binding,"workspace_revision":app.workspaces.revision}),
    )
}
fn base(app: &App, state: &State, c: &Capture) -> Value {
    let owned = state.transaction.as_ref().filter(|t| same(&t.capture, c));
    json!({"preferences_tools":1,"schema_version":1,"revision":state.revision.max(1).to_string(),"native_preferences_open":app.panels.settings,"preview_active":state.transaction.is_some(),"transaction_id":owned.map(|t|t.id),"expires_in_seconds":owned.map(|t|t.deadline.saturating_duration_since(Instant::now()).as_secs()),"cleanup":state.last,"cleanup_reason":state.reason})
}
pub(super) fn execute(app: &mut App, state: &mut State, c: &Capture) -> Result<Value, &'static str> {
    observe(app, state);
    prepare(app, state, c)?;
    let mut result = base(app, state, c);
    match c.tool {
        Tool::PreferencesSchema => {
            result["controls"]=json!(Id::ALL.into_iter().map(|id|{let reason=available(app,id,false).err();json!({"id":id.key(),"type":id.kind(),"writable":id.writable(),"requires_active":id.active(),"storage":"profile","range":bounds(app,id).map(|(min,max)|json!({"min":min,"max":max})),"available":reason.is_none(),"reason":reason,"write_reason":if id.writable(){None}else{Some("native_output_only")}})}).collect::<Vec<_>>());
            result["graphics_quality_levels"] = json!({"0":"potato","1":"low","3":"medium","5":"high","6":"deluxe","custom":null});
        }
        Tool::PreferencesGet => {
            let effective = Values::capture(&app.settings);
            let saved = app.settings.curated_saved.get(&app.settings);
            let defaults = Values::capture(&crate::settings::Settings::default());
            result["values"]=json!(ids(c)?.into_iter().map(|id| {let hidden=id==Id::LookAtTargets && available(app,id,true).is_err();(id.key(),if hidden {json!({"available":false,"reason":"native_name_restriction"})}else{json!({"effective":effective.get(id),"saved":saved.get(id),"default":defaults.get(id),"unsaved":effective.get(id)!=saved.get(id),"persisted":true})})}).collect::<BTreeMap<_,_>>());
        }
        Tool::PreferencesPreview => {
            let before = Values::capture(&app.settings);
            let saved = app.settings.curated_saved.get(&app.settings);
            let mut held = Held::default();
            let changes = changes(c)?;
            if changes.contains_key(&Id::DrawDistance) {
                held.1 = Some(app.settings.graphics_saved_active.clone());
                app.settings.graphics_saved_active = None;
            }
            for (id, v) in &changes {
                held.0.insert(
                    *id,
                    Field {
                        before: before.get(*id),
                        saved: saved.get(*id),
                        expected: *v,
                        external: false,
                    },
                );
                id.write(&mut app.settings, *v);
            }
            app.settings.curated_held = Some(held);
            state.cleanup_source = Some((app.world.agent_id, app.world.login.as_ref().ok_or("native_not_ready")?.session_id));
            let ttl = c.arguments.get("ttl_seconds").and_then(Value::as_u64).unwrap_or(120);
            state.transaction = Some(Transaction {
                id: Uuid::new_v4(),
                capture: c.clone(),
                deadline: Instant::now() + Duration::from_secs(ttl),
                active: changes.keys().any(|id| id.active()),
            });
            app.curated_effects(&before);
            observe(app, state);
            result = base(app, state, c);
            result["status"] = json!("submitted");
            result["saved_changed"] = json!(false);
        }
        Tool::PreferencesAccept => {
            app.settings.curated_held = None;
            state.transaction = None;
            app.settings.save();
            state.reason = "accepted";
            observe(app, state);
            result = base(app, state, c);
            result["status"] = json!("accepted");
            result["persistence"] = json!("normal_settings_save_queued");
        }
        Tool::PreferencesCancel => {
            cleanup(app, state, "cancelled_by_source");
            result = base(app, state, c);
            result["status"] = json!("cancelled");
        }
        _ => {}
    }
    Ok(result)
}
pub(super) fn tick(app: &mut App, assistant: &mut Assistant) {
    observe(app, &mut assistant.preferences);
    let reason = assistant.preferences.transaction.as_ref().and_then(|t| {
        if !assistant.runtime.capture_current(&t.capture) || !binding_matches(app, assistant, &t.capture) {
            Some("authority_or_source_changed")
        } else if t.deadline <= Instant::now() {
            Some("expired")
        } else if app.panels.settings {
            Some("native_preferences_opened")
        } else if app
            .settings
            .curated_held
            .as_ref()
            .is_some_and(|held| held.0.keys().any(|id| available(app, *id, false).is_err()))
        {
            Some("native_availability_changed")
        } else {
            None
        }
    });
    if let Some(reason) = reason {
        cleanup(app, &mut assistant.preferences, reason);
    }
    if assistant.preferences.transaction.is_none() && app.settings.curated_held.is_some() {
        cleanup(app, &mut assistant.preferences, "pending_owned_cleanup");
    }
}
pub(super) fn native_opening(app: &mut App, assistant: &mut Assistant) {
    if assistant.preferences.transaction.is_some() {
        cleanup(app, &mut assistant.preferences, "native_preferences_opened");
    }
}
impl App {
    fn curated_effects(&mut self, before: &Values) {
        let after = Values::capture(&self.settings);
        if before.get(Id::UiScale) != after.get(Id::UiScale) {
            self.egui_ctx.set_zoom_factor(super::super::ui_scale(&self.settings));
        }
        if before.get(Id::DrawDistance) != after.get(Id::DrawDistance) {
            self.scene.draw_distance = self.settings.draw_distance;
            self.net.send(aurora_net::NetCommand::SetDrawDistance(self.settings.draw_distance));
        }
        self.egui_ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transaction_identity_and_expiry_are_checked_independently_of_visible_uuid() {
        let now = Instant::now();
        let original = super::super::creative::synthetic_capture(Tool::PreferencesPreview);
        let id = Uuid::new_v4();
        let state = State {
            transaction: Some(Transaction {
                id,
                capture: original.clone(),
                deadline: now + Duration::from_secs(120),
                active: true,
            }),
            ..Default::default()
        };
        let mut accept = original.clone();
        accept.tool = Tool::PreferencesAccept;
        accept.arguments = json!({"transaction_id":id,"expected_revision":"1"}).as_object().unwrap().clone();
        assert!(owned_transaction(&state, &accept, now).is_ok());
        assert_eq!(
            owned_transaction(&state, &accept, now + Duration::from_secs(120)).err(),
            Some("preference_preview_expired")
        );
        for dimension in 0..4 {
            let mut c = accept.clone();
            match dimension {
                0 => c.client += 1,
                1 => c.endpoint = HostId::from_bytes([9; 16]).unwrap(),
                2 => c.binding.session.as_mut().unwrap().generation += 1,
                _ => c.policy_revision += 1,
            }
            assert_eq!(owned_transaction(&state, &c, now).err(), Some("preference_transaction_not_owned"));
        }
    }
    #[test]
    fn revision_strings_are_exact_and_activity_comes_from_native_descriptors() {
        let mut c = super::super::creative::synthetic_capture(Tool::PreferencesPreview);
        for bad in ["", "0", "01", "1 ", "+1", "1e2", "18446744073709551615"] {
            c.arguments = json!({"expected_revision":bad}).as_object().unwrap().clone();
            assert!(revision(&c.arguments).is_err());
        }
        c.arguments = json!({"changes":{"tap_hold_run":false},"expected_revision":"1"})
            .as_object()
            .unwrap()
            .clone();
        assert!(!requires_active(&State::default(), &c));
        c.arguments = json!({"changes":{"camera_fov":1.0},"expected_revision":"1"})
            .as_object()
            .unwrap()
            .clone();
        assert!(requires_active(&State::default(), &c));
    }
}
