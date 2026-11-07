//! Fixed Character1–5 correlation, from characterQuickAction/openCharacter
//! (Legacy Viewer fssessionlifecycle.h / tools/session-host/main.cpp, abbe3cb0).
//! These facts/tickets are DATA, not native window or control permission. The
//! actual actor/native consumer must still revalidate current owned authority.
use super::slots::SlotId;
use aurora_session::{Binding, HostId, InstallationId, MAX_WORKERS, Phase};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Choice {
    Unavailable,
    Opening,
    OpenLogin,
    Activate,
    ShowViewer,
}
impl Choice {
    fn actionable(self) -> bool {
        matches!(self, Self::OpenLogin | Self::Activate | Self::ShowViewer)
    }
}

/// Nonreused actual App/window observation carried in authenticated metadata.
/// Nonzero data alone is NEVER evidence that a Window is alive or owned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct WindowTag {
    pub app: u64,
    pub window: u64,
}
impl WindowTag {
    fn valid(self) -> bool {
        self.app != 0 && self.window != 0
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Occupant {
    pub binding: Binding,
    pub lease_epoch: u64,
    pub phase: Phase,
    pub window: Option<WindowTag>,
    pub current: bool,
    pub managed: bool,
    pub ordinary_window: bool,
    pub blocked: bool,
}
/// Only actual actor ownership may supply these observations at each REAL
/// lifecycle transition. Native receipts, leases and OS checks stay external.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Facts {
    pub available: bool,
    pub reserved: bool,
    pub occupant: Option<Occupant>,
}
fn choice(facts: Facts, busy: bool) -> Choice {
    if busy {
        return Choice::Unavailable;
    }
    let Some(occupant) = facts.occupant else {
        return if facts.reserved {
            Choice::Opening
        } else if facts.available {
            Choice::OpenLogin
        } else {
            Choice::Unavailable
        };
    };
    if facts.available || facts.reserved || !occupant.current || !occupant.managed || occupant.blocked {
        return Choice::Unavailable;
    }
    if occupant.phase == Phase::Online {
        return if occupant.binding.session.is_some() && occupant.window.is_some() {
            Choice::Activate
        } else {
            Choice::Unavailable
        };
    }
    if !occupant.ordinary_window {
        return Choice::Unavailable;
    }
    if occupant.window.is_some() {
        Choice::ShowViewer
    } else {
        Choice::Opening
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Error {
    Invalid,
    Exhausted,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Ticket {
    instance: u64,
    slot: SlotId,
    incarnation: u64,
    action: Choice,
    expected: Facts,
}
impl Ticket {
    pub fn slot(self) -> SlotId {
        self.slot
    }
    pub fn action(self) -> Choice {
        self.action
    }
    /// Expected metadata only; actual consumer must check fresh real proof.
    pub fn expected(self) -> Facts {
        self.expected
    }
}
#[derive(Clone, Copy, Default)]
struct State {
    facts: Facts,
    incarnation: u64,
    observed: bool,
}
/// No clock, IO or native effects. Every actual busy/eligibility transition
/// must call observe/set_busy/boundary before any publication or dispatch.
pub(super) struct Tracker {
    installation: InstallationId,
    host: HostId,
    instance: u64,
    serial: u64,
    busy: bool,
    exhausted: bool,
    states: [State; MAX_WORKERS],
}
impl Tracker {
    pub fn new(installation: InstallationId, host: HostId) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let instance = NEXT
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |value| value.checked_add(1))
            .unwrap_or(0);
        Self {
            installation,
            host,
            instance,
            serial: 0,
            busy: false,
            exhausted: instance == 0,
            states: [State::default(); MAX_WORKERS],
        }
    }
    fn index(slot: SlotId) -> usize {
        usize::from(u8::from(slot))
    }
    fn next(&mut self) -> Result<u64, Error> {
        if self.exhausted {
            return Err(Error::Exhausted);
        }
        let Some(next) = self.serial.checked_add(1) else {
            self.exhausted = true;
            self.states = [State::default(); MAX_WORKERS];
            return Err(Error::Exhausted);
        };
        self.serial = next;
        Ok(next)
    }
    fn valid(&self, facts: Facts) -> bool {
        facts.occupant.is_none_or(|occupant| {
            occupant.binding.worker.installation == self.installation
                && occupant.binding.worker.host == self.host
                && occupant.binding.validate().is_ok()
                && occupant.lease_epoch != 0
                && occupant.window.is_none_or(WindowTag::valid)
        })
    }
    pub fn observe(&mut self, slot: SlotId, facts: Facts) -> Result<bool, Error> {
        if self.exhausted {
            return Err(Error::Exhausted);
        }
        let at = Self::index(slot);
        if !self.valid(facts) {
            let incarnation = self.next()?;
            self.states[at] = State {
                facts: Facts::default(),
                incarnation,
                observed: false,
            };
            return Err(Error::Invalid);
        }
        if self.states[at].observed && self.states[at].facts == facts {
            return Ok(false);
        }
        let incarnation = self.next()?;
        self.states[at] = State {
            facts,
            incarnation,
            observed: true,
        };
        Ok(true)
    }
    /// Include unpublished A→busy→A / mode transitions. Never derive this only
    /// from the latest painted/roster snapshot, even if it looks unchanged.
    pub fn boundary(&mut self) -> Result<(), Error> {
        for at in 0..MAX_WORKERS {
            let incarnation = self.next()?;
            self.states[at].incarnation = incarnation;
        }
        Ok(())
    }
    pub fn set_busy(&mut self, busy: bool) -> Result<bool, Error> {
        if self.exhausted {
            return Err(Error::Exhausted);
        }
        if self.busy == busy {
            return Ok(false);
        }
        self.boundary()?;
        self.busy = busy;
        Ok(true)
    }
    pub fn choice(&self, slot: SlotId) -> Choice {
        let state = &self.states[Self::index(slot)];
        if self.exhausted || !state.observed {
            Choice::Unavailable
        } else {
            choice(state.facts, self.busy)
        }
    }
    pub fn capture(&self, slot: SlotId) -> Option<Ticket> {
        let action = self.choice(slot);
        action.actionable().then(|| Ticket {
            instance: self.instance,
            slot,
            incarnation: self.states[Self::index(slot)].incarnation,
            action,
            expected: self.states[Self::index(slot)].facts,
        })
    }
    pub fn current(&self, ticket: Ticket) -> bool {
        self.capture(ticket.slot) == Some(ticket)
    }
}

#[cfg(test)]
mod tests;
