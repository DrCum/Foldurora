//! Native event-thread owner of one character's assistant. Host preferences do
//! not enter this module. IO and endpoint lifetime run on a bounded worker.
use super::{App, Screen};
use aurora_mcp::assistant::{
    permissions::{Class, Level},
    runtime::{Capture, Runtime},
    tools::{DEFINITIONS, Tool},
};
use aurora_session::{AccountKey, Binding, HostId, SessionId, SessionIdentity, WorkerId, WorkerIdentity};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

mod creative;
mod inventory;
mod io;
mod preferences;
mod ui;
mod workspaces;
mod world;

pub(super) struct Assistant {
    pub(super) runtime: Runtime,
    pub(super) binding: Option<Binding>,
    native_login: Option<Uuid>,
    standalone_host: HostId,
    generation: u64,
    pub(super) open: bool,
    io: io::Owner,
    wanted: Option<(u64, Binding)>,
    activated: Option<u64>,
    scopes: BTreeMap<String, Option<crate::session_host::worker::action::NativeActionScope>>,
    dynamic_active: BTreeSet<String>,
    evidence: BTreeMap<String, Value>,
    native_policy: Option<[u64; 5]>,
    announced: BTreeSet<String>,
    inventory: inventory::State,
    world: world::State,
    creative: creative::State,
    preferences: preferences::State,
    workspaces: workspaces::State,
}
impl Default for Assistant {
    fn default() -> Self {
        Self::with_io(io::Owner::new())
    }
}
impl Assistant {
    fn with_io(io: io::Owner) -> Self {
        Self {
            runtime: Runtime::default(),
            binding: None,
            native_login: None,
            standalone_host: HostId::from_bytes(*Uuid::new_v4().as_bytes()).expect("UUIDv4 is nonzero"),
            generation: 0,
            open: false,
            io,
            wanted: None,
            activated: None,
            scopes: BTreeMap::new(),
            dynamic_active: BTreeSet::new(),
            evidence: BTreeMap::new(),
            native_policy: None,
            announced: BTreeSet::new(),
            inventory: Default::default(),
            world: Default::default(),
            creative: Default::default(),
            preferences: Default::default(),
            workspaces: Default::default(),
        }
    }
}
impl Assistant {
    fn disable(&mut self) {
        self.io.stop();
        self.runtime.disable();
        self.wanted = None;
        self.activated = None;
        self.scopes.clear();
        self.dynamic_active.clear();
        self.evidence.clear();
        self.announced.clear();
    }
    fn source(&mut self, app: &App) -> Option<Binding> {
        let login = app.world.login.as_ref().filter(|l| {
            !l.agent_id.is_nil()
                && !l.session_id.is_nil()
                && l.agent_id == app.world.agent_id
                && (matches!(app.screen, Screen::World) || (self.binding.is_some() && app.world.teleporting))
                && !app.demo
        });
        let raw = login.map(|l| l.session_id);
        if raw != self.native_login {
            self.disable();
            self.native_login = raw;
            self.generation = self.generation.checked_add(1)?;
        }
        login?;
        let account = AccountKey::parse_hex(&crate::characters::identity(&app.settings.login_uri(), &app.settings.username)).ok()?;
        let grid = crate::session_host::transport::grid_key(&app.settings.login_uri()).ok()?;
        if let Some(bridge) = &app.host_session {
            let (binding, _lease) = bridge.bound_session()?;
            return binding.session.filter(|s| s.account == account && s.grid == grid).map(|_| binding);
        }
        let installation = self.io.snapshot().installation?;
        let old = self
            .binding
            .and_then(|b| b.session)
            .filter(|s| s.account == account && s.grid == grid && s.generation == self.generation);
        let session = old.unwrap_or_else(|| SessionIdentity {
            session: SessionId::from_bytes(*Uuid::new_v4().as_bytes()).expect("UUIDv4 nonzero"),
            account,
            grid,
            generation: self.generation,
        });
        Some(Binding {
            worker: WorkerIdentity {
                installation,
                host: self.standalone_host,
                worker: WorkerId::from_bytes(*app.characters.own_instance().as_bytes()).ok()?,
            },
            session: Some(session),
        })
    }
    fn tick(&mut self, app: &mut App) {
        app.world.rlva_configure(app.settings.rlva_enabled);
        app.world.rlva.preferences.clone_from(&app.settings.rlva);
        let policy = world::policy_stamp(app);
        world::observe_policy(&self.runtime, &mut self.native_policy, policy);
        let source = self.source(app);
        if source != self.binding {
            self.disable();
            self.binding = source;
        }
        let active = active_character_allowed(app);
        self.runtime.observe(source, active);
        if self.runtime.configure(app.settings.local_assistant.clone()).is_err() {
            self.disable();
            world::retire(app, self);
            inventory::retire(app, self);
            preferences::tick(app, self);
            workspaces::tick(app, self);
            creative::tick(app, self);
            return;
        }
        self.runtime.capabilities(DEFINITIONS.iter().map(|d| d.tool).filter(|tool| {
            inventory::supports(*tool)
                || world::supports(*tool)
                || creative::supports(*tool)
                || preferences::supports(*tool)
                || workspaces::supports(*tool)
        }));
        self.scopes.retain(|id, _| self.runtime.has_operation(id));
        self.dynamic_active.retain(|id| self.runtime.has_operation(id));
        self.evidence.retain(|id, _| self.runtime.has_operation(id));
        self.announced.retain(|id| self.runtime.has_operation(id));
        for approval in self.runtime.snapshot().approvals {
            if self.announced.insert(approval.capture.request_id) {
                self.open = true;
            }
        }
        if let Some((wanted, binding)) = self.wanted {
            let state = self.io.snapshot();
            if state.failed {
                self.disable();
            } else if self.activated != Some(wanted)
                && let Some((token, endpoint)) = state.ready
                && token == wanted
            {
                if self.runtime.enable(binding, endpoint).is_ok() {
                    self.activated = Some(token);
                } else {
                    self.disable();
                }
            }
        }
        inventory::tick(app, self);
        world::tick(app, self);
        creative::tick(app, self);
        preferences::tick(app, self);
        workspaces::tick(app, self);
        for capture in self.runtime.preparations() {
            if !binding_matches(app, self, &capture) {
                self.runtime.reject(&capture.request_id, "stale_source");
                continue;
            }
            if inventory::supports(capture.tool) && !creative::handles(&self.creative, &capture) {
                match inventory::preparation(app, self, &capture) {
                    Ok(false) => continue,
                    Err(code) => {
                        self.runtime.reject(&capture.request_id, code);
                        continue;
                    }
                    Ok(true) => {}
                }
            }
            if workspaces::supports(capture.tool) {
                match workspaces::preparation(app, self, &capture) {
                    Ok(false) => continue,
                    Err(code) => {
                        self.runtime.reject(&capture.request_id, code);
                        continue;
                    }
                    Ok(true) => {}
                }
            }
            match prepare(app, self, &capture) {
                Ok(extra) => {
                    let original = if creative::handles(&self.creative, &capture) {
                        creative::original_receipt(&self.creative, &capture)
                    } else {
                        inventory::original_receipt(&self.inventory, &capture)
                    };
                    match original {
                        Ok(Some(original)) => {
                            if let Err(code) = self.runtime.bind_original_receipt(&capture, &original) {
                                self.runtime.reject(&capture.request_id, code);
                                continue;
                            }
                        }
                        Ok(None) => {}
                        Err(code) => {
                            self.runtime.reject(&capture.request_id, code);
                            continue;
                        }
                    }
                    let requires_active = capture.tool.definition().active || preferences::requires_active(&self.preferences, &capture);
                    if requires_active {
                        self.dynamic_active.insert(capture.request_id.clone());
                    }
                    let scope = if app.host_session.is_some() && requires_active {
                        app.host_session.as_ref().and_then(|b| b.native_action_scope())
                    } else {
                        None
                    };
                    if app.host_session.is_some() && requires_active && scope.is_none() {
                        self.runtime.reject(&capture.request_id, "inactive_character");
                        continue;
                    }
                    self.scopes.insert(capture.request_id.clone(), scope);
                    if inventory::supports(capture.tool) && !creative::handles(&self.creative, &capture) {
                        self.evidence.insert(
                            capture.request_id.clone(),
                            inventory::approval_evidence(app, &self.inventory, &capture),
                        );
                    }
                    if world::supports(capture.tool) {
                        match world::evidence(app, &self.world, &capture) {
                            Ok(evidence) => {
                                self.evidence.insert(capture.request_id.clone(), evidence);
                            }
                            Err(code) => {
                                self.runtime.reject(&capture.request_id, code);
                                continue;
                            }
                        }
                    }
                    if creative::handles(&self.creative, &capture) {
                        match creative::evidence(app, &self.creative, &capture) {
                            Ok(evidence) => {
                                self.evidence.insert(capture.request_id.clone(), evidence);
                            }
                            Err(code) => {
                                self.runtime.reject(&capture.request_id, code);
                                continue;
                            }
                        }
                    }
                    if preferences::supports(capture.tool) {
                        match preferences::evidence(app, &self.preferences, &capture) {
                            Ok(evidence) => {
                                self.evidence.insert(capture.request_id.clone(), evidence);
                            }
                            Err(code) => {
                                self.runtime.reject(&capture.request_id, code);
                                continue;
                            }
                        }
                    }
                    if workspaces::supports(capture.tool) {
                        match workspaces::evidence(app, &self.workspaces, &capture) {
                            Ok(evidence) => {
                                self.evidence.insert(capture.request_id.clone(), evidence);
                            }
                            Err(code) => {
                                self.runtime.reject(&capture.request_id, code);
                                continue;
                            }
                        }
                    }
                    let force_review = inventory::force_review(&self.inventory, &capture);
                    let _ = self
                        .runtime
                        .prepared_with_review(&capture.request_id, &extra, requires_active, force_review);
                }
                Err(code) => self.runtime.reject(&capture.request_id, code),
            }
        }
        for capture in self.runtime.ready() {
            let original_scope = self.scopes.get(&capture.request_id).copied();
            let scope_ok = match original_scope {
                Some(Some(scope)) => app.host_session.as_ref().is_some_and(|b| b.native_action_current(scope)),
                Some(None) => true,
                None => false,
            };
            if !scope_ok
                || !binding_matches(app, self, &capture)
                || self.dynamic_active.contains(&capture.request_id) && !active_allowed(app)
            {
                self.runtime.reject(&capture.request_id, "stale_native_scope");
                continue;
            }
            let extra = match prepare(app, self, &capture) {
                Ok(extra) => extra,
                Err(code) => {
                    self.runtime.reject(&capture.request_id, code);
                    continue;
                }
            };
            if extra.iter().any(|c| !capture.required_classes.contains(c)) {
                self.runtime.reject(&capture.request_id, "native_policy_changed");
                continue;
            }
            if world::supports(capture.tool)
                && world::evidence(app, &self.world, &capture).ok().as_ref() != self.evidence.get(&capture.request_id)
            {
                self.runtime.reject(&capture.request_id, "native_evidence_changed");
                continue;
            }
            if creative::handles(&self.creative, &capture)
                && creative::evidence(app, &self.creative, &capture).ok().as_ref() != self.evidence.get(&capture.request_id)
            {
                self.runtime.reject(&capture.request_id, "native_evidence_changed");
                continue;
            }
            if preferences::supports(capture.tool)
                && preferences::evidence(app, &self.preferences, &capture).ok().as_ref() != self.evidence.get(&capture.request_id)
            {
                self.runtime.reject(&capture.request_id, "native_evidence_changed");
                continue;
            }
            if workspaces::supports(capture.tool)
                && workspaces::evidence(app, &self.workspaces, &capture).ok().as_ref() != self.evidence.get(&capture.request_id)
            {
                self.runtime.reject(&capture.request_id, "native_evidence_changed");
                continue;
            }
            let engine = self.runtime.clone();
            let _ = engine.execute(&capture.request_id, |capture| execute(app, self, capture));
        }
    }
}
impl App {
    /// Called after ordinary Poser layers are collected and before scene pose
    /// evaluation, even when Local Assistant access is disabled or paused.
    pub(super) fn sync_creative_pose_layers(&mut self) {
        if let Some(mut assistant) = self.character_mcp.take() {
            preferences::tick(self, &mut assistant);
            workspaces::tick(self, &mut assistant);
            creative::scene(self, &mut assistant);
            self.character_mcp = Some(assistant);
        }
    }
    pub(super) fn begin_character_update(&self) {
        if let Some(assistant) = &self.character_mcp {
            assistant.runtime.begin_native_update();
        }
    }
    pub(super) fn end_character_update(&self) {
        if let Some(assistant) = &self.character_mcp {
            for capture in assistant.runtime.protected_receipt_descriptors() {
                let current = binding_matches(self, assistant, &capture)
                    && if creative::handles(&assistant.creative, &capture) {
                        creative::validate_receipt(self, &assistant.creative, &capture)
                    } else if preferences::supports(capture.tool) {
                        preferences::prepare(self, &assistant.preferences, &capture).is_ok() || !capture.tool.definition().read_only
                    } else if workspaces::supports(capture.tool) {
                        workspaces::prepare(self, &assistant.workspaces, &capture).is_ok() || !capture.tool.definition().read_only
                    } else if inventory::supports(capture.tool) {
                        inventory::validate_receipt(self, &assistant.inventory, &capture)
                    } else if world::supports(capture.tool) {
                        world::validate_receipt(self, &assistant.world, &capture)
                    } else {
                        true
                    };
                if !current {
                    let _ = assistant.runtime.reject_withheld(&capture);
                }
            }
            assistant.runtime.end_native_update();
        }
    }
    pub(super) fn revoke_character_actions(&self) {
        if let Some(assistant) = &self.character_mcp {
            assistant.runtime.revoke_active();
        }
    }
    pub(super) fn tick_character_mcp(&mut self) {
        let Some(mut assistant) = self.character_mcp.take() else {
            return;
        };
        assistant.tick(self);
        self.character_mcp = Some(assistant);
    }
    pub(super) fn show_character_mcp(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        let Some(mut assistant) = self.character_mcp.take() else {
            return;
        };
        let changed = ui::show(ctx, p, &mut assistant, &mut self.settings.local_assistant, &self.settings.username);
        if changed {
            if assistant.runtime.configure(self.settings.local_assistant.clone()).is_err() {
                assistant.disable();
            }
            self.settings.save();
        }
        self.character_mcp = Some(assistant);
    }
}
fn active_allowed(app: &App) -> bool {
    active_character_allowed(app) && !app.world.teleporting
}
fn active_character_allowed(app: &App) -> bool {
    app.gfx.is_some()
        && !app.exit_prompt.open
        && !app.login_form.busy
        && app.options_ui.capture.is_none()
        && match &app.host_session {
            Some(b) => b.native_action_scope().is_some(),
            None => app.focused,
        }
}
fn binding_matches(app: &App, assistant: &Assistant, capture: &Capture) -> bool {
    assistant.binding == Some(capture.binding)
        && assistant.runtime.snapshot().viewer_session == Some(capture.endpoint)
        && (matches!(app.screen, Screen::World) || app.world.teleporting)
        && app
            .world
            .login
            .as_ref()
            .is_some_and(|l| Some(l.session_id) == assistant.native_login && l.agent_id == app.world.agent_id)
        && capture.binding.session.is_some_and(|s| {
            AccountKey::parse_hex(&crate::characters::identity(&app.settings.login_uri(), &app.settings.username)).ok() == Some(s.account)
                && crate::session_host::transport::grid_key(&app.settings.login_uri()).ok() == Some(s.grid)
        })
}
/// Native asynchronous continuations must call this before additional effects.
pub(super) fn continuation_current(app: &App, capture: &Capture) -> bool {
    app.character_mcp.as_ref().is_some_and(|assistant| {
        binding_matches(app, assistant, capture)
            && assistant.runtime.configuration().as_ref() == Some(&app.settings.local_assistant)
            && assistant.runtime.capture_current(capture)
            && original_scope_current(app, assistant, capture)
    })
}
fn original_scope_current(app: &App, assistant: &Assistant, capture: &Capture) -> bool {
    if !capture.tool.definition().active && !assistant.dynamic_active.contains(&capture.request_id) {
        return true;
    }
    (if capture.tool == Tool::NavigationTeleport {
        active_character_allowed(app)
    } else {
        active_allowed(app)
    }) && match assistant.scopes.get(&capture.request_id) {
        Some(Some(scope)) => app.host_session.as_ref().is_some_and(|b| b.native_action_current(*scope)),
        Some(None) => app.host_session.is_none(),
        None => false,
    }
}
pub(super) fn world_native_command(app: &mut App, command: &aurora_net::NetCommand) {
    world::native_command(app, command);
}
pub(super) fn snapshot_current(app: &App, id: Uuid) -> bool {
    world::snapshot_current(app, id) || inventory::snapshot_current(app, id)
}
pub(super) fn snapshot_pixels(app: &mut App, id: Uuid, size: (u32, u32), rgba: Vec<u8>) {
    if inventory::snapshot_current(app, id) {
        inventory::snapshot_pixels(app, id, size, rgba);
    } else {
        world::snapshot_pixels(app, id, size, rgba);
    }
}
pub(super) fn source_current(app: &App, capture: &Capture) -> bool {
    app.character_mcp
        .as_ref()
        .is_some_and(|assistant| binding_matches(app, assistant, capture))
}
pub(super) fn world_native_event(app: &mut App, event: &aurora_net::NetEvent) -> bool {
    world::native_event(app, event)
}
pub(super) fn inventory_native_event(app: &mut App, event: &aurora_net::NetEvent) -> bool {
    inventory::native_event(app, event)
}
pub(super) fn inventory_continuation(app: &App, request: Uuid) -> bool {
    inventory::continuation(app, request)
}
pub(super) fn inventory_native_command(app: &mut App, command: &aurora_net::NetCommand) {
    inventory::native_command(app, command);
}
fn prepare(app: &mut App, assistant: &Assistant, capture: &Capture) -> Result<Vec<Class>, &'static str> {
    if workspaces::supports(capture.tool) {
        workspaces::prepare(app, &assistant.workspaces, capture)
    } else if preferences::supports(capture.tool) {
        preferences::prepare(app, &assistant.preferences, capture)
    } else if creative::handles(&assistant.creative, capture) {
        creative::prepare(app, &assistant.creative, capture)
    } else if inventory::supports(capture.tool) {
        inventory::prepare(app, &assistant.inventory, capture)
    } else if world::supports(capture.tool) {
        world::prepare(app, &assistant.world, capture)
    } else if creative::supports(capture.tool) {
        creative::prepare(app, &assistant.creative, capture)
    } else {
        Err("unsupported_capability")
    }
}
fn execute(app: &mut App, assistant: &mut Assistant, capture: &Capture) -> Value {
    if workspaces::supports(capture.tool) {
        workspaces::execute(app, &mut assistant.workspaces, capture).unwrap_or_else(failure)
    } else if preferences::supports(capture.tool) {
        preferences::execute(app, &mut assistant.preferences, capture).unwrap_or_else(failure)
    } else if creative::handles(&assistant.creative, capture) {
        creative::execute(app, assistant, capture)
    } else if inventory::supports(capture.tool) {
        inventory::execute(app, assistant, capture)
    } else if world::supports(capture.tool) {
        world::execute(app, assistant, capture)
    } else if creative::supports(capture.tool) {
        creative::execute(app, assistant, capture)
    } else {
        failure("unsupported_capability")
    }
}
pub(super) fn workspace_placement_current(app: &App, operation: Uuid) -> bool {
    app.character_mcp
        .as_ref()
        .is_some_and(|assistant| workspaces::placement_current(app, assistant, operation))
}
pub(super) fn preferences_native_opening(app: &mut App) {
    if let Some(mut assistant) = app.character_mcp.take() {
        preferences::native_opening(app, &mut assistant);
        app.character_mcp = Some(assistant);
    }
}
pub(super) fn show_native_shared_pose(app: &mut App, ctx: &egui::Context, p: &crate::theme::Palette) {
    if let Some(mut assistant) = app.character_mcp.take() {
        creative::ui(app, &mut assistant.creative, ctx, p);
        app.character_mcp = Some(assistant);
    }
}
fn failure(code: &'static str) -> Value {
    aurora_mcp::wire::error(code)
}
fn string<'a>(args: &'a serde_json::Map<String, Value>, key: &str, max: usize) -> Result<&'a str, &'static str> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| s.len() <= max && !s.chars().any(|c| c == '\0'))
        .ok_or("invalid_args")
}
fn uuid(args: &serde_json::Map<String, Value>, key: &str) -> Result<Uuid, &'static str> {
    Uuid::parse_str(string(args, key, 36)?)
        .ok()
        .filter(|id| !id.is_nil())
        .ok_or("invalid_args")
}
fn allow_keys(args: &serde_json::Map<String, Value>, keys: &[&str]) -> Result<(), &'static str> {
    if args.keys().any(|k| !keys.contains(&k.as_str())) {
        Err("invalid_args")
    } else {
        Ok(())
    }
}
fn limit(args: &serde_json::Map<String, Value>, default: usize, maximum: usize) -> Result<usize, &'static str> {
    match args.get("limit") {
        None => Ok(default),
        Some(n) => n
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| *n > 0 && *n <= maximum)
            .ok_or("invalid_args"),
    }
}

#[cfg(test)]
mod catalogue_tests {
    use super::*;
    #[test]
    fn every_character_tool_has_one_native_adapter_or_closed_runtime_disposition() {
        use aurora_mcp::assistant::tools::DEFINITIONS;
        assert_eq!(DEFINITIONS.len(), 153);
        let mut names = std::collections::BTreeSet::new();
        let mut counts = [0usize; 5];
        for definition in DEFINITIONS {
            let tool = definition.tool;
            assert!(names.insert(definition.name), "duplicate tool name");
            let adapters = [
                inventory::supports(tool),
                world::supports(tool),
                creative::supports(tool),
                preferences::supports(tool),
                workspaces::supports(tool),
            ];
            let transport = matches!(
                tool,
                Tool::ViewerList | Tool::ViewerStatus | Tool::ViewerSelect | Tool::AssistantOperationStatus
            );
            let denied = matches!(tool, Tool::ConfirmAction | Tool::InventoryPurge | Tool::InventoryEmptyTrash);
            assert_eq!(
                adapters.iter().filter(|v| **v).count() + usize::from(transport) + usize::from(denied),
                1,
                "missing or ambiguous native disposition for {}",
                definition.name
            );
            for (i, enabled) in adapters.into_iter().enumerate() {
                counts[i] += usize::from(enabled);
            }
        }
        assert_eq!(counts, [74, 36, 25, 6, 5]);
        assert!(Tool::ConfirmAction.native_confirmation_only());
        assert!(Tool::InventoryPurge.permanently_denied());
        assert!(Tool::InventoryEmptyTrash.permanently_denied());
    }
}
