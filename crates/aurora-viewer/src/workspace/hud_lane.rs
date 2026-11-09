//! One reviewed, additive native transaction at a time. No World/IO authority.
use super::model::{HudRef, HudSet};
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Progress {
    #[default]
    Idle,
    Review,
    Running,
    Complete,
    Unavailable,
    Declined,
    Interrupted,
    TimedOut,
    Failed,
}
#[derive(Clone)]
pub(crate) struct Review {
    pub nonce: Uuid,
    pub wanted: HudSet,
}
struct Run {
    queued: VecDeque<Vec<HudRef>>,
    flight: Option<Vec<HudRef>>,
    deadline: Instant,
}
pub(crate) struct Lane<S> {
    scope: Option<S>,
    pub review: Option<Review>,
    run: Option<Run>,
    pub progress: Progress,
}
impl<S> Default for Lane<S> {
    fn default() -> Self {
        Self {
            scope: None,
            review: None,
            run: None,
            progress: Progress::Idle,
        }
    }
}
impl<S: Eq> Lane<S> {
    pub(crate) fn bind(&mut self, scope: Option<S>) {
        if self.scope != scope {
            self.retire();
            self.scope = scope;
        }
    }
    pub(crate) fn retire(&mut self) {
        let pending = self.review.take().is_some() | self.run.take().is_some();
        if pending || self.progress == Progress::Interrupted {
            // An already accepted native transaction cannot be undone here.
            self.progress = Progress::Interrupted;
        } else {
            self.progress = Progress::Idle;
        }
        self.review = None;
        self.run = None;
        self.scope = None;
    }
    pub(crate) fn offer(&mut self, wanted: HudSet) {
        self.review = None;
        self.run = None;
        if self.scope.is_none() {
            self.progress = Progress::Unavailable;
        } else if wanted.is_empty() {
            self.progress = Progress::Complete;
        } else {
            self.review = Some(Review {
                nonce: Uuid::new_v4(),
                wanted,
            });
            self.progress = Progress::Review;
        }
    }
    pub(crate) fn refuse(&mut self) {
        self.review = None;
        self.run = None;
        self.progress = Progress::Unavailable;
    }
    /// Consume the exact review once. A later OK or replay cannot authorize it.
    pub(crate) fn decide(&mut self, nonce: Uuid, accepted: bool, now: Instant) -> bool {
        if self.scope.is_none() || self.review.as_ref().is_none_or(|r| r.nonce != nonce) {
            return false;
        }
        let review = self.review.take().expect("matched HUD review");
        if !accepted {
            self.progress = Progress::Declined;
            return true;
        }
        let mut points = std::collections::BTreeMap::<u8, Vec<HudRef>>::new();
        for reference in review.wanted.refs() {
            points.entry(reference.point).or_default().push(*reference);
        }
        self.run = Some(Run {
            queued: points.into_values().collect(),
            flight: None,
            deadline: now + Duration::from_secs(30),
        });
        self.progress = Progress::Running;
        true
    }
    pub(crate) fn busy(&self) -> bool {
        self.run.is_some()
    }
    /// Actual worn observations are the only completion proof. Never resend an
    /// accepted batch, and never start another while any native request exists.
    pub(crate) fn next(&mut self, now: Instant, native_busy: bool, worn: impl Fn(HudRef) -> bool) -> Option<Vec<HudRef>> {
        let run = self.run.as_mut()?;
        if now >= run.deadline {
            self.run = None;
            self.progress = Progress::TimedOut;
            return None;
        }
        if let Some(flight) = &run.flight {
            if !flight.iter().all(|r| worn(*r)) {
                return None;
            }
            run.flight = None;
        }
        if native_busy {
            return None;
        }
        while let Some(batch) = run.queued.pop_front() {
            let missing: Vec<_> = batch.into_iter().filter(|r| !worn(*r)).collect();
            if !missing.is_empty() {
                run.flight = Some(missing.clone());
                return Some(missing);
            }
        }
        self.run = None;
        self.progress = Progress::Complete;
        None
    }
    pub(crate) fn dispatch_failed(&mut self) {
        self.run = None;
        self.progress = Progress::Failed;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn set() -> HudSet {
        HudSet::try_from(vec![
            HudRef {
                item: Uuid::from_u128(1),
                point: 31,
            },
            HudRef {
                item: Uuid::from_u128(2),
                point: 32,
            },
        ])
        .unwrap()
    }
    fn approved(now: Instant) -> Lane<u64> {
        let mut lane = Lane::default();
        lane.bind(Some(1));
        lane.offer(set());
        assert!(lane.decide(lane.review.as_ref().unwrap().nonce, true, now));
        lane
    }
    #[test]
    fn workspace_hud_review_is_one_shot_and_scope_changes_retire_approval() {
        let now = Instant::now();
        let mut lane = Lane::default();
        lane.bind(Some(1));
        lane.offer(set());
        let nonce = lane.review.as_ref().unwrap().nonce;
        assert!(!lane.decide(Uuid::new_v4(), true, now));
        assert!(lane.decide(nonce, false, now));
        assert!(!lane.decide(nonce, true, now));
        lane.offer(set());
        let nonce = lane.review.as_ref().unwrap().nonce;
        lane.bind(Some(2));
        assert!(!lane.decide(nonce, true, now));
        assert!(!lane.busy());
        lane.offer(set());
        assert_eq!(lane.progress, Progress::Review);
        assert!(lane.decide(lane.review.as_ref().unwrap().nonce, true, now));
        lane.retire();
        lane.bind(Some(2));
        assert!(lane.next(now, false, |_| false).is_none());
        assert_eq!(lane.progress, Progress::Interrupted);
    }
    #[test]
    fn workspace_hud_lane_serializes_native_requests_and_requires_observed_wear() {
        let now = Instant::now();
        let mut lane = approved(now);
        assert!(lane.next(now, true, |_| false).is_none());
        let first = lane.next(now, false, |_| false).unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].point, 31);
        assert!(lane.next(now, false, |_| false).is_none());
        assert!(lane.next(now, true, |r| r.point == 31).is_none());
        let second = lane.next(now, false, |r| r.point == 31).unwrap();
        assert_eq!(second[0].point, 32);
        assert!(lane.next(now, false, |_| false).is_none());
        assert_eq!(lane.progress, Progress::Running);
        assert!(lane.next(now, false, |_| true).is_none());
        assert_eq!(lane.progress, Progress::Complete);
    }
    #[test]
    fn workspace_hud_lane_has_one_deadline_and_does_not_retry_failed_dispatch() {
        let now = Instant::now();
        let mut lane = approved(now);
        assert!(lane.next(now + Duration::from_secs(29), false, |_| false).is_some());
        assert!(lane.next(now + Duration::from_secs(30), false, |_| false).is_none());
        assert_eq!(lane.progress, Progress::TimedOut);
        let mut lane = approved(now);
        assert!(lane.next(now, false, |_| false).is_some());
        lane.dispatch_failed();
        assert!(lane.next(now, false, |_| false).is_none());
        assert_eq!(lane.progress, Progress::Failed);
    }
}
