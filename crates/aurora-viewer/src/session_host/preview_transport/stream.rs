use super::auth::{Authenticated, Keys};
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

const SOURCE_BYTES: usize = 64 * 1024;
const TOTAL_BYTES: usize = 256 * 1024;
const ROLE_OPERATIONS: usize = 16;
const PARTIAL_MILLIS: u64 = 1000;
const METADATA_QUEUE: usize = 32;

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
/// and authenticated policy, then consume this through `admit` before any UI
/// publication, texture copy or image cache replacement.
///
/// Actual allocation credit travels with the only owned Vec. Field order frees
/// bytes before credit on rejection/drop; no split Vec/credit accessor or Clone
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
    pub fn copy_from(stamp: PreviewStamp, input: &[u8], budget: &ImageBudget) -> Result<Self, Error> {
        if input.len() != stamp.geometry().payload_bytes() || input.as_chunks::<4>().0.iter().any(|pixel| pixel[3] != 255) {
            return Err(Error::Invalid);
        }
        let mut pixels = Self::blank(stamp, budget)?;
        pixels.bytes.copy_from_slice(input);
        Ok(pixels)
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
    pub fn admit(self, owner: &mut SourceImages, now: u64) -> Result<AdmittedPixels, ImageError> {
        if !self.transport_current() {
            return Err(ImageError::Stale);
        }
        let image = owner.admit(self.stamp, self.credit.clone(), now)?;
        Ok(AdmittedPixels { bytes: self.bytes, image })
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
    Metadata(Metadata),
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
    pixels: Option<UnadmittedPixels>,
    body: Option<hmac::Context>,
    finish: Option<[u8; TAG_BYTES]>,
    position: usize,
    began: Option<u64>,
}
impl Outgoing {
    fn new(header: Header, metadata: MetadataBytes, pixels: Option<UnadmittedPixels>, keys: &Keys) -> Self {
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
        }
    }
    fn write(&mut self, io: &mut impl Write, keys: &Keys, now: u64, allowance: &mut Allowance<'_>) -> Result<bool, Error> {
        if self
            .began
            .is_some_and(|began| now.checked_sub(began).is_none_or(|age| age >= PARTIAL_MILLIS))
        {
            return Err(Error::Expired);
        }
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
            (self.pixels.as_ref().ok_or(Error::Invalid)?.bytes.as_slice(), pixel_start)
        } else {
            (&self.finish.as_ref().ok_or(Error::Invalid)?[..], pixel_end)
        };
        let start = self.position - offset;
        let end = (start + allowance.available()).min(part.len());
        match io.write(&part[start..end]) {
            Ok(0) => Err(Error::Closed),
            Ok(count) => {
                self.began.get_or_insert(now);
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
            decoded: None,
            pixels: None,
            body: None,
        }
    }
    fn read(
        &mut self,
        io: &mut impl Read,
        auth: &Authenticated,
        budget: &ImageBudget,
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
            if header.lane() != auth.scope.lane || header.connection() != auth.scope.connection {
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
                    self.pixels = Some(UnadmittedPixels::blank(stamp, budget)?);
                }
            }
            self.body = Some(auth.receive.body(header, metadata, &self.header_tag));
        }
        if let Some(header) = self.header
            && self.position == header.frame_bytes()
        {
            auth.receive.verify_finish(self.body.take().ok_or(Error::Invalid)?, &self.finish)?;
            return match header.lane() {
                Lane::Metadata => Ok(Some(Received::Metadata(self.decoded.take().ok_or(Error::Invalid)?))),
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
            closed: false,
        }
    }
    fn close(&mut self) {
        self.closed = true;
        self.incoming = Incoming::new();
        self.outgoing.clear();
    }
    fn enqueue(&mut self, metadata: MetadataBytes, pixels: Option<UnadmittedPixels>) -> Result<(), Error> {
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
            sequence,
            metadata.as_slice().len(),
            pixels.as_ref().map_or(0, |pixels| pixels.bytes.len()),
        )?;
        self.outgoing.push_back(Outgoing::new(header, metadata, pixels, &self.auth.send));
        self.sent = sequence;
        Ok(())
    }
    fn poll(
        &mut self,
        io: &mut (impl Read + Write),
        budget: &ImageBudget,
        now: u64,
        allowance: &mut Allowance<'_>,
        output: &mut Vec<Received>,
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
            if read {
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
                    match frame.write(io, &self.auth.send, now, allowance) {
                        Ok(true) => {
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
    pixels: Endpoint,
    budget: ImageBudget,
    validity: Arc<AtomicU64>,
    policy: Option<PreviewPolicy>,
    now: u64,
    began: Option<u64>,
    policy_seen: Option<u64>,
}
impl Pair {
    pub fn new(metadata: Authenticated, pixels: Authenticated, budget: ImageBudget) -> Result<Self, Error> {
        if metadata.scope.lane != Lane::Metadata
            || pixels.scope.lane != Lane::Pixels
            || metadata.side != pixels.side
            || metadata.scope.source != pixels.scope.source
            || metadata.scope.connection != pixels.scope.connection
        {
            return Err(Error::Binding);
        }
        Ok(Self {
            metadata: Endpoint::new(metadata),
            pixels: Endpoint::new(pixels),
            budget,
            validity: Arc::new(AtomicU64::new(1)),
            policy: None,
            now: 0,
            began: None,
            policy_seen: None,
        })
    }
    fn retire_pixels(&mut self) {
        self.validity.store(0, Ordering::Release);
        self.pixels.close();
    }
    pub fn close(&mut self) {
        self.retire_pixels();
        self.metadata.close();
    }
    fn metadata_effect(&mut self, metadata: Metadata) -> Result<(), Error> {
        match metadata {
            Metadata::Invalidate { .. } => self.retire_pixels(),
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
            .and_then(|metadata| self.metadata.enqueue(metadata, None));
        if result.is_err() {
            self.close();
        }
        result
    }
    pub fn queue_pixels(&mut self, pixels: UnadmittedPixels) -> Result<(), Error> {
        if pixels.stamp.claim().source() != self.pixels.auth.scope.source
            || pixels.stamp.claim().connection() != self.pixels.auth.scope.connection
            || !pixels.transport_current()
        {
            self.close();
            return Err(Error::Binding);
        }
        if self.pixels.auth.side != Side::Worker {
            self.close();
            return Err(Error::Direction);
        }
        self.pixels.enqueue(wire::encode_stamp(pixels.stamp)?, Some(pixels))
    }
    pub fn poll(
        &mut self,
        metadata: &mut (impl Read + Write),
        pixels: &mut (impl Read + Write),
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
        if let Err(error) = self.metadata.poll(metadata, &self.budget, now, &mut allowance, &mut output) {
            self.close();
            return Err(error);
        }
        for packet in &output {
            if let Received::Metadata(metadata) = packet {
                if matches!(metadata, Metadata::Policy { .. }) {
                    self.policy_seen = Some(now);
                }
                self.metadata_effect(*metadata)?;
            }
        }
        // A quiet-but-open metadata socket is not fresh privacy authority.
        // Workers publish Policy independently; host pixels wait for it and
        // retire after one second without a complete authenticated update.
        if self.metadata.auth.side == Side::Host && now.saturating_sub(self.policy_seen.unwrap_or(began)) >= PARTIAL_MILLIS {
            self.close();
            return Err(Error::Expired);
        }
        if !self.pixels.closed && (self.metadata.auth.side == Side::Worker || self.policy_seen.is_some()) {
            let before = output.len();
            if let Err(error) = self.pixels.poll(pixels, &self.budget, now, &mut allowance, &mut output) {
                self.close();
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
