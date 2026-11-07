//! Actual LastActive producer. All socket/MAC/pixel copying stays on existing
//! owners. These calls require the reviewed sealed V4 APIs and root module join.
use super::demand::{Applied, Decision, Ledger};
use super::{App, Gfx};
use crate::session_host::preview_io::{
    CapturePermitGuard, ConnectionToken, ControlGuard, EpochFence, PreviewCommand, ReceivedCommand, SourceRetentionGuard, WorkerState,
};
use aurora_render::viewport_capture::{CaptureCap, CaptureError, CaptureOutcome, CaptureRetirementProgress, CaptureToken};
use aurora_render::{PhotoContext, Renderer, WorldViewport};
use aurora_session::preview_image::{CaptureClaim, PreviewEpochs, PreviewLayout, PreviewPolicy, PreviewSource, PreviewStamp};
use aurora_session::preview_wire::{DemandConfiguration, DemandRejection, Unavailable};
use std::time::{Duration, Instant};

const TTL: Duration = Duration::from_secs(1);
const RENEW: Duration = Duration::from_millis(50);
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Pass {
    Cpu,
    Observe,
    BeforeRender(WorldViewport),
}
#[derive(Clone, Copy, PartialEq, Eq)]
struct RetentionKey {
    source: PreviewSource,
    epochs: PreviewEpochs,
    physical: (u64, u64, u64),
}
#[derive(Clone, Copy, PartialEq, Eq)]
struct CaptureKey {
    layout: PreviewLayout,
    config: Applied,
    generation: u64,
}
struct Pending {
    token: ConnectionToken,
    claim: CaptureClaim,
    deadline: Instant,
    gpu: Option<CaptureToken>,
}
struct Context {
    token: Option<ConnectionToken>,
    policy: PreviewPolicy,
    layout: Option<PreviewLayout>,
    retention: SourceRetentionGuard,
    capture: Option<EpochFence>,
    observed_at: Instant,
}
pub(super) struct Producer {
    ledger: Ledger,
    connection: Option<ConnectionToken>,
    retain_key: Option<RetentionKey>,
    control: Option<ControlGuard>,
    retain: Option<EpochFence>,
    capture_key: Option<CaptureKey>,
    capture: Option<EpochFence>,
    pending: Option<Pending>,
    last_capture: Option<Instant>,
    published: Option<(PreviewPolicy, Option<PreviewLayout>, Instant)>,
    sequence: u64,
    ever_requested: bool,
    retirement_needed: bool,
    exhausted: bool,
}
impl Default for Producer {
    fn default() -> Self {
        debug_assert!(std::mem::size_of::<Self>() < crate::session_host::worker::NATIVE_PREVIEW_METADATA_BYTES);
        Self {
            ledger: Ledger::default(),
            connection: None,
            retain_key: None,
            control: None,
            retain: None,
            capture_key: None,
            capture: None,
            pending: None,
            last_capture: None,
            published: None,
            sequence: 0,
            ever_requested: false,
            retirement_needed: false,
            exhausted: false,
        }
    }
}
impl Producer {
    pub fn capture_barrier(&self) {
        if let Some(fence) = &self.capture {
            fence.revoke();
        }
    }
    pub fn retire(&mut self) {
        self.capture_barrier();
        if let Some(fence) = &self.retain {
            fence.revoke();
        }
        self.retain_key = None;
        self.control = None;
        self.retain = None;
        self.published = None;
    }
    fn fresh(received: Instant, now: Instant) -> bool {
        now.checked_duration_since(received).is_some_and(|age| age < TTL)
    }
    fn unavailable(app: &App, pending: &Pending, reason: Unavailable) {
        if let Some(channel) = app.host_session.as_ref().and_then(|bridge| bridge.preview_channel()) {
            let _ = channel.try_unavailable(pending.token, pending.claim, reason);
        }
    }
    fn cancel(&mut self, app: &App, renderer: &mut Renderer, reason: Unavailable) {
        let had_capture = self.capture.take().map(|fence| fence.revoke()).is_some();
        self.capture_key = None;
        if let Some(pending) = self.pending.take() {
            if let Some(token) = pending.gpu {
                if renderer.cancel_viewport_capture(token).is_err() {
                    renderer.invalidate_viewport_capture();
                }
                self.retirement_needed = true;
            }
            Self::unavailable(app, &pending, reason);
        } else if had_capture && self.ever_requested {
            renderer.invalidate_viewport_capture();
            self.retirement_needed = true;
        }
        self.published = None;
    }
    fn deny(&mut self, app: &mut App, renderer: &mut Renderer) {
        self.cancel(app, renderer, Unavailable::Denied);
        self.retire();
        self.ledger.reset_domain();
        self.connection = None;
        if let Some(bridge) = &mut app.host_session {
            bridge.suspend_preview_source();
        }
    }
    fn new_metadata_domain(&mut self, token: Option<ConnectionToken>) -> bool {
        let Some(token) = token else { return false };
        let changed = self.connection.is_none_or(|old| !old.same_metadata(token));
        self.connection = Some(token);
        changed
    }
    fn observe(&mut self, app: &mut App, gfx: &mut Gfx, photo: PhotoContext, now: Instant, force: bool) -> Option<Context> {
        if let Some(bridge) = &mut app.host_session {
            bridge.refresh_preview_source();
        }
        let token = app
            .host_session
            .as_ref()
            .and_then(|bridge| bridge.preview_channel())
            .and_then(|channel| channel.connection());
        if self.new_metadata_domain(token) {
            self.cancel(app, &mut gfx.renderer, Unavailable::Denied);
            self.ledger.reset_domain();
        }
        // None also means the try-only mailbox read was busy. It cannot
        // authorize capture or refresh a pending deadline, but it is not proof
        // of a new metadata domain. A genuine reconnect has a fresh token and
        // resets the old demand above before any new admission.

        app.configure_host_chat_rules();
        let remembered = self.ledger.current().is_some() && token.is_some();
        let facts = app.host_preview_facts(Some(gfx), photo, remembered);
        if app.host_preview.sync(facts, now) {
            self.cancel(app, &mut gfx.renderer, Unavailable::Denied);
        }
        let Some(policy) = app.host_preview.policy.filter(|policy| policy.access().retain_last_active) else {
            self.deny(app, &mut gfx.renderer);
            return None;
        };
        let Some(proof) = app
            .host_session
            .as_ref()
            .and_then(|bridge| bridge.preview_source_proof())
            .filter(|proof| proof.source() == policy.source())
        else {
            self.deny(app, &mut gfx.renderer);
            return None;
        };
        let Ok(control) = ControlGuard::from_native(&proof) else {
            self.deny(app, &mut gfx.renderer);
            return None;
        };
        let Some(physical) = gfx.renderer.viewport_capture_retention_guard().filter(|guard| guard.current()) else {
            self.deny(app, &mut gfx.renderer);
            return None;
        };
        let key = RetentionKey {
            source: policy.source(),
            epochs: policy.epochs(),
            physical: (physical.instance(), physical.device_epoch(), physical.generation()),
        };
        if self.retain_key != Some(key)
            || self.control.as_ref().is_none_or(|old| !old.same_domain(&control))
            || self.retain.as_ref().is_none_or(|fence| !fence.current())
        {
            self.cancel(app, &mut gfx.renderer, Unavailable::Denied);
            if let Some(old) = self.retain.take() {
                old.revoke();
            }
            self.retain = Some(EpochFence::new());
            self.retain_key = Some(key);
        }
        self.control = Some(control.clone());
        let mut layout = app
            .host_preview
            .observed
            .and_then(|observed| native_layout(policy, self.ledger.current(), observed.active, observed.viewport))
            .filter(|_| !self.exhausted);
        let next = layout.zip(self.ledger.current()).map(|(layout, config)| CaptureKey {
            layout,
            config,
            generation: gfx.renderer.viewport_capture_generation(),
        });
        if self.capture_key != next || self.capture.as_ref().is_some_and(|fence| !fence.current()) {
            self.cancel(app, &mut gfx.renderer, Unavailable::Denied);
            if let Some(mut next) = next {
                next.generation = gfx.renderer.viewport_capture_generation();
                self.capture_key = Some(next);
                self.capture = Some(EpochFence::new());
            }
        }
        let capture = self.capture.as_ref().filter(|fence| fence.current()).cloned();
        let policy = if capture.is_none() && policy.access().capture_active {
            PreviewPolicy::new(
                policy.source(),
                policy.epochs(),
                aurora_session::preview_image::PreviewAccess {
                    capture_active: false,
                    ..policy.access()
                },
            )
            .ok()?
        } else {
            policy
        };
        if capture.is_none() {
            layout = None;
        }
        let expiry = now.checked_add(TTL)?.min(control.expires_at());
        let retention = SourceRetentionGuard::new(control, policy, expiry, self.retain.as_ref()?.clone(), physical).ok()?;
        let context = Context {
            token,
            policy,
            layout,
            retention,
            capture,
            observed_at: now,
        };
        let changed = self
            .published
            .is_none_or(|(old, old_layout, at)| old != policy || old_layout != layout || now.saturating_duration_since(at) >= RENEW);
        if force || changed {
            if let Some(bridge) = &mut app.host_session {
                bridge.resume_preview_source();
                if !bridge.register_preview_capture_fence(context.capture.clone()) {
                    self.deny(app, &mut gfx.renderer);
                    return None;
                }
            }
            let state = WorkerState {
                policy,
                layout,
                retention: context.retention.clone(),
                capture: context.capture.clone(),
                native_observed_at: context.observed_at,
            };
            if app
                .host_session
                .as_ref()
                .and_then(|bridge| bridge.preview_channel())
                .is_none_or(|channel| channel.publish_state(state).is_err())
            {
                self.deny(app, &mut gfx.renderer);
                return None;
            }
            self.published = Some((policy, layout, now));
        }
        Some(context)
    }
    fn command(&mut self, app: &mut App, gfx: &mut Gfx, context: &Context, received: ReceivedCommand, now: Instant) -> bool {
        if context.token.is_none_or(|current| !current.same_metadata(received.token))
            || received.token.source() != context.policy.source()
            || !context.retention.current(Instant::now())
        {
            return false;
        }
        match received.command {
            PreviewCommand::RememberActive { demand, size, rate } => {
                let config = DemandConfiguration { size, rate };
                let decision = self.ledger.classify(demand, config);
                let receipt = match decision {
                    Decision::Known(receipt) => Self::fresh(received.received_at, now).then_some(receipt),
                    Decision::Reject(reason) => {
                        Ledger::rejection(context.policy.source(), received.token.connection(), demand, config, reason)
                    }
                    Decision::Apply => {
                        let denial = (!Self::fresh(received.received_at, now)).then_some(DemandRejection::Expired);
                        if denial.is_none() {
                            self.cancel(app, &mut gfx.renderer, Unavailable::Denied);
                        }
                        self.ledger
                            .apply(context.policy.source(), received.token.connection(), demand, config, denial)
                    }
                };
                if let Some(receipt) = receipt
                    && let Some(channel) = app.host_session.as_ref().and_then(|bridge| bridge.preview_channel())
                {
                    let _ = channel.try_demand_receipt(received.token, receipt);
                }
                true
            }
            PreviewCommand::Stop { demand } => {
                if Self::fresh(received.received_at, now) && self.ledger.can_stop(demand) {
                    if self.ledger.current().is_some_and(|current| current.demand == demand) {
                        self.cancel(app, &mut gfx.renderer, Unavailable::Denied);
                    }
                    if self.ledger.stop(demand)
                        && let Some(channel) = app.host_session.as_ref().and_then(|bridge| bridge.preview_channel())
                    {
                        let _ = channel.acknowledge_stop(received.token, demand);
                    }
                    true
                } else {
                    false
                }
            }
            PreviewCommand::Capture(claim) => {
                let deadline = received.received_at.checked_add(TTL);
                let valid = Self::fresh(received.received_at, now)
                    && context.token == Some(received.token)
                    && claim_matches(context.policy, context.layout, self.ledger.current(), *claim)
                    && self.capture.as_ref().is_some_and(|fence| fence.current())
                    && self.rate_ready(now)
                    && app
                        .host_session
                        .as_ref()
                        .and_then(|bridge| bridge.preview_channel())
                        .is_some_and(|channel| channel.frame_available());
                let incoming = Pending {
                    token: received.token,
                    claim: *claim,
                    deadline: deadline.unwrap_or(now),
                    gpu: None,
                };
                if !valid || self.pending.is_some() {
                    Self::unavailable(
                        app,
                        &incoming,
                        if !Self::fresh(received.received_at, now) {
                            Unavailable::Expired
                        } else {
                            Unavailable::Busy
                        },
                    );
                } else {
                    self.pending = Some(incoming);
                }
                false
            }
        }
    }
    fn rate_ready(&self, now: Instant) -> bool {
        if self.exhausted {
            return false;
        }
        let Some(config) = self.ledger.current() else { return false };
        self.last_capture.is_none_or(|last| {
            now.checked_duration_since(last)
                .is_some_and(|age| age >= Duration::from_millis(config.config.rate.interval_millis()))
        })
    }
    fn pending_current(&self, pending: &Pending, app: &App, gfx: &Gfx, context: &Context, now: Instant) -> bool {
        context.token == Some(pending.token)
            && context.capture.as_ref().is_some_and(|capture| capture.current())
            && context.retention.current(now)
            && pending_authority_matches(
                context.policy,
                context.layout,
                self.ledger.current(),
                pending,
                app.host_session
                    .as_ref()
                    .and_then(|bridge| bridge.preview_authority(&gfx.window, &gfx.renderer)),
                now,
            )
    }
    fn current_pending(&self, app: &App, gfx: &Gfx, context: &Context, now: Instant) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|pending| self.pending_current(pending, app, gfx, context, now))
    }
    fn complete(&mut self, app: &App, gfx: &mut Gfx, context: &Context) {
        if self.pending.as_ref().is_none_or(|pending| pending.gpu.is_none()) {
            return;
        }
        if !self.current_pending(app, gfx, context, Instant::now())
            || app
                .host_session
                .as_ref()
                .and_then(|bridge| bridge.preview_channel())
                .is_none_or(|channel| !channel.frame_available())
        {
            self.cancel(app, &mut gfx.renderer, Unavailable::Denied);
            return;
        }
        match gfx.renderer.poll_viewport_capture() {
            Ok(Some(CaptureOutcome::Ready(frame))) => {
                let Some(pending) = self.pending.take() else { return };
                self.retirement_needed = true;
                if pending.gpu != Some(frame.request().token())
                    || pending.deadline <= Instant::now()
                    || !gfx.renderer.viewport_capture_current(&frame)
                {
                    Self::unavailable(app, &pending, Unavailable::Denied);
                    return;
                }
                let Some(publication) = gfx.renderer.viewport_capture_guard(&frame) else {
                    Self::unavailable(app, &pending, Unavailable::Denied);
                    return;
                };
                let Some(sequence) = self.sequence.checked_add(1) else {
                    self.exhausted = true;
                    self.capture_barrier();
                    Self::unavailable(app, &pending, Unavailable::Denied);
                    return;
                };
                self.sequence = sequence;
                let Some(capture) = context.capture.clone() else {
                    Self::unavailable(app, &pending, Unavailable::Denied);
                    return;
                };
                let expiry = pending.deadline.min(publication.deadline());
                let Ok(guard) = CapturePermitGuard::new(
                    context.retention.clone(),
                    pending.token,
                    pending.claim,
                    publication,
                    capture,
                    expiry,
                ) else {
                    Self::unavailable(app, &pending, Unavailable::Denied);
                    return;
                };
                let Ok(stamp) = PreviewStamp::new(
                    pending.claim,
                    sequence,
                    frame.request().boundary().surface_epoch(),
                    frame.frame(),
                    pending.claim.geometry(),
                ) else {
                    Self::unavailable(app, &pending, Unavailable::Denied);
                    return;
                };
                // Polling may have yielded ownership while the OS window or
                // accepted native grant changed. Requery actual native state at
                // the effect boundary; the pre-poll Context cannot prove it.
                if !self.pending_current(&pending, app, gfx, context, Instant::now()) {
                    Self::unavailable(app, &pending, Unavailable::Denied);
                    return;
                }
                if let Some(channel) = app.host_session.as_ref().and_then(|bridge| bridge.preview_channel()) {
                    if channel.connection() != Some(pending.token) || !channel.frame_available() {
                        Self::unavailable(app, &pending, Unavailable::Denied);
                        return;
                    }
                    if channel.try_frame(guard, stamp, frame).is_err() {
                        // The moved allocation is dropped by the image owner. Never retry/copy.
                        Self::unavailable(app, &pending, Unavailable::Busy);
                    }
                }
            }
            Ok(Some(CaptureOutcome::Failed(token, error))) => {
                if self.pending.as_ref().is_some_and(|pending| pending.gpu == Some(token)) {
                    self.failure(app, &mut gfx.renderer, error);
                }
            }
            Err(error) => self.failure(app, &mut gfx.renderer, error),
            Ok(None) => {}
        }
    }
    fn freeze(&mut self, app: &App, gfx: &mut Gfx, context: &Context, viewport: WorldViewport) {
        if self.pending.as_ref().is_none_or(|pending| pending.gpu.is_some()) {
            return;
        }
        let exact = context
            .layout
            .is_some_and(|layout| layout.source_rect() == [viewport.x, viewport.y, viewport.width, viewport.height]);
        if !exact
            || !self.current_pending(app, gfx, context, Instant::now())
            || !self.rate_ready(Instant::now())
            || app
                .host_session
                .as_ref()
                .and_then(|bridge| bridge.preview_channel())
                .is_none_or(|channel| !channel.frame_available())
        {
            self.cancel(app, &mut gfx.renderer, Unavailable::Denied);
            return;
        }
        let cap = match self.pending.as_ref().unwrap().claim.cap() {
            aurora_session::PreviewSize::Small => CaptureCap::Small,
            aurora_session::PreviewSize::Medium => CaptureCap::Medium,
            aurora_session::PreviewSize::Large => CaptureCap::Large,
        };
        match gfx.renderer.request_viewport_capture(viewport, cap) {
            Ok(token) => {
                self.pending.as_mut().unwrap().gpu = Some(token);
                self.ever_requested = true;
                self.retirement_needed = true;
                self.last_capture = Some(Instant::now());
            }
            Err(error) => self.failure(app, &mut gfx.renderer, error),
        }
    }
    fn failure(&mut self, app: &App, renderer: &mut Renderer, error: CaptureError) {
        if matches!(error, CaptureError::Exhausted | CaptureError::Clock) {
            self.exhausted = true;
        }
        self.cancel(app, renderer, reason(error));
    }
    fn cleanup(&mut self, renderer: &mut Renderer) {
        if !self.retirement_needed || self.pending.as_ref().is_some_and(|pending| pending.gpu.is_some()) {
            return;
        }
        match renderer.poll_viewport_capture_retirement() {
            Ok(CaptureRetirementProgress::Idle | CaptureRetirementProgress::NoWork) => self.retirement_needed = false,
            Ok(CaptureRetirementProgress::Pending) => {}
            Err(_) => self.capture_barrier(),
        }
    }
    pub fn drive(&mut self, app: &mut App, gfx: &mut Gfx, photo: PhotoContext, pass: Pass) {
        if app
            .host_session
            .as_ref()
            .and_then(|bridge| bridge.preview_channel())
            .is_some_and(|channel| channel.ended())
            && let Some(bridge) = &mut app.host_session
        {
            bridge.disable_preview();
        }
        let mut context = self.observe(app, gfx, photo, Instant::now(), false);
        if pass == Pass::Cpu
            && let Some(current) = &context
        {
            let received = app
                .host_session
                .as_ref()
                .and_then(|bridge| bridge.preview_channel())
                .and_then(|channel| channel.take_command());
            if let Some(received) = received
                && self.command(app, gfx, current, received, Instant::now())
            {
                context = self.observe(app, gfx, photo, Instant::now(), true);
            }
        }
        if self.pending.as_ref().is_some_and(|pending| pending.deadline <= Instant::now()) {
            self.cancel(app, &mut gfx.renderer, Unavailable::Expired);
        }
        if let Some(context) = context {
            self.complete(app, gfx, &context);
            if let Pass::BeforeRender(viewport) = pass {
                self.freeze(app, gfx, &context, viewport);
            }
        }
        self.cleanup(&mut gfx.renderer);
    }
}
fn reason(error: CaptureError) -> Unavailable {
    match error {
        CaptureError::Busy | CaptureError::Capacity => Unavailable::Busy,
        CaptureError::TimedOut => Unavailable::Expired,
        CaptureError::Unsupported => Unavailable::Unsupported,
        _ => Unavailable::Denied,
    }
}

impl App {
    pub(super) fn has_preview_producer(&self) -> bool {
        self.host_preview.producer.is_some() || self.host_session.as_ref().and_then(|bridge| bridge.preview_channel()).is_some()
    }
    pub(super) fn run_preview_producer(&mut self, gfx: Option<&mut Gfx>, photo: PhotoContext, pass: Pass) -> bool {
        if !self.has_preview_producer() {
            return false;
        }
        // A temporary take is an explicit borrow. Demand comes from this local
        // owner, never from observing the temporarily empty App producer slot.
        let mut producer = self.host_preview.producer.take().unwrap_or_default();
        if let Some(gfx) = gfx {
            producer.drive(self, gfx, photo, pass);
        } else {
            producer.retire();
            producer.ledger.reset_domain();
            if let Some(bridge) = &mut self.host_session {
                bridge.suspend_preview_source();
            }
        }
        if self.host_session.as_ref().and_then(|bridge| bridge.preview_channel()).is_some()
            || producer.pending.is_some()
            || producer.retirement_needed
        {
            self.host_preview.producer = Some(producer);
        }
        true
    }
}
impl Drop for Producer {
    fn drop(&mut self) {
        self.retire();
    }
}

/// The same exact native correlation is used before poll and AFTER Ready at
/// handoff. Authority must be freshly queried by the caller from the OS/bridge;
/// copied layout or an old Context never substitutes for that query.
fn pending_authority_matches(
    policy: PreviewPolicy,
    layout: Option<PreviewLayout>,
    applied: Option<Applied>,
    pending: &Pending,
    authority: Option<crate::session_host::worker::PreviewAuthority>,
    now: Instant,
) -> bool {
    pending.deadline > now
        && claim_matches(policy, layout, applied, pending.claim)
        && authority.is_some_and(|authority| {
            authority.scope.expires_at > now
                && authority.scope.source == policy.source()
                && authority.activation == pending.claim.activation()
                && authority.boundary.surface_epoch() == pending.claim.epochs().surface()
                && layout.is_some_and(|layout| {
                    authority.boundary.device_epoch() == layout.device_epoch()
                        && [authority.boundary.size().0, authority.boundary.size().1] == layout.source_size()
                })
        })
}

/// Metadata correlation only. This never substitutes for strict native window,
/// current lease/privacy/physical guards immediately before request/publication.
fn claim_matches(policy: PreviewPolicy, layout: Option<PreviewLayout>, applied: Option<Applied>, claim: CaptureClaim) -> bool {
    claim.matches_policy(policy)
        && applied.is_some_and(|config| config.demand == claim.demand() && config.config.size == claim.cap())
        && layout.is_some_and(|layout| {
            layout.matches_policy(policy)
                && layout.demand() == claim.demand()
                && layout.geometry(claim.cap()).is_ok_and(|geometry| geometry == claim.geometry())
        })
}

/// A layout hint is published before claims exist. Its inputs were observed by
/// the shared native facts model; this helper itself grants no capture authority.
fn native_layout(
    policy: PreviewPolicy,
    config: Option<Applied>,
    active: Option<super::ActiveGrant>,
    view: Option<WorldViewport>,
) -> Option<PreviewLayout> {
    if !policy.access().capture_active {
        return None;
    }
    let config = config?;
    let active = active?;
    let view = view?;
    let boundary = active.boundary;
    if active.activation != policy.access().activation || boundary.surface_epoch() != policy.epochs().surface() {
        return None;
    }
    let layout = PreviewLayout::new(
        policy.source(),
        policy.epochs(),
        active.activation,
        config.demand,
        boundary.device_epoch(),
        [boundary.size().0, boundary.size().1],
        [view.x, view.y, view.width, view.height],
    )
    .ok()?;
    layout.geometry(config.config.size).ok()?;
    Some(layout)
}

#[cfg(test)]
#[path = "producer_tests.rs"]
mod tests;
