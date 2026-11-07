//! Pure authority and bounded metadata for a future authenticated image lane.
//! No socket, image decoder, pixel Vec, capture command or GPU grant is provided.
//! Serde here is a strict metadata representation, not an image wire protocol.
//! Size/default/history behavior follows the Legacy `fssessionpreview.h` and
//! `fssessionmonitormodel.h`; first images are explicitly last-active history.

use crate::{MAX_WORKERS, PreviewDemand, PreviewSize, SessionIdentity, WorkerIdentity};
use serde::{Deserialize, Serialize};
use std::sync::{
    Arc,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};

pub const MAX_IMAGE_BYTES: usize = 640 * 360 * 4;
pub const HOST_IMAGE_BYTES: usize = 64 * 1024 * 1024;
pub const IMAGE_AUTHORITY_MILLIS: u64 = 1000;
pub const STALE_IMAGE_MILLIS: u64 = 5000;
static NEXT_SOURCE_IMAGES: AtomicU64 = AtomicU64::new(1);

fn source_instance(counter: &AtomicU64) -> Result<u64, ImageError> {
    counter
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |next| next.checked_add(1))
        .map_err(|_| ImageError::Exhausted)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageError {
    Invalid,
    Stale,
    Busy,
    Capacity,
    Clock,
    Exhausted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "SourceFields", into = "SourceFields")]
pub struct PreviewSource {
    worker: WorkerIdentity,
    session: SessionIdentity,
    lease_epoch: u64,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceFields {
    worker: WorkerIdentity,
    session: SessionIdentity,
    lease_epoch: u64,
}

impl PreviewSource {
    pub fn new(worker: WorkerIdentity, session: SessionIdentity, lease_epoch: u64) -> Result<Self, ImageError> {
        if session.generation == 0 || lease_epoch == 0 {
            return Err(ImageError::Invalid);
        }
        Ok(Self {
            worker,
            session,
            lease_epoch,
        })
    }
    pub fn worker(self) -> WorkerIdentity {
        self.worker
    }
    pub fn session(self) -> SessionIdentity {
        self.session
    }
    pub fn lease_epoch(self) -> u64 {
        self.lease_epoch
    }
}
impl TryFrom<SourceFields> for PreviewSource {
    type Error = &'static str;
    fn try_from(value: SourceFields) -> Result<Self, Self::Error> {
        Self::new(value.worker, value.session, value.lease_epoch).map_err(|_| "invalid preview source")
    }
}
impl From<PreviewSource> for SourceFields {
    fn from(value: PreviewSource) -> Self {
        Self {
            worker: value.worker,
            session: value.session,
            lease_epoch: value.lease_epoch,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "EpochFields", into = "EpochFields")]
pub struct PreviewEpochs {
    revision: u64,
    privacy: u64,
    render: u64,
    viewport: u64,
    surface: u64,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EpochFields {
    revision: u64,
    privacy: u64,
    render: u64,
    viewport: u64,
    surface: u64,
}

impl PreviewEpochs {
    pub fn new(revision: u64, privacy: u64, render: u64, viewport: u64, surface: u64) -> Result<Self, ImageError> {
        if [revision, privacy, render, viewport, surface].contains(&0) {
            return Err(ImageError::Invalid);
        }
        Ok(Self {
            revision,
            privacy,
            render,
            viewport,
            surface,
        })
    }
    pub fn revision(self) -> u64 {
        self.revision
    }
    pub fn privacy(self) -> u64 {
        self.privacy
    }
    pub fn render(self) -> u64 {
        self.render
    }
    pub fn viewport(self) -> u64 {
        self.viewport
    }
    pub fn surface(self) -> u64 {
        self.surface
    }
    fn follows(self, old: Self) -> bool {
        self.revision >= old.revision
            && self.privacy >= old.privacy
            && self.render >= old.render
            && self.viewport >= old.viewport
            && (self.render > old.render || self.surface >= old.surface)
    }
}
impl TryFrom<EpochFields> for PreviewEpochs {
    type Error = &'static str;
    fn try_from(value: EpochFields) -> Result<Self, Self::Error> {
        Self::new(value.revision, value.privacy, value.render, value.viewport, value.surface).map_err(|_| "invalid preview epochs")
    }
}
impl From<PreviewEpochs> for EpochFields {
    fn from(value: PreviewEpochs) -> Self {
        Self {
            revision: value.revision,
            privacy: value.privacy,
            render: value.render,
            viewport: value.viewport,
            surface: value.surface,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewAccess {
    pub ready: bool,
    pub active: bool,
    pub activation: u64,
    pub capture_active: bool,
    pub retain_last_active: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "PolicyFields", into = "PolicyFields")]
pub struct PreviewPolicy {
    source: PreviewSource,
    epochs: PreviewEpochs,
    access: PreviewAccess,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyFields {
    source: PreviewSource,
    epochs: PreviewEpochs,
    access: PreviewAccess,
}

impl PreviewPolicy {
    pub fn new(source: PreviewSource, epochs: PreviewEpochs, access: PreviewAccess) -> Result<Self, ImageError> {
        if (access.active && (!access.ready || access.activation == 0))
            || (access.retain_last_active && !access.ready)
            || (access.capture_active && (!access.active || !access.retain_last_active))
        {
            return Err(ImageError::Invalid);
        }
        Ok(Self { source, epochs, access })
    }
    pub fn source(self) -> PreviewSource {
        self.source
    }
    pub fn epochs(self) -> PreviewEpochs {
        self.epochs
    }
    pub fn access(self) -> PreviewAccess {
        self.access
    }
}
impl TryFrom<PolicyFields> for PreviewPolicy {
    type Error = &'static str;
    fn try_from(value: PolicyFields) -> Result<Self, Self::Error> {
        Self::new(value.source, value.epochs, value.access).map_err(|_| "invalid preview policy")
    }
}
impl From<PreviewPolicy> for PolicyFields {
    fn from(value: PreviewPolicy) -> Self {
        Self {
            source: value.source,
            epochs: value.epochs,
            access: value.access,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "GeometryFields", into = "GeometryFields")]
pub struct PreviewGeometry {
    source_size: [u32; 2],
    rect: [u32; 4],
    cap: PreviewSize,
    size: [u32; 2],
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GeometryFields {
    source_size: [u32; 2],
    rect: [u32; 4],
    cap: PreviewSize,
    size: [u32; 2],
}

impl PreviewGeometry {
    pub fn new(source_size: [u32; 2], rect: [u32; 4], cap: PreviewSize) -> Result<Self, ImageError> {
        let [x, y, width, height] = rect;
        if width == 0
            || height == 0
            || source_size.contains(&0)
            || x.checked_add(width).is_none_or(|right| right > source_size[0])
            || y.checked_add(height).is_none_or(|bottom| bottom > source_size[1])
        {
            return Err(ImageError::Invalid);
        }
        let (max_w, max_h) = cap.dimensions();
        let mut w = u64::from(max_w);
        let mut h = w * u64::from(height) / u64::from(width);
        if h > u64::from(max_h) {
            h = u64::from(max_h);
            w = h * u64::from(width) / u64::from(height);
        }
        w -= w % 4;
        if w < 4 || h < 4 {
            return Err(ImageError::Invalid);
        }
        Ok(Self {
            source_size,
            rect,
            cap,
            size: [w as u32, h as u32],
        })
    }
    pub fn source_size(self) -> [u32; 2] {
        self.source_size
    }
    pub fn rect(self) -> [u32; 4] {
        self.rect
    }
    pub fn cap(self) -> PreviewSize {
        self.cap
    }
    pub fn size(self) -> [u32; 2] {
        self.size
    }
    pub fn row_bytes(self) -> u32 {
        self.size[0] * 4
    }
    pub fn payload_bytes(self) -> usize {
        (u64::from(self.row_bytes()) * u64::from(self.size[1])) as usize
    }
}
impl TryFrom<GeometryFields> for PreviewGeometry {
    type Error = &'static str;
    fn try_from(value: GeometryFields) -> Result<Self, Self::Error> {
        let geometry = Self::new(value.source_size, value.rect, value.cap).map_err(|_| "invalid preview geometry")?;
        if geometry.size != value.size {
            return Err("preview size does not fit source");
        }
        Ok(geometry)
    }
}
impl From<PreviewGeometry> for GeometryFields {
    fn from(value: PreviewGeometry) -> Self {
        Self {
            source_size: value.source_size,
            rect: value.rect,
            cap: value.cap,
            size: value.size,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewFormat {
    Rgba8OpaqueSrgb,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewKind {
    LastActive,
}

/// This request association is issued by SourceImages, never by a pixel peer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "ClaimFields", into = "ClaimFields")]
pub struct CaptureClaim {
    source: PreviewSource,
    epochs: PreviewEpochs,
    activation: u64,
    instance: u64,
    connection: u64,
    demand: u64,
    request: u64,
    geometry: PreviewGeometry,
}
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClaimFields {
    source: PreviewSource,
    epochs: PreviewEpochs,
    activation: u64,
    instance: u64,
    connection: u64,
    demand: u64,
    request: u64,
    geometry: PreviewGeometry,
}
impl CaptureClaim {
    pub fn source(self) -> PreviewSource {
        self.source
    }
    pub fn request(self) -> u64 {
        self.request
    }
    pub fn connection(self) -> u64 {
        self.connection
    }
    pub fn cap(self) -> PreviewSize {
        self.geometry.cap
    }
    pub fn geometry(self) -> PreviewGeometry {
        self.geometry
    }
    fn valid(self) -> bool {
        self.instance != 0 && self.activation != 0 && self.connection != 0 && self.demand != 0 && self.request != 0
    }
}
impl TryFrom<ClaimFields> for CaptureClaim {
    type Error = &'static str;
    fn try_from(value: ClaimFields) -> Result<Self, Self::Error> {
        let claim = Self {
            source: value.source,
            epochs: value.epochs,
            activation: value.activation,
            instance: value.instance,
            connection: value.connection,
            demand: value.demand,
            request: value.request,
            geometry: value.geometry,
        };
        if !claim.valid() {
            return Err("invalid preview claim");
        }
        Ok(claim)
    }
}
impl From<CaptureClaim> for ClaimFields {
    fn from(value: CaptureClaim) -> Self {
        Self {
            source: value.source,
            epochs: value.epochs,
            activation: value.activation,
            instance: value.instance,
            connection: value.connection,
            demand: value.demand,
            request: value.request,
            geometry: value.geometry,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "StampFields", into = "StampFields")]
pub struct PreviewStamp {
    claim: CaptureClaim,
    sequence: u64,
    surface_epoch: u64,
    frame: u64,
    geometry: PreviewGeometry,
    format: PreviewFormat,
    kind: PreviewKind,
}
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StampFields {
    claim: CaptureClaim,
    sequence: u64,
    surface_epoch: u64,
    frame: u64,
    geometry: PreviewGeometry,
    format: PreviewFormat,
    kind: PreviewKind,
}
impl PreviewStamp {
    pub fn new(claim: CaptureClaim, sequence: u64, surface_epoch: u64, frame: u64, geometry: PreviewGeometry) -> Result<Self, ImageError> {
        if !claim.valid() || sequence == 0 || surface_epoch != claim.epochs.surface || geometry != claim.geometry {
            return Err(ImageError::Invalid);
        }
        Ok(Self {
            claim,
            sequence,
            surface_epoch,
            frame,
            geometry,
            format: PreviewFormat::Rgba8OpaqueSrgb,
            kind: PreviewKind::LastActive,
        })
    }
    pub fn claim(self) -> CaptureClaim {
        self.claim
    }
    pub fn sequence(self) -> u64 {
        self.sequence
    }
    pub fn surface_epoch(self) -> u64 {
        self.surface_epoch
    }
    pub fn frame(self) -> u64 {
        self.frame
    }
    pub fn geometry(self) -> PreviewGeometry {
        self.geometry
    }
    pub fn format(self) -> PreviewFormat {
        self.format
    }
    pub fn kind(self) -> PreviewKind {
        self.kind
    }
}
impl TryFrom<StampFields> for PreviewStamp {
    type Error = &'static str;
    fn try_from(value: StampFields) -> Result<Self, Self::Error> {
        Self::new(value.claim, value.sequence, value.surface_epoch, value.frame, value.geometry).map_err(|_| "invalid preview stamp")
    }
}
impl From<PreviewStamp> for StampFields {
    fn from(value: PreviewStamp) -> Self {
        Self {
            claim: value.claim,
            sequence: value.sequence,
            surface_epoch: value.surface_epoch,
            frame: value.frame,
            geometry: value.geometry,
            format: value.format,
            kind: value.kind,
        }
    }
}

/// Same-host request/receive bounds, not an invented shared worker clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureAge {
    sent: u64,
    received: u64,
}
impl CaptureAge {
    pub fn new(sent: u64, received: u64) -> Result<Self, ImageError> {
        if received < sent || received.checked_sub(sent).is_none_or(|elapsed| elapsed > IMAGE_AUTHORITY_MILLIS) {
            return Err(ImageError::Clock);
        }
        Ok(Self { sent, received })
    }
    /// Inclusive youngest/oldest possible age. Receipt/paint never rejuvenates.
    pub fn bounds(self, now: u64) -> Result<(u64, u64), ImageError> {
        Ok((
            now.checked_sub(self.received).ok_or(ImageError::Clock)?,
            now.checked_sub(self.sent).ok_or(ImageError::Clock)?,
        ))
    }
    pub fn stale(self, now: u64) -> Result<bool, ImageError> {
        Ok(self.bounds(now)?.1 > STALE_IMAGE_MILLIS)
    }
}

#[derive(Clone)]
pub struct ImageBudget(Arc<Budget>);
struct Budget {
    limit: usize,
    used: AtomicUsize,
}
struct Credit {
    budget: ImageBudget,
    bytes: usize,
}
/// Credit follows the actual payload's clones until all owners drop it. A
/// separate physical pixel copy requires a separate reservation before copying.
#[derive(Clone)]
pub struct ImageCredit(Arc<Credit>);
impl ImageBudget {
    /// Construct once per host image service, then pass clones to every source,
    /// receiver and replacement model. Recreating a source must reuse this pool;
    /// a new independent pool is not a way to retire still-held allocations.
    pub fn new(limit: usize) -> Result<Self, ImageError> {
        if limit == 0 || limit > HOST_IMAGE_BYTES {
            return Err(ImageError::Capacity);
        }
        Ok(Self(Arc::new(Budget {
            limit,
            used: AtomicUsize::new(0),
        })))
    }
    pub fn used(&self) -> usize {
        self.0.used.load(Ordering::Acquire)
    }
    fn can_reserve(&self, bytes: usize) -> bool {
        self.used().checked_add(bytes).is_some_and(|next| next <= self.0.limit)
    }
    pub fn reserve(&self, geometry: PreviewGeometry) -> Result<ImageCredit, ImageError> {
        let bytes = geometry.payload_bytes();
        if bytes == 0 || bytes > MAX_IMAGE_BYTES {
            return Err(ImageError::Invalid);
        }
        self.0
            .used
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes).filter(|next| *next <= self.0.limit)
            })
            .map_err(|_| ImageError::Capacity)?;
        Ok(ImageCredit(Arc::new(Credit {
            budget: self.clone(),
            bytes,
        })))
    }
}
impl ImageCredit {
    pub fn bytes(&self) -> usize {
        self.0.bytes
    }
}
impl Drop for Credit {
    fn drop(&mut self) {
        self.budget.0.used.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

struct Validity {
    epoch: AtomicU64,
    expires: AtomicU64,
}
#[derive(Clone)]
pub struct AdmittedImage {
    stamp: PreviewStamp,
    age: CaptureAge,
    credit: ImageCredit,
    validity: Arc<Validity>,
    epoch: u64,
}
impl AdmittedImage {
    pub fn stamp(&self) -> PreviewStamp {
        self.stamp
    }
    pub fn age(&self) -> CaptureAge {
        self.age
    }
    pub fn bytes(&self) -> usize {
        self.credit.bytes()
    }
    /// A copied snapshot cannot keep revoked pixels eligible. Surface code must
    /// check this immediately before presenting and erase its own old textures.
    /// This revokes eligibility; it does not zero already-borrowed bytes, CPU
    /// texture storage or an OS surface. That erasure belongs to integration.
    pub fn current(&self, now: u64) -> bool {
        self.epoch == self.validity.epoch.load(Ordering::Acquire)
            && now < self.validity.expires.load(Ordering::Acquire)
            && self.age.bounds(now).is_ok()
            && self.epoch == self.validity.epoch.load(Ordering::Acquire)
    }
}

struct Pending {
    claim: CaptureClaim,
    sent: u64,
}

/// One exact source for its lifetime, one pending request and one retained
/// admitted metadata handle. Only an authenticated owner may synchronize policy;
/// these booleans themselves never prove or grant native rendering authority.
pub struct SourceImages {
    source: PreviewSource,
    instance: u64,
    policy: Option<PreviewPolicy>,
    validity: Arc<Validity>,
    budget: ImageBudget,
    pending: Option<Pending>,
    image: Option<AdmittedImage>,
    next_request: u64,
    connection: u64,
    demand: u64,
    sequence: u64,
    now: u64,
    retired: bool,
}
impl Drop for SourceImages {
    fn drop(&mut self) {
        self.retire();
    }
}
impl SourceImages {
    pub fn new(source: PreviewSource, budget: ImageBudget) -> Result<Self, ImageError> {
        let instance = source_instance(&NEXT_SOURCE_IMAGES)?;
        Ok(Self {
            source,
            instance,
            budget,
            policy: None,
            validity: Arc::new(Validity {
                epoch: AtomicU64::new(1),
                expires: AtomicU64::new(0),
            }),
            pending: None,
            image: None,
            next_request: 1,
            connection: 0,
            demand: 0,
            sequence: 0,
            now: 0,
            retired: false,
        })
    }

    pub fn sync(&mut self, policy: PreviewPolicy, now: u64) -> Result<(), ImageError> {
        self.tick(now)?;
        if self.retired {
            return Err(ImageError::Stale);
        }
        if policy.source != self.source {
            self.retire();
            return Err(ImageError::Stale);
        }
        let expires = now.checked_add(IMAGE_AUTHORITY_MILLIS).ok_or_else(|| {
            self.retire();
            ImageError::Clock
        })?;
        if let Some(old) = self.policy {
            if !policy.epochs.follows(old.epochs) || policy.access.activation < old.access.activation {
                self.purge()?;
                return Err(ImageError::Stale);
            }
            if policy.epochs == old.epochs && policy.access.retain_last_active != old.access.retain_last_active {
                self.purge()?;
                return Err(ImageError::Invalid);
            }
            if policy.epochs != old.epochs {
                self.purge()?;
            }
            if policy.access.activation != old.access.activation || !policy.access.capture_active {
                self.pending = None;
            }
        }
        if !policy.access.retain_last_active {
            self.purge()?;
        }
        self.policy = Some(policy);
        self.validity.expires.store(expires, Ordering::Release);
        Ok(())
    }

    pub fn request(&mut self, connection: u64, demand: u64, geometry: PreviewGeometry, now: u64) -> Result<CaptureClaim, ImageError> {
        self.tick(now)?;
        let policy = self.policy.filter(|policy| policy.access.capture_active).ok_or(ImageError::Stale)?;
        if self.retired || self.validity.expires.load(Ordering::Acquire) <= now {
            return Err(ImageError::Stale);
        }
        if self.pending.is_some() {
            return Err(ImageError::Busy);
        }
        // This early skip avoids known futile capture work. The IO allocator
        // must still reserve credit atomically before actually allocating.
        if !self.budget.can_reserve(geometry.payload_bytes()) {
            return Err(ImageError::Capacity);
        }
        if connection == 0 || demand == 0 || connection < self.connection || connection == self.connection && demand < self.demand {
            return Err(ImageError::Stale);
        }
        let next = self.next_request.checked_add(1).ok_or_else(|| {
            self.retire();
            ImageError::Exhausted
        })?;
        if connection != self.connection {
            self.sequence = 0;
        }
        self.connection = connection;
        self.demand = demand;
        let claim = CaptureClaim {
            source: self.source,
            epochs: policy.epochs,
            activation: policy.access.activation,
            instance: self.instance,
            connection,
            demand,
            request: self.next_request,
            geometry,
        };
        self.next_request = next;
        self.pending = Some(Pending { claim, sent: now });
        Ok(claim)
    }

    /// Metadata admission follows authenticated payload verification. Credit
    /// must have been reserved before the future IO layer allocates that body.
    pub fn admit(&mut self, stamp: PreviewStamp, credit: ImageCredit, now: u64) -> Result<AdmittedImage, ImageError> {
        self.tick(now)?;
        let policy = self.policy.filter(|policy| policy.access.capture_active).ok_or(ImageError::Stale)?;
        let pending = self.pending.as_ref().ok_or(ImageError::Stale)?;
        if self.retired
            || self.validity.expires.load(Ordering::Acquire) <= now
            || stamp.claim != pending.claim
            || stamp.claim.source != self.source
            || stamp.claim.epochs != policy.epochs
            || stamp.claim.activation != policy.access.activation
            || stamp.sequence <= self.sequence
            || credit.bytes() != stamp.geometry.payload_bytes()
            || !Arc::ptr_eq(&credit.0.budget.0, &self.budget.0)
        {
            return Err(ImageError::Stale);
        }
        let age = CaptureAge::new(pending.sent, now)?;
        let image = AdmittedImage {
            stamp,
            age,
            credit,
            validity: self.validity.clone(),
            epoch: self.validity.epoch.load(Ordering::Acquire),
        };
        self.pending = None;
        self.sequence = stamp.sequence;
        self.image = Some(image.clone());
        Ok(image)
    }

    pub fn retained(&mut self, now: u64) -> Result<Option<&AdmittedImage>, ImageError> {
        self.tick(now)?;
        Ok(self.image.as_ref().filter(|image| image.current(now)))
    }

    /// Source/account/grid/generation/lease replacement, detach or source loss
    /// retires this object permanently. A new source needs a new model.
    pub fn retire(&mut self) {
        self.retired = true;
        self.validity.expires.store(0, Ordering::Release);
        self.pending = None;
        self.image = None;
        self.policy = None;
    }

    fn purge(&mut self) -> Result<(), ImageError> {
        self.pending = None;
        self.image = None;
        self.validity.expires.store(0, Ordering::Release);
        let epoch = self.validity.epoch.load(Ordering::Acquire);
        let next = epoch.checked_add(1).ok_or_else(|| {
            self.retire();
            ImageError::Exhausted
        })?;
        self.validity.epoch.store(next, Ordering::Release);
        Ok(())
    }

    fn tick(&mut self, now: u64) -> Result<(), ImageError> {
        if now < self.now {
            self.retire();
            return Err(ImageError::Clock);
        }
        self.now = now;
        if self.validity.expires.load(Ordering::Acquire) <= now {
            self.purge()?;
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| now.saturating_sub(pending.sent) >= IMAGE_AUTHORITY_MILLIS)
        {
            self.pending = None;
        }
        Ok(())
    }
}

/// A surface's request, not an authority. At most four fixed + one cycling.
#[derive(Clone, Copy)]
pub struct PreviewViewDemand {
    pub source: PreviewSource,
    pub demand: PreviewDemand,
    pub size: PreviewSize,
}
#[derive(Clone, Copy)]
pub struct PreviewSourceDemand {
    pub source: PreviewSource,
    pub demand: PreviewDemand,
    pub size: PreviewSize,
}

/// Deduplicate views before rate admission. Reservations survive invisible
/// subscribers; source readiness/inactive facts must agree across duplicates.
pub fn unique_preview_demands(views: &[PreviewViewDemand]) -> Result<Vec<PreviewSourceDemand>, ImageError> {
    if views.len() > MAX_WORKERS {
        return Err(ImageError::Capacity);
    }
    let mut unique: Vec<PreviewSourceDemand> = Vec::with_capacity(MAX_WORKERS);
    for view in views {
        if view.demand.acknowledged_half_fps > 10 || view.demand.pending_half_fps > 10 {
            return Err(ImageError::Invalid);
        }
        if let Some(found) = unique.iter_mut().find(|found| found.source == view.source) {
            if found.demand.inactive != view.demand.inactive || found.demand.session_current != view.demand.session_current {
                return Err(ImageError::Invalid);
            }
            found.demand.acknowledged_half_fps = found.demand.acknowledged_half_fps.max(view.demand.acknowledged_half_fps);
            found.demand.pending_half_fps = found.demand.pending_half_fps.max(view.demand.pending_half_fps);
            if view.demand.visible {
                found.demand.requested = found.demand.requested.max(view.demand.requested);
                if view.size.bytes() > found.size.bytes() {
                    found.size = view.size;
                }
                found.demand.visible = true;
            }
        } else {
            let mut demand = view.demand;
            if !demand.visible {
                demand.requested = None;
            }
            unique.push(PreviewSourceDemand {
                source: view.source,
                demand,
                size: if demand.visible { view.size } else { PreviewSize::default() },
            });
        }
    }
    Ok(unique)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AccountKey, GridKey, HostId, InstallationId, PreviewRate, SessionId, WorkerId, admit_previews, admit_previews_with_active,
    };

    fn source(id: u8) -> PreviewSource {
        PreviewSource::new(
            WorkerIdentity {
                installation: InstallationId::from_bytes([1; 32]).unwrap(),
                host: HostId::from_bytes([2; 16]).unwrap(),
                worker: WorkerId::from_bytes([id; 16]).unwrap(),
            },
            SessionIdentity {
                session: SessionId::from_bytes([id; 16]).unwrap(),
                account: AccountKey::from_bytes([id; 32]).unwrap(),
                grid: GridKey::from_bytes([3; 32]).unwrap(),
                generation: 1,
            },
            1,
        )
        .unwrap()
    }
    fn epochs(revision: u64) -> PreviewEpochs {
        PreviewEpochs::new(revision, 1, 1, 1, 1).unwrap()
    }
    fn policy(id: u8) -> PreviewPolicy {
        PreviewPolicy::new(
            source(id),
            epochs(1),
            PreviewAccess {
                ready: true,
                active: true,
                activation: 7,
                capture_active: true,
                retain_last_active: true,
            },
        )
        .unwrap()
    }
    fn geometry() -> PreviewGeometry {
        PreviewGeometry::new([1920, 1080], [100, 100, 1600, 900], PreviewSize::Small).unwrap()
    }
    fn model(budget: &ImageBudget) -> SourceImages {
        let mut model = SourceImages::new(source(1), budget.clone()).unwrap();
        model.sync(policy(1), 0).unwrap();
        model
    }
    fn admit(model: &mut SourceImages, budget: &ImageBudget, sequence: u64, now: u64) -> AdmittedImage {
        let claim = model.request(1, 1, geometry(), now).unwrap();
        let stamp = PreviewStamp::new(claim, sequence, 1, 0, geometry()).unwrap();
        model.admit(stamp, budget.reserve(geometry()).unwrap(), now + 1).unwrap()
    }

    #[test]
    fn preview_image_metadata_rejects_unknown_fields_bad_sources_flags_and_geometry() {
        let valid = serde_json::to_value(policy(1)).unwrap();
        assert_eq!(serde_json::from_value::<PreviewPolicy>(valid.clone()).unwrap(), policy(1));
        for (path, value) in [
            (vec!["unexpected"], serde_json::json!(true)),
            (vec!["source", "lease_epoch"], serde_json::json!(0)),
            (vec!["source", "session", "generation"], serde_json::json!(0)),
            (vec!["source", "worker", "worker"], serde_json::json!("00".repeat(16))),
            (vec!["epochs", "revision"], serde_json::json!(0)),
            (vec!["access", "ready"], serde_json::json!(false)),
            (vec!["access", "active"], serde_json::json!(false)),
            (vec!["access", "retain_last_active"], serde_json::json!(false)),
            (vec!["access", "activation"], serde_json::json!(0)),
        ] {
            let mut invalid = valid.clone();
            let mut at = &mut invalid;
            for part in &path[..path.len() - 1] {
                at = &mut at[*part];
            }
            at[*path.last().unwrap()] = value;
            assert!(serde_json::from_value::<PreviewPolicy>(invalid).is_err(), "{path:?}");
        }
        let mut bad = serde_json::to_value(geometry()).unwrap();
        bad["size"] = serde_json::json!([640, 360]);
        assert!(serde_json::from_value::<PreviewGeometry>(bad).is_err());
        assert!(PreviewGeometry::new([1920, 1080], [u32::MAX, 0, 4, 4], PreviewSize::Small).is_err());
        assert!(PreviewGeometry::new([1920, 1080], [0, 0, 0, 1], PreviewSize::Small).is_err());
        for rect in [[0, 0, 1, u32::MAX], [0, 0, u32::MAX, 1]] {
            assert!(PreviewGeometry::new([u32::MAX, u32::MAX], rect, PreviewSize::Large).is_err());
        }
        for cap in [PreviewSize::Small, PreviewSize::Medium, PreviewSize::Large] {
            let fitted = PreviewGeometry::new([1920, 1080], [0, 0, 1920, 1080], cap).unwrap();
            let (width, height) = cap.dimensions();
            assert_eq!(fitted.size(), [width, height]);
            assert_eq!(fitted.payload_bytes(), cap.bytes() as usize);
            assert!(fitted.payload_bytes() <= MAX_IMAGE_BYTES);
        }
    }

    #[test]
    fn preview_image_stamp_has_one_exact_format_kind_request_and_geometry() {
        let budget = ImageBudget::new(HOST_IMAGE_BYTES).unwrap();
        let mut model = model(&budget);
        let claim = model.request(1, 1, geometry(), 10).unwrap();
        let stamp = PreviewStamp::new(claim, 1, 1, 0, geometry()).unwrap();
        assert_eq!(stamp.format(), PreviewFormat::Rgba8OpaqueSrgb);
        assert_eq!(stamp.kind(), PreviewKind::LastActive);
        assert_eq!(stamp.frame(), 0);
        assert!(PreviewStamp::new(claim, 0, 1, 0, geometry()).is_err());
        assert!(
            PreviewStamp::new(claim, 1, 2, 0, geometry()).is_err(),
            "surface must match captured authority"
        );
        let other = PreviewGeometry::new([1920, 1080], [0, 0, 1920, 1080], PreviewSize::Small).unwrap();
        assert!(PreviewStamp::new(claim, 1, 1, 0, other).is_err());
        let encoded = serde_json::to_value(stamp).unwrap();
        for (field, value) in [
            ("format", serde_json::json!("jpeg")),
            ("kind", serde_json::json!("live")),
            ("surface_epoch", serde_json::json!(0)),
            ("pixels", serde_json::json!([])),
        ] {
            let mut invalid = encoded.clone();
            invalid[field] = value;
            assert!(serde_json::from_value::<PreviewStamp>(invalid).is_err());
        }
        let mut invalid = encoded.clone();
        invalid["claim"]["request"] = serde_json::json!(0);
        assert!(serde_json::from_value::<PreviewStamp>(invalid).is_err());
        assert_eq!(serde_json::from_value::<PreviewStamp>(encoded).unwrap(), stamp);
    }

    #[test]
    fn preview_image_parking_keeps_only_already_admitted_history_and_rejects_late_frames() {
        let budget = ImageBudget::new(HOST_IMAGE_BYTES).unwrap();
        let mut model = model(&budget);
        let historical = admit(&mut model, &budget, 1, 10);
        let late_claim = model.request(1, 1, geometry(), 20).unwrap();
        let late = PreviewStamp::new(late_claim, 2, 1, 1, geometry()).unwrap();
        let parked = PreviewPolicy::new(
            source(1),
            epochs(1),
            PreviewAccess {
                ready: true,
                active: false,
                activation: 8,
                capture_active: false,
                retain_last_active: true,
            },
        )
        .unwrap();
        model.sync(parked, 21).unwrap();
        assert!(historical.current(22));
        assert_eq!(model.retained(22).unwrap().unwrap().stamp(), historical.stamp());
        assert!(model.admit(late, budget.reserve(geometry()).unwrap(), 22).is_err());
        assert!(model.request(1, 1, geometry(), 23).is_err());
        assert_eq!(
            historical.stamp().claim.activation,
            7,
            "retained frame is historical, never relabelled as new activation 8"
        );
        let mut resume = policy(1);
        resume.access.activation = 9;
        model.sync(resume, 24).unwrap();
        let fresh = model.request(1, 1, geometry(), 25).unwrap();
        assert_ne!(fresh.request(), late_claim.request());
        assert!(model.admit(late, budget.reserve(geometry()).unwrap(), 26).is_err());
        assert!(model.retained(26).unwrap().is_some());
    }

    #[test]
    fn preview_image_source_privacy_view_and_device_revocations_invalidate_held_snapshots() {
        let budget = ImageBudget::new(HOST_IMAGE_BYTES).unwrap();
        for changed in [
            PreviewEpochs::new(2, 2, 1, 1, 1).unwrap(),
            PreviewEpochs::new(2, 1, 2, 1, 1).unwrap(),
            PreviewEpochs::new(2, 1, 1, 2, 1).unwrap(),
            PreviewEpochs::new(2, 1, 1, 1, 2).unwrap(),
        ] {
            let mut model = model(&budget);
            let held = admit(&mut model, &budget, 1, 10);
            let mut next = policy(1);
            next.epochs = changed;
            next.access.capture_active = false;
            next.access.retain_last_active = false;
            model.sync(next, 12).unwrap();
            assert!(!held.current(12));
            assert!(model.retained(12).unwrap().is_none());
            next.epochs = PreviewEpochs::new(3, changed.privacy, changed.render, changed.viewport, changed.surface).unwrap();
            next.access.capture_active = true;
            next.access.retain_last_active = true;
            model.sync(next, 13).unwrap();
            assert!(!held.current(13));
            assert!(model.retained(13).unwrap().is_none(), "restoring permission cannot revive pixels");
        }
        let mut model = model(&budget);
        let held = admit(&mut model, &budget, 1, 20);
        assert!(model.sync(policy(2), 22).is_err());
        assert!(!held.current(22));
        assert!(
            model.sync(policy(1), 23).is_err(),
            "source loss permanently retires this authority instance"
        );
        let original = source(1);
        let mut changed_session = original.session();
        changed_session.account = AccountKey::from_bytes([9; 32]).unwrap();
        let account = PreviewSource::new(original.worker(), changed_session, 1).unwrap();
        changed_session = original.session();
        changed_session.grid = GridKey::from_bytes([9; 32]).unwrap();
        let grid = PreviewSource::new(original.worker(), changed_session, 1).unwrap();
        changed_session = original.session();
        changed_session.generation = 2;
        let generation = PreviewSource::new(original.worker(), changed_session, 1).unwrap();
        let lease = PreviewSource::new(original.worker(), original.session(), 2).unwrap();
        for replacement in [account, grid, generation, lease] {
            let mut replaced = super::tests::model(&budget);
            let held = admit(&mut replaced, &budget, 1, 30);
            let mut next = policy(1);
            next.source = replacement;
            assert!(replaced.sync(next, 32).is_err());
            assert!(!held.current(32));
        }
    }

    #[test]
    fn preview_image_expiry_future_clock_replay_and_exhaustion_fail_closed() {
        let instances = AtomicU64::new(u64::MAX - 1);
        assert_eq!(source_instance(&instances), Ok(u64::MAX - 1));
        assert_eq!(source_instance(&instances), Err(ImageError::Exhausted));
        assert_eq!(source_instance(&instances), Err(ImageError::Exhausted));
        let budget = ImageBudget::new(HOST_IMAGE_BYTES).unwrap();
        let mut model = model(&budget);
        let held = admit(&mut model, &budget, 5, 10);
        let claim = model.request(1, 1, geometry(), 20).unwrap();
        let replay = PreviewStamp::new(claim, 5, 1, 1, geometry()).unwrap();
        assert!(model.admit(replay, budget.reserve(geometry()).unwrap(), 21).is_err());
        let wrong_credit = budget
            .reserve(PreviewGeometry::new([640, 360], [0, 0, 640, 360], PreviewSize::Large).unwrap())
            .unwrap();
        let fresh = PreviewStamp::new(claim, 6, 1, 1, geometry()).unwrap();
        assert!(model.admit(fresh, wrong_credit, 22).is_err());
        assert!(!held.current(1000));
        assert!(model.retained(1000).unwrap().is_none());
        assert!(model.admit(fresh, budget.reserve(geometry()).unwrap(), 1001).is_err());
        assert!(model.request(1, 1, geometry(), 1002).is_err());
        model.sync(policy(1), 1003).unwrap();
        model.next_request = u64::MAX;
        assert!(matches!(model.request(1, 1, geometry(), 1004), Err(ImageError::Exhausted)));
        assert!(model.sync(policy(1), 1005).is_err());
        let mut backwards = super::tests::model(&budget);
        let held = admit(&mut backwards, &budget, 1, 10);
        assert!(matches!(backwards.retained(9), Err(ImageError::Clock)));
        assert!(!held.current(11));
        assert!(CaptureAge::new(10, 9).is_err());
        assert!(CaptureAge::new(0, 1001).is_err());
        let age = CaptureAge::new(10, 20).unwrap();
        assert_eq!(age.bounds(30).unwrap(), (10, 20));
        assert!(age.bounds(19).is_err());
        assert!(age.stale(5011).unwrap());
        let mut epoch_limit = super::tests::model(&budget);
        epoch_limit.validity.epoch.store(u64::MAX, Ordering::Release);
        assert!(matches!(epoch_limit.retained(1000), Err(ImageError::Exhausted)));
        assert!(epoch_limit.sync(policy(1), 1001).is_err());
    }

    #[test]
    fn preview_image_recreated_model_never_reuses_request_and_activation_rollback_is_rejected() {
        let budget = ImageBudget::new(HOST_IMAGE_BYTES).unwrap();
        let mut first = model(&budget);
        let old = first.request(1, 1, geometry(), 10).unwrap();
        drop(first);
        let mut replacement = model(&budget);
        let current = replacement.request(1, 1, geometry(), 10).unwrap();
        assert_ne!(
            old, current,
            "fresh model instance separates even otherwise identical request counters"
        );
        let old_stamp = PreviewStamp::new(old, 1, 1, 0, geometry()).unwrap();
        assert!(replacement.admit(old_stamp, budget.reserve(geometry()).unwrap(), 11).is_err());
        let current_stamp = PreviewStamp::new(current, 1, 1, 0, geometry()).unwrap();
        let image = replacement.admit(current_stamp, budget.reserve(geometry()).unwrap(), 12).unwrap();
        let mut rollback = policy(1);
        rollback.access.activation = 6;
        assert!(replacement.sync(rollback, 13).is_err());
        assert!(!image.current(13));
        assert!(replacement.retained(13).unwrap().is_none());
    }

    #[test]
    fn preview_image_memory_credit_stays_charged_after_model_or_cache_is_removed() {
        let bytes = geometry().payload_bytes();
        let budget = ImageBudget::new(bytes).unwrap();
        let mut model = model(&budget);
        let held = admit(&mut model, &budget, 1, 10);
        let clone = held.clone();
        drop(model);
        assert!(!held.current(12));
        assert_eq!(budget.used(), bytes);
        assert!(budget.reserve(geometry()).is_err());
        let mut replacement = super::tests::model(&budget);
        assert!(
            matches!(replacement.request(2, 1, geometry(), 12), Err(ImageError::Capacity)),
            "replacement source uses the same pool while old image clones remain"
        );
        drop(held);
        assert_eq!(budget.used(), bytes);
        drop(clone);
        assert_eq!(budget.used(), 0);
        assert!(replacement.request(2, 1, geometry(), 13).is_ok());
        assert!(budget.reserve(geometry()).is_ok());
        assert!(ImageBudget::new(HOST_IMAGE_BYTES + 1).is_err());
    }

    #[test]
    fn preview_image_unique_visible_sources_use_defaults_and_five_inactive_budget() {
        assert_eq!(PreviewRate::default(), PreviewRate::One);
        assert_eq!(PreviewSize::default(), PreviewSize::Small);
        let demand = PreviewDemand {
            requested: Some(PreviewRate::Five),
            visible: true,
            inactive: true,
            session_current: true,
            acknowledged_half_fps: 0,
            pending_half_fps: 0,
        };
        let views: Vec<_> = (1..=5)
            .map(|id| PreviewViewDemand {
                source: source(id),
                demand,
                size: PreviewSize::Small,
            })
            .collect();
        let unique = unique_preview_demands(&views).unwrap();
        let demands: Vec<_> = unique.iter().map(|source| source.demand).collect();
        let rates = admit_previews_with_active(&demands, 20, false).unwrap();
        assert_eq!(rates, vec![Some(PreviewRate::Two); 5]);
        assert_eq!(rates.iter().flatten().map(|rate| rate.half_fps()).sum::<u32>(), 20);
        assert!(
            admit_previews(&demands, 20).is_err(),
            "preserve Some-Active facade's four-source cap"
        );
        let duplicates = [
            PreviewViewDemand {
                source: source(1),
                demand,
                size: PreviewSize::Small,
            },
            PreviewViewDemand {
                source: source(1),
                demand: PreviewDemand {
                    requested: Some(PreviewRate::Two),
                    ..demand
                },
                size: PreviewSize::Large,
            },
            PreviewViewDemand {
                source: source(2),
                demand,
                size: PreviewSize::Small,
            },
        ];
        let unique = unique_preview_demands(&duplicates).unwrap();
        assert_eq!(unique.len(), 2);
        assert_eq!(unique[0].demand.requested, Some(PreviewRate::Five));
        assert_eq!(unique[0].size, PreviewSize::Large);
        let hidden = PreviewViewDemand {
            source: source(1),
            demand: PreviewDemand {
                visible: false,
                acknowledged_half_fps: 10,
                ..demand
            },
            size: PreviewSize::Large,
        };
        let unique = unique_preview_demands(&[duplicates[0], hidden]).unwrap();
        assert_eq!(unique[0].demand.acknowledged_half_fps, 10);
        assert_eq!(unique[0].size, PreviewSize::Small);
        let occupied = unique[0].demand;
        let rates = admit_previews_with_active(&[occupied, demand, demand], 20, false).unwrap();
        assert!(rates.iter().flatten().map(|rate| rate.half_fps()).sum::<u32>() <= 20);
        let conflicting = PreviewViewDemand {
            demand: PreviewDemand { inactive: false, ..demand },
            ..duplicates[0]
        };
        assert!(unique_preview_demands(&[duplicates[0], conflicting]).is_err());
    }
}
