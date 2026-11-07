//! Bounded Shell-owned monitor bindings and exact operation correlations.
//! Port of the pinned Legacy tools/session-host/main.cpp monitor actions.
//! No window, pixels, transport endpoint or canonical editor is owned here.

use super::monitor_model::{self, Binding, Exchange, MonitorSet, Options, Source, Ticket, View};
use super::monitor_ui::{Action, Command, State};
use super::preview_projection::{
    ExchangeOutcome, ExchangeReceipt, PreviewError, PreviewRead, PreviewStatus, SettingsOutcome, SettingsReceipt, ViewDescriptor,
    ViewNamespace, ViewSetRequest,
};
use super::slots::SlotId;
use super::transport::HostController;
use aurora_session::chat::ChatSource;
use aurora_session::preview_image::PreviewSource;
use aurora_session::{HostedViewport, MAX_WORKERS};

/// Always called while the OLD model/source/state still exists. The native
/// parent drains that surface's delivered edit prefix, then retires its owner.
pub(super) type Flush<'a> = dyn FnMut(View, Binding, State) + 'a;

pub(super) const fn index(view: View) -> usize {
    match view {
        View::Fixed1 => 0,
        View::Fixed2 => 1,
        View::Fixed3 => 2,
        View::Fixed4 => 3,
        View::Cycling => 4,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SettingsPending {
    request: u64,
    namespace: ViewNamespace,
    ticket: Ticket,
    binding: Binding,
    base: State,
    desired: Options,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ExchangePending {
    request: u64,
    namespace: ViewNamespace,
    exchange: Exchange,
    viewport: HostedViewport,
}
#[derive(Default)]
struct Entry {
    state: State,
    namespace: Option<ViewNamespace>,
    identity: Option<(Source, u64)>,
    settings: Option<SettingsPending>,
}

pub(super) struct MonitorManager {
    model: MonitorSet,
    entries: [Entry; 5],
    sources: Vec<Source>,
    active: Option<PreviewSource>,
    order: [SlotId; MAX_WORKERS],
    descriptors: Vec<ViewDescriptor>,
    admitted: Option<ViewSetRequest>,
    dirty: bool,
    exchange: Option<ExchangePending>,
    timer_suspended: bool,
    resume_at: Option<u64>,
}

impl MonitorManager {
    pub fn new() -> Result<Self, monitor_model::Error> {
        Ok(Self {
            model: MonitorSet::new()?,
            entries: Default::default(),
            sources: Vec::new(),
            active: None,
            order: std::array::from_fn(|index| SlotId::try_from(index as u8).expect("bounded stable slot")),
            descriptors: Vec::new(),
            admitted: None,
            dirty: false,
            exchange: None,
            timer_suspended: false,
            resume_at: None,
        })
    }
    pub fn model(&self) -> &MonitorSet {
        &self.model
    }
    pub fn state(&self, view: View) -> State {
        self.entries[index(view)].state
    }
    pub fn namespace(&self, view: View) -> Option<ViewNamespace> {
        self.entries[index(view)].namespace
    }
    pub fn sources(&self) -> &[Source] {
        &self.sources
    }
    pub fn current_read(&self, view: View, read: &PreviewRead) -> bool {
        self.admitted.is_some_and(|request| {
            read.view() == view
                && Some(read.namespace()) == self.namespace(view)
                && read.generation() == request.generation()
                && read.request() == request.request()
                && self
                    .model
                    .binding(view)
                    .is_some_and(|binding| binding.enabled && binding.visible && binding.source.preview() == read.source())
                && read.view_current()
        })
    }
    fn bump(&mut self, view: View) -> Result<(), PreviewError> {
        let state = &mut self.entries[index(view)].state;
        state.revision = state.revision.checked_add(1).ok_or(PreviewError::Exhausted)?;
        Ok(())
    }
    fn flush_all(&self, flush: &mut Flush<'_>) {
        for view in View::ALL {
            if let Some(binding) = self.model.binding(view) {
                flush(view, binding, self.state(view));
            }
        }
    }
    fn reconcile(&mut self) -> Result<(), PreviewError> {
        for view in View::ALL {
            let binding = self.model.binding(view).filter(|binding| binding.enabled && binding.visible);
            let identity = binding.map(|binding| (binding.source, binding.incarnation));
            let entry = &mut self.entries[index(view)];
            if identity != entry.identity {
                entry.identity = identity;
                entry.namespace = identity.map(|_| ViewNamespace::fresh()).transpose()?;
                entry.settings = None;
                entry.state.request = None;
                entry.state.failed = false;
                entry.state.accepted = binding.map_or_else(Options::default, |binding| binding.options);
                self.bump(view)?;
                self.dirty = true;
            }
        }
        Ok(())
    }

    /// Caller acquired this real try-only status immediately before dispatch.
    /// Stale/contended readers cannot mutate source history or receipt state.
    pub fn sync(&mut self, status: &PreviewStatus, order: [SlotId; MAX_WORKERS], flush: &mut Flush<'_>) -> Result<(), PreviewError> {
        if !status.current() {
            return Err(PreviewError::Stale);
        }
        let sources = status
            .sources()
            .iter()
            .map(|source| {
                Source::new(
                    source.slot,
                    ChatSource {
                        worker: source.source.worker(),
                        session: source.source.session(),
                        lease_epoch: source.source.lease_epoch(),
                    },
                    source.available,
                )
                .map_err(|_| PreviewError::Invalid)
            })
            .collect::<Result<Vec<_>, _>>()?;
        if sources != self.sources || status.active() != self.active || order != self.order {
            // Privacy/source replacement may invalidate a composer as well as
            // pixels. Flush actual old native input before that sync changes it.
            self.flush_all(flush);
            if !status.current() {
                return Err(PreviewError::Stale);
            }
            self.model.sync(&sources, order, status.active()).map_err(model_error)?;
            self.sources = sources;
            self.active = status.active();
            self.order = order;
            self.reconcile()?;
        }
        for receipt in status.settings_receipts() {
            self.settings_receipt(*receipt, flush)?;
        }
        for receipt in status.exchange_receipts() {
            self.exchange_receipt(*receipt, flush)?;
        }
        Ok(())
    }

    pub fn open(&mut self, source: PreviewSource, flush: &mut Flush<'_>) -> Result<View, PreviewError> {
        let source = self
            .sources
            .iter()
            .find(|item| item.preview() == source)
            .copied()
            .ok_or(PreviewError::Stale)?;
        self.flush_all(flush);
        let view = self.model.open_fixed(source).map_err(model_error)?;
        self.reconcile()?;
        Ok(view)
    }
    pub fn open_cycling(&mut self, now: u64, flush: &mut Flush<'_>) -> Result<View, PreviewError> {
        self.flush_all(flush);
        self.model.open_cycler(now).map_err(model_error)?;
        self.reconcile()?;
        Ok(View::Cycling)
    }
    pub fn close(&mut self, view: View, flush: &mut Flush<'_>) -> Result<(), PreviewError> {
        if let Some(binding) = self.model.binding(view) {
            flush(view, binding, self.state(view));
        }
        self.model.close(view).map_err(model_error)?;
        self.reconcile()
    }
    /// Minimize stops admission and removes all host/UI pixels; a later explicit
    /// Open reuses the binding with a fresh namespace, never restores old pixels.
    pub fn hide(&mut self, view: View, flush: &mut Flush<'_>) -> Result<(), PreviewError> {
        if let Some(binding) = self.model.binding(view) {
            flush(view, binding, self.state(view));
        }
        self.model.set_visible(view, false).map_err(model_error)?;
        self.reconcile()
    }
    pub fn cancel_input(&mut self) -> Result<(), PreviewError> {
        self.model.cancel_input().map_err(model_error)
    }

    /// Advances the real synchronous view fence even on queue failure. Existing
    /// owners must retire until a matching accepted read becomes available.
    pub fn publish(&mut self, controller: &HostController) -> Result<(), PreviewError> {
        if !self.dirty {
            return Ok(());
        }
        let desired = self.desired();
        self.admitted = None;
        let request = controller.set_preview_views(desired.clone())?;
        self.descriptors = desired;
        self.admitted = Some(request);
        self.dirty = false;
        Ok(())
    }
    fn desired(&self) -> Vec<ViewDescriptor> {
        View::ALL
            .into_iter()
            .filter_map(|view| {
                let binding = self.model.binding(view).filter(|binding| binding.enabled && binding.visible)?;
                Some(ViewDescriptor {
                    view,
                    namespace: self.namespace(view)?,
                    ticket: self.model.capture_view(view)?,
                    source: binding.source.preview(),
                    visible: true,
                    options: binding.options,
                })
            })
            .collect()
    }

    pub fn validate(&self, action: &Action) -> bool {
        let Action::Control {
            view,
            ticket,
            binding,
            base,
            playing,
            interval,
            command,
        } = action
        else {
            return false;
        };
        if !self.model.validate_view(*ticket)
            || self.model.binding(*view) != Some(*binding)
            || self.state(*view) != *base
            || self.model.playing() != *playing
            || self.model.interval() != *interval
        {
            return false;
        }
        match *command {
            Command::Exchange(ticket) => self.exchange.is_none() && self.model.can_exchange(ticket),
            Command::Chat(ticket, _) | Command::Pin(ticket, _) => self.model.validate(ticket),
            Command::Settings {
                ticket, base: original, ..
            } => original == *base && base.request.is_none() && self.model.validate(ticket),
            Command::Navigate(navigation) => self.model.validate_navigation(navigation),
            Command::Pause | Command::Play | Command::Interval(_) => *view == View::Cycling,
        }
    }
    /// Actual native pin has already been queried by the parent; requested pin
    /// is never used as the published checkmark. All actions revalidate first.
    #[allow(clippy::too_many_arguments)]
    pub fn control(
        &mut self,
        action: &Action,
        controller: &HostController,
        viewport: Option<HostedViewport>,
        now: u64,
        observed_pin: Option<bool>,
        flush: &mut Flush<'_>,
    ) -> Result<bool, PreviewError> {
        if !self.validate(action) {
            return Ok(false);
        }
        let Action::Control {
            view,
            binding,
            base,
            command,
            ..
        } = action
        else {
            return Ok(false);
        };
        let view = *view;
        match *command {
            Command::Chat(_, chat) => {
                flush(view, *binding, *base);
                self.entries[index(view)].state.chat = chat;
                self.bump(view)?;
                self.cancel_input()?;
            }
            Command::Pin(_, _) => {
                let pinned = observed_pin.ok_or(PreviewError::Unavailable)?;
                self.entries[index(view)].state.pinned = pinned;
                self.bump(view)?;
                self.cancel_input()?;
            }
            Command::Settings { ticket, base, desired } => {
                let namespace = self.namespace(view).ok_or(PreviewError::Stale)?;
                let request = match controller.set_preview_options(view, namespace, ticket, base, desired) {
                    Ok(request) => request,
                    Err(error) => {
                        self.entries[index(view)].state.failed = true;
                        self.bump(view)?;
                        return Err(error);
                    }
                };
                self.entries[index(view)].settings = Some(SettingsPending {
                    request,
                    namespace,
                    ticket,
                    binding: *binding,
                    base,
                    desired,
                });
                self.entries[index(view)].state.request = Some((request, desired));
                self.entries[index(view)].state.failed = false;
                self.bump(view)?;
            }
            Command::Exchange(ticket) => {
                let viewport = viewport.ok_or(PreviewError::Unavailable)?;
                let namespace = self.namespace(view).ok_or(PreviewError::Stale)?;
                flush(view, *binding, *base);
                let Some(exchange) = self.model.begin_exchange(ticket).map_err(model_error)? else {
                    return Ok(false);
                };
                match controller.exchange_preview(namespace, exchange, viewport) {
                    Ok(request) => {
                        self.exchange = Some(ExchangePending {
                            request,
                            namespace,
                            exchange,
                            viewport,
                        })
                    }
                    Err(error) => {
                        self.model.finish_exchange(exchange, false).map_err(model_error)?;
                        return Err(error);
                    }
                }
            }
            Command::Navigate(navigation) => {
                flush(view, *binding, *base);
                self.model.navigate(navigation).map_err(model_error)?;
                self.reconcile()?;
            }
            Command::Pause => {
                flush(view, *binding, *base);
                self.model.pause().map_err(model_error)?;
            }
            Command::Play => {
                flush(view, *binding, *base);
                self.model.play(now).map_err(model_error)?;
                self.reconcile()?;
            }
            Command::Interval(seconds) => {
                flush(view, *binding, *base);
                self.model.set_interval(seconds, now).map_err(model_error)?;
            }
        }
        Ok(true)
    }

    fn settings_receipt(&mut self, receipt: SettingsReceipt, flush: &mut Flush<'_>) -> Result<(), PreviewError> {
        let view = receipt.view;
        let Some(pending) = self.entries[index(view)].settings else {
            return Ok(());
        };
        if pending.request != receipt.request
            || pending.namespace != receipt.namespace
            || pending.ticket != receipt.ticket
            || pending.base != receipt.base
            || pending.desired != receipt.desired
            || self.namespace(view) != Some(receipt.namespace)
        {
            return Ok(());
        }
        // Consume only this exact terminal operation, including rejected/unknown.
        self.entries[index(view)].settings = None;
        let state = self.state(view);
        let mut accepted = None;
        if let SettingsOutcome::Accepted {
            options,
            effective,
            demand,
            native_revision,
        } = receipt.outcome
            && options == pending.desired
            && effective == pending.desired
            && demand != 0
            && native_revision != 0
            && self.model.binding(view) == Some(pending.binding)
        {
            flush(view, pending.binding, state);
            if self.model.ack_options(view, pending.binding, options).map_err(model_error)? {
                accepted = Some(options);
            }
        }
        let entry = &mut self.entries[index(view)];
        entry.state.request = None;
        entry.state.failed = accepted.is_none();
        if let Some(options) = accepted {
            entry.state.accepted = options;
            if let Some(descriptor) = self
                .descriptors
                .iter_mut()
                .find(|descriptor| descriptor.view == view && descriptor.namespace == receipt.namespace)
            {
                descriptor.options = options;
            }
        }
        self.bump(view)
    }
    fn exchange_receipt(&mut self, receipt: ExchangeReceipt, flush: &mut Flush<'_>) -> Result<(), PreviewError> {
        let Some(pending) = self.exchange else { return Ok(()) };
        if pending.request != receipt.request || pending.namespace != receipt.namespace || pending.exchange != receipt.exchange {
            return Ok(());
        }
        self.exchange = None;
        let view = receipt.exchange.view();
        if let Some(binding) = self.model.binding(view) {
            flush(view, binding, self.state(view));
        }
        let success = matches!(receipt.outcome, ExchangeOutcome::Succeeded { activation, viewport } if activation != 0 && viewport == pending.viewport);
        self.model.finish_exchange(receipt.exchange, success).map_err(model_error)?;
        self.reconcile()?;
        // While Exchange is pending the model deliberately issues no capture
        // tickets. A close/privacy Stop may already have published an empty set;
        // refresh surviving views once the exact terminal operation is consumed.
        self.dirty = true;
        Ok(())
    }

    /// UI-only popup/gesture suspension must not invalidate its own Close button.
    /// Resume rebases one deadline; timers never catch up or perform native IO.
    pub fn tick(&mut self, now: u64, blocked: bool, input_busy: bool, flush: &mut Flush<'_>) -> Result<(), PreviewError> {
        self.model.set_blocked(blocked).map_err(model_error)?;
        let suspended = blocked || input_busy;
        if suspended {
            self.timer_suspended = true;
            self.resume_at = None;
            return Ok(());
        }
        if self.timer_suspended {
            self.timer_suspended = false;
            self.resume_at = now.checked_add(u64::from(self.model.interval()) * 1000);
            // Rebase exactly once at resume, without selecting a recovery source.
            self.model.set_interval(self.model.interval(), now).map_err(model_error)?;
        }
        if self.resume_at.is_some_and(|deadline| now < deadline) {
            return Ok(());
        }
        self.resume_at = None;
        if self.model.playing() && self.model.binding(View::Cycling).is_some() {
            if let Some(binding) = self.model.binding(View::Cycling) {
                // Playing has no editable composer; only its own delivered
                // input belongs to a possible timer-driven rebind.
                flush(View::Cycling, binding, self.state(View::Cycling));
            }
            self.model.tick(now).map_err(model_error)?;
            self.reconcile()?;
        }
        Ok(())
    }
}

fn model_error(error: monitor_model::Error) -> PreviewError {
    match error {
        monitor_model::Error::Busy | monitor_model::Error::Capacity => PreviewError::Busy,
        monitor_model::Error::Unavailable => PreviewError::Unavailable,
        monitor_model::Error::Exhausted => PreviewError::Exhausted,
        monitor_model::Error::Invalid | monitor_model::Error::Clock => PreviewError::Invalid,
    }
}

#[cfg(test)]
mod tests;
