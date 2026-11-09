//! Frozen, source-owned test recipes. A report read never advances work. Only
//! the declared steps can use this module's durable scope; it grants no generic
//! action renewal, interpreter, hidden-channel listener or fixture mutation.
use super::*;
use crate::session_host::worker::action::NativeActionScope;
use aurora_net::build::BuildCmd;
type Key = (HostId, u64, Uuid);
#[derive(Default)]
pub(super) struct State {
    runs: BTreeMap<Key, Run>,
    intents: BTreeMap<Key, Capture>,
    completed: Vec<(Capture, Value)>,
    evidence: interactions::FrozenEvidence,
}
struct Run {
    capture: Capture,
    scope: RecipeScope,
    objects: BTreeMap<Uuid, Value>,
    steps: Vec<Step>,
    index: usize,
    deadline: Instant,
    since: Instant,
    started: Instant,
    phase_started: bool,
    observed: u64,
    output: u64,
    journal_epoch: u64,
    errors: u64,
    fail_errors: bool,
    state: &'static str,
    reason: &'static str,
    report: Vec<Value>,
    assertions: usize,
    unverified: usize,
}
struct RecipeScope {
    managed: Option<NativeActionScope>,
    native_login: Uuid,
}
#[derive(Clone)]
struct Step {
    action: Action,
    object: Option<Uuid>,
    params: serde_json::Map<String, Value>,
    timeout: u64,
    expected: bool,
}
#[derive(Clone, Copy, PartialEq)]
enum Action {
    Touch,
    Reset,
    Running,
    SendChannel,
    WaitOutput,
    AssertRunning,
    AssertContents,
    Delay,
}
pub(super) fn supports(t: Tool) -> bool {
    matches!(t, Tool::ScriptTestBegin | Tool::ScriptTestStatus | Tool::ScriptTestStop)
}
fn owner(c: &Capture, id: Uuid) -> Key {
    (c.endpoint, c.client, id)
}
fn run<'a>(state: &'a super::State, c: &Capture) -> Result<&'a Run, &'static str> {
    let id = uuid(&c.arguments, "run_id")?;
    state
        .testing
        .runs
        .get(&owner(c, id))
        .filter(|r| r.capture.binding == c.binding)
        .ok_or("unknown_test_run")
}
pub(super) fn original(state: &super::State, c: &Capture) -> Result<Option<Capture>, &'static str> {
    if c.tool == Tool::ScriptTestStatus {
        Ok(Some(run(state, c)?.capture.clone()))
    } else {
        Ok(None)
    }
}
fn step_capture(c: &Capture, s: &Step) -> Capture {
    let mut c = c.clone();
    c.arguments = s.params.clone();
    c
}
fn parse(app: &App, state: &super::State, c: &Capture) -> Result<(BTreeMap<Uuid, Value>, Vec<Step>, Vec<Class>), &'static str> {
    let a = &c.arguments;
    args(
        c,
        &[
            "client_request_id",
            "object_ids",
            "steps",
            "timeout_seconds",
            "fail_on_runtime_errors",
        ],
    )?;
    uuid(a, "client_request_id")?;
    let timeout = number(a, "timeout_seconds", 0, 600)?;
    if timeout == 0 {
        return Err("invalid_test_timeout");
    }
    let ids = a
        .get("object_ids")
        .and_then(Value::as_array)
        .filter(|v| !v.is_empty() && v.len() <= 32)
        .ok_or("invalid_test_scope")?;
    let mut scope = BTreeMap::new();
    for id in ids {
        let id = Uuid::parse_str(id.as_str().ok_or("invalid_args")?).map_err(|_| "invalid_args")?;
        if scope.insert(id, interactions::freeze(app, id)?).is_some() {
            return Err("duplicate_test_object");
        }
    }
    let values = a
        .get("steps")
        .and_then(Value::as_array)
        .filter(|v| !v.is_empty() && v.len() <= 64)
        .ok_or("test_step_limit")?;
    let mut steps = Vec::new();
    let mut classes = vec![Class::WorldRead, Class::TaskInventoryRead, Class::ScriptDebugRead, Class::Read];
    for value in values {
        let mut p = value.as_object().cloned().ok_or("invalid_test_step")?;
        let name = string(&p, "action", 32)?.to_owned();
        p.remove("action");
        let action = match name.as_str() {
            "touch" => Action::Touch,
            "reset" => Action::Reset,
            "running" => Action::Running,
            "send_channel" => Action::SendChannel,
            "wait_output" => Action::WaitOutput,
            "assert_running" => Action::AssertRunning,
            "assert_contents" => Action::AssertContents,
            "delay" => Action::Delay,
            _ => return Err("unsupported_test_action"),
        };
        let allowed: &[&str] = match action {
            Action::Touch => &["object_id", "face", "uv", "local_point"],
            Action::Reset => &["object_id", "item_id", "snapshot_id"],
            Action::Running => &["object_id", "item_id", "snapshot_id", "running"],
            Action::SendChannel => &["object_id", "channel", "text"],
            Action::WaitOutput => &["object_id", "text", "match", "expected", "timeout_seconds"],
            Action::AssertRunning | Action::AssertContents => &["object_id", "item_id", "expected", "timeout_seconds"],
            Action::Delay => &["seconds"],
        };
        if p.keys().any(|k| !allowed.contains(&k.as_str())) {
            return Err("test_step_field_mismatch");
        }
        let object = if action == Action::Delay {
            None
        } else {
            let id = uuid(&p, "object_id")?;
            if !scope.contains_key(&id) {
                return Err("test_target_outside_scope");
            }
            Some(id)
        };
        let mut step = Step {
            action,
            object,
            params: p,
            timeout: 0,
            expected: true,
        };
        match action {
            Action::Touch => {
                let idx = interactions::object(app, object.ok_or("invalid_args")?)?;
                interactions::surface(app, idx, &step.params)?;
                classes.push(Class::ObjectInteract);
            }
            Action::Reset | Action::Running => {
                let it = objects::item(app, state, &step_capture(c, &step))?;
                if it.asset_type != 10 || it.owner_mask & (rules::COPY | rules::MODIFY) != (rules::COPY | rules::MODIFY) {
                    return Err("native_script_permission_denied");
                }
                let info = app
                    .script_info(aurora_net::scripts::ScriptTarget {
                        object: object.ok_or("invalid_args")?,
                        item: it.item_id,
                    })
                    .ok_or("item_unavailable")?;
                if !info.can_edit {
                    return Err("native_script_permission_denied");
                }
                if action == Action::Running {
                    step.params.get("running").and_then(Value::as_bool).ok_or("invalid_args")?;
                }
                classes.push(Class::ScriptRun);
            }
            Action::SendChannel => {
                interactions::channel(&step.params)?;
                classes.extend([Class::ObjectInteract, Class::ChatSendNearby]);
            }
            Action::WaitOutput | Action::AssertRunning | Action::AssertContents => {
                step.timeout = number(&step.params, "timeout_seconds", 0, 120)? as u64;
                if step.timeout == 0 {
                    return Err("invalid_assertion_timeout");
                }
                step.expected = step
                    .params
                    .get("expected")
                    .and_then(Value::as_bool)
                    .ok_or("assertion_expected_required")?;
                if action == Action::WaitOutput {
                    let text = string(&step.params, "text", 1024)?;
                    if text.is_empty() {
                        return Err("output_expectation_required");
                    }
                    if step
                        .params
                        .get("match")
                        .is_some_and(|m| !matches!(m.as_str(), Some("contains" | "equals")))
                    {
                        return Err("invalid_output_match");
                    }
                } else {
                    uuid(&step.params, "item_id")?;
                }
            }
            Action::Delay => {
                step.timeout = number(&step.params, "seconds", 0, 60)? as u64;
                if step.timeout == 0 {
                    return Err("invalid_test_delay");
                }
            }
        }
        steps.push(step);
    }
    classes.sort();
    classes.dedup();
    Ok((scope, steps, classes))
}
fn source(app: &App, assistant: &Assistant, run: &Run) -> bool {
    super::super::binding_matches(app, assistant, &run.capture)
        && assistant.runtime.configuration().as_ref() == Some(&app.settings.local_assistant)
        && assistant.runtime.capture_current(&run.capture)
        && app.world.login.as_ref().is_some_and(|l| l.session_id == run.scope.native_login)
        && super::super::active_allowed(app)
        && match run.scope.managed {
            Some(old) => app
                .host_session
                .as_ref()
                .and_then(|b| b.native_action_scope())
                .is_some_and(|fresh| {
                    fresh.source() == old.source() && fresh.activation() == old.activation() && fresh.generation() == old.generation()
                }),
            None => app.host_session.is_none(),
        }
}
fn facts(app: &App, run: &Run) -> bool {
    diagnostics::privacy(app).is_ok()
        && run
            .objects
            .iter()
            .all(|(id, old)| interactions::freeze(app, *id).as_ref() == Ok(old))
}
pub(super) fn prepare(app: &App, state: &super::State, c: &Capture) -> Result<Vec<Class>, &'static str> {
    match c.tool {
        Tool::ScriptTestBegin => {
            if state.testing.runs.values().any(|r| r.state == "running") {
                return Err("test_run_busy");
            }
            diagnostics::privacy(app)?;
            if !app.world.assistant_observations.enabled {
                return Err("diagnostic_capture_unavailable");
            }
            let (objects, _, classes) = parse(app, state, c)?;
            interactions::freeze_capture(&state.testing.evidence, c, json!(objects))?;
            Ok(classes)
        }
        Tool::ScriptTestStatus => {
            args(c, &["run_id"])?;
            let r = run(state, c)?;
            if r.capture.policy_revision != c.policy_revision || !facts(app, r) || r.started.elapsed() > Duration::from_secs(605) {
                return Err("stale_test_report");
            }
            Ok(vec![
                Class::WorldRead,
                Class::TaskInventoryRead,
                Class::ScriptDebugRead,
                Class::Read,
            ])
        }
        Tool::ScriptTestStop => {
            args(c, &["run_id"])?;
            run(state, c)?;
            Ok(vec![])
        }
        _ => Err("unsupported_capability"),
    }
}
pub(super) fn execute(app: &mut App, assistant: &mut Assistant, c: &Capture) -> Result<Value, &'static str> {
    match c.tool {
        Tool::ScriptTestBegin => {
            let id = uuid(&c.arguments, "client_request_id")?;
            let key = owner(c, id);
            if assistant.inventory.testing.intents.contains_key(&key) {
                return Err("test_intent_already_used");
            }
            if assistant.inventory.testing.runs.len() >= 16 || assistant.inventory.testing.intents.len() >= 4096 {
                return Err("test_capacity");
            }
            let (objects, steps, _) = parse(app, &assistant.inventory, c)?;
            let managed = assistant.scopes.get(&c.request_id).copied().ok_or("native_scope_missing")?;
            let scope = RecipeScope {
                managed,
                native_login: app.world.login.as_ref().ok_or("native_not_ready")?.session_id,
            };
            let now = Instant::now();
            let journal = &app.world.assistant_observations;
            let run = Run {
                capture: c.clone(),
                scope,
                objects,
                steps,
                index: 0,
                deadline: now + Duration::from_secs(number(&c.arguments, "timeout_seconds", 0, 600)? as u64),
                since: now,
                started: now,
                phase_started: false,
                observed: 0,
                output: journal.serial,
                journal_epoch: journal.epoch,
                errors: journal.serial,
                fail_errors: boolean(&c.arguments, "fail_on_runtime_errors", true)?,
                state: "running",
                reason: "",
                report: Vec::new(),
                assertions: 0,
                unverified: 0,
            };
            assistant.inventory.testing.intents.insert(key, c.clone());
            assistant.inventory.testing.runs.insert(key, run);
            Ok(json!({"status":"submitted","run_id":id.to_string(),"operation_id":c.request_id,"retry_safe":false}))
        }
        Tool::ScriptTestStatus => {
            let r = run(&assistant.inventory, c)?;
            Ok(report(uuid(&c.arguments, "run_id")?, r))
        }
        Tool::ScriptTestStop => {
            let id = uuid(&c.arguments, "run_id")?;
            let r = assistant
                .inventory
                .testing
                .runs
                .get_mut(&owner(c, id))
                .filter(|r| r.capture.binding == c.binding)
                .ok_or("unknown_test_run")?;
            if r.state == "running" {
                r.state = "cancelled";
                r.reason = "explicit_stop";
                assistant.inventory.testing.completed.push((r.capture.clone(), report(id, r)));
            }
            Ok(json!({"run_id":id.to_string(),"state":r.state,"reason":r.reason}))
        }
        _ => Err("unsupported_capability"),
    }
}
fn report(id: Uuid, r: &Run) -> Value {
    json!({"run_id":id.to_string(),"state":r.state,"reason":r.reason,"step":r.index,"total_steps":r.steps.len(),"steps":r.report,"assertions_observed":r.assertions,"submitted_actions_unverified":r.unverified,"assertion_scope":"declared_assertions_only","actions_confirmed":r.unverified==0,"source_revision_known":false,"source_hash":null,"retry_safe":false,"untrusted_data":true})
}
fn end(r: &mut Run, state: &'static str, reason: &'static str) {
    r.state = state;
    r.reason = reason;
}
fn advance(r: &mut Run, observed: bool) {
    r.report
        .push(json!({"step":r.index,"observed":observed,"submitted_unverified":!observed}));
    r.index += 1;
    r.phase_started = false;
    r.since = Instant::now();
    if r.index == r.steps.len() {
        end(
            r,
            if r.assertions > 0 { "passed" } else { "completed_unverified" },
            "declared_steps_complete",
        );
    }
}
fn step(app: &mut App, assistant: &mut Assistant, r: &mut Run) -> Result<(), &'static str> {
    let s = r.steps.get(r.index).cloned().ok_or("test_step_missing")?;
    let object = s.object.unwrap_or_default();
    let now = Instant::now();
    let engine = assistant.runtime.clone();
    match s.action {
        Action::Delay => {
            if !r.phase_started {
                r.since = now;
                r.phase_started = true;
            }
            if r.since.elapsed() >= Duration::from_secs(s.timeout) {
                advance(r, true);
            }
        }
        Action::Touch => {
            engine.continue_action(&r.capture, || interactions::touch(app, object, &s.params))??;
            r.unverified += 1;
            advance(r, false);
        }
        Action::SendChannel => {
            engine.continue_action(&r.capture, || {
                interactions::send_channel(app, &mut assistant.inventory.interactions, &s.params)
            })??;
            r.unverified += 1;
            advance(r, false);
        }
        Action::Reset | Action::Running => {
            let c = step_capture(&r.capture, &s);
            let it = objects::item(app, &assistant.inventory, &c)?.clone();
            let idx = interactions::object(app, object)?;
            let handle = app.world.objects.get(idx).ok_or("object_unavailable")?.key.region;
            let command = if s.action == Action::Reset {
                BuildCmd::ScriptReset {
                    handle,
                    object_id: object,
                    item_id: it.item_id,
                }
            } else {
                BuildCmd::SetScriptRunning {
                    handle,
                    object_id: object,
                    item_id: it.item_id,
                    running: s.params.get("running").and_then(Value::as_bool).ok_or("invalid_args")?,
                }
            };
            engine.continue_action(&r.capture, || {
                objects::execution_changed(&mut assistant.inventory, r.capture.binding, object, it.item_id);
                app.send(NetCommand::Build(command));
            })?;
            r.unverified += 1;
            advance(r, false);
        }
        Action::AssertRunning => {
            let item = uuid(&s.params, "item_id")?;
            if !r.phase_started {
                engine.continue_action(&r.capture, || {
                    objects::recipe_query(app, &mut assistant.inventory, &r.capture, object, item)
                })??;
                r.since = now;
                r.phase_started = true;
            } else if let Some((value, _, _, at)) =
                objects::running_value(&assistant.inventory, &r.capture, object, item).filter(|v| v.3 >= r.since)
            {
                let _ = at;
                if value != s.expected {
                    return Err("running_assertion_failed");
                }
                r.assertions += 1;
                advance(r, true);
            } else if r.since.elapsed() >= Duration::from_secs(s.timeout) {
                return Err("running_assertion_unobserved");
            }
        }
        Action::AssertContents => {
            if !r.phase_started {
                r.observed = objects::observation(&assistant.inventory, object);
                let idx = interactions::object(app, object)?;
                engine.continue_action(&r.capture, || {
                    app.build.request_contents(&app.world, idx, true);
                    app.flush_build();
                })?;
                r.since = now;
                r.phase_started = true;
            } else if objects::observation(&assistant.inventory, object) > r.observed {
                let item = uuid(&s.params, "item_id")?;
                let contents = app
                    .build
                    .contents
                    .objects
                    .get(&object)
                    .filter(|c| !c.loading)
                    .ok_or("contents_unobserved")?;
                if contents.items.iter().any(|i| i.item_id == item) != s.expected {
                    return Err("contents_assertion_failed");
                }
                r.assertions += 1;
                advance(r, true);
            } else if r.since.elapsed() >= Duration::from_secs(s.timeout) {
                return Err("contents_assertion_unobserved");
            }
        }
        Action::WaitOutput => {
            if !r.phase_started {
                r.since = now;
                r.phase_started = true;
            }
            let journal = &app.world.assistant_observations;
            if !journal.enabled
                || journal.epoch != r.journal_epoch
                || journal.entries.front().is_some_and(|e| r.output.saturating_add(1) < e.serial)
            {
                return Err("diagnostic_gap_inconclusive");
            }
            let text = string(&s.params, "text", 1024)?;
            let equals = s.params.get("match").and_then(Value::as_str) == Some("equals");
            let matched = journal
                .entries
                .iter()
                .filter(|e| e.serial > r.output && e.object == object)
                .any(|e| if equals { e.text == text } else { e.text.contains(text) });
            if matched {
                if !s.expected {
                    return Err("unexpected_output");
                }
                r.output = journal.serial;
                r.assertions += 1;
                advance(r, true);
            } else if r.since.elapsed() >= Duration::from_secs(s.timeout) {
                if s.expected {
                    return Err("expected_output_unobserved");
                }
                r.output = journal.serial;
                r.assertions += 1;
                advance(r, true);
            }
        }
    }
    Ok(())
}
pub(super) fn tick(app: &mut App, assistant: &mut Assistant) {
    for (c, value) in std::mem::take(&mut assistant.inventory.testing.completed) {
        let _ = assistant.runtime.complete(&c, value);
    }
    let keys: Vec<_> = assistant.inventory.testing.runs.keys().copied().collect();
    for key in keys {
        let Some(mut r) = assistant.inventory.testing.runs.remove(&key) else {
            continue;
        };
        if r.state == "running" {
            if !source(app, assistant, &r) || !facts(app, &r) {
                end(&mut r, "cancelled", "source_or_native_authority_revoked");
            } else if Instant::now() >= r.deadline {
                end(&mut r, "inconclusive", "run_timeout");
            } else {
                let journal = &app.world.assistant_observations;
                let gap = !journal.enabled
                    || journal.epoch != r.journal_epoch
                    || journal.entries.front().is_some_and(|e| r.errors.saturating_add(1) < e.serial);
                let runtime_error = r.fail_errors
                    && journal
                        .entries
                        .iter()
                        .any(|e| e.serial > r.errors && e.error && r.objects.contains_key(&e.object));
                r.errors = journal.serial;
                if gap {
                    end(&mut r, "inconclusive", "diagnostic_gap");
                } else if runtime_error {
                    end(&mut r, "failed", "runtime_error_observed");
                } else if let Err(reason) = step(app, assistant, &mut r) {
                    let state = if matches!(
                        reason,
                        "running_assertion_failed" | "contents_assertion_failed" | "unexpected_output"
                    ) {
                        "failed"
                    } else {
                        "inconclusive"
                    };
                    end(&mut r, state, reason);
                }
            }
            if r.state != "running" {
                let value = report(key.2, &r);
                let _ = assistant.runtime.complete(&r.capture, value);
            }
        }
        assistant.inventory.testing.runs.insert(key, r);
    }
}
