//! Host-owned Monitor projection. UI receives sealed reads, never IO endpoints
//! or pixels. Control and metadata ordering is checked under the single store
//! lock before touching SourceImages; capture admission remains Broker-owned.
mod demand;
mod prepare;
mod store;

use super::monitor_model::{Exchange, Options, Ticket, View};
use super::monitor_ui::State;
use super::preview_io::{ControlGuard, HostChannel};
use super::private_image::{PrivateImage, UiPixelBudget};
use super::slots::SlotId;
use super::transport::preview_control::{AcceptedPreviewSource, AcceptedViewSource, Frontdoor};
use aurora_session::preview_image::{AdmittedImage, PreviewSource};
use aurora_session::{HostedViewport, MAX_WORKERS};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};
use std::time::Instant;

pub(super) use prepare::PreparedImage;
pub(super) use store::{BootstrapAdmission, BrokerProjection, RequestKind};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);
fn fresh() -> Result<u64, PreviewError> {
    NEXT_ID
        .try_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_add(1))
        .map_err(|_| PreviewError::Exhausted)
}

/// Presentation identity only, independent of the model's per-instance serial.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ViewNamespace(u64);
impl ViewNamespace {
    pub fn fresh() -> Result<Self, PreviewError> {
        fresh().map(Self)
    }
    pub fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PreviewError {
    Busy,
    Stale,
    Unsupported,
    Unavailable,
    Invalid,
    Exhausted,
    Stopped,
}
impl From<super::transport::HostError> for PreviewError {
    fn from(value: super::transport::HostError) -> Self {
        match value {
            super::transport::HostError::Busy => Self::Busy,
            super::transport::HostError::Unsupported => Self::Unsupported,
            super::transport::HostError::Stopped => Self::Stopped,
            super::transport::HostError::Unavailable => Self::Unavailable,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ViewDescriptor {
    pub view: View,
    pub namespace: ViewNamespace,
    pub ticket: Ticket,
    pub source: PreviewSource,
    pub visible: bool,
    pub options: Options,
}
impl ViewDescriptor {
    pub fn valid(self) -> bool {
        self.namespace.0 != 0 && self.ticket.view() == self.view && self.ticket.source().preview() == self.source
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ViewSetRequest {
    generation: u64,
    request: u64,
}
impl ViewSetRequest {
    pub(in crate::session_host) fn admitted(generation: u64, request: u64) -> Self {
        Self { generation, request }
    }
    pub fn generation(self) -> u64 {
        self.generation
    }
    pub fn request(self) -> u64 {
        self.request
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ImageStatus {
    NoImage,
    Unsupported,
    Denied,
    Busy,
    Pending,
    LastActive,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ViewSource {
    pub slot: SlotId,
    pub source: PreviewSource,
    pub available: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SettingsOutcome {
    Accepted {
        options: Options,
        effective: Options,
        demand: u64,
        native_revision: u64,
    },
    Rejected,
    Cancelled,
    Uncertain,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SettingsReceipt {
    pub request: u64,
    pub view: View,
    pub namespace: ViewNamespace,
    pub ticket: Ticket,
    pub base: State,
    pub desired: Options,
    pub outcome: SettingsOutcome,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ExchangeOutcome {
    Succeeded { activation: u64, viewport: HostedViewport },
    RolledBack,
    Rejected,
    Cancelled,
    Uncertain,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ExchangeReceipt {
    pub request: u64,
    pub namespace: ViewNamespace,
    pub exchange: Exchange,
    pub outcome: ExchangeOutcome,
}

/// Small immutable accepted-control publication. No HostSnapshot is imported.
#[derive(Clone)]
pub(in crate::session_host) struct ControlSnapshot {
    serial: u64,
    sources: Vec<(ViewSource, AcceptedViewSource)>,
    images: Vec<AcceptedPreviewSource>,
    active: Option<PreviewSource>,
    guard: super::preview_io::EpochFence,
}
impl ControlSnapshot {
    fn current(&self) -> bool {
        self.guard.current()
    }
}

struct Shared {
    instance: u64,
    frontdoor: Arc<Frontdoor>,
    control: Mutex<Arc<ControlSnapshot>>,
    control_head: Arc<AtomicU64>,
    store: Mutex<store::Store>,
    io: Mutex<Option<(u64, Arc<HostChannel>)>>,
    io_generation: AtomicU64,
    io_failed: AtomicBool,
    clock: Instant,
    ui_budget: UiPixelBudget,
    operations: Arc<AtomicUsize>,
    retirement: AtomicBool,
}
impl Shared {
    fn try_store(&self) -> Result<StoreGuard<'_>, PreviewError> {
        match self.store.try_lock() {
            Ok(locked) => Ok(StoreGuard {
                shared: self,
                locked: Some(locked),
            }),
            Err(std::sync::TryLockError::WouldBlock) => Err(PreviewError::Busy),
            Err(std::sync::TryLockError::Poisoned(poisoned)) => {
                self.frontdoor.retire();
                self.retirement.swap(true, Ordering::AcqRel);
                drop(poisoned.into_inner());
                self.try_retire_store();
                Err(PreviewError::Unavailable)
            }
        }
    }
    fn retire_store(&self) {
        // Permanent RMW latch pairs with every StoreGuard's post-unlock RMW.
        // Neither retirement nor cleanup can recreate source/image authority.
        self.retirement.swap(true, Ordering::AcqRel);
        self.try_retire_store();
    }
    fn try_retire_store(&self) {
        let mut store = match self.store.try_lock() {
            Ok(store) => store,
            Err(std::sync::TryLockError::WouldBlock) => return,
            Err(std::sync::TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        };
        store.retire_terminal(self);
        let garbage = store.drain_garbage();
        drop(store);
        drop(garbage);
    }
    fn try_control(&self) -> Result<std::sync::MutexGuard<'_, Arc<ControlSnapshot>>, PreviewError> {
        self.control.try_lock().map_err(|error| match error {
            std::sync::TryLockError::WouldBlock => PreviewError::Busy,
            std::sync::TryLockError::Poisoned(_) => {
                self.frontdoor.retire();
                PreviewError::Unavailable
            }
        })
    }
    fn now(&self) -> u64 {
        self.clock.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
    }
    fn observed(&self, at: Instant) -> Result<u64, PreviewError> {
        if at < self.clock {
            return Err(PreviewError::Stale);
        }
        u64::try_from(at.duration_since(self.clock).as_millis()).map_err(|_| PreviewError::Exhausted)
    }
}

/// Every actual Store borrower owns terminal cleanup until AFTER unlocking.
/// This also covers early returns and stop racing the final under-lock check.
struct StoreGuard<'a> {
    shared: &'a Shared,
    locked: Option<std::sync::MutexGuard<'a, store::Store>>,
}
impl std::ops::Deref for StoreGuard<'_> {
    type Target = store::Store;
    fn deref(&self) -> &Self::Target {
        self.locked.as_deref().expect("live Store guard")
    }
}
impl std::ops::DerefMut for StoreGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.locked.as_deref_mut().expect("live Store guard")
    }
}
impl StoreGuard<'_> {
    fn release(&mut self, before_unlock: impl FnOnce()) {
        let Some(mut store) = self.locked.take() else { return };
        if self.shared.retirement.fetch_or(false, Ordering::AcqRel) || !self.shared.frontdoor.current() {
            self.shared.retirement.swap(true, Ordering::AcqRel);
            store.retire_terminal(self.shared);
        }
        let garbage = store.drain_garbage();
        before_unlock();
        drop(store);
        drop(garbage);
        // If stop won before this RMW, we try cleanup after releasing the lock.
        // If stop wins after it, the mutex is already free: stop either cleans
        // itself or transfers to a newer actual StoreGuard. No spin or wait.
        if self.shared.retirement.fetch_or(false, Ordering::AcqRel) || !self.shared.frontdoor.current() {
            self.shared.retirement.swap(true, Ordering::AcqRel);
            self.shared.try_retire_store();
        }
    }
}
impl Drop for StoreGuard<'_> {
    fn drop(&mut self) {
        self.release(|| {});
    }
}

#[derive(Clone)]
pub(super) struct PreviewReader(Arc<Shared>);
impl PreviewReader {
    pub(in crate::session_host) fn new(frontdoor: Arc<Frontdoor>) -> Self {
        let instance = fresh().unwrap_or(0);
        if instance == 0 {
            frontdoor.retire();
        }
        let head = Arc::new(AtomicU64::new(1));
        let initial = ControlSnapshot {
            serial: 1,
            sources: Vec::new(),
            images: Vec::new(),
            active: None,
            guard: super::preview_io::EpochFence::generation(head.clone(), 1),
        };
        Self(Arc::new(Shared {
            instance,
            frontdoor,
            control: Mutex::new(Arc::new(initial)),
            control_head: head,
            store: Mutex::new(store::Store::new()),
            io: Mutex::new(None),
            io_generation: AtomicU64::new(1),
            io_failed: AtomicBool::new(false),
            clock: Instant::now(),
            ui_budget: ui_budget(),
            operations: Arc::new(AtomicUsize::new(0)),
            retirement: AtomicBool::new(false),
        }))
    }
    pub fn ui_budget(&self) -> UiPixelBudget {
        self.0.ui_budget.clone()
    }
    pub fn try_read(&self, view: View, namespace: ViewNamespace) -> Result<PreviewRead, PreviewError> {
        store::read(&self.0, view, namespace)
    }
    pub fn try_status(&self) -> Result<PreviewStatus, PreviewError> {
        store::status(&self.0)
    }
    pub(in crate::session_host) fn reserve(&self) -> Result<OperationSlot, PreviewError> {
        if !self.0.frontdoor.current() || self.0.instance == 0 {
            return Err(PreviewError::Unavailable);
        }
        self.0
            .operations
            .try_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(1).filter(|used| *used <= 16)
            })
            .map_err(|_| PreviewError::Busy)?;
        Ok(OperationSlot(self.0.operations.clone()))
    }
}
/// Bounded operation ownership, not authority. One accepted FIFO packet retains
/// this slot through publication of its immutable terminal receipt.
pub(in crate::session_host) struct OperationSlot(Arc<AtomicUsize>);
impl Drop for OperationSlot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
fn ui_budget() -> UiPixelBudget {
    static BUDGET: std::sync::OnceLock<UiPixelBudget> = std::sync::OnceLock::new();
    BUDGET.get_or_init(UiPixelBudget::new).clone()
}

/// View/control validity is separate from IO/image readiness. Old/None workers
/// can keep an exact canonical Chat owner with an image-free safe body.
#[derive(Clone)]
pub(super) struct PreviewRead {
    view: ViewDescriptor,
    generation: u64,
    request: u64,
    view_guard: super::preview_io::EpochFence,
    source: AcceptedViewSource,
    control_head: super::preview_io::EpochFence,
    shared: Arc<Shared>,
    now: u64,
    status: ImageStatus,
    image: Option<Arc<PreparedImage>>,
    image_guard: Option<ImageReadGuard>,
    commands: bool,
    settings: Vec<SettingsReceipt>,
    exchanges: Vec<ExchangeReceipt>,
}
#[derive(Clone)]
struct ImageReadGuard {
    control: ControlGuard,
    metadata_head: super::preview_io::PublicationGuard,
    service: u64,
}
impl PreviewRead {
    pub fn view(&self) -> View {
        self.view.view
    }
    pub fn namespace(&self) -> ViewNamespace {
        self.view.namespace
    }
    pub fn source(&self) -> PreviewSource {
        self.view.source
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn request(&self) -> u64 {
        self.request
    }
    pub fn now(&self) -> u64 {
        self.shared.now().max(self.now)
    }
    pub fn view_current(&self) -> bool {
        self.shared.frontdoor.current()
            && self.view_guard.current()
            && self.source.current(Instant::now())
            && self.shared.frontdoor.view_generation() == self.generation
    }
    pub fn image_current(&self) -> bool {
        self.view_current() && self.image.as_ref().is_some_and(|image| image.current())
    }
    pub fn commands_current(&self) -> bool {
        self.commands && self.view_current() && self.control_head.current()
    }
    /// Preserve only the ORIGINAL stable view scope during image contention.
    /// The native owner erases its prior private image before using this body;
    /// canonical Chat still applies its own current conversation permissions.
    pub fn view_only(&self) -> Option<Self> {
        if !self.view_current() {
            return None;
        }
        let body = Self {
            view: self.view,
            generation: self.generation,
            request: self.request,
            view_guard: self.view_guard.clone(),
            source: self.source.clone(),
            control_head: self.control_head.clone(),
            shared: self.shared.clone(),
            now: self.now,
            status: ImageStatus::Busy,
            image: None,
            image_guard: None,
            commands: false,
            settings: Vec::new(),
            exchanges: Vec::new(),
        };
        body.view_current().then_some(body)
    }
    pub fn image_status(&self) -> ImageStatus {
        self.status
    }
    pub fn prepared(&self) -> Option<&Arc<PreparedImage>> {
        self.image.as_ref().filter(|_| self.image_current())
    }
    pub fn refresh_prepared(&self, old: &Arc<PreparedImage>) -> Option<Arc<PreparedImage>> {
        old.refresh(self).map(Arc::new)
    }
    /// The single GUI manager also compares its ENTIRE captured State before
    /// calling HostController. This backend check covers independently owned
    /// binding/config facts; it does not invent a second chat/pin/UI revision.
    pub(in crate::session_host) fn admits_settings(&self, ticket: Ticket, base: State) -> bool {
        self.commands_current() && base.request.is_none() && self.view.ticket.same_binding(ticket) && self.view.options == base.accepted
    }
    pub fn settings_receipts(&self) -> &[SettingsReceipt] {
        &self.settings
    }
    pub fn exchange_receipts(&self) -> &[ExchangeReceipt] {
        &self.exchanges
    }
}
pub(super) struct PreviewStatus {
    control: Arc<ControlSnapshot>,
    generation: u64,
    request: u64,
    view_guard: super::preview_io::EpochFence,
    shared: Arc<Shared>,
    sources: Vec<ViewSource>,
    settings: Vec<SettingsReceipt>,
    exchanges: Vec<ExchangeReceipt>,
}
impl PreviewStatus {
    pub fn current(&self) -> bool {
        self.shared.frontdoor.current()
            && self.control.current()
            && self.view_guard.current()
            && self.control.sources.iter().all(|(_, source)| source.current(Instant::now()))
    }
    pub fn sources(&self) -> &[ViewSource] {
        &self.sources
    }
    pub fn active(&self) -> Option<PreviewSource> {
        self.control.active
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn request(&self) -> u64 {
        self.request
    }
    pub fn settings_receipts(&self) -> &[SettingsReceipt] {
        &self.settings
    }
    pub fn exchange_receipts(&self) -> &[ExchangeReceipt] {
        &self.exchanges
    }
}

#[cfg(test)]
mod tests;
