//! Login-bound native preparation, approval and execution. IPC may enqueue an
//! intent; only the owning native thread prepares/executes it. A permission
//! change, disconnect or new login invalidates every old capture.
use super::{
    permissions::{Class, Configuration, Level},
    tools::Tool,
};
use crate::wire::{ClientId, error};
use aurora_session::{Binding, HostId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

pub const MAX_PENDING: usize = 128;
pub const MAX_QUEUED: usize = 16;
pub const MAX_ARGUMENT_BYTES: usize = 48 * 1024;
pub const MAX_RESULT_BYTES: usize = 24 * 1024;
const LIFETIME: Duration = Duration::from_secs(70);
const FRESH: Duration = Duration::from_secs(5);
const RETAIN: Duration = Duration::from_secs(600);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum ViewerRequest {
    Status {
        viewer_session: HostId,
    },
    Invoke {
        viewer_session: HostId,
        tool: Tool,
        arguments: serde_json::Map<String, Value>,
        request_id: Option<String>,
    },
    OperationStatus {
        viewer_session: HostId,
        request_id: String,
    },
    Cancel {
        viewer_session: HostId,
        request_id: String,
    },
    /// Private image bytes, addressed only by a native-produced opaque token.
    /// No filesystem path or arbitrary asset UUID is accepted.
    Artifact {
        viewer_session: HostId,
        request_id: String,
        artifact_id: String,
        offset: usize,
        length: usize,
    },
    /// Internal SDK image read: only the original approved CameraSnapshot's
    /// bounded native JPEG preview. No MIME, file path or image bytes input.
    Image {
        viewer_session: HostId,
        request_id: String,
        artifact_id: String,
    },
}
impl ViewerRequest {
    fn source(&self) -> HostId {
        match self {
            Self::Status { viewer_session }
            | Self::Invoke { viewer_session, .. }
            | Self::OperationStatus { viewer_session, .. }
            | Self::Cancel { viewer_session, .. } => *viewer_session,
            Self::Artifact { viewer_session, .. } | Self::Image { viewer_session, .. } => *viewer_session,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capture {
    /// Assigned by the authenticated transport, never a caller argument.
    pub client: ClientId,
    pub endpoint: HostId,
    pub request_id: String,
    pub binding: Binding,
    pub tool: Tool,
    pub arguments: serde_json::Map<String, Value>,
    pub policy_revision: u64,
    pub required_classes: Vec<Class>,
}

#[derive(Clone, Debug)]
pub struct Approval {
    pub capture: Capture,
    pub classes: Vec<Class>,
    pub expires_in: Duration,
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub enabled: bool,
    pub paused: bool,
    pub ready: bool,
    pub viewer_session: Option<HostId>,
    pub approvals: Vec<Approval>,
}

enum Stage {
    Preparation,
    Approval,
    Ready,
    Executing,
    Submitted(Value),
    Finished(Value),
    Denied(&'static str),
}
struct Operation {
    client: ClientId,
    capture: Capture,
    classes: Vec<Class>,
    active: bool,
    ever_submitted: bool,
    amendment: Option<Value>,
    original_receipt_source: Option<Capture>,
    stage: Stage,
    created: Instant,
    execution_deadline: Option<Instant>,
    finished: Option<Instant>,
    artifact: Option<Artifact>,
}
struct Artifact {
    id: uuid::Uuid,
    data: Vec<u8>,
    preview: Option<CameraPreview>,
}
pub const MAX_CAMERA_PREVIEW_BYTES: usize = 12 * 1024;
struct CameraPreview {
    width: u32,
    height: u32,
    jpeg: Vec<u8>,
}
struct State {
    observed: Option<(Binding, Instant)>,
    enabled_for: Option<Binding>,
    endpoint: Option<HostId>,
    active: bool,
    paused: bool,
    config: Configuration,
    configuration_valid: bool,
    revision: u64,
    capabilities: BTreeSet<Tool>,
    operations: BTreeMap<String, Operation>,
    native_busy: bool,
}

#[derive(Clone)]
pub struct Runtime(Arc<Mutex<State>>);
impl Default for Runtime {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(State {
            observed: None,
            enabled_for: None,
            endpoint: None,
            active: false,
            paused: false,
            config: Configuration::default(),
            configuration_valid: true,
            revision: 1,
            capabilities: BTreeSet::new(),
            operations: BTreeMap::new(),
            native_busy: false,
        })))
    }
}
impl Runtime {
    pub fn policy_revision(&self) -> u64 {
        self.0.lock().map_or(0, |s| s.revision)
    }
    /// Owning-thread observation admission, independent of the public delivery
    /// barrier held throughout a native frame. It grants no permission class.
    pub fn native_observation_ready(&self) -> bool {
        self.0.lock().is_ok_and(|s| s.ready(Instant::now()))
    }
    pub fn has_operation(&self, id: &str) -> bool {
        self.0.lock().is_ok_and(|s| s.operations.contains_key(id))
    }
    pub fn configuration(&self) -> Option<Configuration> {
        self.0.lock().ok().map(|s| s.config.clone())
    }
    pub fn capture_current(&self, capture: &Capture) -> bool {
        self.0.lock().is_ok_and(|s| {
            s.operations.get(&capture.request_id).is_some_and(|o| {
                o.capture == *capture && s.valid(o, Instant::now()) && matches!(o.stage, Stage::Executing | Stage::Submitted(_))
            })
        })
    }
    /// Owning native adapters may cache this exact delivery deadline while
    /// outside execute's mutex. It grants no action or result body access.
    pub fn retained_receipt_authority(&self, capture: &Capture) -> Option<Instant> {
        let s = self.0.lock().ok()?;
        let o = s.operations.get(&capture.request_id)?;
        if o.capture != *capture || !s.valid(o, Instant::now()) || !matches!(o.stage, Stage::Submitted(_) | Stage::Finished(_)) {
            return None;
        }
        o.execution_deadline.or_else(|| o.created.checked_add(LIFETIME))
    }
    /// Closed native status alias. Its source remains authoritative at execute
    /// and at every later receipt, including after an asynchronous IPC cancel.
    pub fn bind_original_receipt(&self, consumer: &Capture, original: &Capture) -> Result<(), &'static str> {
        let permitted = (consumer.tool == Tool::InventoryUploadStatus
            && matches!(
                original.tool,
                Tool::InventoryUploadPreview | Tool::InventoryUploadExecute | Tool::InventorySnapshotUpload
            ))
            || (consumer.tool == Tool::ScriptTestStatus && original.tool == Tool::ScriptTestBegin)
            || (consumer.tool == Tool::InventoryContentOperationStatus
                && matches!(
                    original.tool,
                    Tool::InventoryReadNotecard
                        | Tool::InventoryReadScript
                        | Tool::InventoryContentGet
                        | Tool::InventoryContentPreview
                        | Tool::InventoryUpdateNotecard
                        | Tool::InventorySaveCompileScript
                ))
            || (consumer.tool == Tool::ObjectOperationStatus
                && matches!(
                    original.tool,
                    Tool::ObjectContentsList
                        | Tool::ObjectScriptStatus
                        | Tool::ObjectScriptGet
                        | Tool::ObjectScriptPreview
                        | Tool::ObjectScriptSaveCompile
                        | Tool::ObjectContentsRename
                        | Tool::ObjectContentsRemove
                        | Tool::ObjectContentsPut
                        | Tool::ObjectContentsReconcile
                        | Tool::ObjectScriptSetRunning
                        | Tool::ObjectScriptReset
                        | Tool::ObjectTakeCopy
                        | Tool::ObjectTake
                        | Tool::ObjectTouch
                        | Tool::ObjectDialogReply
                        | Tool::ObjectChannelSend
                        | Tool::BuildRezPrimitive
                        | Tool::BuildRezInventory
                        | Tool::ObjectRezIdentify
                ));
        if !permitted
            || consumer.client != original.client
            || consumer.endpoint != original.endpoint
            || consumer.binding != original.binding
            || consumer.policy_revision != original.policy_revision
        {
            return Err("stale_original_receipt");
        }
        let mut s = self.0.lock().map_err(|_| "unavailable")?;
        let source = s.operations.get(&original.request_id).ok_or("stale_original_receipt")?;
        if source.capture != *original
            || source.original_receipt_source.is_some()
            || !s.valid(source, Instant::now())
            || !matches!(source.stage, Stage::Submitted(_) | Stage::Finished(_))
        {
            return Err("stale_original_receipt");
        }
        let target = s.operations.get(&consumer.request_id).ok_or("unknown_operation")?;
        if target.capture != *consumer || !s.valid(target, Instant::now()) || !matches!(target.stage, Stage::Preparation) {
            return Err("stale_source");
        }
        s.operations
            .get_mut(&consumer.request_id)
            .ok_or("unknown_operation")?
            .original_receipt_source = Some(original.clone());
        Ok(())
    }
    /// Native owners inspect retained result candidates while delivery is
    /// closed. No body crosses this interface; each adapter retains its own
    /// immutable evidence for any protected body it produced.
    pub fn protected_receipt_descriptors(&self) -> Vec<Capture> {
        self.0.lock().map_or_else(
            |_| Vec::new(),
            |s| {
                s.operations
                    .values()
                    .filter(|o| matches!(o.stage, Stage::Submitted(_) | Stage::Finished(_)))
                    .take(MAX_PENDING)
                    .map(|o| o.capture.clone())
                    .collect()
            },
        )
    }
    /// Withhold only this exact result. An old or foreign capture cannot retire
    /// another caller's receipt, and no native mutation is replayed or undone.
    pub fn reject_withheld(&self, capture: &Capture) -> Result<(), &'static str> {
        let mut s = self.0.lock().map_err(|_| "unavailable")?;
        let o = s.operations.get_mut(&capture.request_id).ok_or("unknown_operation")?;
        if o.capture != *capture || !matches!(o.stage, Stage::Submitted(_) | Stage::Finished(_)) {
            return Err("stale_source");
        }
        o.stage = Stage::Denied("native_result_access_revoked");
        o.artifact = None;
        o.finished = Some(Instant::now());
        Ok(())
    }
    /// Must come from the owning event thread's current actual login. Never use
    /// a GUI cache or profile slot to refresh this observation.
    pub fn observe(&self, binding: Option<Binding>, active: bool) {
        if let Ok(mut s) = self.0.lock() {
            let valid = binding.filter(|b| b.validate().is_ok() && b.session.is_some());
            if s.observed.as_ref().map(|(b, _)| *b) != valid || !s.fresh(Instant::now()) {
                s.disable();
            }
            s.observed = valid.map(|b| (b, Instant::now()));
            if s.active && !active {
                s.revoke_active();
            }
            s.active = active;
            s.prune();
        }
    }
    /// Native state is changing. IPC can observe health, but cannot retrieve a
    /// cached private result until the owning thread publishes its new policy.
    pub fn begin_native_update(&self) {
        if let Ok(mut s) = self.0.lock() {
            s.native_busy = true;
        }
    }
    pub fn end_native_update(&self) {
        if let Ok(mut s) = self.0.lock() {
            s.native_busy = false;
        }
    }
    pub fn native_policy_changed(&self) {
        if let Ok(mut s) = self.0.lock() {
            s.invalidate("native_policy_changed");
        }
    }
    pub fn revoke_active(&self) {
        if let Ok(mut s) = self.0.lock() {
            s.revoke_active();
        }
    }
    /// Linearize a bounded asynchronous native commit against authenticated
    /// disconnect, cancellation and permission revocation. The closure cannot
    /// call Runtime, wait, perform IO, or retain a viewer reference.
    pub fn continue_action<R>(&self, capture: &Capture, commit: impl FnOnce() -> R) -> Result<R, &'static str> {
        let s = self.0.lock().map_err(|_| "unavailable")?;
        let operation = s.operations.get(&capture.request_id).ok_or("unknown_operation")?;
        if operation.capture != *capture || !s.valid(operation, Instant::now()) || !matches!(operation.stage, Stage::Submitted(_)) {
            return Err("stale_source");
        }
        Ok(commit())
    }
    /// Trusted native preparation may load bounded inventory metadata only
    /// under explicit Read=Allow. Mutation authority stays pending until the
    /// loaded scope has been frozen and presented for its actual approval.
    pub fn continue_preparation<R>(&self, capture: &Capture, class: Class, commit: impl FnOnce() -> R) -> Result<R, &'static str> {
        let s = self.0.lock().map_err(|_| "unavailable")?;
        let operation = s.operations.get(&capture.request_id).ok_or("unknown_operation")?;
        if class != Class::Read || s.config.permissions.level(class) != Level::Allow {
            return Err("preparation_read_not_allowed");
        }
        if operation.capture != *capture || !s.valid(operation, Instant::now()) || !matches!(operation.stage, Stage::Preparation) {
            return Err("stale_source");
        }
        Ok(commit())
    }
    /// A newly approved, source-bound reconciliation may amend factual state
    /// of an earlier submitted Contents copy. It cannot restore old action
    /// authority, replay the copy or extend either operation's lifetime.
    pub fn amend_reconciled(&self, original: &Capture, reconcile: &Capture, value: Value) -> Result<(), &'static str> {
        if original.tool != Tool::ObjectContentsPut
            || reconcile.tool != Tool::ObjectContentsReconcile
            || original.client != reconcile.client
            || original.endpoint != reconcile.endpoint
            || original.binding != reconcile.binding
        {
            return Err("stale_reconciliation");
        }
        if serde_json::to_vec(&value).map_or(true, |bytes| bytes.len() > MAX_RESULT_BYTES) {
            return Err("response_limit");
        }
        let mut s = self.0.lock().map_err(|_| "unavailable")?;
        let current = s.operations.get(&reconcile.request_id).ok_or("unknown_operation")?;
        if current.capture != *reconcile || !s.valid(current, Instant::now()) || !matches!(current.stage, Stage::Finished(_)) {
            return Err("stale_reconciliation");
        }
        let old = s.operations.get_mut(&original.request_id).ok_or("unknown_operation")?;
        if old.capture != *original || !old.ever_submitted {
            return Err("invalid_original_operation");
        }
        match &mut old.stage {
            Stage::Submitted(body) | Stage::Finished(body) => *body = value.clone(),
            Stage::Denied(_) => {}
            _ => return Err("invalid_original_operation"),
        }
        old.amendment = Some(value);
        Ok(())
    }
    pub fn install_artifact(&self, capture: &Capture, id: uuid::Uuid, data: Vec<u8>) -> Result<(), &'static str> {
        if id.is_nil() || data.is_empty() || data.len() > 8 * 1024 * 1024 {
            return Err("artifact_limit");
        }
        let mut s = self.0.lock().map_err(|_| "unavailable")?;
        let operation = s.operations.get(&capture.request_id).ok_or("unknown_operation")?;
        if operation.capture != *capture || !s.valid(operation, Instant::now()) || !matches!(operation.stage, Stage::Submitted(_)) {
            return Err("stale_source");
        }
        if s.operations.values().filter(|o| o.artifact.is_some()).count() >= 2 {
            return Err("artifact_capacity");
        }
        s.operations.get_mut(&capture.request_id).ok_or("unknown_operation")?.artifact = Some(Artifact { id, data, preview: None });
        Ok(())
    }
    pub fn install_camera_preview(
        &self,
        capture: &Capture,
        id: uuid::Uuid,
        width: u32,
        height: u32,
        jpeg: Vec<u8>,
    ) -> Result<(), &'static str> {
        if capture.tool != Tool::CameraSnapshot
            || width == 0
            || height == 0
            || width > 256
            || height > 256
            || jpeg.len() > MAX_CAMERA_PREVIEW_BYTES
            || !jpeg.starts_with(&[255, 216])
            || !jpeg.ends_with(&[255, 217])
        {
            return Err("camera_preview_limit");
        }
        let mut s = self.0.lock().map_err(|_| "unavailable")?;
        let operation = s.operations.get_mut(&capture.request_id).ok_or("unknown_operation")?;
        if operation.capture != *capture {
            return Err("stale_source");
        }
        // Validate without exposing a reference that can outlive this lock.
        if !s
            .operations
            .get(&capture.request_id)
            .is_some_and(|o| s.valid(o, Instant::now()) && matches!(o.stage, Stage::Submitted(_)))
        {
            return Err("stale_source");
        }
        let artifact = s
            .operations
            .get_mut(&capture.request_id)
            .and_then(|o| o.artifact.as_mut())
            .filter(|a| a.id == id)
            .ok_or("artifact_unavailable")?;
        artifact.preview = Some(CameraPreview { width, height, jpeg });
        Ok(())
    }
    /// Explicit native user enablement for the currently observed login only.
    /// Endpoint creation and publication occur only after this succeeds.
    pub fn enable(&self, binding: Binding, endpoint: HostId) -> Result<(), &'static str> {
        let mut s = self.0.lock().map_err(|_| "unavailable")?;
        if !s.configuration_valid {
            return Err("invalid_permissions");
        }
        if !s.fresh(Instant::now()) || s.observed.as_ref().map(|(b, _)| *b) != Some(binding) {
            return Err("stale_source");
        }
        s.disable();
        s.enabled_for = Some(binding);
        s.endpoint = Some(endpoint);
        Ok(())
    }
    pub fn disable(&self) {
        if let Ok(mut s) = self.0.lock() {
            s.disable();
        }
    }
    pub fn pause(&self, paused: bool) {
        if let Ok(mut s) = self.0.lock()
            && s.paused != paused
        {
            s.paused = paused;
            s.invalidate("policy_changed");
        }
    }
    pub fn configure(&self, configuration: Configuration) -> Result<(), &'static str> {
        let mut s = self.0.lock().map_err(|_| "unavailable")?;
        if !configuration.permissions.validate() {
            // Malformed persisted permissions must never retain an earlier
            // Allow policy or permit re-enablement with default permissions.
            s.configuration_valid = false;
            s.disable();
            return Err("invalid_permissions");
        }
        if !s.configuration_valid || s.config != configuration {
            s.configuration_valid = true;
            s.config = configuration;
            s.invalidate("policy_changed");
        }
        Ok(())
    }
    /// The adapter advertises only native implementations it actually joins.
    pub fn capabilities(&self, tools: impl IntoIterator<Item = Tool>) {
        if let Ok(mut s) = self.0.lock() {
            let next: BTreeSet<_> = tools
                .into_iter()
                .filter(|t| !t.permanently_denied() && !t.native_confirmation_only())
                .collect();
            if s.capabilities != next {
                s.capabilities = next;
                s.invalidate("capability_changed");
            }
        }
    }
    pub fn snapshot(&self) -> Snapshot {
        let Ok(mut s) = self.0.lock() else {
            return Snapshot {
                enabled: false,
                paused: false,
                ready: false,
                viewer_session: None,
                approvals: vec![],
            };
        };
        s.prune();
        let now = Instant::now();
        Snapshot {
            enabled: s.enabled_for.is_some(),
            paused: s.paused,
            ready: s.ready(now) && !s.native_busy,
            viewer_session: s.endpoint,
            approvals: s
                .operations
                .values()
                .filter(|o| matches!(o.stage, Stage::Approval) && s.valid(o, now))
                .map(|o| Approval {
                    capture: o.capture.clone(),
                    classes: o.classes.clone(),
                    expires_in: LIFETIME.saturating_sub(now.saturating_duration_since(o.created)),
                })
                .collect(),
        }
    }
    pub fn request(&self, client: ClientId, request: ViewerRequest) -> Value {
        match self.0.lock() {
            Ok(mut s) => s.request(client, request),
            Err(_) => error("unavailable"),
        }
    }
    pub fn disconnected(&self, client: ClientId) {
        if let Ok(mut s) = self.0.lock() {
            s.operations.retain(|_, o| o.client != client);
        }
    }
    /// A preparation may inspect current typed native state but must not mutate,
    /// fetch protected assets, or return private data before permission checks.
    pub fn preparations(&self) -> Vec<Capture> {
        let Ok(mut s) = self.0.lock() else {
            return vec![];
        };
        s.prune();
        let now = Instant::now();
        s.operations
            .values()
            .filter(|o| matches!(o.stage, Stage::Preparation) && s.valid(o, now))
            .take(4)
            .map(|o| o.capture.clone())
            .collect()
    }
    /// Required extra classes come only from native preparation (for example a
    /// group Chat recipient), never from remote claims about recipient type.
    pub fn prepared(&self, id: &str, extra: &[Class], requires_active: bool) -> Result<(), &'static str> {
        self.prepared_with_review(id, extra, requires_active, false)
    }
    /// Trusted native preparation may require a real viewer review of a frozen
    /// spending plan even when its ordinary permission classes are Allow.
    /// This method is not an IPC operation and cannot resolve the review.
    pub fn prepared_with_review(
        &self,
        id: &str,
        extra: &[Class],
        requires_active: bool,
        force_native_review: bool,
    ) -> Result<(), &'static str> {
        let mut s = self.0.lock().map_err(|_| "unavailable")?;
        let now = Instant::now();
        let o = s.operations.get(id).ok_or("unknown_operation")?;
        if !matches!(o.stage, Stage::Preparation) || !s.valid(o, now) {
            return Err("stale_source");
        }
        let mut classes = o.classes.clone();
        for class in extra {
            if !classes.contains(class) {
                classes.push(*class);
            }
        }
        let denied = classes.iter().any(|c| s.config.permissions.level(*c) == Level::Never);
        let ask = force_native_review || classes.iter().any(|c| s.config.permissions.level(*c) == Level::Ask);
        let active = o.active || requires_active;
        let unavailable = active && !s.active;
        let o = s.operations.get_mut(id).ok_or("unknown_operation")?;
        o.classes = classes;
        o.capture.required_classes = o.classes.clone();
        o.active = active;
        o.stage = if denied {
            Stage::Denied("permission_denied")
        } else if unavailable {
            Stage::Denied("inactive_character")
        } else if ask {
            Stage::Approval
        } else {
            Stage::Ready
        };
        if denied || unavailable {
            o.finished = Some(now);
        }
        Ok(())
    }
    pub fn reject(&self, id: &str, reason: &'static str) {
        if let Ok(mut s) = self.0.lock()
            && let Some(o) = s.operations.get_mut(id)
        {
            o.stage = Stage::Denied(reason);
            o.finished = Some(Instant::now());
        }
    }
    /// Called after an actual native approval or decline, never by IPC.
    pub fn resolve(&self, id: &str, allow: bool) -> Result<(), &'static str> {
        let mut s = self.0.lock().map_err(|_| "unavailable")?;
        let now = Instant::now();
        let o = s.operations.get(id).ok_or("unknown_operation")?;
        if !matches!(o.stage, Stage::Approval) || !s.valid(o, now) {
            return Err("stale_source");
        }
        let o = s.operations.get_mut(id).ok_or("unknown_operation")?;
        o.stage = if allow { Stage::Ready } else { Stage::Denied("permission_denied") };
        if !allow {
            o.finished = Some(now);
        }
        Ok(())
    }
    pub fn ready(&self) -> Vec<Capture> {
        let Ok(mut s) = self.0.lock() else {
            return vec![];
        };
        s.prune();
        let now = Instant::now();
        s.operations
            .values()
            .filter(|o| matches!(o.stage, Stage::Ready) && s.valid(o, now))
            .take(4)
            .map(|o| o.capture.clone())
            .collect()
    }
    /// Rechecks immediately before the synchronous native boundary. The closure
    /// must only submit bounded native work; it must never perform blocking IO.
    pub fn execute(&self, id: &str, execute: impl FnOnce(&Capture) -> Value) -> Result<(), &'static str> {
        let mut s = self.0.lock().map_err(|_| "unavailable")?;
        let now = Instant::now();
        let o = s.operations.get(id).ok_or("unknown_operation")?;
        if !matches!(o.stage, Stage::Ready) || !s.valid(o, now) {
            return Err("stale_source");
        }
        let capture = o.capture.clone();
        let operation = s.operations.get_mut(id).ok_or("unknown_operation")?;
        // Only this closed, natively prepared and approved bounded recipe can
        // extend execution. Its approval window itself remains seventy seconds.
        if capture.tool == Tool::ScriptTestBegin {
            let seconds = capture
                .arguments
                .get("timeout_seconds")
                .and_then(Value::as_u64)
                .filter(|seconds| (1..=600).contains(seconds));
            let Some(seconds) = seconds else {
                operation.stage = Stage::Denied("invalid_test_timeout");
                operation.finished = Some(now);
                return Err("invalid_test_timeout");
            };
            operation.execution_deadline = Some(now + Duration::from_secs((seconds + 5).max(70)));
        }
        // Native preparation admits only an exact frozen upload plan. The
        // batch gets its own bounded execution window after real approval;
        // the preceding preparation/Ask window remains seventy seconds.
        if capture.tool == Tool::InventoryUploadExecute {
            operation.execution_deadline = Some(now + Duration::from_secs(600));
        }
        if capture.tool == Tool::PreferencesPreview {
            let seconds = capture
                .arguments
                .get("ttl_seconds")
                .map_or(Some(120), Value::as_u64)
                .filter(|s| (1..=300).contains(s));
            let Some(seconds) = seconds else {
                operation.stage = Stage::Denied("invalid_preferences_ttl");
                operation.finished = Some(now);
                return Err("invalid_preferences_ttl");
            };
            operation.execution_deadline = Some(now + Duration::from_secs((seconds + 5).max(70)));
        }
        operation.stage = Stage::Executing;
        let value = execute(&capture);
        let o = s.operations.get_mut(id).ok_or("unknown_operation")?;
        if serde_json::to_vec(&value).map_or(true, |b| b.len() > MAX_RESULT_BYTES) {
            o.stage = Stage::Denied("response_limit");
        } else if value["status"] == "submitted" {
            o.ever_submitted = true;
            o.stage = Stage::Submitted(value);
        } else {
            o.stage = Stage::Finished(value);
        }
        o.finished = Some(Instant::now());
        Ok(())
    }
    /// Native asynchronous completion must retain this capture. It cannot
    /// schedule new effects: any continuation needs a new final native check.
    pub fn complete(&self, capture: &Capture, result: Value) -> Result<(), &'static str> {
        let mut s = self.0.lock().map_err(|_| "unavailable")?;
        let o = s.operations.get(&capture.request_id).ok_or("unknown_operation")?;
        if o.capture != *capture || !s.valid(o, Instant::now()) || !matches!(o.stage, Stage::Submitted(_)) {
            return Err("stale_source");
        }
        if serde_json::to_vec(&result).map_or(true, |b| b.len() > MAX_RESULT_BYTES) {
            return Err("response_limit");
        }
        let o = s.operations.get_mut(&capture.request_id).ok_or("unknown_operation")?;
        o.stage = Stage::Finished(result);
        o.finished = Some(Instant::now());
        Ok(())
    }
}
impl crate::wire::HostHandler for Runtime {
    fn request(&self, client: ClientId, request: crate::wire::HostRequest) -> Value {
        match request {
            crate::wire::HostRequest::Assistant { request } => self.request(client, request),
            _ => error("wrong_authority_domain"),
        }
    }
    fn disconnected(&self, client: ClientId) {
        self.disconnected(client);
    }
}
impl State {
    fn revoke_active(&mut self) {
        self.active = false;
        for o in self.operations.values_mut().filter(|o| o.active) {
            o.stage = Stage::Denied("native_action_revoked");
            o.artifact = None;
            o.finished = Some(Instant::now());
        }
    }
    fn fresh(&self, now: Instant) -> bool {
        self.observed
            .as_ref()
            .is_some_and(|(_, at)| now.saturating_duration_since(*at) <= FRESH)
    }
    fn ready(&self, now: Instant) -> bool {
        self.configuration_valid
            && !self.paused
            && self.endpoint.is_some()
            && self.enabled_for.is_some()
            && self.fresh(now)
            && self.observed.as_ref().map(|(b, _)| *b) == self.enabled_for
    }
    fn invalidate(&mut self, why: &'static str) {
        self.revision = self.revision.saturating_add(1);
        for o in self.operations.values_mut() {
            o.stage = Stage::Denied(why);
            o.artifact = None;
            o.finished = Some(Instant::now());
        }
    }
    fn disable(&mut self) {
        self.enabled_for = None;
        self.endpoint = None;
        self.paused = false;
        self.invalidate("source_changed");
        self.operations.clear();
    }
    fn source_ready(&self, now: Instant) -> bool {
        self.endpoint.is_some()
            && self.enabled_for.is_some()
            && self.fresh(now)
            && self.observed.as_ref().map(|(b, _)| *b) == self.enabled_for
    }
    fn valid(&self, o: &Operation, now: Instant) -> bool {
        (self.ready(now) || (o.capture.tool == Tool::ScriptTestStop && self.source_ready(now)))
            && Some(o.capture.binding) == self.enabled_for
            && o.capture.policy_revision == self.revision
            && (!o.active || self.active)
            && o.classes.iter().all(|c| self.config.permissions.level(*c) != Level::Never)
            && o.execution_deadline
                .map_or_else(|| now.saturating_duration_since(o.created) < LIFETIME, |deadline| now < deadline)
            && self.original_receipt_current(o, now)
    }
    fn original_receipt_current(&self, o: &Operation, now: Instant) -> bool {
        let Some(capture) = &o.original_receipt_source else {
            return true;
        };
        self.operations.get(&capture.request_id).is_some_and(|source| {
            source.capture == *capture
                && source.original_receipt_source.is_none()
                && self.valid(source, now)
                && matches!(source.stage, Stage::Submitted(_) | Stage::Finished(_))
        })
    }
    fn prune(&mut self) {
        let now = Instant::now();
        for o in self.operations.values_mut() {
            if o.execution_deadline
                .map_or_else(|| now.saturating_duration_since(o.created) >= LIFETIME, |deadline| now >= deadline)
                && !matches!(o.stage, Stage::Finished(_) | Stage::Denied(_))
            {
                o.stage = Stage::Denied("operation_expired");
                o.finished = Some(now);
            }
        }
        self.operations
            .retain(|_, o| o.finished.is_none_or(|at| now.saturating_duration_since(at) < RETAIN));
    }
    fn receipt(&self, client: ClientId, id: &str) -> Value {
        // Every receipt route, including an idempotent Invoke, observes the
        // native delivery barrier before exposing retained private results.
        if self.native_busy {
            return error("native_busy");
        }
        let Some(o) = self.operations.get(id).filter(|o| o.client == client) else {
            return error("unknown_operation");
        };
        if Some(o.capture.binding) != self.enabled_for || o.capture.policy_revision != self.revision {
            return error("source_changed");
        }
        if !self.original_receipt_current(o, Instant::now()) {
            return error("access_revoked");
        }
        match &o.stage {
            Stage::Preparation => json!({"status":"preparing","request_id":id}),
            Stage::Approval => json!({"status":"pending_approval","request_id":id}),
            Stage::Ready | Stage::Executing => json!({"status":"queued","request_id":id}),
            Stage::Submitted(value) => {
                if !self.valid(o, Instant::now()) {
                    return error("access_revoked");
                }
                json!({"status":"submitted","request_id":id,"result":value})
            }
            Stage::Denied(code) => error(code),
            Stage::Finished(value) => {
                if !self.valid(o, Instant::now()) {
                    return error("access_revoked");
                }
                json!({"status":"complete","request_id":id,"result":value})
            }
        }
    }
    fn request(&mut self, client: ClientId, request: ViewerRequest) -> Value {
        self.prune();
        if client == 0
            || Some(request.source()) != self.endpoint
            || !self.fresh(Instant::now())
            || self.enabled_for.is_none()
            || self.observed.as_ref().map(|(b, _)| *b) != self.enabled_for
        {
            return error("viewer_unavailable");
        }
        if self.native_busy
            && matches!(
                request,
                ViewerRequest::OperationStatus { .. } | ViewerRequest::Artifact { .. } | ViewerRequest::Image { .. }
            )
        {
            return error("native_busy");
        }
        match request {
            ViewerRequest::Status { .. } => {
                json!({"status":"ok","viewer_session":self.endpoint,"paused":self.paused,"ready":self.ready(Instant::now())&&!self.native_busy,"permissions":self.config.permissions,"capabilities":self.capabilities.iter().map(|t|t.definition().name).collect::<Vec<_>>(),"policy_revision":self.revision})
            }
            ViewerRequest::OperationStatus { request_id, .. } => self.receipt(client, &request_id),
            ViewerRequest::Cancel { request_id, .. } => {
                let Some(o) = self.operations.get_mut(&request_id).filter(|o| o.client == client) else {
                    return error("unknown_operation");
                };
                if matches!(o.stage, Stage::Submitted(_)) {
                    return json!({"status":"outcome_unknown","request_id":request_id});
                }
                o.stage = Stage::Denied("cancelled");
                o.finished = Some(Instant::now());
                json!({"status":"cancelled","request_id":request_id})
            }
            ViewerRequest::Artifact {
                request_id,
                artifact_id,
                offset,
                length,
                ..
            } => {
                if length == 0 || length > 8192 {
                    return error("artifact_chunk_limit");
                }
                let Some(o) = self
                    .operations
                    .get(&request_id)
                    .filter(|o| o.client == client && self.valid(o, Instant::now()) && matches!(o.stage, Stage::Finished(_)))
                else {
                    return error("artifact_unavailable");
                };
                let Some(artifact) = o.artifact.as_ref().filter(|a| a.id.to_string() == artifact_id) else {
                    return error("artifact_unavailable");
                };
                if offset > artifact.data.len() {
                    return error("invalid_artifact_offset");
                }
                let end = offset.saturating_add(length).min(artifact.data.len());
                let data = &artifact.data[offset..end];
                let mut hex = String::with_capacity(data.len() * 2);
                const DIGITS: &[u8; 16] = b"0123456789abcdef";
                for byte in data {
                    hex.push(DIGITS[(byte >> 4) as usize] as char);
                    hex.push(DIGITS[(byte & 15) as usize] as char);
                }
                json!({"artifact_id":artifact_id,"offset":offset,"next_offset":end,"total_bytes":artifact.data.len(),"mime_type":"image/jpeg","encoding":"hex","data":hex,"complete":end==artifact.data.len()})
            }
            ViewerRequest::Image {
                request_id, artifact_id, ..
            } => {
                use base64::Engine;
                let Some(o) = self.operations.get(&request_id).filter(|o| {
                    o.client == client
                        && o.capture.tool == Tool::CameraSnapshot
                        && self.valid(o, Instant::now())
                        && matches!(o.stage, Stage::Finished(_))
                }) else {
                    return error("image_unavailable");
                };
                let Some(preview) = o
                    .artifact
                    .as_ref()
                    .filter(|a| a.id.to_string() == artifact_id)
                    .and_then(|a| a.preview.as_ref())
                else {
                    return error("image_unavailable");
                };
                json!({"status":"ok","mime_type":"image/jpeg","width":preview.width,"height":preview.height,"bytes":preview.jpeg.len(),"data_base64":base64::engine::general_purpose::STANDARD.encode(&preview.jpeg)})
            }
            ViewerRequest::Invoke {
                tool,
                arguments,
                request_id,
                ..
            } => {
                if !self.ready(Instant::now()) && !(tool == Tool::ScriptTestStop && self.source_ready(Instant::now())) {
                    return error("viewer_paused");
                }
                if tool.permanently_denied() {
                    return error("permanent_delete_unavailable");
                }
                if tool.native_confirmation_only() {
                    return error("native_approval_required");
                }
                if !self.capabilities.contains(&tool) {
                    return error("unsupported_capability");
                }
                if serde_json::to_vec(&arguments).map_or(true, |b| b.len() > MAX_ARGUMENT_BYTES) {
                    return error("request_limit");
                }
                let classes: Vec<_> = tool.definition().class.into_iter().collect();
                if classes.iter().any(|c| self.config.permissions.level(*c) == Level::Never) {
                    return error("permission_denied");
                }
                if tool.definition().active && !self.active {
                    return error("inactive_character");
                }
                if let Some(id) = request_id.as_ref() {
                    if uuid::Uuid::parse_str(id).is_err() {
                        return error("invalid_request_id");
                    }
                    if let Some(old) = self.operations.get(id) {
                        if old.client != client || old.capture.tool != tool || old.capture.arguments != arguments {
                            return error("request_id_conflict");
                        }
                        return self.receipt(client, id);
                    }
                }
                if self.operations.len() >= MAX_PENDING
                    || self
                        .operations
                        .values()
                        .filter(|o| matches!(o.stage, Stage::Preparation | Stage::Ready | Stage::Approval | Stage::Executing))
                        .count()
                        >= MAX_QUEUED
                {
                    return error("capacity");
                }
                let Some(binding) = self.enabled_for else {
                    return error("viewer_unavailable");
                };
                let id = request_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                self.operations.insert(
                    id.clone(),
                    Operation {
                        client,
                        capture: Capture {
                            client,
                            endpoint: self.endpoint.expect("enabled endpoint checked above"),
                            request_id: id.clone(),
                            binding,
                            tool,
                            arguments,
                            policy_revision: self.revision,
                            required_classes: classes.clone(),
                        },
                        classes,
                        active: tool.definition().active,
                        ever_submitted: false,
                        amendment: None,
                        original_receipt_source: None,
                        stage: Stage::Preparation,
                        created: Instant::now(),
                        execution_deadline: None,
                        finished: None,
                        artifact: None,
                    },
                );
                json!({"status":"preparing","request_id":id})
            }
        }
    }
}

#[cfg(test)]
mod tests;
