//! Future IO-owner admission bookkeeping only; no listener/endpoint is enabled.
use super::{Error, Scope};
use aurora_session::preview_wire::Lane;
use aurora_session::{HostId, InstallationId, MAX_WORKERS};

#[derive(Clone, Copy)]
struct Slot {
    scope: Scope,
    connected: bool,
}

pub(crate) struct Admissions {
    installation: InstallationId,
    host: HostId,
    slots: [Option<Slot>; 2 * MAX_WORKERS],
}
impl Admissions {
    pub fn new(installation: InstallationId, host: HostId) -> Self {
        Self {
            installation,
            host,
            slots: [None; 2 * MAX_WORKERS],
        }
    }
    pub fn begin(&mut self, scope: Scope) -> Result<usize, Error> {
        let worker = scope.source.worker();
        if scope.connection == 0 || worker.installation != self.installation || worker.host != self.host {
            return Err(Error::Binding);
        }
        for slot in self.slots.iter().flatten().filter(|slot| slot.scope.source.worker() == worker) {
            if slot.scope.lane == scope.lane {
                return Err(Error::Busy);
            }
            if slot.scope.source != scope.source || slot.scope.connection != scope.connection {
                return Err(Error::Binding);
            }
        }
        let unique = self
            .slots
            .iter()
            .flatten()
            .map(|slot| slot.scope.source.worker().worker)
            .collect::<std::collections::BTreeSet<_>>();
        if !unique.contains(&worker.worker) && unique.len() == MAX_WORKERS {
            return Err(Error::Capacity);
        }
        let index = self.slots.iter().position(Option::is_none).ok_or(Error::Capacity)?;
        self.slots[index] = Some(Slot { scope, connected: false });
        Ok(index)
    }
    pub fn finish(&mut self, index: usize, scope: Scope) -> Result<(), Error> {
        if scope.lane == Lane::Pixels
            && !self.slots.iter().flatten().any(|slot| {
                slot.connected
                    && slot.scope.lane == Lane::Metadata
                    && slot.scope.source == scope.source
                    && slot.scope.connection == scope.connection
            })
        {
            return Err(Error::Authentication);
        }
        let slot = self
            .slots
            .get_mut(index)
            .and_then(Option::as_mut)
            .filter(|slot| slot.scope == scope)
            .ok_or(Error::Binding)?;
        if slot.connected {
            return Err(Error::Busy);
        }
        slot.connected = true;
        Ok(())
    }
    pub fn release(&mut self, index: usize, scope: Scope) {
        if self.slots.get(index).and_then(|slot| *slot).is_none_or(|slot| slot.scope != scope) {
            return;
        }
        self.slots[index] = None;
        if scope.lane == Lane::Metadata {
            for slot in &mut self.slots {
                if slot.is_some_and(|slot| slot.scope.source == scope.source && slot.scope.connection == scope.connection) {
                    *slot = None;
                }
            }
        }
    }
}

#[derive(Default)]
pub(crate) struct RoundRobin {
    next: usize,
}
impl RoundRobin {
    pub fn order(&mut self, count: usize) -> Result<Vec<usize>, Error> {
        if count == 0 || count > MAX_WORKERS {
            return Err(Error::Capacity);
        }
        let order = (0..count).map(|offset| (self.next + offset) % count).collect();
        self.next = (self.next + 1) % count;
        Ok(order)
    }
}
