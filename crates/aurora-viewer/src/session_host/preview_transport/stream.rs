use super::auth::{Authenticated, Keys};
use super::captured::CapturedBody;
use super::{Error, Side};
use aurora_session::preview_image::{AdmittedImage, ImageBudget, ImageCredit, ImageError, PreviewPolicy, PreviewStamp, SourceImages};
use aurora_session::preview_wire::{self as wire, HEADER_BYTES, Header, Lane, MAX_METADATA_BYTES, Metadata, MetadataBytes, TAG_BYTES};
use ring::hmac;
use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

const SOURCE_BYTES: usize = 64 * 1024;
const TOTAL_BYTES: usize = 256 * 1024;
const ROLE_OPERATIONS: usize = 16;
const PARTIAL_MILLIS: u64 = 1000;
const METADATA_QUEUE: usize = 32;

#[cfg(test)]
pub(super) fn worker_framing_bound(pair: &Pair) -> (usize, [usize; 2], [usize; 2]) {
    let queued = [pair.metadata.outgoing.len(), pair.pixels.as_ref().map_or(0, |p| p.outgoing.len())];
    let capacities = [
        pair.metadata.outgoing.capacity(),
        pair.pixels.as_ref().map_or(0, |p| p.outgoing.capacity()),
    ];
    // Actual allocated deque capacities (not just logical lengths), current
    // receive storage and conservative transient output/auth/command reserves.
    // Each role consumes at most ROLE_OPERATIONS reads, hence output <=2*that;
    // the Vec's power-of-two growth fits this already power-of-two allowance.
    let bytes = std::mem::size_of::<Pair>()
        + capacities.iter().sum::<usize>() * std::mem::size_of::<Outgoing>()
        + 2 * ROLE_OPERATIONS * std::mem::size_of::<Received>()
        + 6 * 96
        + 4 * std::mem::size_of::<Metadata>()
        + 4 * 4096;
    (bytes, queued, capacities)
}

#[cfg(test)]
pub(crate) fn worker_storage_ceiling() -> usize {
    std::mem::size_of::<Pair>()
        + (METADATA_QUEUE + 4) * std::mem::size_of::<Outgoing>()
        + 2 * ROLE_OPERATIONS * std::mem::size_of::<Received>()
        + 6 * 96
        + 4 * 4096
        + 4 * std::mem::size_of::<Metadata>()
}

/// One shared tick budget across all <=5 source pairs. Start at a rotating
/// source each tick; metadata always precedes that source's pixel work.
pub(crate) struct TickBudget {
    bytes: usize,
    operations: usize,
}
impl TickBudget {
    pub fn new() -> Self {
        Self {
            bytes: TOTAL_BYTES,
            operations: 5 * 2 * ROLE_OPERATIONS,
        }
    }
    pub fn remaining_bytes(&self) -> usize {
        self.bytes
    }
    pub fn external(&mut self, bytes: usize, operations: usize) -> (usize, usize) {
        let granted = (bytes.min(self.bytes), operations.min(self.operations));
        self.bytes -= granted.0;
        self.operations -= granted.1;
        granted
    }
}
struct Allowance<'a> {
    tick: &'a mut TickBudget,
    bytes: usize,
}
impl Allowance<'_> {
    fn available(&self) -> usize {
        self.bytes.min(self.tick.bytes)
    }
    fn operation(&mut self, operations: &mut usize) -> bool {
        if *operations == 0 || self.tick.operations == 0 || self.available() == 0 {
            return false;
        }
        *operations -= 1;
        self.tick.operations -= 1;
        true
    }
    fn used(&mut self, bytes: usize) {
        self.bytes -= bytes;
        self.tick.bytes -= bytes;
    }
}

/// Authenticated transport bytes, explicitly NOT an admitted/drawable image.
/// Scope/MAC/format checks cannot prove a current capture claim, native grant,
/// control lease or policy. The owner must sync SourceImages from fresh control
/// and authenticated policy, then consume this through admission before any UI
/// publication, texture copy or image cache replacement.
///
/// Actual allocation credit travels with the only owned Vec. Field order frees
/// bytes before credit on drop (recovery keeps both); no split Vec/credit accessor or Clone
/// can silently keep a physical image after refunding its reservation.
pub(crate) struct UnadmittedPixels {
    stamp: PreviewStamp,
    bytes: Vec<u8>,
    credit: ImageCredit,
    validity: Option<(Arc<AtomicU64>, u64)>,
}
impl UnadmittedPixels {
    fn blank(stamp: PreviewStamp, budget: &ImageBudget) -> Result<Self, Error> {
        let credit = budget.reserve(stamp.geometry()).map_err(|_| Error::Capacity)?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(credit.bytes()).map_err(|_| Error::Capacity)?;
        bytes.resize(credit.bytes(), 0);
        Ok(Self {
            stamp,
            bytes,
            credit,
            validity: None,
        })
    }
    #[cfg(test)]
    pub fn copy_from(stamp: PreviewStamp, input: &[u8], budget: &ImageBudget) -> Result<Self, Error> {
        if input.len() != stamp.geometry().payload_bytes() || input.as_chunks::<4>().0.iter().any(|pixel| pixel[3] != 255) {
            return Err(Error::Invalid);
        }
        let mut pixels = Self::blank(stamp, budget)?;
        pixels.bytes.copy_from_slice(input);
        Ok(pixels)
    }
    pub fn claim(&self) -> aurora_session::preview_image::CaptureClaim {
        self.stamp.claim()
    }
    pub fn transport_current(&self) -> bool {
        self.validity
            .as_ref()
            .is_none_or(|(current, epoch)| *epoch != 0 && current.load(Ordering::Acquire) == *epoch)
    }
    #[cfg(test)]
    pub(super) fn test_bytes(&self) -> Option<&[u8]> {
        self.transport_current().then_some(self.bytes.as_slice())
    }
    /// Owns both bytes and their original credit through final model admission.
    /// A failed `SourceImages::admit` drops its trial clone while this object's
    /// reservation still exists; rejection then frees bytes before refunding.
    #[cfg(test)]
    pub fn admit(self, owner: &mut SourceImages, now: u64) -> Result<AdmittedPixels, ImageError> {
        self.try_admit(owner, now).map_err(|(error, rejected)| {
            drop(rejected);
            error
        })
    }
    /// Recover rejected ownership for retirement outside the owner's lock.
    /// Recovery grants no retry authority: model clock/pending changes and the
    /// transport validity check remain exactly those of ordinary admission.
    #[expect(
        clippy::result_large_err,
        reason = "Return the bounded indivisible stamped Vec/credit without boxing or copying under the owner lock."
    )]
    pub fn try_admit(self, owner: &mut SourceImages, now: u64) -> Result<AdmittedPixels, (ImageError, Self)> {
        if !self.transport_current() {
            return Err((ImageError::Stale, self));
        }
        match owner.admit(self.stamp, self.credit.clone(), now) {
            Ok(image) => Ok(AdmittedPixels { bytes: self.bytes, image }),
            Err(error) => Err((error, self)),
        }
    }
}
impl std::fmt::Debug for UnadmittedPixels {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("UnadmittedPreviewPixels([redacted])")
    }
}

/// The final admission handle and its exact allocation stay inseparable. The
/// caller must still recheck `bytes(now)` immediately before any physical copy
/// or present, reserve separate credit for copies, and erase revoked textures.
pub(crate) struct AdmittedPixels {
    // Free the physical allocation before dropping its model-held credit.
    bytes: Vec<u8>,
    image: AdmittedImage,
}
impl AdmittedPixels {
    #[cfg(test)]
    pub fn image(&self) -> &AdmittedImage {
        &self.image
    }
    pub fn bytes(&self, now: u64) -> Option<&[u8]> {
        self.image.current(now).then_some(self.bytes.as_slice())
    }
}
impl std::fmt::Debug for AdmittedPixels {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AdmittedPreviewPixels([redacted])")
    }
}
pub(crate) enum Received {
    Metadata { metadata: Metadata, received_at: Instant },
    UnadmittedPixels(UnadmittedPixels),
}
impl std::fmt::Debug for Received {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PreviewReceived([redacted])")
    }
}

struct Outgoing {
    header: Header,
    metadata: MetadataBytes,
    header_tag: [u8; TAG_BYTES],
    pixels: Option<OutgoingPixels>,
    body: Option<hmac::Context>,
    finish: Option<[u8; TAG_BYTES]>,
    position: usize,
    began: Option<u64>,
    local_began: Option<Instant>,
    live: bool,
}
enum OutgoingPixels {
    #[cfg(test)]
    Owned(UnadmittedPixels),
    Captured(CapturedBody),
}
impl OutgoingPixels {
    fn bytes(&self) -> &[u8] {
        match self {
            #[cfg(test)]
            Self::Owned(pixels) => &pixels.bytes,
            Self::Captured(frame) => frame.bytes(),
        }
    }
    fn check(&self) -> Result<(), Error> {
        match self {
            #[cfg(test)]
            Self::Owned(pixels) => {
                if pixels.transport_current() {
                    Ok(())
                } else {
                    Err(Error::Expired)
                }
            }
            Self::Captured(frame) => frame.check(),
        }
    }
}
impl Outgoing {
    fn new(header: Header, metadata: MetadataBytes, pixels: Option<OutgoingPixels>, keys: &Keys) -> Self {
        let header_tag = keys.header(header, metadata.as_slice());
        let body = Some(keys.body(header, metadata.as_slice(), &header_tag));
        Self {
            header,
            metadata,
            header_tag,
            pixels,
            body,
            finish: None,
            position: 0,
            began: None,
            local_began: None,
            live: false,
        }
    }
    fn write(&mut self, io: &mut impl Write, keys: &Keys, now: u64, allowance: &mut Allowance<'_>) -> Result<bool, Error> {
        if let Some(pixels) = &self.pixels {
            pixels.check()?;
        }
        if self
            .began
            .is_some_and(|began| now.checked_sub(began).is_none_or(|age| age >= PARTIAL_MILLIS))
        {
            return Err(Error::Expired);
        }
        if self
            .local_began
            .is_some_and(|began| began.elapsed() >= Duration::from_millis(PARTIAL_MILLIS))
        {
            return Err(Error::Expired);
        }
        let call_began = Instant::now();
        let header = self.header.encode();
        let metadata = self.metadata.as_slice();
        let pixel_start = HEADER_BYTES + metadata.len() + TAG_BYTES;
        let pixel_end = pixel_start + self.header.pixel_bytes();
        if self.position == pixel_end && self.finish.is_none() {
            self.finish = Some(keys.finish(self.body.take().ok_or(Error::Invalid)?));
        }
        let (part, offset) = if self.position < HEADER_BYTES {
            (&header[..], 0)
        } else if self.position < HEADER_BYTES + metadata.len() {
            (metadata, HEADER_BYTES)
        } else if self.position < pixel_start {
            (&self.header_tag[..], HEADER_BYTES + metadata.len())
        } else if self.position < pixel_end {
            (self.pixels.as_ref().ok_or(Error::Invalid)?.bytes(), pixel_start)
        } else {
            (&self.finish.as_ref().ok_or(Error::Invalid)?[..], pixel_end)
        };
        let start = self.position - offset;
        let end = (start + allowance.available()).min(part.len());
        match io.write(&part[start..end]) {
            Ok(0) => Err(Error::Closed),
            Ok(count) => {
                self.began.get_or_insert(now);
                if self.live {
                    self.local_began.get_or_insert(call_began);
                }
                if self
                    .local_began
                    .is_some_and(|began| began.elapsed() >= Duration::from_millis(PARTIAL_MILLIS))
                {
                    return Err(Error::Expired);
                }
                if self.position >= pixel_start && self.position < pixel_end {
                    self.body.as_mut().ok_or(Error::Invalid)?.update(&part[start..start + count]);
                }
                self.position += count;
                allowance.used(count);
                Ok(self.position == self.header.frame_bytes())
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Err(Error::Busy),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => Ok(false),
            Err(error) => Err(Error::Io(error.kind())),
        }
    }
}

struct Incoming {
    header_bytes: [u8; HEADER_BYTES],
    header: Option<Header>,
    metadata: [u8; MAX_METADATA_BYTES],
    header_tag: [u8; TAG_BYTES],
    finish: [u8; TAG_BYTES],
    position: usize,
    began: Option<u64>,
    local_began: Option<Instant>,
    live: bool,
    decoded: Option<Metadata>,
    pixels: Option<UnadmittedPixels>,
    body: Option<hmac::Context>,
}
impl Incoming {
    fn new() -> Self {
        Self {
            header_bytes: [0; HEADER_BYTES],
            header: None,
            metadata: [0; MAX_METADATA_BYTES],
            header_tag: [0; TAG_BYTES],
            finish: [0; TAG_BYTES],
            position: 0,
            began: None,
            local_began: None,
            live: false,
            decoded: None,
            pixels: None,
            body: None,
        }
    }
    fn read(
        &mut self,
        io: &mut impl Read,
        auth: &Authenticated,
        budget: Option<&ImageBudget>,
        received: u64,
        now: u64,
        allowance: &mut Allowance<'_>,
    ) -> Result<Option<Received>, Error> {
        if self
            .began
            .is_some_and(|began| now.checked_sub(began).is_none_or(|age| age >= PARTIAL_MILLIS))
        {
            return Err(Error::Expired);
        }
        if self
            .local_began
            .is_some_and(|began| began.elapsed() >= Duration::from_millis(PARTIAL_MILLIS))
        {
            return Err(Error::Expired);
        }
        let call_began = Instant::now();
        let metadata_len = self.header.map_or(0, Header::metadata_bytes);
        let pixel_start = HEADER_BYTES + metadata_len + TAG_BYTES;
        let pixel_end = pixel_start + self.header.map_or(0, Header::pixel_bytes);
        let (part, offset) = if self.position < HEADER_BYTES {
            (&mut self.header_bytes[..], 0)
        } else if self.position < HEADER_BYTES + metadata_len {
            (&mut self.metadata[..metadata_len], HEADER_BYTES)
        } else if self.position < pixel_start {
            (&mut self.header_tag[..], HEADER_BYTES + metadata_len)
        } else if self.position < pixel_end {
            (&mut self.pixels.as_mut().ok_or(Error::Invalid)?.bytes[..], pixel_start)
        } else {
            (&mut self.finish[..], pixel_end)
        };
        let start = self.position - offset;
        let end = (start + allowance.available()).min(part.len());
        match io.read(&mut part[start..end]) {
            Ok(0) => return Err(Error::Closed),
            Ok(count) => {
                self.began.get_or_insert(now);
                if self.live {
                    self.local_began.get_or_insert(call_began);
                }
                if self
                    .local_began
                    .is_some_and(|began| began.elapsed() >= Duration::from_millis(PARTIAL_MILLIS))
                {
                    return Err(Error::Expired);
                }
                if self.position >= pixel_start && self.position < pixel_end {
                    let chunk = &part[start..start + count];
                    if chunk
                        .iter()
                        .enumerate()
                        .any(|(index, byte)| (start + index) % 4 == 3 && *byte != 255)
                    {
                        return Err(Error::Invalid);
                    }
                    self.body.as_mut().ok_or(Error::Invalid)?.update(chunk);
                }
                self.position += count;
                allowance.used(count);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Err(Error::Busy),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => return Ok(None),
            Err(error) => return Err(Error::Io(error.kind())),
        }
        if self.position == HEADER_BYTES {
            let header = Header::decode(&self.header_bytes)?;
            if header.lane() != auth.scope.lane
                || header.connection() != auth.scope.connection
                || header.role_epoch() != auth.scope.role_epoch
            {
                return Err(Error::Binding);
            }
            if header.direction() != auth.side.incoming() {
                return Err(Error::Direction);
            }
            if received.checked_add(1) != Some(header.sequence()) {
                return Err(Error::Sequence);
            }
            self.header = Some(header);
        }
        if self.header.is_some() && self.position == pixel_start {
            let header = self.header.ok_or(Error::Invalid)?;
            let metadata = &self.metadata[..header.metadata_bytes()];
            auth.receive.verify_header(header, metadata, &self.header_tag)?;
            // Authentication precedes both decoding attacker-controlled image
            // geometry and the atomic shared reservation/actual allocation.
            match header.lane() {
                Lane::Metadata => {
                    let metadata = wire::decode_metadata(header, metadata)?;
                    if metadata.source() != auth.scope.source {
                        return Err(Error::Binding);
                    }
                    self.decoded = Some(metadata);
                }
                Lane::Pixels => {
                    let stamp = wire::decode_stamp(header, metadata)?;
                    if stamp.claim().source() != auth.scope.source {
                        return Err(Error::Binding);
                    }
                    self.pixels = Some(UnadmittedPixels::blank(stamp, budget.ok_or(Error::Direction)?)?);
                }
            }
            self.body = Some(auth.receive.body(header, metadata, &self.header_tag));
        }
        if let Some(header) = self.header
            && self.position == header.frame_bytes()
        {
            auth.receive.verify_finish(self.body.take().ok_or(Error::Invalid)?, &self.finish)?;
            return match header.lane() {
                Lane::Metadata => Ok(Some(Received::Metadata {
                    metadata: self.decoded.take().ok_or(Error::Invalid)?,
                    received_at: Instant::now(),
                })),
                Lane::Pixels => Ok(Some(Received::UnadmittedPixels(self.pixels.take().ok_or(Error::Invalid)?))),
            };
        }
        Ok(None)
    }
}

struct Endpoint {
    auth: Authenticated,
    incoming: Incoming,
    outgoing: VecDeque<Outgoing>,
    sent: u64,
    received: u64,
    flushed: u64,
    closed: bool,
}
impl Endpoint {
    fn new(auth: Authenticated) -> Self {
        Self {
            auth,
            incoming: Incoming::new(),
            outgoing: VecDeque::new(),
            sent: 0,
            received: 0,
            flushed: 0,
            closed: false,
        }
    }
    fn close(&mut self) {
        self.closed = true;
        self.incoming = Incoming::new();
        self.outgoing.clear();
    }
    fn enqueue(&mut self, metadata: MetadataBytes, pixels: Option<OutgoingPixels>) -> Result<(), Error> {
        if self.closed {
            return Err(Error::Closed);
        }
        let capacity = if self.auth.scope.lane == Lane::Metadata {
            METADATA_QUEUE
        } else {
            1
        };
        if self.outgoing.len() >= capacity {
            return Err(Error::Busy);
        }
        let sequence = self.sent.checked_add(1).ok_or(Error::Sequence)?;
        let header = Header::new(
            self.auth.scope.lane,
            self.auth.side.outgoing(),
            self.auth.scope.connection,
            self.auth.scope.role_epoch,
            sequence,
            metadata.as_slice().len(),
            pixels.as_ref().map_or(0, |pixels| pixels.bytes().len()),
        )?;
        self.outgoing.push_back(Outgoing::new(header, metadata, pixels, &self.auth.send));
        self.sent = sequence;
        Ok(())
    }
    fn poll(
        &mut self,
        io: &mut (impl Read + Write),
        budget: Option<&ImageBudget>,
        now: u64,
        allowance: &mut Allowance<'_>,
        output: &mut Vec<Received>,
        source_guard: Option<&super::super::preview_io::guards::IoSourceGuard>,
    ) -> Result<(), Error> {
        if self.closed {
            return Err(Error::Closed);
        }
        if self
            .incoming
            .began
            .is_some_and(|began| now.checked_sub(began).is_none_or(|age| age >= PARTIAL_MILLIS))
            || self
                .outgoing
                .front()
                .and_then(|frame| frame.began)
                .is_some_and(|began| now.checked_sub(began).is_none_or(|age| age >= PARTIAL_MILLIS))
        {
            self.close();
            return Err(Error::Expired);
        }
        let mut operations = ROLE_OPERATIONS;
        let mut read = true;
        let mut write = true;
        while (read || write) && allowance.operation(&mut operations) {
            if source_guard.is_some_and(|guard| !guard.current()) {
                self.close();
                return Err(Error::Expired);
            }
            if read {
                self.incoming.live = source_guard.is_some();
                match self.incoming.read(io, &self.auth, budget, self.received, now, allowance) {
                    Ok(Some(packet)) => {
                        self.received = self.incoming.header.ok_or(Error::Invalid)?.sequence();
                        self.incoming = Incoming::new();
                        output.push(packet);
                    }
                    Ok(None) => {}
                    Err(Error::Busy) => read = false,
                    Err(error) => {
                        self.close();
                        return Err(error);
                    }
                }
            }
            if write && allowance.operation(&mut operations) {
                if let Some(frame) = self.outgoing.front_mut() {
                    frame.live = source_guard.is_some();
                    match frame.write(io, &self.auth.send, now, allowance) {
                        Ok(true) => {
                            self.flushed = frame.header.sequence();
                            self.outgoing.pop_front();
                        }
                        Ok(false) => {}
                        Err(Error::Busy) => write = false,
                        Err(error) => {
                            self.close();
                            return Err(error);
                        }
                    }
                } else {
                    write = false;
                }
            }
        }
        Ok(())
    }
}

/// Independent authenticated roles, single in-progress image and shared byte
/// credit. Metadata errors or export/incarnation loss retire the pixel role;
/// later Allowed messages cannot revive its bytes or keys. A fresh pair is
/// required. Final display/native authority still belongs to SourceImages.
pub(crate) struct Pair {
    metadata: Endpoint,
    pixels: Option<Endpoint>,
    pixel_epoch: u64,
    budget: Option<ImageBudget>,
    validity: Arc<AtomicU64>,
    policy: Option<PreviewPolicy>,
    now: u64,
    began: Option<u64>,
    policy_seen: Option<u64>,
    source_guard: Option<super::super::preview_io::guards::IoSourceGuard>,
}
impl Pair {
    #[cfg(test)]
    pub fn new(metadata: Authenticated, pixels: Authenticated, budget: ImageBudget) -> Result<Self, Error> {
        Self::from_roles(metadata, pixels, Some(budget))
    }
    #[cfg(test)]
    fn from_roles(metadata: Authenticated, pixels: Authenticated, budget: Option<ImageBudget>) -> Result<Self, Error> {
        if pixels.scope.lane != Lane::Pixels
            || !pixels.scope.valid()
            || metadata.side != pixels.side
            || metadata.scope.source != pixels.scope.source
            || metadata.scope.connection != pixels.scope.connection
        {
            return Err(Error::Binding);
        }
        let mut pair = Self::metadata(metadata, budget)?;
        pair.pixel_epoch = pixels.scope.role_epoch;
        pair.pixels = Some(Endpoint::new(pixels));
        Ok(pair)
    }
    /// Metadata can authenticate while parked, before any pixel connection.
    pub fn metadata(metadata: Authenticated, budget: Option<ImageBudget>) -> Result<Self, Error> {
        if metadata.scope.lane != Lane::Metadata || !metadata.scope.valid() || (metadata.side == Side::Host) != budget.is_some() {
            return Err(Error::Binding);
        }
        Ok(Self {
            metadata: Endpoint::new(metadata),
            pixels: None,
            pixel_epoch: 0,
            budget,
            validity: Arc::new(AtomicU64::new(1)),
            policy: None,
            now: 0,
            began: None,
            policy_seen: None,
            source_guard: None,
        })
    }
    pub fn set_source_guard(&mut self, guard: super::super::preview_io::guards::IoSourceGuard) -> Result<(), Error> {
        if guard.control.source() != self.metadata.auth.scope.source || !guard.current() {
            self.close();
            return Err(Error::Binding);
        }
        self.source_guard = Some(guard);
        Ok(())
    }
    pub fn metadata_queued(&self) -> u64 {
        self.metadata.sent
    }
    pub fn metadata_flushed(&self) -> u64 {
        self.metadata.flushed
    }
    pub fn pixel_pending(&self) -> bool {
        self.pixels.as_ref().is_some_and(|p| !p.outgoing.is_empty())
    }
    pub fn has_pixels(&self) -> bool {
        self.pixels.is_some()
    }
    pub fn suspend_pixels(&mut self) {
        self.validity.store(0, Ordering::Release);
        if let Some(mut pixel) = self.pixels.take() {
            pixel.close();
        }
    }
    fn retire_pixels(&mut self) {
        self.suspend_pixels();
    }
    pub fn close(&mut self) {
        self.retire_pixels();
        self.metadata.close();
    }
    fn metadata_effect(&mut self, metadata: Metadata) -> Result<(), Error> {
        match metadata {
            Metadata::Invalidate { .. } => self.retire_pixels(),
            Metadata::CaptureStopped { policy, pixel_epoch, .. } => {
                if pixel_epoch != self.pixel_epoch {
                    return Ok(());
                }
                return self.metadata_effect(Metadata::Policy {
                    connection: metadata.connection(),
                    policy,
                });
            }
            Metadata::Policy { policy, .. } => {
                if let Some(old) = self.policy {
                    let a = old.epochs();
                    let b = policy.epochs();
                    if b.revision() < a.revision()
                        || b.privacy() < a.privacy()
                        || b.render() < a.render()
                        || b.viewport() < a.viewport()
                        || (b.render() == a.render() && b.surface() < a.surface())
                        || policy.access().activation < old.access().activation
                        || (a == b && old.access().retain_last_active != policy.access().retain_last_active)
                    {
                        self.close();
                        return Err(Error::Binding);
                    }
                    if a != b || old.access().activation != policy.access().activation {
                        self.retire_pixels();
                    }
                }
                if !policy.access().capture_active {
                    self.retire_pixels();
                }
                self.policy = Some(policy);
            }
            _ => {}
        }
        Ok(())
    }
    pub fn queue_metadata(&mut self, metadata: Metadata) -> Result<(), Error> {
        if metadata.source() != self.metadata.auth.scope.source || metadata.connection() != self.metadata.auth.scope.connection {
            self.close();
            return Err(Error::Binding);
        }
        if metadata.direction() != self.metadata.auth.side.outgoing() {
            self.close();
            return Err(Error::Direction);
        }
        self.metadata_effect(metadata)?;
        let result = wire::encode_metadata(metadata)
            .map_err(Error::from)
            .and_then(|m| self.metadata.enqueue(m, None));
        if result.is_err() {
            self.close();
        }
        result
    }
    #[cfg(test)]
    pub fn queue_pixels(&mut self, pixels: UnadmittedPixels) -> Result<(), Error> {
        if pixels.stamp.claim().source() != self.metadata.auth.scope.source
            || pixels.stamp.claim().connection() != self.metadata.auth.scope.connection
            || !pixels.transport_current()
        {
            self.close();
            return Err(Error::Binding);
        }
        if self.metadata.auth.side != Side::Worker {
            self.close();
            return Err(Error::Direction);
        }
        self.pixels
            .as_mut()
            .ok_or(Error::Closed)?
            .enqueue(wire::encode_stamp(pixels.stamp)?, Some(OutgoingPixels::Owned(pixels)))
    }
    #[cfg(test)]
    pub fn queue_captured(
        &mut self,
        stamp: PreviewStamp,
        frame: aurora_render::viewport_capture::CapturedViewport,
        guard: aurora_render::viewport_capture::CapturePublicationGuard,
    ) -> Result<(), Error> {
        self.queue_captured_body(stamp, CapturedBody::new(stamp, frame, guard)?)
    }
    pub fn queue_guarded_capture(
        &mut self,
        stamp: PreviewStamp,
        frame: aurora_render::viewport_capture::CapturedViewport,
        guard: Box<super::super::preview_io::CapturePermitGuard>,
    ) -> Result<(), Error> {
        self.queue_captured_body(stamp, CapturedBody::guarded(stamp, frame, guard)?)
    }
    fn queue_captured_body(&mut self, stamp: PreviewStamp, frame: CapturedBody) -> Result<(), Error> {
        if self.metadata.auth.side != Side::Worker {
            return Err(Error::Direction);
        }
        if stamp.claim().source() != self.metadata.auth.scope.source || stamp.claim().connection() != self.metadata.auth.scope.connection {
            return Err(Error::Binding);
        }
        self.pixels
            .as_mut()
            .ok_or(Error::Closed)?
            .enqueue(wire::encode_stamp(stamp)?, Some(OutgoingPixels::Captured(frame)))
    }
    #[cfg(test)]
    pub(crate) fn queue_captured_fixture(
        &mut self,
        stamp: PreviewStamp,
        frame: aurora_render::viewport_capture::CapturedViewport,
        current: Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<(), Error> {
        self.queue_captured_body(stamp, CapturedBody::fixture(stamp, frame, current)?)
    }
    #[cfg(test)]
    pub(crate) fn queue_captured_fixture_gate(
        &mut self,
        stamp: PreviewStamp,
        frame: aurora_render::viewport_capture::CapturedViewport,
        current: Arc<std::sync::atomic::AtomicBool>,
        gate: super::super::preview_io::EpochFence,
    ) -> Result<(), Error> {
        self.queue_captured_body(stamp, CapturedBody::fixture_gate(stamp, frame, current, gate)?)
    }
    #[cfg(test)]
    pub fn worker(metadata: Authenticated, pixels: Authenticated) -> Result<Self, Error> {
        if metadata.side != Side::Worker {
            return Err(Error::Direction);
        }
        Self::from_roles(metadata, pixels, None)
    }
    pub fn attach_pixels(&mut self, auth: Authenticated) -> Result<(), Error> {
        self.attach_role(auth, false)
    }
    /// Complete an already-started authenticated role solely to retire it via
    /// exact stop/ACK. This creates no capture or frame-admission permission.
    pub fn attach_retiring(&mut self, auth: Authenticated) -> Result<(), Error> {
        self.attach_role(auth, true)
    }
    fn attach_role(&mut self, auth: Authenticated, retiring: bool) -> Result<(), Error> {
        if self.metadata.closed
            || !auth.scope.valid()
            || auth.scope.lane != Lane::Pixels
            || auth.side != self.metadata.auth.side
            || auth.scope.source != self.metadata.auth.scope.source
            || auth.scope.connection != self.metadata.auth.scope.connection
        {
            return Err(Error::Binding);
        }
        if self.pixels.is_some() {
            return Err(Error::Busy);
        }
        if auth.scope.role_epoch <= self.pixel_epoch
            || self
                .policy
                .is_none_or(|p| !p.access().retain_last_active || (!retiring && !p.access().capture_active))
        {
            return Err(Error::Sequence);
        }
        self.pixel_epoch = auth.scope.role_epoch;
        self.validity = Arc::new(AtomicU64::new(1));
        self.pixels = Some(Endpoint::new(auth));
        Ok(())
    }
    #[cfg(test)]
    pub fn poll(
        &mut self,
        metadata: &mut (impl Read + Write),
        pixels: &mut (impl Read + Write),
        now: u64,
        tick: &mut TickBudget,
    ) -> Result<Vec<Received>, Error> {
        self.poll_roles(metadata, Some(pixels), now, tick)
    }
    /// Service host first polls every source's metadata, then rotates pixels
    /// with the remainder of each source's64KiB allowance.
    pub fn poll_pixels_only<T: Read + Write>(
        &mut self,
        stream: &mut T,
        now: u64,
        tick: &mut TickBudget,
        max_bytes: usize,
    ) -> Result<Vec<Received>, Error> {
        if now < self.now || self.metadata.closed {
            self.close();
            return Err(Error::Expired);
        }
        self.now = now;
        if self.metadata.auth.side == Side::Host && self.policy_seen.is_none() {
            return Err(Error::Binding);
        }
        let mut output = Vec::new();
        let mut allowance = Allowance {
            tick,
            bytes: max_bytes.min(SOURCE_BYTES),
        };
        let Some(endpoint) = &mut self.pixels else { return Ok(output) };
        if let Err(error) = endpoint.poll(
            stream,
            self.budget.as_ref(),
            now,
            &mut allowance,
            &mut output,
            self.source_guard.as_ref(),
        ) {
            if error == Error::CaptureRevoked {
                self.retire_pixels();
            } else {
                self.close();
            }
            return Err(error);
        }
        let epoch = self.validity.load(Ordering::Acquire);
        for packet in &mut output {
            if let Received::UnadmittedPixels(pixels) = packet {
                pixels.validity = Some((self.validity.clone(), epoch));
            }
        }
        Ok(output)
    }
    pub fn poll_roles<T: Read + Write>(
        &mut self,
        metadata: &mut (impl Read + Write),
        pixels: Option<&mut T>,
        now: u64,
        tick: &mut TickBudget,
    ) -> Result<Vec<Received>, Error> {
        if now < self.now {
            self.close();
            return Err(Error::Expired);
        }
        self.now = now;
        let began = *self.began.get_or_insert(now);
        let mut allowance = Allowance { tick, bytes: SOURCE_BYTES };
        let mut output = Vec::new();
        if let Err(error) = self.metadata.poll(
            metadata,
            self.budget.as_ref(),
            now,
            &mut allowance,
            &mut output,
            self.source_guard.as_ref(),
        ) {
            self.close();
            return Err(error);
        }
        for packet in &output {
            if let Received::Metadata { metadata, .. } = packet {
                if matches!(metadata, Metadata::Policy { .. })
                    || matches!(metadata,Metadata::CaptureStopped{pixel_epoch,..} if *pixel_epoch==self.pixel_epoch)
                {
                    self.policy_seen = Some(now);
                }
                self.metadata_effect(*metadata)?;
            }
        }
        if self.metadata.auth.side == Side::Host && now.saturating_sub(self.policy_seen.unwrap_or(began)) >= PARTIAL_MILLIS {
            self.close();
            return Err(Error::Expired);
        }
        if (self.metadata.auth.side == Side::Worker || self.policy_seen.is_some())
            && let (Some(endpoint), Some(stream)) = (&mut self.pixels, pixels)
        {
            let before = output.len();
            if let Err(error) = endpoint.poll(
                stream,
                self.budget.as_ref(),
                now,
                &mut allowance,
                &mut output,
                self.source_guard.as_ref(),
            ) {
                if error == Error::CaptureRevoked {
                    self.retire_pixels();
                } else {
                    self.close();
                }
                return Err(error);
            }
            let epoch = self.validity.load(Ordering::Acquire);
            for packet in &mut output[before..] {
                if let Received::UnadmittedPixels(pixels) = packet {
                    pixels.validity = Some((self.validity.clone(), epoch));
                }
            }
        }
        Ok(output)
    }
}
impl Drop for Pair {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod owned_admission_tests {
    use super::super::tests::{policy, source};
    use super::*;
    use aurora_session::PreviewSize;
    use aurora_session::preview_image::{IMAGE_AUTHORITY_MILLIS, PreviewGeometry};

    fn fixture() -> (SourceImages, UnadmittedPixels, ImageBudget, PreviewGeometry) {
        fixture_with_slots(1)
    }
    fn fixture_with_slots(slots: usize) -> (SourceImages, UnadmittedPixels, ImageBudget, PreviewGeometry) {
        let geometry = PreviewGeometry::new([640, 360], [0, 0, 640, 360], PreviewSize::Small).unwrap();
        let budget = ImageBudget::new(geometry.payload_bytes() * slots).unwrap();
        let mut model = SourceImages::new(source(3), budget.clone()).unwrap();
        model.sync(policy(), 0).unwrap();
        let claim = model.request(8, 1, geometry, 1).unwrap();
        let stamp = PreviewStamp::new(claim, 1, 1, 1, geometry).unwrap();
        let mut pixels = UnadmittedPixels::blank(stamp, &budget).unwrap();
        pixels.bytes.fill(255);
        (model, pixels, budget, geometry)
    }
    fn rejected(result: Result<AdmittedPixels, (ImageError, UnadmittedPixels)>) -> (ImageError, UnadmittedPixels) {
        match result {
            Err(value) => value,
            Ok(_) => panic!("expected owned rejection"),
        }
    }
    #[test]
    fn preview_pixels_owned_model_rejection_returns_exact_allocation_and_can_retry_only_original_pending() {
        let (mut owner, pixels, budget, geometry) = fixture_with_slots(2);
        let pointer = pixels.bytes.as_ptr();
        let length = pixels.bytes.len();
        let capacity = pixels.bytes.capacity();
        let stamp = pixels.stamp;
        let mut wrong = SourceImages::new(source(3), budget.clone()).unwrap();
        wrong.sync(policy(), 0).unwrap();
        let wrong_claim = wrong.request(8, 1, geometry, 1).unwrap();
        let (error, pixels) = rejected(pixels.try_admit(&mut wrong, 2));
        assert_eq!(error, ImageError::Stale);
        assert_eq!(pixels.bytes.as_ptr(), pointer);
        assert_eq!(pixels.bytes.len(), length);
        assert_eq!(pixels.bytes.capacity(), capacity);
        assert_eq!(pixels.stamp, stamp);
        assert_eq!(budget.used(), length);
        let spare = budget.reserve(geometry).unwrap();
        assert_eq!(budget.used(), 2 * length);
        assert!(budget.reserve(geometry).is_err());
        drop(spare);
        assert_eq!(budget.used(), length);
        assert!(
            wrong.cancel_capture(wrong_claim),
            "failed admission did not consume another model's pending request"
        );
        let admitted = pixels.try_admit(&mut owner, 2).unwrap();
        assert_eq!(admitted.bytes(2).unwrap().as_ptr(), pointer);
        assert_eq!(admitted.image().stamp(), stamp);
        owner.retire();
        assert!(admitted.bytes(2).is_none());
        assert_eq!(budget.used(), length);
        drop(admitted);
        assert_eq!(budget.used(), 0);
    }
    #[test]
    fn preview_pixels_owned_transport_revocation_keeps_credit_and_never_revives_validity() {
        let (mut owner, mut pixels, budget, geometry) = fixture();
        let pointer = pixels.bytes.as_ptr();
        let claim = pixels.claim();
        let epoch = Arc::new(AtomicU64::new(7));
        pixels.validity = Some((epoch.clone(), 7));
        epoch.store(8, Ordering::Release);
        let (error, pixels) = rejected(pixels.try_admit(&mut owner, 2));
        assert_eq!(error, ImageError::Stale);
        assert_eq!(pixels.bytes.as_ptr(), pointer);
        assert_eq!(budget.used(), geometry.payload_bytes());
        let (error, pixels) = rejected(pixels.try_admit(&mut owner, 2));
        assert_eq!(error, ImageError::Stale);
        assert!(!pixels.transport_current());
        assert_eq!(epoch.load(Ordering::Acquire), 8);
        assert!(owner.cancel_capture(claim), "transport guard rejected before model admission");
        drop(pixels);
        assert_eq!(budget.used(), 0);
    }
    #[test]
    fn preview_pixels_owned_expiry_preserves_model_clock_purge_and_cannot_replay_into_fresh_pending() {
        let (mut owner, pixels, budget, geometry) = fixture_with_slots(2);
        let old = pixels.claim();
        let pointer = pixels.bytes.as_ptr();
        let (error, pixels) = rejected(pixels.try_admit(&mut owner, IMAGE_AUTHORITY_MILLIS));
        assert_eq!(error, ImageError::Stale);
        assert!(!owner.cancel_capture(old));
        assert_eq!(pixels.bytes.as_ptr(), pointer);
        assert_eq!(budget.used(), geometry.payload_bytes());
        owner.sync(policy(), IMAGE_AUTHORITY_MILLIS + 1).unwrap();
        let fresh = owner.request(8, 1, geometry, IMAGE_AUTHORITY_MILLIS + 2).unwrap();
        assert_ne!(old, fresh);
        let (error, pixels) = rejected(pixels.try_admit(&mut owner, IMAGE_AUTHORITY_MILLIS + 3));
        assert_eq!(error, ImageError::Stale);
        assert_eq!(pixels.bytes.as_ptr(), pointer);
        assert!(owner.cancel_capture(fresh));
        assert!(owner.retained(IMAGE_AUTHORITY_MILLIS + 3).unwrap().is_none());
        drop(pixels);
        assert_eq!(budget.used(), 0);
    }
    #[test]
    fn preview_pixels_consuming_wrapper_still_drops_rejected_allocation() {
        let (mut owner, pixels, budget, _) = fixture();
        assert!(owner.cancel_capture(pixels.claim()));
        assert_eq!(pixels.admit(&mut owner, 2).unwrap_err(), ImageError::Stale);
        assert_eq!(budget.used(), 0);
    }
}

#[cfg(test)]
mod service_deadline_tests {
    use super::super::tests::{scope, scoped_sessions};
    use super::*;
    struct Probe(usize);
    impl Read for Probe {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            self.0 += 1;
            Err(io::ErrorKind::WouldBlock.into())
        }
    }
    impl Write for Probe {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.0 += 1;
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    #[test]
    fn preview_io_partial_frame_actual_deadline_precedes_io_even_when_loop_clock_is_stale() {
        let (host, worker) = scoped_sessions(scope(Lane::Metadata));
        let metadata = Metadata::Stop {
            source: scope(Lane::Metadata).source,
            connection: 8,
            demand: 1,
        };
        let bytes = wire::encode_metadata(metadata).unwrap();
        let header = Header::new(Lane::Metadata, wire::Direction::HostToWorker, 8, 1, 1, bytes.as_slice().len(), 0).unwrap();
        let mut outgoing = Outgoing::new(header, bytes, None, &host.send);
        outgoing.live = true;
        outgoing.began = Some(0);
        outgoing.local_began = Some(Instant::now() - Duration::from_secs(2));
        let mut incoming = Incoming::new();
        incoming.live = true;
        incoming.began = Some(0);
        incoming.local_began = Some(Instant::now() - Duration::from_secs(2));
        let mut probe = Probe(0);
        let mut tick = TickBudget::new();
        let mut allowance = Allowance {
            tick: &mut tick,
            bytes: 64 * 1024,
        };
        assert!(matches!(
            outgoing.write(&mut probe, &host.send, 0, &mut allowance),
            Err(Error::Expired)
        ));
        assert!(matches!(
            incoming.read(&mut probe, &worker, None, 0, 0, &mut allowance),
            Err(Error::Expired)
        ));
        assert_eq!(probe.0, 0);
    }
}
