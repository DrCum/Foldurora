//! Exact request/circuit ownership for TeleportCancel (message template 72).
//! The simulator packet has no request UUID; the native ledger supplies that
//! correlation and refuses cancellation after arrival or a replacement request.
use std::net::SocketAddr;
use uuid::Uuid;
#[derive(Default)]
pub(super) struct Ledger {
    pending: Option<Pending>,
}
struct Pending {
    request: Uuid,
    circuit: (SocketAddr, Uuid),
    started: bool,
    allowed: bool,
}
impl Ledger {
    pub fn begin(&mut self, request: Option<Uuid>, circuit: Option<(SocketAddr, Uuid)>) {
        self.pending = request.filter(|id| !id.is_nil()).zip(circuit).map(|(request, circuit)| Pending {
            request,
            circuit,
            started: false,
            allowed: true,
        });
    }
    pub fn started(&mut self, circuit: (SocketAddr, Uuid), flags: u32) {
        let Some(p) = self.pending.as_mut().filter(|p| p.circuit == circuit) else {
            return;
        };
        if p.started {
            self.pending = None;
            return;
        }
        p.started = true;
        // TELEPORT_FLAGS_DISABLE_CANCEL: a server-owned forced transition may
        // not acquire cancellation authority from an earlier viewer request.
        // llteleportflags.h, Folderstorm abbe3cb0: disable-cancel is bit 11.
        p.allowed = flags & ((1 << 11) | (1 << 15)) == 0;
    }
    pub fn retire(&mut self) {
        self.pending = None;
    }
    pub fn cancel(&mut self, request: Uuid, circuit: Option<(SocketAddr, Uuid)>) -> Option<SocketAddr> {
        if !self
            .pending
            .as_ref()
            .is_some_and(|p| p.request == request && Some(p.circuit) == circuit && p.allowed)
        {
            return None;
        }
        self.pending.take().map(|p| p.circuit.0)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancel_requires_exact_request_and_same_circuit_incarnation_once() {
        let circuit = ("127.0.0.1:9000".parse().unwrap(), Uuid::new_v4());
        let request = Uuid::new_v4();
        let mut ledger = Ledger::default();
        ledger.begin(Some(request), Some(circuit));
        assert!(ledger.cancel(Uuid::new_v4(), Some(circuit)).is_none());
        assert!(ledger.cancel(request, Some((circuit.0, Uuid::new_v4()))).is_none());
        assert_eq!(ledger.cancel(request, Some(circuit)), Some(circuit.0));
        assert!(ledger.cancel(request, Some(circuit)).is_none());
        ledger.begin(Some(request), Some(circuit));
        ledger.begin(None, Some(circuit));
        assert!(ledger.cancel(request, Some(circuit)).is_none());
        ledger.begin(Some(request), Some(circuit));
        ledger.started(circuit, 1 << 11);
        assert!(ledger.cancel(request, Some(circuit)).is_none());
        ledger.begin(Some(request), Some(circuit));
        ledger.retire();
        assert!(ledger.cancel(request, Some(circuit)).is_none());
    }
}
