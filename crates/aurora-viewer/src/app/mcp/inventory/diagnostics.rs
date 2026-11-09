use super::*;
#[derive(Default)]
pub(super) struct State {
    scope: Option<(Binding, HostId, u64)>,
    cursors: BTreeMap<Uuid, Cursor>,
}
struct Cursor {
    endpoint: HostId,
    client: u64,
    policy: u64,
    epoch: u64,
    through: u64,
    object: Option<Uuid>,
}
pub(super) fn supports(tool: Tool) -> bool {
    tool == Tool::ScriptDebugPoll
}
pub(super) fn privacy(app: &App) -> Result<(), &'static str> {
    if !app.world.shared_chat_privacy_allowed() || app.world.rlva.has("viewscript") || !app.world.mutes.is_loaded() {
        Err("native_diagnostic_restriction")
    } else {
        Ok(())
    }
}
pub(super) fn prepare(app: &App, _: &State, c: &Capture) -> Result<Vec<Class>, &'static str> {
    args(c, &["object_id", "cursor", "start", "limit"])?;
    privacy(app)?;
    if app.settings.local_assistant.permissions.level(Class::ScriptDebugRead) != Level::Allow {
        return Err("diagnostic_capture_requires_allow");
    }
    if !app.world.assistant_observations.enabled {
        return Err("diagnostic_capture_unavailable");
    }
    limit(&c.arguments, 50, 100)?;
    if let Some(id) = optional_uuid(&c.arguments, "object_id")? {
        super::interactions::object(app, id)?;
    }
    match (c.arguments.get("cursor"), c.arguments.get("start")) {
        (Some(v), None) => {
            Uuid::parse_str(v.as_str().ok_or("invalid_args")?).map_err(|_| "invalid_args")?;
        }
        (None, Some(v)) if matches!(v.as_str(), Some("now" | "retained")) => {}
        _ => return Err("invalid_args"),
    };
    Ok(Vec::new())
}
fn capture_authorized(runtime: &Runtime, level: Level, privacy: bool) -> bool {
    runtime.native_observation_ready() && level == Level::Allow && privacy
}
pub(super) fn tick(app: &mut App, assistant: &mut Assistant) {
    let revision = assistant.runtime.policy_revision();
    let scope = assistant
        .binding
        .zip(assistant.runtime.snapshot().viewer_session)
        .map(|(b, e)| (b, e, revision));
    if assistant.inventory.diagnostics.scope != scope {
        app.world.assistant_observations.clear();
        assistant.inventory.diagnostics.cursors.clear();
        assistant.inventory.diagnostics.scope = scope;
    }
    let enabled = scope.is_some()
        && capture_authorized(
            &assistant.runtime,
            app.settings.local_assistant.permissions.level(Class::ScriptDebugRead),
            privacy(app).is_ok(),
        );
    app.world.assistant_observations.set_enabled(enabled);
}
pub(super) fn execute(app: &App, state: &mut State, c: &Capture) -> Result<Value, &'static str> {
    privacy(app)?;
    let journal = &app.world.assistant_observations;
    let object = optional_uuid(&c.arguments, "object_id")?;
    let after = if c.arguments.contains_key("cursor") {
        let id = uuid(&c.arguments, "cursor")?;
        let old = state
            .cursors
            .get(&id)
            .filter(|old| {
                old.endpoint == c.endpoint
                    && old.client == c.client
                    && old.policy == c.policy_revision
                    && old.epoch == journal.epoch
                    && old.object == object
            })
            .ok_or("stale_diagnostic_cursor")?;
        old.through
    } else if c.arguments.get("start").and_then(Value::as_str) == Some("now") {
        journal.serial
    } else {
        journal.entries.front().map_or(journal.serial, |e| e.serial.saturating_sub(1))
    };
    let first = journal.entries.front().map_or(journal.serial.saturating_add(1), |e| e.serial);
    let gap = after.saturating_add(1) < first;
    let max = limit(&c.arguments, 50, 100)?;
    let (mut rows, mut next, mut bytes) = (Vec::new(), after, 0usize);
    for entry in journal.entries.iter().filter(|e| e.serial > after) {
        if object.is_none_or(|id| id == entry.object)
            && super::interactions::object(app, entry.object).is_ok()
            && app.world.assistant_diagnostic_allowed(entry.object, &entry.text)
        {
            let row = json!({"serial":entry.serial.to_string(),"object_id":entry.object.to_string(),"text":entry.text,"runtime_error":entry.error,"age_seconds":entry.at.elapsed().as_secs_f64(),"script_id":null,"line":null,"untrusted_data":true});
            let size = serde_json::to_vec(&row).map_or(24 * 1024, |b| b.len());
            if rows.len() >= max || bytes + size > 18 * 1024 {
                break;
            }
            bytes += size;
            rows.push(row);
        }
        next = entry.serial;
    }
    if state.cursors.len() >= 256 {
        state.cursors.clear();
    }
    let id = Uuid::new_v4();
    state.cursors.insert(
        id,
        Cursor {
            endpoint: c.endpoint,
            client: c.client,
            policy: c.policy_revision,
            epoch: journal.epoch,
            through: next,
            object,
        },
    );
    Ok(json!({"entries":rows,"next_cursor":id.to_string(),"gap":gap,"reconcile_required":gap,"untrusted_data":true}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_busy_blocks_delivery_without_disabling_the_filtered_diagnostic_journal() {
        let c = synthetic_capture(Tool::ScriptDebugPoll);
        let runtime = Runtime::default();
        runtime.observe(Some(c.binding), true);
        runtime.enable(c.binding, c.endpoint).unwrap();
        let mut journal = crate::world::assistant_observations::Observations::default();
        runtime.begin_native_update();
        assert!(!runtime.snapshot().ready);
        journal.set_enabled(capture_authorized(&runtime, Level::Allow, true));
        assert!(journal.enabled);
        journal.record(
            &aurora_net::ChatMessage {
                from_name: "owned script".into(),
                source_id: Uuid::from_u128(2),
                owner_id: Uuid::from_u128(1),
                source_type: aurora_net::ChatSourceType::Object,
                chat_type: aurora_net::ChatType::OwnerSay,
                position: glam::Vec3::ZERO,
                message: "expected output".into(),
            },
            Uuid::from_u128(1),
        );
        assert_eq!(journal.entries.len(), 1);
        assert_eq!(journal.entries[0].text, "expected output");
        let epoch = journal.epoch;
        runtime.pause(true);
        journal.set_enabled(capture_authorized(&runtime, Level::Allow, true));
        assert!(!journal.enabled);
        assert!(journal.entries.is_empty());
        assert_ne!(journal.epoch, epoch);
        runtime.end_native_update();
        assert!(!capture_authorized(&runtime, Level::Allow, true));
        runtime.pause(false);
        assert!(!capture_authorized(&runtime, Level::Ask, true));
        assert!(!capture_authorized(&runtime, Level::Allow, false));
    }
}
