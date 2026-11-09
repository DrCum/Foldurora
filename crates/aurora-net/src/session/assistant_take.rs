//! Exact native DeRez transaction provenance. Ordinary UI DeRez is unchanged.
use super::*;
use crate::types::NativeTakeProgress;
/// Native inventory evidence retains its protocol kind. Bulk updates do not
/// carry SimApproved: they provide item observations only, never an ACK.
#[derive(Clone, Copy, Debug)]
pub(super) enum InventoryKind {
    Create { sim_approved: Option<bool> },
    Bulk,
}
#[derive(Clone, Copy, Debug)]
pub(super) struct InventoryObservation {
    pub transaction: Uuid,
    pub kind: InventoryKind,
}
#[derive(Default)]
pub(super) struct Pending {
    requests: HashMap<Uuid, Request>,
}
struct Request {
    source: LoginSource,
    circuit: Uuid,
    handle: RegionHandle,
    local: u32,
    at: Instant,
}
impl Session<'_> {
    pub(super) fn assistant_take(
        &mut self,
        source: LoginSource,
        request: Uuid,
        handle: RegionHandle,
        object: Uuid,
        local: u32,
        destination: Uuid,
        copy: bool,
    ) {
        self.assistant_takes
            .requests
            .retain(|_, r| r.at.elapsed() < Duration::from_secs(120));
        let address = self.sim_for_handle(handle);
        if request.is_nil()
            || object.is_nil()
            || destination.is_nil()
            || local == 0
            || !source.matches_login(&self.login, self.agent_id())
            || self.assistant_takes.requests.len() >= 64
            || self.assistant_takes.requests.contains_key(&request)
            || address.is_none()
        {
            emit(
                self.sh,
                NetEvent::AssistantTake {
                    source,
                    request,
                    progress: NativeTakeProgress::Rejected,
                },
            );
            return;
        }
        let address = address.expect("checked address");
        let circuit = self.sims[&address].incarnation;
        let mut message = DeRezObject::default();
        message.agent_data.agent_id = self.agent_id();
        message.agent_data.session_id = self.session_id();
        message.agent_block.group_id = Uuid::nil();
        message.agent_block.destination = if copy {
            crate::build::derez::ACQUIRE_TO_AGENT_INVENTORY
        } else {
            crate::build::derez::TAKE_INTO_AGENT_INVENTORY
        };
        message.agent_block.destination_id = destination;
        message.agent_block.transaction_id = request;
        message.agent_block.packet_count = 1;
        message.agent_block.packet_number = 0;
        message.object_data.push(de_rez_object::ObjectData { object_local_id: local });
        self.assistant_takes.requests.insert(
            request,
            Request {
                source: source.clone(),
                circuit,
                handle,
                local,
                at: Instant::now(),
            },
        );
        self.send(address, &message, true);
        emit(
            self.sh,
            NetEvent::AssistantTake {
                source,
                request,
                progress: NativeTakeProgress::Submitted { circuit },
            },
        );
    }
    fn take_source(&self, address: SocketAddr, request: Uuid) -> Option<LoginSource> {
        let request = self.assistant_takes.requests.get(&request)?;
        let sim = self.sims.get(&address)?;
        (request.at.elapsed() < Duration::from_secs(120)
            && sim.handle == request.handle
            && sim.incarnation == request.circuit
            && request.source.matches_login(&self.login, self.agent_id()))
        .then(|| request.source.clone())
    }
    pub(super) fn assistant_take_ack(&mut self, address: SocketAddr, request: Uuid, success: bool) {
        if let Some(source) = self.take_source(address, request) {
            emit(
                self.sh,
                NetEvent::AssistantTake {
                    source,
                    request,
                    progress: NativeTakeProgress::Ack(success),
                },
            );
        }
    }
    pub(super) fn assistant_take_items(&mut self, address: SocketAddr, request: Uuid, approved: bool, items: &[crate::inventory::InvItem]) {
        self.assistant_take_inventory(
            address,
            InventoryObservation {
                transaction: request,
                kind: InventoryKind::Create {
                    sim_approved: Some(approved),
                },
            },
            items,
        );
    }
    pub(super) fn assistant_take_inventory(
        &mut self,
        address: SocketAddr,
        observation: InventoryObservation,
        items: &[crate::inventory::InvItem],
    ) {
        let Some(source) = self.take_source(address, observation.transaction) else {
            return;
        };
        // Missing Create approval is unavailable evidence. It must not become
        // a positive observation through LLSD's false/default conversions.
        if matches!(observation.kind, InventoryKind::Create { sim_approved: None }) {
            return;
        }
        let rejected = matches!(observation.kind, InventoryKind::Create { sim_approved: Some(false) }) || items.len() > 16;
        emit(
            self.sh,
            NetEvent::AssistantTake {
                source,
                request: observation.transaction,
                progress: if rejected {
                    NativeTakeProgress::Rejected
                } else {
                    NativeTakeProgress::Items(items.to_vec())
                },
            },
        );
    }
    pub(super) fn assistant_take_removed(&mut self, address: SocketAddr, locals: &[u32]) {
        let requests: Vec<_> = self
            .assistant_takes
            .requests
            .iter()
            .filter(|(_, r)| locals.contains(&r.local))
            .map(|(id, _)| *id)
            .collect();
        for request in requests {
            if let Some(source) = self.take_source(address, request) {
                emit(
                    self.sh,
                    NetEvent::AssistantTake {
                        source,
                        request,
                        progress: NativeTakeProgress::Removed,
                    },
                );
            }
        }
    }
}
