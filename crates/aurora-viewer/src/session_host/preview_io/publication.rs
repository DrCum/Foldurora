//! Ordered metadata publications are observation identity, never permission.
//! The final Store must compare whole-set/origin highwaters before any mutation.
use super::state::Observation;
use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PublicationStamp {
    publisher: u64,
    serial: u64,
}
impl PublicationStamp {
    pub fn publisher(self) -> u64 {
        self.publisher
    }
    pub fn serial(self) -> u64 {
        self.serial
    }
}
struct Head {
    publisher: u64,
    serial: AtomicU64,
    next: AtomicU64,
    retired: std::sync::atomic::AtomicBool,
}
pub(super) struct Publisher(Arc<Head>);
impl Publisher {
    pub fn new(publisher: u64) -> Self {
        Self(Arc::new(Head {
            publisher,
            serial: AtomicU64::new(0),
            next: AtomicU64::new(1),
            retired: std::sync::atomic::AtomicBool::new(false),
        }))
    }
    pub fn retire(&self) {
        self.0.retired.store(true, Ordering::Release);
        self.invalidate();
    }
    pub fn invalidate(&self) {
        self.0.serial.store(0, Ordering::Release);
    }
    pub fn publish(&self, mailbox: &mut state::HostMailbox) -> Result<(), Error> {
        if self.0.retired.load(Ordering::Acquire) {
            return Err(Error::Closed);
        }
        if mailbox.updates.len() > MAX_WORKERS {
            self.invalidate();
            return Err(Error::Capacity);
        }
        let sources: Vec<_> = mailbox
            .updates
            .iter()
            .map(|u| VersionedObservation {
                observation: u.observation.clone(),
            })
            .collect();
        if mailbox.observations.as_ref().is_some_and(|old| old.current() && old.same(&sources)) {
            return Ok(());
        }
        let serial = self
            .0
            .next
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_add(1))
            .map_err(|_| {
                self.invalidate();
                Error::Sequence
            })?;
        if self.0.publisher == 0 || serial == 0 {
            self.invalidate();
            return Err(Error::Sequence);
        }
        let stamp = PublicationStamp {
            publisher: self.0.publisher,
            serial,
        };
        mailbox.observations = Some(VersionedObservationSet {
            guard: PublicationGuard {
                stamp,
                head: self.0.clone(),
            },
            sources: sources.into(),
        });
        // Content is installed under the mailbox lock BEFORE the current head.
        self.0.serial.store(serial, Ordering::Release);
        Ok(())
    }
}
impl Drop for Publisher {
    fn drop(&mut self) {
        self.retire();
    }
}
#[derive(Clone)]
pub(crate) struct PublicationGuard {
    stamp: PublicationStamp,
    head: Arc<Head>,
}
impl PublicationGuard {
    pub fn stamp(&self) -> PublicationStamp {
        self.stamp
    }
    pub fn current(&self) -> bool {
        !self.head.retired.load(Ordering::Acquire)
            && self.stamp.publisher != 0
            && self.stamp.publisher == self.head.publisher
            && self.head.serial.load(Ordering::Acquire) == self.stamp.serial
    }
}
#[derive(Clone)]
pub(crate) struct VersionedObservationSet {
    guard: PublicationGuard,
    sources: Arc<[VersionedObservation]>,
}
impl VersionedObservationSet {
    pub fn stamp(&self) -> PublicationStamp {
        self.guard.stamp()
    }
    pub fn current(&self) -> bool {
        self.guard.current()
    }
    pub fn head_guard(&self) -> PublicationGuard {
        self.guard.clone()
    }
    pub fn sources(&self) -> &[VersionedObservation] {
        &self.sources
    }
    fn same(&self, sources: &[VersionedObservation]) -> bool {
        self.sources.len() == sources.len() && self.sources.iter().zip(sources).all(|(a, b)| a.same(b))
    }
}
#[derive(Clone)]
pub(crate) struct VersionedObservation {
    observation: Observation,
}
impl VersionedObservation {
    pub fn observation(&self) -> &Observation {
        &self.observation
    }
    pub fn demand_receipt(&self) -> Option<&DemandReceipt> {
        self.observation.demand_receipt.as_ref()
    }
    pub fn accepted_demand(&self) -> Option<&DemandReceipt> {
        self.observation.accepted_demand.as_ref()
    }
    pub fn domain(&self) -> &ObservationDomain {
        &self.observation.domain
    }
    pub fn policy(&self) -> Option<&PolicyReceipt> {
        self.observation.policy_receipt.as_ref()
    }
    pub fn layout(&self) -> Option<&LayoutReceipt> {
        self.observation.layout.and(self.observation.layout_receipt.as_ref())
    }
    fn same(&self, other: &Self) -> bool {
        let (a, b) = (&self.observation, &other.observation);
        a.domain.same_domain(&b.domain)
            && a.token == b.token
            && a.policy == b.policy
            && a.policy_received_at == b.policy_received_at
            && a.layout == b.layout
            && a.policy_receipt == b.policy_receipt
            && a.layout_receipt == b.layout_receipt
            && a.status == b.status
            && a.stopped_demand == b.stopped_demand
            && a.unavailable == b.unavailable
            && a.demand_receipt == b.demand_receipt
            && a.accepted_demand == b.accepted_demand
            && a.retention.same(&b.retention)
            && a.capture.same(&b.capture)
            && Arc::ptr_eq(&a.emergency, &b.emergency)
    }
}
#[derive(Clone)]
pub(crate) struct ObservationDomain {
    publisher: u64,
    source: PreviewSource,
    registration: u64,
    connection: u64,
    control: EpochFence,
    retention: EpochFence,
}
impl ObservationDomain {
    pub(super) fn new(publisher: u64, permit: &state::HostPermit, connection: u64, retention: EpochFence) -> Self {
        Self {
            publisher,
            source: permit.source,
            registration: permit.registration,
            connection,
            control: permit.control.witness.clone(),
            retention,
        }
    }
    #[cfg(test)]
    pub fn publisher(&self) -> u64 {
        self.publisher
    }
    pub fn source(&self) -> PreviewSource {
        self.source
    }
    pub fn registration(&self) -> u64 {
        self.registration
    }
    #[cfg(test)]
    pub fn metadata_connection(&self) -> u64 {
        self.connection
    }
    pub fn same_domain(&self, other: &Self) -> bool {
        self.publisher == other.publisher
            && self.source == other.source
            && self.registration == other.registration
            && self.connection == other.connection
            && self.control.same(&other.control)
            && self.retention.same(&other.retention)
    }
    pub fn matches_control(&self, control: &ControlGuard) -> bool {
        self.source == control.source && self.control.same(&control.witness)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ReceiptKind {
    Policy,
    Layout,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ReceiptOrigin {
    kind: ReceiptKind,
    publisher: u64,
    connection: u64,
    serial: u64,
}
impl ReceiptOrigin {
    pub fn publisher(self) -> u64 {
        self.publisher
    }
    pub fn connection(self) -> u64 {
        self.connection
    }
    pub fn serial(self) -> u64 {
        self.serial
    }
    pub(super) fn next(domain: &ObservationDomain, previous: Option<Self>, kind: ReceiptKind) -> Result<Self, Error> {
        if previous.is_some_and(|p| p.kind != kind) {
            return Err(Error::Sequence);
        }
        let serial = previous.map_or(Some(1), |p| p.serial.checked_add(1)).ok_or(Error::Sequence)?;
        Ok(Self {
            kind,
            publisher: domain.publisher,
            connection: domain.connection,
            serial,
        })
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct PolicyReceipt {
    origin: ReceiptOrigin,
    received_at: Instant,
    policy: PreviewPolicy,
}
impl PolicyReceipt {
    pub fn origin(self) -> ReceiptOrigin {
        self.origin
    }
    pub fn received_at(self) -> Instant {
        self.received_at
    }
    pub fn policy(self) -> PreviewPolicy {
        self.policy
    }
    pub(super) fn new(origin: ReceiptOrigin, received_at: Instant, policy: PreviewPolicy) -> Self {
        Self {
            origin,
            received_at,
            policy,
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct LayoutReceipt {
    origin: ReceiptOrigin,
    received_at: Instant,
    layout: PreviewLayout,
}
impl LayoutReceipt {
    pub fn origin(self) -> ReceiptOrigin {
        self.origin
    }
    #[cfg(test)]
    pub fn received_at(self) -> Instant {
        self.received_at
    }
    pub fn layout(self) -> PreviewLayout {
        self.layout
    }
    pub(super) fn new(origin: ReceiptOrigin, received_at: Instant, layout: PreviewLayout) -> Self {
        Self {
            origin,
            received_at,
            layout,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::service::Shared;
    use super::super::state::{HostMailbox, HostPermit, SourceUpdate};
    use super::*;
    fn fixture() -> (Shared<HostMailbox>, Observation, PreviewPolicy) {
        let control = super::super::tests::control(super::super::tests::source(1));
        let policy = super::super::tests::state(control.clone(), Some(EpochFence::new())).policy;
        let permit = HostPermit {
            source: control.source,
            control,
            secret: PreviewSecret::synthetic([7; 32]).unwrap(),
            registration: 1,
        };
        let shared = Shared::new(HostMailbox::new());
        let token = ConnectionToken {
            source: permit.source,
            connection: 1,
            pixel_epoch: 1,
            incarnation: shared.incarnation,
        };
        let observation = Observation::new(token, &permit, EpochFence::new(), shared.emergency.clone());
        (shared, observation, policy)
    }
    fn publish(shared: &Shared<HostMailbox>, observation: Observation) -> VersionedObservationSet {
        let mut m = shared.mailbox.lock().unwrap();
        m.updates.clear();
        m.updates.push(SourceUpdate { observation, frame: None });
        shared.publication.publish(&mut m).unwrap();
        m.observations.clone().unwrap()
    }
    #[test]
    fn preview_io_ordered_metadata_peeks_status_and_layout_never_renew_original_policy_receipt() {
        let (shared, mut observation, policy) = fixture();
        let original = Instant::now() - Duration::from_millis(700);
        observation.install_policy(policy, original).unwrap();
        let first = publish(&shared, observation.clone());
        assert!(first.current());
        assert!(first.head_guard().current());
        assert_eq!(first.stamp(), first.head_guard().stamp());
        assert_eq!(first.stamp().publisher(), shared.incarnation);
        assert!(first.stamp().serial() > 0);
        let same = publish(&shared, observation.clone());
        assert_eq!(first.stamp(), same.stamp());
        observation.status = state::Status::Pixels;
        let status = publish(&shared, observation.clone());
        assert!(!first.current());
        assert!(status.current());
        assert!(status.stamp().serial() > first.stamp().serial());
        let receipt = status.sources()[0].policy().unwrap();
        assert_eq!(receipt.received_at(), original);
        assert_eq!(receipt.policy(), policy);
        assert_eq!(receipt.origin().serial(), 1);
        assert_eq!(receipt.origin().publisher(), shared.incarnation);
        assert_eq!(receipt.origin().connection(), 1);
        let layout = PreviewLayout::new(policy.source(), policy.epochs(), 1, 1, 1, [640, 360], [0, 0, 640, 360]).unwrap();
        observation.install_layout(layout, original + Duration::from_millis(200)).unwrap();
        let with_layout = publish(&shared, observation.clone());
        let r = with_layout.sources()[0].layout().unwrap();
        assert_eq!(r.layout(), layout);
        assert_eq!(r.received_at(), original + Duration::from_millis(200));
        assert_eq!(r.origin().serial(), 1);
        assert_ne!(r.origin(), receipt.origin());
        assert!(with_layout.sources()[0].policy() == status.sources()[0].policy());
        observation.layout = None;
        let without = publish(&shared, observation.clone());
        assert!(without.sources()[0].layout().is_none());
        observation.install_layout(layout, original + Duration::from_millis(300)).unwrap();
        let restored = publish(&shared, observation.clone());
        assert_eq!(restored.sources()[0].layout().unwrap().origin().serial(), 2);
        observation.install_policy(policy, original + Duration::from_millis(400)).unwrap();
        let heartbeat = publish(&shared, observation);
        assert_eq!(heartbeat.sources()[0].policy().unwrap().origin().serial(), 2);
        assert_eq!(
            heartbeat.sources()[0].policy().unwrap().received_at(),
            original + Duration::from_millis(400)
        );
        assert!(!status.current());
    }
    #[test]
    fn preview_io_ordered_metadata_empty_roster_and_new_source_domain_retire_old_copies_not_new_domain() {
        let (shared, mut observation, policy) = fixture();
        let old_control = super::super::tests::control(policy.source());
        for _ in 0..3 {
            observation.install_policy(policy, Instant::now()).unwrap();
        }
        let old = publish(&shared, observation.clone());
        let token = observation.token;
        let permit = HostPermit {
            source: policy.source(),
            control: old_control.clone(),
            secret: PreviewSecret::synthetic([7; 32]).unwrap(),
            registration: 2,
        };
        let mut replacement = Observation::new(
            ConnectionToken { connection: 2, ..token },
            &permit,
            EpochFence::new(),
            shared.emergency.clone(),
        );
        replacement.install_policy(policy, Instant::now()).unwrap();
        let new = publish(&shared, replacement);
        assert!(!old.current());
        assert!(new.current());
        let (a, b) = (old.sources()[0].domain(), new.sources()[0].domain());
        assert!(!a.same_domain(b));
        assert_eq!(b.metadata_connection(), 2);
        assert_eq!(b.registration(), 2);
        assert_eq!(b.publisher(), shared.incarnation);
        assert_eq!(b.source(), policy.source());
        assert!(b.matches_control(&old_control));
        assert!(!a.matches_control(&old_control));
        assert_eq!(new.sources()[0].policy().unwrap().origin().serial(), 1);
        assert_eq!(old.sources()[0].policy().unwrap().origin().serial(), 3);
        assert!(new.sources()[0].demand_receipt().is_none());
        assert!(new.sources()[0].accepted_demand().is_none());
        let mut m = shared.mailbox.lock().unwrap();
        m.updates.clear();
        shared.publication.publish(&mut m).unwrap();
        let empty = m.observations.clone().unwrap();
        assert!(empty.current());
        assert!(empty.sources().is_empty());
        assert!(!new.current());
        drop(m);
        shared.publication.invalidate();
        assert!(!empty.current());
    }
    #[test]
    fn preview_io_policy_original_receipt_time_is_charged_after_copy_delay_without_equal_origin_renewal() {
        let (shared, mut observation, policy) = fixture();
        let received = Instant::now() - Duration::from_millis(900);
        observation.install_policy(policy, received).unwrap();
        let set = publish(&shared, observation);
        let receipt = *set.sources()[0].policy().unwrap();
        let mut images = aurora_session::preview_image::SourceImages::new(policy.source(), ImageBudget::new(1024 * 1024).unwrap()).unwrap();
        images.sync_observed(receipt.policy(), 10, 910).unwrap();
        let geometry = aurora_session::preview_image::PreviewGeometry::new([640, 360], [0, 0, 640, 360], PreviewSize::Small).unwrap();
        let claim = images.request(1, 1, geometry, 1009).unwrap();
        assert!(images.cancel_capture(claim));
        // Mapping is anchored to ORIGINAL receipt, not the later publication read.
        assert_eq!(receipt.received_at(), received);
        assert!(images.request(1, 1, geometry, 1010).is_err());
    }
    #[test]
    fn preview_io_publication_overflow_and_service_recreation_never_reuse_a_current_head() {
        let (shared, observation, _) = fixture();
        let old = publish(&shared, observation.clone());
        shared.publication.0.next.store(u64::MAX, Ordering::Release);
        let mut changed = observation;
        changed.status = state::Status::Fault;
        let mut m = shared.mailbox.lock().unwrap();
        m.updates[0].observation = changed;
        assert!(shared.publication.publish(&mut m).is_err());
        assert!(!old.current());
        drop(m);
        let (replacement, observation, _) = fixture();
        let fresh = publish(&replacement, observation);
        assert_ne!(old.stamp().publisher(), fresh.stamp().publisher());
        assert!(fresh.current());
        assert!(!old.current());
        replacement.publication.retire();
        assert!(!fresh.current());
        let mut m = replacement.mailbox.lock().unwrap();
        assert!(matches!(replacement.publication.publish(&mut m), Err(Error::Closed)));
        assert!(!fresh.head_guard().current());
    }
}
