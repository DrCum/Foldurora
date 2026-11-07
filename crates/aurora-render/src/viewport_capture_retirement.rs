//! Request-origin cleanup tracking, independent of new-capture eligibility.
//! A backend Idle observation is valid only for the exact frontend revision
//! probed after its last handoff/invalidation, never from a timer or byte count.

use crate::render_thread::{Done, Job, Owner, TrySubmitError};
use crate::viewport_capture::{CaptureError, CaptureToken};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureRetirementProgress {
    /// This renderer has never accepted a viewport request. No owner access.
    NoWork,
    Pending,
    /// Maps/submitted callbacks have retired; cached/held credits may remain.
    Idle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Purpose {
    Complete,
    Retire,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Probe {
    origin: CaptureToken,
    revision: u64,
    serial: u64,
    purpose: Purpose,
}
impl Probe {
    pub fn origin(self) -> CaptureToken {
        self.origin
    }
    pub fn purpose(self) -> Purpose {
        self.purpose
    }
}

#[derive(Default)]
pub(crate) struct Retirement {
    origin: Option<CaptureToken>,
    revision: u64,
    serial: u64,
    flight: Option<Probe>,
    idle: bool,
    exhausted: bool,
}

impl Retirement {
    pub fn available(&self) -> Result<(), CaptureError> {
        if self.exhausted { Err(CaptureError::Exhausted) } else { Ok(()) }
    }
    pub fn request(&mut self, origin: CaptureToken) -> Result<(), CaptureError> {
        self.available()?;
        self.origin = Some(origin);
        self.dirty();
        self.available()
    }
    /// Keep the last real origin after Idle: a later surface/generation change
    /// must also retire that request's cached pair. Never arm an empty tracker.
    pub fn dirty(&mut self) {
        if self.origin.is_none() {
            return;
        }
        self.idle = false;
        match self.revision.checked_add(1) {
            Some(revision) => self.revision = revision,
            None => self.exhausted = true,
        }
    }
    pub fn progress(&self, frontend_pending: bool) -> Result<CaptureRetirementProgress, CaptureError> {
        self.available()?;
        Ok(if self.origin.is_none() {
            CaptureRetirementProgress::NoWork
        } else if self.idle && self.flight.is_none() && !frontend_pending {
            CaptureRetirementProgress::Idle
        } else {
            CaptureRetirementProgress::Pending
        })
    }
    fn probe(&mut self, purpose: Purpose) -> Result<Option<Probe>, CaptureError> {
        self.available()?;
        if self.idle || self.flight.is_some() {
            return Ok(None);
        }
        let Some(origin) = self.origin else { return Ok(None) };
        let Some(serial) = self.serial.checked_add(1) else {
            self.exhausted = true;
            return Err(CaptureError::Exhausted);
        };
        self.serial = serial;
        let probe = Probe {
            origin,
            revision: self.revision,
            serial,
            purpose,
        };
        self.flight = Some(probe);
        Ok(Some(probe))
    }
    fn rejected(&mut self, probe: Probe) {
        if self.flight == Some(probe) {
            self.flight = None;
        }
    }
    pub fn receive(&mut self, probe: Probe, pending: bool) {
        if probe.serial != self.serial || self.flight != Some(probe) {
            return;
        }
        self.flight = None;
        if self.origin == Some(probe.origin) && self.revision == probe.revision && !self.exhausted {
            self.idle = !pending;
        }
    }
    /// Actual production owner seam, shared by ordinary result polling and
    /// retirement-only polling. Rejection consumes the serial, never its origin.
    pub fn try_submit(
        &mut self,
        owner: &Owner<Job, Done>,
        fences: &crate::session_fence::Requests,
        purpose: Purpose,
    ) -> Result<Option<Done>, CaptureError> {
        if fences.pending() {
            return Ok(None);
        }
        let Some(probe) = self.probe(purpose)? else { return Ok(None) };
        match owner.try_submit_owned(Job::MaintainViewportCapture(probe)) {
            Ok(submitted) => Ok(submitted.previous),
            Err(TrySubmitError::Busy(_)) => {
                self.rejected(probe);
                Ok(None)
            }
            Err(TrySubmitError::WorkerGone(_)) => {
                self.rejected(probe);
                Err(CaptureError::DeviceLost)
            }
        }
    }
}

pub(crate) fn owner_result(probe: Probe, maintain: impl FnOnce() -> bool) -> Done {
    Done::ViewportCaptureMaintenance {
        probe,
        pending: maintain(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render_thread::handoff;
    use crate::viewport_capture::{CaptureBoundary, CaptureBudget, CaptureCap, CaptureQueue};

    fn token() -> CaptureToken {
        CaptureQueue::new(
            CaptureBoundary::new(1, 1, (640, 360)).unwrap(),
            CaptureBudget::new(1024 * 1024).unwrap(),
        )
        .unwrap()
        .request(crate::WorldViewport::full((640, 360)), CaptureCap::Small, 0)
        .unwrap()
        .token()
    }
    fn complete(worker: &crate::render_thread::Worker<Job, Done>, pending: bool) -> Probe {
        let (Job::MaintainViewportCapture(probe), _) = worker.next().unwrap() else {
            panic!("maintenance only")
        };
        worker.complete(owner_result(probe, || pending));
        probe
    }
    fn receive(tracker: &mut Retirement, owner: &Owner<Job, Done>) {
        let Some(Done::ViewportCaptureMaintenance { probe, pending }) = owner.try_take_ready().unwrap() else {
            panic!("exact maintenance reply")
        };
        tracker.receive(probe, pending);
    }
    #[test]
    fn viewport_retirement_never_requested_or_physical_change_does_not_access_dead_owner() {
        let (owner, worker) = handoff();
        drop(worker);
        let mut tracker = Retirement::default();
        tracker.dirty();
        assert_eq!(tracker.progress(false), Ok(CaptureRetirementProgress::NoWork));
        assert!(
            tracker
                .try_submit(&owner, &crate::session_fence::Requests::default(), Purpose::Retire)
                .unwrap()
                .is_none()
        );
        assert_eq!(tracker.serial, 0);
    }
    #[test]
    fn viewport_retirement_actual_owner_polls_until_backend_callback_map_retirement() {
        let (owner, worker) = handoff();
        let mut tracker = Retirement::default();
        tracker.request(token()).unwrap();
        tracker
            .try_submit(&owner, &crate::session_fence::Requests::default(), Purpose::Retire)
            .unwrap();
        let first = complete(&worker, true);
        receive(&mut tracker, &owner);
        assert_eq!(tracker.progress(false), Ok(CaptureRetirementProgress::Pending));
        tracker
            .try_submit(&owner, &crate::session_fence::Requests::default(), Purpose::Retire)
            .unwrap();
        let last = complete(&worker, false);
        assert_ne!(first, last);
        receive(&mut tracker, &owner);
        assert_eq!(tracker.progress(false), Ok(CaptureRetirementProgress::Idle));
        assert!(
            tracker
                .try_submit(&owner, &crate::session_fence::Requests::default(), Purpose::Retire)
                .unwrap()
                .is_none()
        );
        tracker.dirty(); // Cancel/surface change retires an already cached pair.
        assert_eq!(tracker.progress(false), Ok(CaptureRetirementProgress::Pending));
        tracker
            .try_submit(&owner, &crate::session_fence::Requests::default(), Purpose::Retire)
            .unwrap();
        complete(&worker, false);
        receive(&mut tracker, &owner);
        assert_eq!(tracker.progress(false), Ok(CaptureRetirementProgress::Idle));
    }
    #[test]
    fn viewport_retirement_stale_idle_cannot_clear_new_request_or_same_token_handoff() {
        for replacement in [false, true] {
            let (owner, worker) = handoff();
            let mut tracker = Retirement::default();
            tracker.request(token()).unwrap();
            tracker
                .try_submit(&owner, &crate::session_fence::Requests::default(), Purpose::Retire)
                .unwrap();
            let old = complete(&worker, false);
            if replacement {
                tracker.request(token()).unwrap();
            } else {
                tracker.dirty(); // The same request now leaves in a FramePacket.
            }
            receive(&mut tracker, &owner);
            assert_eq!(tracker.progress(false), Ok(CaptureRetirementProgress::Pending));
            tracker
                .try_submit(&owner, &crate::session_fence::Requests::default(), Purpose::Retire)
                .unwrap();
            let fresh = complete(&worker, false);
            receive(&mut tracker, &owner);
            tracker.receive(old, false); // Duplicate/out-of-order completion.
            assert_ne!(old, fresh);
            assert_eq!(tracker.progress(false), Ok(CaptureRetirementProgress::Idle));
            assert_eq!(tracker.progress(true), Ok(CaptureRetirementProgress::Pending));
        }
    }
    #[test]
    fn viewport_retirement_recreated_frontend_rejects_old_probe_with_same_local_counters() {
        let mut old = Retirement::default();
        old.request(token()).unwrap();
        let previous = old.probe(Purpose::Retire).unwrap().unwrap();
        drop(old);
        let mut replacement = Retirement::default();
        replacement.request(token()).unwrap();
        let current = replacement.probe(Purpose::Retire).unwrap().unwrap();
        assert_eq!(previous.serial, current.serial);
        assert_eq!(previous.revision, current.revision);
        assert_ne!(previous.origin, current.origin);
        replacement.receive(previous, false);
        assert_eq!(replacement.progress(false), Ok(CaptureRetirementProgress::Pending));
        replacement.receive(current, false);
        assert_eq!(replacement.progress(false), Ok(CaptureRetirementProgress::Idle));
    }
    #[test]
    fn viewport_retirement_complete_and_retire_jobs_keep_exact_origin_and_purpose() {
        let (owner, worker) = handoff();
        let mut tracker = Retirement::default();
        let origin = token();
        tracker.request(origin).unwrap();
        let fences = crate::session_fence::Requests::default();
        tracker.try_submit(&owner, &fences, Purpose::Complete).unwrap();
        let previous = complete(&worker, false);
        assert_eq!(previous.purpose(), Purpose::Complete);
        tracker.dirty();
        receive(&mut tracker, &owner);
        assert_eq!(tracker.progress(false), Ok(CaptureRetirementProgress::Pending));
        tracker.try_submit(&owner, &fences, Purpose::Retire).unwrap();
        let retiring = complete(&worker, false);
        assert_eq!(retiring.purpose(), Purpose::Retire);
        assert_eq!(retiring.origin(), origin);
        receive(&mut tracker, &owner);
        assert_eq!(tracker.progress(false), Ok(CaptureRetirementProgress::Idle));
    }
    #[test]
    fn viewport_retirement_busy_owner_consumes_probe_without_losing_origin_or_reusing_serial() {
        let (owner, worker) = handoff();
        owner.try_submit(Job::Pick { x: 0.0, y: 0.0 }).unwrap().unwrap();
        let mut tracker = Retirement::default();
        tracker.request(token()).unwrap();
        tracker
            .try_submit(&owner, &crate::session_fence::Requests::default(), Purpose::Retire)
            .unwrap();
        let rejected = tracker.serial;
        assert!(tracker.flight.is_none());
        assert!(matches!(worker.next().unwrap().0, Job::Pick { .. }));
        worker.complete(Done::Pick(None));
        assert!(matches!(
            tracker
                .try_submit(&owner, &crate::session_fence::Requests::default(), Purpose::Retire)
                .unwrap(),
            Some(Done::Pick(None))
        ));
        let accepted = complete(&worker, false);
        assert!(accepted.serial > rejected);
        receive(&mut tracker, &owner);
        assert_eq!(tracker.progress(false), Ok(CaptureRetirementProgress::Idle));
    }
    #[test]
    fn viewport_retirement_session_request_wins_actual_owner_without_creating_a_proof() {
        let (owner, worker) = handoff();
        let mut fences = crate::session_fence::Requests::default();
        fences.request(9).unwrap();
        owner.try_submit(Job::SessionFence(9)).unwrap().unwrap();
        let mut tracker = Retirement::default();
        tracker.request(token()).unwrap();
        assert!(tracker.try_submit(&owner, &fences, Purpose::Retire).unwrap().is_none());
        assert_eq!(tracker.serial, 0, "pending fence precedes probe allocation");
        assert!(matches!(worker.next().unwrap().0, Job::SessionFence(9)));
        worker.complete(Done::SessionFence(crate::session_fence::Reply::Pending(9)));
        assert!(fences.cancel(9));
        assert!(matches!(
            tracker.try_submit(&owner, &fences, Purpose::Retire).unwrap(),
            Some(Done::SessionFence(crate::session_fence::Reply::Pending(9)))
        ));
        complete(&worker, false);
        receive(&mut tracker, &owner);
        assert_eq!(tracker.progress(false), Ok(CaptureRetirementProgress::Idle));
        assert!(fences.take(9).is_err(), "maintenance never makes a session completion");
    }
    #[test]
    fn viewport_retirement_exhaustion_and_owner_loss_never_report_idle() {
        for revision in [false, true] {
            let mut tracker = Retirement::default();
            tracker.request(token()).unwrap();
            if revision {
                tracker.revision = u64::MAX;
                tracker.dirty();
            } else {
                tracker.serial = u64::MAX;
                assert_eq!(tracker.probe(Purpose::Retire), Err(CaptureError::Exhausted));
            }
            assert_eq!(tracker.progress(false), Err(CaptureError::Exhausted));
        }
        let (owner, worker) = handoff();
        drop(worker);
        let mut tracker = Retirement::default();
        tracker.request(token()).unwrap();
        assert!(matches!(
            tracker.try_submit(&owner, &crate::session_fence::Requests::default(), Purpose::Retire),
            Err(CaptureError::DeviceLost)
        ));
        assert_eq!(tracker.progress(false), Ok(CaptureRetirementProgress::Pending));
    }
}
