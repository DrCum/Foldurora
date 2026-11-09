//! A native file dialog intentionally changes focus/UI revision. It cannot
//! preserve ordinary action tickets. Keep a separate explicit intent, still
//! requiring fresh native authority before each effect; no delayed replay.
use crate::workspace::{
    controller::{Controller, Source},
    model::{Arrangement, Background},
};
use std::time::{Duration, Instant};
use uuid::Uuid;

pub(crate) type Target = Option<(Arrangement, usize)>;
pub(crate) struct Intent {
    instance: Uuid,
    binding: u64,
    source: Source,
    nonce: u64,
    background: Background,
    pub target: Target,
    deadline: Option<Instant>,
}
impl Intent {
    /// Caller has just consumed an actual source-bound UI action and checked
    /// native input authority. The nonce is retired by every other workspace
    /// action; this type does not turn a matching identity into native permission.
    pub(crate) fn new(c: &Controller, nonce: u64, target: Target) -> Option<Self> {
        if nonce == 0 || !c.ready() || c.preview.is_some() {
            return None;
        }
        Some(Self {
            instance: c.instance,
            binding: c.binding,
            source: c.source.clone()?,
            nonce,
            background: c.background(),
            target,
            deadline: None,
        })
    }
    pub(crate) fn normalized(&mut self, now: Instant) {
        // One absolute post-dialog deadline, never extended by another frame.
        if self.deadline.is_none() {
            self.deadline = Some(now + Duration::from_secs(2));
        }
    }
    /// Called only after a fresh native gate submits the private publication.
    /// IO duration is not a focus-restoration wait; its receipt starts a new,
    /// single bounded acceptance stage rather than replaying the original UI.
    pub(crate) fn publishing(&mut self) {
        self.deadline = None;
    }
    pub(crate) fn published(&mut self, now: Instant) {
        self.deadline = Some(now + Duration::from_secs(2));
    }
    pub(crate) fn expired(&self, now: Instant) -> bool {
        self.deadline.is_some_and(|t| now >= t)
    }
    pub(crate) fn matches(&self, c: &Controller, actual: Option<&Source>, nonce: u64, now: Instant) -> bool {
        !self.expired(now)
            && self.nonce == nonce
            && self.instance == c.instance
            && self.binding == c.binding
            && actual == Some(&self.source)
            && c.source.as_ref() == Some(&self.source)
            && c.preview.is_none()
            && !c.exhausted
            && c.background() == self.background
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::{controller::Status, model::Catalogue};
    #[test]
    fn workspace_local_import_intent_survives_only_expected_revision_change_and_has_one_post_dialog_deadline() {
        let s = Source {
            agent: Uuid::new_v4(),
            login: Uuid::new_v4(),
            grid: "a".repeat(64),
            hosted: None,
        };
        let mut c = Controller {
            source: Some(s.clone()),
            permit: true,
            status: Status::Ready,
            catalogue: Some(Catalogue::empty(s.agent, s.grid.clone())),
            ..Default::default()
        };
        let now = Instant::now();
        let mut intent = Intent::new(&c, 7, None).unwrap();
        c.permit = false;
        c.advance();
        c.permit = true;
        c.advance();
        assert!(
            intent.matches(&c, Some(&s), 7, now),
            "native authority is still a separate required App gate"
        );
        assert!(!intent.matches(&c, Some(&s), 8, now), "another workspace action retires its nonce");
        intent.normalized(now);
        intent.normalized(now + Duration::from_secs(1));
        assert!(!intent.expired(now + Duration::from_millis(1999)));
        assert!(intent.expired(now + Duration::from_secs(2)));
        c.catalogue.as_mut().unwrap().background = Background::Solid { rgb: [1, 2, 3] };
        assert!(!intent.matches(&c, Some(&s), 7, now));
    }
    #[test]
    fn workspace_local_import_intent_refuses_same_identity_logical_rebind_and_a_replaced_controller() {
        let s = Source {
            agent: Uuid::new_v4(),
            login: Uuid::new_v4(),
            grid: "a".repeat(64),
            hosted: None,
        };
        let mut c = Controller {
            source: Some(s.clone()),
            permit: true,
            status: Status::Ready,
            catalogue: Some(Catalogue::empty(s.agent, s.grid.clone())),
            ..Default::default()
        };
        let now = Instant::now();
        let intent = Intent::new(&c, 1, None).unwrap();
        c.instance = Uuid::new_v4();
        assert!(!intent.matches(&c, Some(&s), 1, now));
        let mut c = Controller {
            source: Some(s.clone()),
            permit: true,
            status: Status::Ready,
            catalogue: Some(Catalogue::empty(s.agent, s.grid.clone())),
            ..Default::default()
        };
        let intent = Intent::new(&c, 1, None).unwrap();
        c.bind(Some(s.clone()));
        assert!(!intent.matches(&c, Some(&s), 1, now));
        assert!(!intent.matches(&c, None, 1, now));
    }
}
