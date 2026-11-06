//! Explicit session handoff fences. A request reaches the backend through the
//! same ordered rendezvous as frame packets. Its callback covers GPU work
//! submitted before that job, independently of ordinary frame pacing.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Capture after applying the native viewport/renderer size and before handing
/// over a prepared frame. Frame proof uses packet index + 1 against after_frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionBoundary {
    pub surface_epoch: u64,
    pub after_frame: u64,
}

/// The previous GPU submissions have completed and presentation was issued for
/// `frame` on this surface epoch. This does not acknowledge compositor scanout.
/// `frame` is a zero-based packet index, absent before any surface presentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionCompletion {
    pub ticket: u64,
    pub frame: Option<u64>,
    pub surface_epoch: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionFenceError {
    /// Inline rendering and XR do not provide this session-host contract.
    Unsupported,
    /// A different ticket still owns the one outstanding fence.
    Busy,
    /// Zero, unknown or already consumed tickets cannot be replayed.
    StaleTicket,
    RendererFailed,
}

impl std::fmt::Display for SessionFenceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Unsupported => "session completion fences are unsupported by this renderer mode",
            Self::Busy => "another session completion fence is pending",
            Self::StaleTicket => "session completion fence ticket is stale or unknown",
            Self::RendererFailed => "renderer failed before session completion",
        })
    }
}

impl std::error::Error for SessionFenceError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reply {
    Pending(u64),
    Complete(SessionCompletion),
    Failed(u64, SessionFenceError),
}

impl Reply {
    fn ticket(self) -> u64 {
        match self {
            Self::Pending(ticket) | Self::Failed(ticket, _) => ticket,
            Self::Complete(completion) => completion.ticket,
        }
    }
}

struct Request {
    ticket: u64,
    handed: bool,
    answer: Option<Result<SessionCompletion, SessionFenceError>>,
}

/// The owner retries only once its preceding poll job has answered. No command
/// queue grows while the render thread or the GPU is busy.
#[derive(Default)]
pub(crate) struct Requests {
    last_ticket: u64,
    active: Option<Request>,
}

impl Requests {
    pub fn request(&mut self, ticket: u64) -> Result<(), SessionFenceError> {
        if let Some(active) = &self.active {
            return if active.ticket == ticket {
                Ok(())
            } else {
                Err(SessionFenceError::Busy)
            };
        }
        if ticket == 0 || ticket <= self.last_ticket {
            return Err(SessionFenceError::StaleTicket);
        }
        self.last_ticket = ticket;
        self.active = Some(Request {
            ticket,
            handed: false,
            answer: None,
        });
        Ok(())
    }

    pub fn check(&self, ticket: u64) -> Result<(), SessionFenceError> {
        if self.active.as_ref().is_some_and(|active| active.ticket == ticket) {
            Ok(())
        } else {
            Err(SessionFenceError::StaleTicket)
        }
    }

    pub fn pending(&self) -> bool {
        self.active.is_some()
    }

    pub fn cancel(&mut self, ticket: u64) -> bool {
        if self.check(ticket).is_err() {
            return false;
        }
        self.active = None;
        true
    }

    pub fn next_job(&self) -> Option<u64> {
        self.active
            .as_ref()
            .filter(|active| !active.handed && active.answer.is_none())
            .map(|active| active.ticket)
    }

    pub fn handed(&mut self, ticket: u64) {
        if let Some(active) = &mut self.active
            && active.ticket == ticket
        {
            active.handed = true;
        }
    }

    pub fn receive(&mut self, reply: Reply) {
        let Some(active) = &mut self.active else {
            return;
        };
        if active.ticket != reply.ticket() || !active.handed || active.answer.is_some() {
            return;
        }
        active.handed = false;
        active.answer = match reply {
            Reply::Pending(_) => None,
            Reply::Complete(completion) => Some(Ok(completion)),
            Reply::Failed(_, error) => Some(Err(error)),
        };
    }

    pub fn take(&mut self, ticket: u64) -> Result<Option<SessionCompletion>, SessionFenceError> {
        self.check(ticket)?;
        let answer = self.active.as_mut().and_then(|active| active.answer.take());
        match answer {
            Some(answer) => {
                self.active = None;
                answer.map(Some)
            }
            None => Ok(None),
        }
    }
}

struct GpuRequest {
    proof: SessionCompletion,
    completed: Arc<AtomicBool>,
}

/// Backend-only state. Each registered callback owns a separate flag and can
/// never stamp a later ticket, frame, or surface epoch with an older completion.
#[derive(Default)]
pub(crate) struct GpuFences {
    last_ticket: u64,
    active: Option<GpuRequest>,
}

impl GpuFences {
    /// Some(flag) registers exactly one callback, after all previous packet
    /// submissions. Further jobs with this ticket only poll that callback.
    pub fn begin(&mut self, proof: SessionCompletion) -> Result<Option<Arc<AtomicBool>>, SessionFenceError> {
        if let Some(active) = &self.active {
            if active.proof.ticket == proof.ticket {
                return Ok(None);
            }
            if proof.ticket < active.proof.ticket {
                return Err(SessionFenceError::StaleTicket);
            }
            // The owner cancelled its old request. Keep at most one registered
            // callback while that GPU work drains; the newer request retries.
            if !active.completed.load(Ordering::Acquire) {
                return Ok(None);
            }
            self.active = None;
        }
        if proof.ticket == 0 || proof.ticket <= self.last_ticket {
            return Err(SessionFenceError::StaleTicket);
        }
        if proof.surface_epoch == 0 {
            return Err(SessionFenceError::RendererFailed);
        }
        self.last_ticket = proof.ticket;
        let completed = Arc::new(AtomicBool::new(false));
        self.active = Some(GpuRequest {
            proof,
            completed: completed.clone(),
        });
        Ok(Some(completed))
    }

    pub fn poll(&mut self, ticket: u64) -> Reply {
        let Some(active) = &self.active else {
            return Reply::Failed(ticket, SessionFenceError::StaleTicket);
        };
        if active.proof.ticket < ticket {
            return Reply::Pending(ticket);
        }
        if active.proof.ticket > ticket {
            return Reply::Failed(ticket, SessionFenceError::StaleTicket);
        }
        if !active.completed.load(Ordering::Acquire) {
            return Reply::Pending(ticket);
        }
        let proof = active.proof;
        self.active = None;
        Reply::Complete(proof)
    }
}

/// Exhaustion fails closed for session proofs without changing standalone
/// surface behavior. Zero stays invalid until this renderer is recreated.
pub(crate) fn next_surface_epoch(epoch: u64) -> u64 {
    if epoch == 0 { 0 } else { epoch.checked_add(1).unwrap_or(0) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proof(ticket: u64, frame: Option<u64>, surface_epoch: u64) -> SessionCompletion {
        SessionCompletion {
            ticket,
            frame,
            surface_epoch,
        }
    }

    #[test]
    fn owner_keeps_one_ordered_request_and_consumes_its_answer_once() {
        let mut requests = Requests::default();
        assert_eq!(requests.request(0), Err(SessionFenceError::StaleTicket));
        requests.request(1).expect("new request");
        requests.request(1).expect("same pending request");
        assert_eq!(requests.request(2), Err(SessionFenceError::Busy));
        assert_eq!(requests.next_job(), Some(1));
        requests.handed(1);
        assert_eq!(requests.next_job(), None);
        requests.receive(Reply::Pending(2));
        assert_eq!(requests.next_job(), None, "unrelated result cannot advance this request");
        requests.receive(Reply::Pending(1));
        assert_eq!(requests.next_job(), Some(1));
        requests.handed(1);
        let completed = proof(1, Some(9), 3);
        requests.receive(Reply::Complete(completed));
        requests.receive(Reply::Failed(1, SessionFenceError::RendererFailed));
        assert_eq!(requests.take(1), Ok(Some(completed)));
        assert_eq!(requests.take(1), Err(SessionFenceError::StaleTicket));
        assert_eq!(requests.request(1), Err(SessionFenceError::StaleTicket));
        requests.request(2).expect("later request");
        requests.handed(2);
        requests.receive(Reply::Complete(completed));
        assert_eq!(requests.take(2), Ok(None), "old result cannot relabel a later request");
    }

    #[test]
    fn a_gpu_callback_proves_only_its_frozen_frame_and_surface() {
        let mut fences = GpuFences::default();
        let first = proof(1, Some(11), 7);
        let callback = fences.begin(first).expect("first fence").expect("register callback");
        assert!(fences.begin(proof(1, Some(99), 8)).expect("same pending fence").is_none());
        assert_eq!(fences.poll(1), Reply::Pending(1));
        assert!(fences.begin(proof(2, Some(99), 8)).expect("cancelled callback drains").is_none());
        assert_eq!(fences.poll(2), Reply::Pending(2));
        callback.store(true, Ordering::Release);
        assert_eq!(fences.poll(1), Reply::Complete(first));
        assert_eq!(fences.poll(1), Reply::Failed(1, SessionFenceError::StaleTicket));
        let second = proof(2, Some(13), 8);
        let newer_callback = fences.begin(second).expect("second fence").expect("new callback");
        callback.store(true, Ordering::Release);
        assert_eq!(fences.poll(2), Reply::Pending(2), "an old callback cannot complete a new ticket");
        newer_callback.store(true, Ordering::Release);
        assert_eq!(fences.poll(2), Reply::Complete(second));
        assert!(matches!(fences.begin(first), Err(SessionFenceError::StaleTicket)));
    }

    #[test]
    fn no_surface_presentation_is_not_a_prepared_frame() {
        let mut fences = GpuFences::default();
        let callback = fences
            .begin(proof(1, None, 1))
            .expect("quiescence before first frame")
            .expect("callback");
        callback.store(true, Ordering::Release);
        assert_eq!(fences.poll(1), Reply::Complete(proof(1, None, 1)));
        assert_eq!(next_surface_epoch(1), 2);
        assert_eq!(next_surface_epoch(u64::MAX), 0);
        assert_eq!(next_surface_epoch(0), 0);
        assert!(matches!(fences.begin(proof(2, Some(0), 0)), Err(SessionFenceError::RendererFailed)));
    }

    #[test]
    fn cancelled_owner_and_gpu_requests_cannot_relabel_the_next_request() {
        let mut requests = Requests::default();
        requests.request(5).expect("request");
        requests.handed(5);
        assert!(!requests.cancel(4));
        assert!(requests.cancel(5));
        assert!(!requests.pending());
        assert_eq!(requests.request(5), Err(SessionFenceError::StaleTicket));
        requests.request(6).expect("later request");
        requests.handed(6);
        requests.receive(Reply::Complete(proof(5, Some(8), 2)));
        assert_eq!(requests.take(6), Ok(None));

        let mut fences = GpuFences::default();
        let old = fences.begin(proof(5, Some(8), 2)).expect("old").expect("old callback");
        assert!(fences.begin(proof(6, Some(9), 3)).expect("wait for old callback").is_none());
        assert_eq!(fences.poll(6), Reply::Pending(6));
        old.store(true, Ordering::Release);
        let current = proof(6, Some(10), 3);
        let callback = fences.begin(current).expect("retire cancelled callback").expect("new callback");
        assert_eq!(fences.poll(6), Reply::Pending(6));
        old.store(true, Ordering::Release);
        assert_eq!(fences.poll(6), Reply::Pending(6));
        callback.store(true, Ordering::Release);
        assert_eq!(fences.poll(6), Reply::Complete(current));
    }
}
