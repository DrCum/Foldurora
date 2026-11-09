//! Named teleports return through the common action boundary after map lookup.
//! RlvHandler::onTeleportCallback retains the issuing object around the delayed
//! teleport. Tickets also discard replies superseded by another destination.
use aurora_net::{NetCommand, RegionHandle};
use glam::Vec3;
use uuid::Uuid;

#[derive(Default)]
pub(super) struct NamedTeleport {
    pending: Option<Pending>,
    owned_arm: Option<Owned>,
    owned: Option<Owned>,
    finished_owned: Option<Uuid>,
}
struct Owned {
    request: Uuid,
    handle: u64,
    position: Vec3,
    look_at: Vec3,
    wire_sent: bool,
    started: bool,
}

struct Pending {
    request: Uuid,
    actor: Uuid,
    position: Vec3,
    look_at: Vec3,
}

impl NamedTeleport {
    pub fn observe(&mut self, command: &NetCommand, actor: Uuid) {
        if matches!(
            command,
            NetCommand::TeleportTo { .. }
                | NetCommand::TeleportToRegion { .. }
                | NetCommand::TeleportHome
                | NetCommand::TeleportLandmark(_)
                | NetCommand::AcceptLure { .. }
        ) {
            self.finished_owned = None;
            self.owned=self.owned_arm.take().filter(|owned|matches!(command,NetCommand::TeleportTo{handle,position,look_at} if owned.handle==*handle && owned.position==*position && owned.look_at==*look_at));
        }
        match command {
            NetCommand::TeleportToRegion {
                request,
                position,
                look_at,
                ..
            } => {
                self.pending = Some(Pending {
                    request: *request,
                    actor,
                    position: *position,
                    look_at: *look_at,
                });
            }
            NetCommand::TeleportTo { .. } | NetCommand::TeleportHome | NetCommand::TeleportLandmark(_) | NetCommand::AcceptLure { .. } => {
                self.pending = None;
            }
            _ => {}
        }
    }
    pub fn arm_owned(&mut self, request: Uuid, command: &NetCommand) {
        self.owned_arm = if let NetCommand::TeleportTo { handle, position, look_at } = command {
            Some(Owned {
                request,
                handle: *handle,
                position: *position,
                look_at: *look_at,
                wire_sent: false,
                started: false,
            })
        } else {
            None
        };
    }
    pub fn finish_owned(&mut self, request: Uuid) -> bool {
        self.owned_arm = None;
        self.owns(request)
    }
    pub fn owns(&self, request: Uuid) -> bool {
        self.owned.as_ref().is_some_and(|p| p.request == request && p.wire_sent)
    }
    pub fn related(&self, request: Uuid) -> bool {
        self.owns(request) || self.finished_owned == Some(request)
    }
    pub fn retire_owned(&mut self, request: Uuid) {
        if self.owned.as_ref().is_some_and(|p| p.request == request) {
            self.owned = None;
        }
        self.owned_arm = None;
        if self.finished_owned == Some(request) {
            self.finished_owned = None;
        }
    }
    pub fn wire_owner(&mut self, command: &NetCommand) -> Option<Uuid> {
        let owned=self.owned.as_mut().filter(|p|!p.wire_sent && matches!(command,NetCommand::TeleportTo{handle,position,look_at} if p.handle==*handle&&p.position==*position&&p.look_at==*look_at))?;
        owned.wire_sent = true;
        Some(owned.request)
    }
    pub fn native_event(&mut self, event: &aurora_net::NetEvent) {
        match event {
            aurora_net::NetEvent::TeleportStarted => {
                if let Some(p) = &mut self.owned {
                    if p.started {
                        self.owned = None;
                    } else {
                        p.started = true;
                    }
                }
            }
            aurora_net::NetEvent::TeleportFinished { .. }
            | aurora_net::NetEvent::TeleportFailed { .. }
            | aurora_net::NetEvent::TeleportLocal { .. }
            | aurora_net::NetEvent::AgentMovementComplete { .. } => {
                if let Some(owned) = self.owned.take() {
                    self.finished_owned = Some(owned.request);
                }
                self.owned_arm = None;
            }
            aurora_net::NetEvent::Disconnected { .. } => {
                self.owned = None;
                self.finished_owned = None;
                self.owned_arm = None;
            }
            _ => {}
        }
    }

    pub fn cancel_scripts(&mut self) {
        if self.pending.as_ref().is_some_and(|p| !p.actor.is_nil()) {
            self.pending = None;
        }
    }

    pub fn resolve(&mut self, request: Uuid, result: Result<RegionHandle, String>) -> Option<(Uuid, Result<NetCommand, String>)> {
        if self.pending.as_ref()?.request != request {
            return None;
        }
        let pending = self.pending.take()?;
        Some((
            pending.actor,
            result.map(|handle| NetCommand::TeleportTo {
                handle,
                position: pending.position,
                look_at: pending.look_at,
            }),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn owned_teleport_marker_cannot_cancel_a_new_manual_or_same_destination_request() {
        let mut pending = NamedTeleport::default();
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        let command = NetCommand::TeleportTo {
            handle: 256,
            position: Vec3::new(10., 20., 30.),
            look_at: Vec3::X,
        };
        pending.arm_owned(first, &command);
        pending.observe(&command, Uuid::nil());
        assert_eq!(pending.wire_owner(&command), Some(first));
        assert!(pending.finish_owned(first));
        assert!(pending.owns(first));
        pending.observe(&command, Uuid::nil());
        assert!(!pending.owns(first));
        assert!(!pending.related(first));
        assert!(pending.wire_owner(&command).is_none());
        pending.arm_owned(second, &command);
        pending.observe(&command, Uuid::nil());
        assert_eq!(pending.wire_owner(&command), Some(second));
        assert!(pending.finish_owned(second));
        pending.retire_owned(first);
        assert!(pending.owns(second));
        pending.native_event(&aurora_net::NetEvent::TeleportFailed {
            reason: "cancelled".into(),
        });
        assert!(!pending.owns(second));
        assert!(pending.related(second));
        pending.observe(&NetCommand::TeleportHome, Uuid::nil());
        assert!(!pending.related(second));
    }

    fn lookup(request: Uuid) -> NetCommand {
        NetCommand::TeleportToRegion {
            request,
            name: "region".into(),
            position: Vec3::new(7.25, 3.5, 20.0),
            look_at: Vec3::Y,
        }
    }

    #[test]
    fn rlva_named_teleports_retain_actor_heading_and_only_accept_the_current_reply_once() {
        let mut pending = NamedTeleport::default();
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        let actor = Uuid::from_u128(3);
        pending.observe(&lookup(a), actor);
        pending.observe(&lookup(b), actor);
        assert!(pending.resolve(a, Err("stale".into())).is_none());
        let (source, command) = pending.resolve(b, Ok(256)).unwrap();
        assert_eq!(source, actor);
        assert!(
            matches!(command, Ok(NetCommand::TeleportTo { handle: 256, position, look_at }) if position == Vec3::new(7.25, 3.5, 20.0) && look_at == Vec3::Y)
        );
        assert!(pending.resolve(b, Ok(256)).is_none());
        pending.observe(&lookup(a), actor);
        pending.cancel_scripts();
        assert!(pending.resolve(a, Ok(256)).is_none());
        pending.observe(&lookup(a), Uuid::nil());
        pending.cancel_scripts();
        assert!(
            pending
                .resolve(a, Err("not found".into()))
                .is_some_and(|(source, result)| source.is_nil() && result.is_err())
        );
        pending.observe(&lookup(a), actor);
        pending.observe(&NetCommand::TeleportHome, Uuid::nil());
        assert!(pending.resolve(a, Ok(256)).is_none());
    }
}
