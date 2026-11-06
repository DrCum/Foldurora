//! Preview-specific typed binary lane. Authentication/framing is supplied by
//! the private purpose-bound channel; this codec never accepts chat packets.
//! Images are actual small RGBA targets, never a full-window capture/resize.

use aurora_session::{Binding, PreviewRate, PreviewSize, SessionIdentity, WorkerIdentity};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

pub(crate) const MAX_HEADER: usize = 4096;
pub(crate) const MAX_RGBA: usize = 640 * 360 * 4;
pub(crate) const MAX_PACKET: usize = 14 + MAX_HEADER + MAX_RGBA;
pub(crate) const CONTROL_FRESH: Duration = Duration::from_millis(750);
pub(crate) const SAMPLE_FRESH: Duration = Duration::from_millis(500);
pub(crate) const BUDGET_ROUND_TRIP: Duration = Duration::from_millis(250);
const MAGIC: &[u8; 8] = b"FLDPREV1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PreviewSource {
    pub worker: WorkerIdentity,
    pub session: SessionIdentity,
    pub lease_epoch: u64,
}

impl PreviewSource {
    pub fn new(binding: Binding, lease_epoch: u64) -> Result<Self, Error> {
        let source = Self {
            worker: binding.worker,
            session: binding.session.ok_or(Error::Source)?,
            lease_epoch,
        };
        source.validate()?;
        Ok(source)
    }
    pub fn binding(self) -> Binding {
        Binding {
            worker: self.worker,
            session: Some(self.session),
        }
    }
    pub fn validate(self) -> Result<(), Error> {
        if self.lease_epoch == 0 || self.session.generation == 0 {
            return Err(Error::Source);
        }
        self.binding().validate().map_err(|_| Error::Source)
    }
}

/// Public registry metadata only. Authentication comes from the private host
/// lane and the host's real child process, never from this advertised claim.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Cohort {
    pub source: PreviewSource,
    pub device_epoch: u64,
    pub pid: u32,
}
impl Cohort {
    pub fn validate(self) -> Result<(), Error> {
        self.source.validate()?;
        if self.device_epoch == 0 || self.pid == 0 {
            return Err(Error::Source);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Peer {
    pub cohort: Cohort,
    pub luid: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Probe {
    pub source: PreviewSource,
    pub challenge: u64,
}

/// Fresh authenticated process/device metadata. This announces no GPU timing
/// or permission; a separate new completed native Sample is still required.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CohortReport {
    pub peer: Peer,
    pub challenge: u64,
}
impl CohortReport {
    pub fn validate(self) -> Result<(), Error> {
        self.peer.cohort.validate()?;
        if self.peer.luid == 0 || self.challenge == 0 {
            return Err(Error::Source);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GpuSample {
    pub peer: Peer,
    pub challenge: u64,
    pub surface_epoch: u64,
    pub frame: u64,
    pub gpu_micros: u32,
}
impl GpuSample {
    pub fn validate(self) -> Result<(), Error> {
        self.peer.cohort.validate()?;
        if self.peer.luid == 0 || self.challenge == 0 || self.surface_epoch == 0 || self.frame == 0 || self.gpu_micros == 0 {
            return Err(Error::Source);
        }
        Ok(())
    }
}

/// The host measured sample_age_ms from its own active-worker challenge. The
/// standby worker also limits its own request round trip to250ms; neither side
/// trusts the other's wall clock. The combined measurement age is at most750ms.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BudgetPermit {
    pub request: u64,
    pub sample: GpuSample,
    pub sample_age_ms: u16,
    pub peers: Vec<Peer>,
}
impl BudgetPermit {
    pub fn validate(&self) -> Result<(), Error> {
        self.sample.validate()?;
        if self.request == 0
            || Duration::from_millis(self.sample_age_ms.into()) > SAMPLE_FRESH
            || self.sample.gpu_micros > 100_000
            || self.peers.is_empty()
            || self.peers.len() >= aurora_session::MAX_WORKERS
        {
            return Err(Error::Source);
        }
        for peer in &self.peers {
            peer.cohort.validate()?;
            if peer.luid == 0 {
                return Err(Error::Source);
            }
        }
        if !self.peers.contains(&self.sample.peer) {
            return Err(Error::Source);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Subscription {
    pub source: PreviewSource,
    pub revision: u64,
    pub size: PreviewSize,
    pub rate: PreviewRate,
    pub visible: bool,
    pub challenge: u64,
    pub budget: Option<BudgetPermit>,
}

impl Subscription {
    pub fn validate(&self) -> Result<(), Error> {
        self.source.validate()?;
        if self.revision == 0 || self.challenge == 0 {
            return Err(Error::Sequence);
        }
        if let Some(budget) = &self.budget {
            budget.validate()?;
            if budget.peers.iter().any(|p| {
                p.cohort.source.worker.host != self.source.worker.host
                    || p.cohort.source.worker.installation != self.source.worker.installation
            }) {
                return Err(Error::Source);
            }
        }
        Ok(())
    }
}

/// No Live status is emitted until a real small-target GPU frame is available.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Availability {
    Disabled,
    Hidden,
    Active,
    Offline,
    Restricted,
    AssetsNotReady,
    Budget,
    Initializing,
    Live,
    Unsupported,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct State {
    pub source: PreviewSource,
    pub subscription_revision: u64,
    /// Strictly advances on identity, lease, visibility, renderer or vision
    /// policy changes. Receivers purge the previous epoch before accepting data.
    pub epoch: u64,
    pub availability: Availability,
    pub challenge: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ImageHeader {
    pub source: PreviewSource,
    pub subscription_revision: u64,
    pub epoch: u64,
    pub sequence: u64,
    pub size: PreviewSize,
    pub challenge: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Header {
    Subscribe(Subscription),
    State(State),
    Image(ImageHeader),
    Probe(Probe),
    Sample(GpuSample),
    Peer(CohortReport),
}

impl Header {
    pub fn source(&self) -> PreviewSource {
        match self {
            Self::Subscribe(s) => s.source,
            Self::State(s) => s.source,
            Self::Image(s) => s.source,
            Self::Probe(p) => p.source,
            Self::Sample(s) => s.peer.cohort.source,
            Self::Peer(p) => p.peer.cohort.source,
        }
    }
    pub fn validate(&self) -> Result<(), Error> {
        self.source().validate()?;
        match self {
            Self::Subscribe(s) => s.validate(),
            Self::State(s) if s.subscription_revision != 0 && s.epoch != 0 && s.challenge != 0 => Ok(()),
            Self::Image(s) if s.subscription_revision != 0 && s.epoch != 0 && s.sequence != 0 && s.challenge != 0 => Ok(()),
            Self::Probe(p) if p.challenge != 0 => Ok(()),
            Self::Sample(s) => s.validate(),
            Self::Peer(p) => p.validate(),
            _ => Err(Error::Sequence),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Error {
    Source,
    Sequence,
    Malformed,
    Oversized,
    Payload,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid preview packet: {self:?}")
    }
}
impl std::error::Error for Error {}

/// A complete private message before/after the authenticated channel. Lengths
/// are checked before JSON deserialization or allocating an image buffer.
pub(crate) fn encode(header: &Header, rgba: &[u8]) -> Result<Vec<u8>, Error> {
    header.validate()?;
    payload_size(header, rgba.len())?;
    let json = serde_json::to_vec(header).map_err(|_| Error::Malformed)?;
    if json.is_empty() || json.len() > MAX_HEADER {
        return Err(Error::Oversized);
    }
    let mut bytes = Vec::with_capacity(14 + json.len() + rgba.len());
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&(json.len() as u16).to_le_bytes());
    bytes.extend_from_slice(&(rgba.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&json);
    bytes.extend_from_slice(rgba);
    Ok(bytes)
}

pub(crate) fn decode(bytes: &[u8]) -> Result<(Header, &[u8]), Error> {
    if bytes.len() > MAX_PACKET {
        return Err(Error::Oversized);
    }
    if bytes.len() < 14 || &bytes[..8] != MAGIC {
        return Err(Error::Malformed);
    }
    let header_len = usize::from(u16::from_le_bytes([bytes[8], bytes[9]]));
    let rgba_len = u32::from_le_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]) as usize;
    if header_len == 0 || header_len > MAX_HEADER || rgba_len > MAX_RGBA {
        return Err(Error::Oversized);
    }
    let body = 14usize.checked_add(header_len).ok_or(Error::Oversized)?;
    if body.checked_add(rgba_len) != Some(bytes.len()) {
        return Err(Error::Malformed);
    }
    let header: Header = serde_json::from_slice(&bytes[14..body]).map_err(|_| Error::Malformed)?;
    header.validate()?;
    payload_size(&header, rgba_len)?;
    Ok((header, &bytes[body..]))
}

fn payload_size(header: &Header, bytes: usize) -> Result<(), Error> {
    let expected = match header {
        Header::Image(image) => image.size.bytes() as usize,
        _ => 0,
    };
    if bytes != expected || bytes > MAX_RGBA {
        Err(Error::Payload)
    } else {
        Ok(())
    }
}

pub(crate) fn current(received_at: Instant, now: Instant) -> bool {
    now.checked_duration_since(received_at).is_some_and(|age| age <= CONTROL_FRESH)
}

#[cfg(test)]
pub(crate) fn fixture_source(worker: u8) -> PreviewSource {
    use aurora_session::{AccountKey, GridKey, HostId, InstallationId, SessionId, WorkerId};
    PreviewSource {
        worker: WorkerIdentity {
            installation: InstallationId::from_bytes([1; 32]).unwrap(),
            host: HostId::from_bytes([2; 16]).unwrap(),
            worker: WorkerId::from_bytes([worker; 16]).unwrap(),
        },
        session: SessionIdentity {
            session: SessionId::from_bytes([4; 16]).unwrap(),
            account: AccountKey::from_bytes([5; 32]).unwrap(),
            grid: GridKey::from_bytes([6; 32]).unwrap(),
            generation: 7,
        },
        lease_epoch: 8,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_session::{AccountKey, GridKey, HostId, InstallationId, SessionId, WorkerId};
    fn source() -> PreviewSource {
        PreviewSource {
            worker: WorkerIdentity {
                installation: InstallationId::from_bytes([1; 32]).unwrap(),
                host: HostId::from_bytes([2; 16]).unwrap(),
                worker: WorkerId::from_bytes([3; 16]).unwrap(),
            },
            session: SessionIdentity {
                session: SessionId::from_bytes([4; 16]).unwrap(),
                account: AccountKey::from_bytes([5; 32]).unwrap(),
                grid: GridKey::from_bytes([6; 32]).unwrap(),
                generation: 7,
            },
            lease_epoch: 8,
        }
    }
    #[test]
    fn small_image_packets_round_trip_exact_source_size_and_bytes_without_chat_limits() {
        for size in [PreviewSize::Small, PreviewSize::Medium, PreviewSize::Large] {
            let header = Header::Image(ImageHeader {
                source: source(),
                subscription_revision: 1,
                epoch: 2,
                sequence: 3,
                size,
                challenge: 4,
            });
            let rgba = vec![7; size.bytes() as usize];
            let encoded = encode(&header, &rgba).unwrap();
            let (actual, bytes) = decode(&encoded).unwrap();
            assert_eq!(actual, header);
            assert_eq!(bytes, rgba);
            assert!(encoded.len() <= MAX_PACKET);
            assert!(encoded.len() > 4096);
        }
    }
    #[test]
    fn image_codec_refuses_truncated_oversized_mismatched_and_non_image_payloads() {
        let header = Header::Image(ImageHeader {
            source: source(),
            subscription_revision: 1,
            epoch: 2,
            sequence: 3,
            size: PreviewSize::Small,
            challenge: 4,
        });
        assert_eq!(encode(&header, &[0; 4]), Err(Error::Payload));
        let state = Header::State(State {
            source: source(),
            subscription_revision: 1,
            epoch: 2,
            availability: Availability::Restricted,
            challenge: 4,
        });
        assert_eq!(encode(&state, &[0; 4]), Err(Error::Payload));
        let mut bytes = encode(&state, &[]).unwrap();
        bytes.push(0);
        assert_eq!(decode(&bytes), Err(Error::Malformed));
        let mut bytes = encode(&state, &[]).unwrap();
        bytes[10..14].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(decode(&bytes), Err(Error::Oversized));
        assert_eq!(decode(&vec![0; MAX_PACKET + 1]), Err(Error::Oversized));
    }
    #[test]
    fn preview_source_requires_exact_bound_session_generation_and_positive_lease() {
        let s = source();
        assert_eq!(PreviewSource::new(s.binding(), s.lease_epoch), Ok(s));
        assert_eq!(
            PreviewSource::new(
                Binding {
                    session: None,
                    ..s.binding()
                },
                8
            ),
            Err(Error::Source)
        );
        assert_eq!(PreviewSource { lease_epoch: 0, ..s }.validate(), Err(Error::Source));
        assert_eq!(
            PreviewSource {
                session: SessionIdentity {
                    generation: 0,
                    ..s.session
                },
                ..s
            }
            .validate(),
            Err(Error::Source)
        );
        let now = Instant::now();
        assert!(current(now, now + CONTROL_FRESH));
        assert!(!current(now, now + CONTROL_FRESH + Duration::from_nanos(1)));
        assert!(!current(now + Duration::from_nanos(1), now));
    }
}
