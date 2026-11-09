//! Lead-owned integration seam; intentionally not registered in mod.rs yet.
//! Add the aurora-mcp path dependency and register this module when connecting
//! authoritative transport publications and native enablement/approval UI.
use super::{slots::SlotId, transport::HostSnapshot};
use aurora_mcp::authority::{Authority, AuthorityError, MetadataLabel, NativePermittedDisplay, ProfileId, ProfilePhase, ProfileProjection};
use aurora_session::{Binding, Phase};

/// Supplied by the native permission owner for this exact fresh observation.
/// Host worker names alone do not establish RLVa/privacy permission.
pub(super) struct PermittedDisplay {
    pub slot: SlotId,
    pub binding: Binding,
    pub display: NativePermittedDisplay,
}

/// Invoke only from the live transport publication path. Polling an old Arc
/// would renew expired authority and is expressly not a valid caller.
pub(super) fn publish_fresh(authority: &Authority, snapshot: &HostSnapshot, permitted: &[PermittedDisplay]) -> Result<(), AuthorityError> {
    if snapshot.stopped || snapshot.registry_error.is_some() {
        return authority.stop();
    }
    let mut profiles = Vec::with_capacity(snapshot.slots.len());
    for slot in &snapshot.slots {
        let workers: Vec<_> = snapshot.workers.iter().filter(|worker| worker.slot == Some(slot.id)).collect();
        if workers.len() > 1 {
            return Err(AuthorityError::Invalid);
        }
        let worker = workers.first().copied();
        let phase = match worker {
            None if slot.available => ProfilePhase::Stopped,
            None => ProfilePhase::Unavailable,
            Some(worker) if !worker.reachable => ProfilePhase::Unavailable,
            Some(worker) => match worker.status.as_ref().map(|status| status.phase) {
                Some(Phase::Online) if worker.binding.session.is_some() => ProfilePhase::Online,
                Some(Phase::Login | Phase::Connecting) => ProfilePhase::Starting,
                _ => ProfilePhase::Unavailable,
            },
        };
        let native_display = worker.filter(|_| phase == ProfilePhase::Online).and_then(|worker| {
            permitted
                .iter()
                .find(|display| display.slot == slot.id && display.binding == worker.binding)
                .map(|d| d.display.clone())
        });
        profiles.push(ProfileProjection {
            id: ProfileId::new(slot.id.number())?,
            label: MetadataLabel::new(slot.label.as_str())?,
            phase,
            binding: worker.map(|worker| worker.binding),
            native_display,
        });
    }
    authority.publish(profiles)
}
