// SPDX-License-Identifier: LGPL-2.1-or-later
//! Native owner-authoritative FSP1 sharing, separate from local-photo consent.
//! Port of Folderstorm fssharedposemanager.cpp/model.h, abbe3cb0.
//! Copyright (c) 2026 Folderstorm contributors.
use super::World;
use aurora_llsd::{Llsd, llsd_map};
use aurora_net::bridge::shared_pose::{Assembler, Owner, Packet, decode_pose, encode_pose, fragment};
pub use aurora_net::bridge::shared_pose::{Joint, Pose};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use uuid::Uuid;

pub struct Remote {
    pub session: Uuid,
    pub generation: Uuid,
    pub pose: Pose,
    pub revision: u32,
    pub revisions: BTreeMap<String, u32>,
    pub seen: Instant,
}
struct Grant {
    session: Uuid,
    token: Uuid,
    pub generation: Uuid,
    seen: Instant,
    heartbeat: Instant,
}
#[derive(Clone)]
pub struct Invitation {
    pub peer: Uuid,
    pub request: Uuid,
    pub expires: Instant,
}
#[derive(Clone)]
pub struct PacketWork {
    pub target: Uuid,
    pub packet: Packet,
    pub operation: Option<Uuid>,
    pub epoch: u64,
}
#[derive(Clone)]
pub struct PeerReply {
    pub peer: Uuid,
    pub data: Llsd,
}
pub struct OwnerEdit {
    pub native_generation: Uuid,
    pub peer: Uuid,
    pub session: Uuid,
    pub token: Uuid,
    pub lease: Uuid,
    pub expected: BTreeMap<String, u32>,
    pub before: Pose,
    pub changes: Pose,
    pub typed: bool,
}
pub struct Service {
    pub owner: Owner,
    pub public_generation: Uuid,
    pub native_generation: Uuid,
    pub available: bool,
    pub ui_open: bool,
    pub remotes: BTreeMap<Uuid, Remote>,
    grants: BTreeMap<Uuid, Grant>,
    pub incoming: VecDeque<Invitation>,
    outgoing: BTreeMap<Uuid, Invitation>,
    peers: BTreeMap<Uuid, (Instant, bool)>,
    cooldown: BTreeMap<Uuid, Instant>,
    closed: BTreeMap<(Uuid, Uuid), Instant>,
    assembler: Assembler,
    out: VecDeque<PacketWork>,
    pub replies: VecDeque<PeerReply>,
    pub edits: VecDeque<OwnerEdit>,
    identity: Option<(Uuid, Uuid, Option<u64>, Uuid)>,
    pub epoch: Arc<AtomicU64>,
    last_packet: Instant,
    last_beacon: Instant,
    last_publish: Instant,
    last_heartbeat: Instant,
    published: Pose,
    published_revision: u32,
    pub publication: Option<Uuid>,
}
impl Default for Service {
    fn default() -> Self {
        let now = Instant::now();
        Self {
            owner: Owner::default(),
            public_generation: Uuid::new_v4(),
            native_generation: Uuid::nil(),
            available: false,
            ui_open: false,
            remotes: BTreeMap::new(),
            grants: BTreeMap::new(),
            incoming: VecDeque::new(),
            outgoing: BTreeMap::new(),
            peers: BTreeMap::new(),
            cooldown: BTreeMap::new(),
            closed: BTreeMap::new(),
            assembler: Assembler::default(),
            out: VecDeque::new(),
            replies: VecDeque::new(),
            edits: VecDeque::new(),
            identity: None,
            epoch: Arc::new(AtomicU64::new(1)),
            last_packet: now,
            last_beacon: now - Duration::from_secs(60),
            last_publish: now,
            last_heartbeat: now,
            published: Pose::new(),
            published_revision: 0,
            publication: None,
        }
    }
}
pub fn present(world: &World, peer: Uuid) -> bool {
    world
        .objects
        .index_of_uuid(&peer)
        .and_then(|i| world.objects.get(i))
        .is_some_and(|o| o.is_avatar() && Some(o.key.region) == world.main_region && !world.mutes.is_muted_id(&peer))
}
fn message(kind: &str, session: Uuid) -> Llsd {
    llsd_map! {"t"=>kind,"s"=>session}
}
fn bounded_tree(value: &Llsd, depth: usize) -> bool {
    if depth > 12 {
        return false;
    }
    match value {
        Llsd::Map(m) => m.len() <= 512 && m.iter().all(|(k, v)| k.len() <= 64 && bounded_tree(v, depth + 1)),
        Llsd::Array(a) => a.len() <= 512 && a.iter().all(|v| bounded_tree(v, depth + 1)),
        _ => true,
    }
}

fn revisions(value: &Llsd) -> Option<BTreeMap<String, u32>> {
    let map = value.as_map()?;
    if map.len() > 512 {
        return None;
    }
    map.iter()
        .map(|(n, r)| match r {
            Llsd::Integer(v) if *v >= 0 => Some((n.clone(), *v as u32)),
            _ => None,
        })
        .collect()
}
fn revdata(revisions: &BTreeMap<String, u32>) -> Llsd {
    Llsd::Map(revisions.iter().map(|(n, r)| (n.clone(), Llsd::Integer(*r as i32))).collect())
}
fn posedata(pose: &Pose) -> Llsd {
    Llsd::Binary(encode_pose(pose).unwrap_or_default())
}
impl Service {
    pub fn is_shared(&self, target: Uuid) -> bool {
        self.identity.is_some_and(|(id, _, _, _)| id == target) && self.owner.active()
            || self.remotes.contains_key(&target)
            || self.grants.contains_key(&target)
    }
    pub fn can_edit(&self, target: Uuid, now: Instant) -> bool {
        self.available
            && self.grants.get(&target).zip(self.remotes.get(&target)).is_some_and(|(g, r)| {
                g.session == r.session
                    && g.seen + Duration::from_secs(10) > now
                    && !r.pose.is_empty()
                    && self.peers.get(&target).is_some_and(|(_, typed)| *typed)
            })
    }
    pub fn grant_generation(&self, target: Uuid) -> Option<Uuid> {
        self.grants.get(&target).map(|g| g.generation)
    }
    pub fn token(&self, target: Uuid) -> Option<(Uuid, Uuid)> {
        self.grants.get(&target).map(|g| (g.session, g.token))
    }
    pub fn invalidate(&mut self) {
        self.epoch.fetch_add(1, Ordering::AcqRel);
        self.owner = Owner::default();
        self.public_generation = Uuid::new_v4();
        self.native_generation = Uuid::nil();
        self.remotes.clear();
        self.grants.clear();
        self.incoming.clear();
        self.outgoing.clear();
        self.peers.clear();
        self.closed.clear();
        self.assembler.clear();
        self.out.clear();
        self.replies.clear();
        self.edits.clear();
        self.published.clear();
        self.available = false;
        self.publication = None;
    }
    pub fn observe(&mut self, world: &World, bridge: Uuid, ready: bool, enabled: bool, native: Option<(Uuid, Pose)>, now: Instant) {
        let identity = (
            world.agent_id,
            world.login.as_ref().map(|l| l.session_id).unwrap_or_default(),
            world.main_region,
            bridge,
        );
        let available = ready
            && enabled
            && !world.teleporting
            && !world.rlva.has("showloc")
            && !world.rlva.has("shownames")
            && !world.rlva.has("shownearby")
            && !world.agent_id.is_nil()
            && !identity.1.is_nil();
        if self.identity != Some(identity) || self.available && !available {
            self.invalidate();
            self.identity = Some(identity);
        }
        if available && !self.available {
            self.available = true;
            self.send(Uuid::nil(), llsd_map! {"t"=>"hello"}, None);
            self.send(Uuid::nil(), llsd_map! {"t"=>"capable","v"=>1,"typed_rotations"=>1}, None);
        }
        let native_generation = native.as_ref().map(|(id, _)| *id).unwrap_or_default();
        if self.owner.active() && self.native_generation != native_generation {
            self.stop_publication(None);
        }
        self.native_generation = native_generation;
        if self.owner.active() {
            if let Some((_, pose)) = native {
                self.owner.observe(pose);
            } else {
                self.stop_publication(None);
            }
        }
        self.remotes
            .retain(|peer, r| present(world, *peer) && r.seen + Duration::from_secs(10) > now);
        self.grants
            .retain(|peer, g| present(world, *peer) && g.seen + Duration::from_secs(10) > now);
        self.incoming.retain(|i| i.expires > now && present(world, i.peer));
        self.outgoing.retain(|peer, i| i.expires > now && present(world, *peer));
        self.peers
            .retain(|peer, (seen, _)| present(world, *peer) && *seen + Duration::from_secs(60) > now);
        self.cooldown.retain(|_, seen| *seen + Duration::from_secs(30) > now);
        self.closed.retain(|_, expires| *expires > now);
        if !self.available {
            return;
        }
        if now.duration_since(self.last_beacon) >= Duration::from_secs(30) {
            self.last_beacon = now;
            self.send(Uuid::nil(), llsd_map! {"t"=>"capable","v"=>1,"typed_rotations"=>1}, None);
        }
        if now.duration_since(self.last_heartbeat) >= Duration::from_secs(2) {
            self.last_heartbeat = now;
            let packets: Vec<_> = self
                .grants
                .iter_mut()
                .map(|(peer, g)| {
                    g.heartbeat = now;
                    (*peer, llsd_map! {"t"=>"heartbeat","s"=>g.session,"g"=>g.token})
                })
                .collect();
            for (peer, data) in packets {
                self.send(peer, data, None);
            }
            if self.owner.active() {
                self.send(
                    Uuid::nil(),
                    llsd_map! {"t"=>"alive","s"=>self.owner.session,"r"=>self.owner.revision as i32},
                    None,
                );
            }
        }
        if self.owner.active() && now.duration_since(self.last_publish) >= Duration::from_millis(200) && self.published != self.owner.pose {
            self.last_publish = now;
            self.snapshot(Uuid::nil(), None);
        }
    }
    pub fn publish(&mut self, native_generation: Uuid, pose: Pose, operation: Option<Uuid>) -> bool {
        if !self.available
            || native_generation.is_nil()
            || self.owner.active()
            || !self.owner.start_for(self.identity.map(|i| i.0).unwrap_or_default(), pose)
        {
            return false;
        }
        self.publication = operation;
        self.native_generation = native_generation;
        self.public_generation = Uuid::new_v4();
        self.snapshot(Uuid::nil(), operation);
        true
    }
    pub fn native_takeover(&mut self) {
        self.epoch.fetch_add(1, Ordering::AcqRel);
        self.publication = None;
        self.public_generation = Uuid::new_v4();
        self.out.clear();
        self.snapshot(Uuid::nil(), None);
    }
    pub fn stop_publication(&mut self, operation: Option<Uuid>) {
        if !self.owner.active() {
            return;
        }
        let session = self.owner.session;
        self.epoch.fetch_add(1, Ordering::AcqRel);
        self.out.clear();
        self.publication = None;
        self.owner = Owner::default();
        self.public_generation = Uuid::new_v4();
        self.published.clear();
        self.send(Uuid::nil(), message("stop", session), operation);
    }
    pub fn request(&mut self, peer: Uuid, now: Instant) {
        if !self.available
            || !self.peers.contains_key(&peer)
            || self.outgoing.len() >= 64
            || self.cooldown.get(&peer).is_some_and(|at| *at + Duration::from_secs(30) > now)
        {
            return;
        }
        let request = Uuid::new_v4();
        self.cooldown.insert(peer, now);
        self.outgoing.insert(
            peer,
            Invitation {
                peer,
                request,
                expires: now + Duration::from_secs(30),
            },
        );
        self.send(peer, llsd_map! {"t"=>"request","id"=>request}, None);
    }
    pub fn answer(&mut self, invitation: Invitation, accept: bool, native: Option<(Uuid, Pose)>, now: Instant) {
        if !self.available
            || !self
                .incoming
                .iter()
                .any(|i| i.peer == invitation.peer && i.request == invitation.request && i.expires > now)
        {
            return;
        }
        self.incoming.retain(|i| i.request != invitation.request);
        if accept && !self.owner.active() {
            if let Some((generation, pose)) = native {
                self.publish(generation, pose, None);
            } else {
                return;
            }
        }
        if accept {
            if let Some(token) = self.owner.grant(invitation.peer, now) {
                self.send(
                    invitation.peer,
                    llsd_map! {"t"=>"grant","s"=>self.owner.session,"id"=>invitation.request,"g"=>token},
                    None,
                );
                self.snapshot(invitation.peer, None);
            }
        } else {
            self.send(invitation.peer, llsd_map! {"t"=>"decline","id"=>invitation.request}, None);
        }
    }
    pub fn revoke(&mut self, peer: Uuid) {
        let session = self.owner.session;
        self.owner.revoke(peer);
        self.send(peer, message("revoke", session), None);
    }
    pub fn snapshot(&mut self, target: Uuid, operation: Option<Uuid>) {
        if !self.owner.active() {
            return;
        }
        let data = llsd_map! {"t"=>"snapshot","s"=>self.owner.session,"r"=>self.owner.revision as i32,"p"=>posedata(&self.owner.pose),"jr"=>revdata(&self.owner.pose.keys().map(|n|(n.clone(),self.owner.revisions.get(n).copied().unwrap_or(0))).collect())};
        self.send(target, data, operation);
        self.published = self.owner.pose.clone();
        self.published_revision = self.owner.revision;
    }
    pub fn send(&mut self, target: Uuid, data: Llsd, operation: Option<Uuid>) {
        let operation = operation.or_else(|| {
            (self.owner.active() && data["s"].as_uuid() == self.owner.session)
                .then_some(self.publication)
                .flatten()
        });
        if !self.available || self.out.len() >= 256 {
            return;
        }
        let bytes = aurora_llsd::to_binary(&data);
        if bytes.len() > 60_000 {
            return;
        }
        let encoded = STANDARD.encode(bytes);
        if let Some(parts) = fragment(Uuid::new_v4(), &encoded) {
            if self.out.len() + parts.len() > 256 {
                return;
            }
            let epoch = self.epoch.load(Ordering::Acquire);
            self.out.extend(parts.into_iter().map(|packet| PacketWork {
                target,
                packet,
                operation,
                epoch,
            }));
        }
    }
    pub fn cancel_packets(&mut self, operation: Uuid) {
        self.out.retain(|p| p.operation != Some(operation));
    }
    pub fn take_packet(&mut self, now: Instant) -> Option<PacketWork> {
        if !self.available || now.duration_since(self.last_packet) < Duration::from_millis(100) {
            return None;
        }
        self.last_packet = now;
        self.out.pop_front()
    }
    pub fn receive(&mut self, world: &World, peer: Uuid, packet: &str, now: Instant) {
        if !self.available || peer == world.agent_id || !present(world, peer) {
            return;
        }
        let Some(packet) = Packet::parse(packet) else {
            return;
        };
        let Some(encoded) = self.assembler.receive(peer, &packet, now) else {
            return;
        };
        let Ok(bytes) = STANDARD.decode(encoded) else {
            return;
        };
        if bytes.len() > 60_000 {
            return;
        }
        let Ok((data, used)) = aurora_llsd::from_binary(&bytes) else {
            return;
        };
        if used != bytes.len() || data.as_map().is_none() || data["t"].as_str().len() > 16 || !bounded_tree(&data, 0) {
            return;
        }
        self.handle(peer, data, now);
    }
    fn handle(&mut self, peer: Uuid, data: Llsd, now: Instant) {
        let kind = data["t"].as_str();
        let session = data["s"].as_uuid();
        match kind {
            "hello" => {
                self.send(peer, llsd_map! {"t"=>"capable","v"=>1,"typed_rotations"=>1}, None);
                self.snapshot(peer, None);
                return;
            }
            "capable" => {
                if data["v"].as_i32() == 1 && self.peers.len() < 64 {
                    self.peers.insert(peer, (now, data["typed_rotations"].as_i32() == 1));
                }
                return;
            }
            "request" => {
                let id = data["id"].as_uuid();
                if id.is_nil()
                    || self.incoming.len() >= 64
                    || self.incoming.iter().any(|i| i.peer == peer)
                    || self.cooldown.get(&peer).is_some_and(|t| *t + Duration::from_secs(30) > now)
                {
                    return;
                }
                self.cooldown.insert(peer, now);
                self.incoming.push_back(Invitation {
                    peer,
                    request: id,
                    expires: now + Duration::from_secs(30),
                });
                self.ui_open = true;
                return;
            }
            "grant" | "decline" => {
                if !self
                    .outgoing
                    .get(&peer)
                    .is_some_and(|i| i.request == data["id"].as_uuid() && i.expires > now)
                {
                    return;
                }
                self.outgoing.remove(&peer);
                if kind == "grant" && !session.is_nil() && !data["g"].as_uuid().is_nil() && self.grants.len() < 64 {
                    self.grants.insert(
                        peer,
                        Grant {
                            session,
                            token: data["g"].as_uuid(),
                            generation: Uuid::new_v4(),
                            seen: now,
                            heartbeat: now,
                        },
                    );
                }
                return;
            }
            _ => {}
        }
        if session.is_nil() {
            return;
        }
        if matches!(kind, "snapshot" | "update" | "alive" | "stop" | "revoke") {
            if self.closed.contains_key(&(peer, session)) {
                return;
            }
            if kind == "stop" {
                if self.remotes.get(&peer).is_some_and(|r| r.session == session) {
                    self.remotes.remove(&peer);
                    self.grants.remove(&peer);
                }
                self.closed.insert((peer, session), now + Duration::from_secs(30));
                return;
            }
            if kind == "revoke" {
                if self.grants.get(&peer).is_some_and(|g| g.session == session) {
                    self.grants.remove(&peer);
                }
                return;
            }
            if self.remotes.len() >= 64 && !self.remotes.contains_key(&peer) {
                return;
            }
            if self.remotes.get(&peer).is_some_and(|r| r.session != session) && kind != "snapshot" {
                return;
            }
            if kind == "alive" {
                if let Some(remote) = self.remotes.get_mut(&peer).filter(|r| r.session == session) {
                    remote.seen = now;
                }
                return;
            }
            let Some(pose) = decode_pose(data["p"].as_binary()) else {
                return;
            };
            let revision = data["r"].as_i32();
            if revision < 0 {
                return;
            }
            let old = self.remotes.get(&peer).filter(|r| r.session == session);
            if kind == "update" && old.is_none_or(|r| r.pose.is_empty() || data["b"].as_i32() < 0 || data["b"].as_i32() > r.revision as i32)
            {
                self.send(peer, message("hello", Uuid::nil()), None);
                return;
            }
            let jr = if data["jr"].as_map().is_some() {
                let Some(jr) = revisions(&data["jr"]) else {
                    return;
                };
                jr
            } else {
                pose.keys().map(|n| (n.clone(), revision as u32)).collect()
            };
            if jr.len() != pose.len() || pose.keys().any(|n| jr.get(n).is_none_or(|r| *r > revision as u32)) {
                return;
            }
            // Validate the complete message before changing any joint or session.
            // A conflicting later joint must not leave an earlier joint applied.
            if let Some(old) = old {
                if kind == "snapshot" && (revision as u32) < old.revision {
                    return;
                }
                if pose
                    .iter()
                    .any(|(n, j)| old.revisions.get(n) == jr.get(n) && old.pose.get(n).is_some_and(|v| v != j))
                {
                    return;
                }
                if kind == "snapshot" && pose.keys().any(|n| old.revisions.get(n).is_some_and(|r| *r > jr[n])) {
                    return;
                }
            }
            let remote = self.remotes.entry(peer).or_insert_with(|| Remote {
                session,
                generation: Uuid::new_v4(),
                pose: Pose::new(),
                revision: 0,
                revisions: BTreeMap::new(),
                seen: now,
            });
            if remote.session != session {
                *remote = Remote {
                    session,
                    generation: Uuid::new_v4(),
                    pose: Pose::new(),
                    revision: 0,
                    revisions: BTreeMap::new(),
                    seen: now,
                };
            }
            if kind == "snapshot" {
                remote.pose = pose;
                remote.revisions = jr;
            } else {
                for (n, j) in pose {
                    let rev = jr[&n];
                    if remote.revisions.get(&n).is_some_and(|r| *r > rev) {
                        continue;
                    }
                    remote.pose.insert(n.clone(), j);
                    remote.revisions.insert(n, rev);
                }
            }
            remote.revision = remote.revision.max(revision as u32);
            remote.seen = now;
            return;
        }
        if matches!(kind, "locked" | "busy" | "ack") {
            if self.replies.len() < 64 {
                self.replies.push_back(PeerReply { peer, data });
            }
            return;
        }
        if kind == "permit" {
            if let Some(g) = self
                .grants
                .get_mut(&peer)
                .filter(|g| g.session == session && g.token == data["g"].as_uuid())
            {
                g.seen = now;
            }
            return;
        }
        if !self.owner.active() || self.owner.session != session {
            return;
        }
        let token = data["g"].as_uuid();
        let lease = data["l"].as_uuid();
        if !self.owner.heartbeat(peer, token, now) {
            return;
        }
        match kind {
            "heartbeat" => self.send(peer, llsd_map! {"t"=>"permit","s"=>session,"g"=>token}, None),
            "leave" => self.owner.revoke(peer),
            "release" => self.owner.release(peer, lease),
            "renew" => {
                self.owner.renew(peer, token, lease, now);
            }
            "acquire" => {
                let Some(expected) = revisions(&data["e"]) else {
                    return;
                };
                if !data["a"].as_bool()
                    || expected.len() != data["j"].as_array().len()
                    || data["j"].as_array().iter().any(|n| !expected.contains_key(n.as_str()))
                {
                    return;
                }
                let locked = self.owner.acquire(peer, token, lease, &expected, now);
                let mut reply =
                    llsd_map! {"t"=>if locked.is_some(){"locked"}else{"busy"},"s"=>session,"l"=>lease,"r"=>self.owner.revision as i32};
                if let Some(pose) = locked {
                    reply.insert("p", posedata(&pose));
                    reply.insert("jr", revdata(&expected));
                }
                self.send(peer, reply, None);
            }
            "edit" => {
                let Some(expected) = revisions(&data["e"]) else {
                    return;
                };
                let Some(rotations) = data["q"].as_map() else {
                    return;
                };
                if !data["a"].as_bool()
                    || !data["f"].as_bool()
                    || data["n"].as_i32() != 1
                    || rotations.len() != expected.len()
                    || rotations.is_empty()
                    || rotations.len() > 64
                    || self.edits.len() >= 64
                    || !self.owner.lease_matches(peer, token, lease, &expected, now)
                {
                    return;
                }
                let mut before = Pose::new();
                let mut changes = Pose::new();
                for (n, q) in rotations {
                    if !crate::ik_edit::poser::rotation_preset::supported(n) {
                        return;
                    }
                    let Some(j) = self.owner.pose.get(n).filter(|j| j.enabled && !j.world_locked && !j.mirrored) else {
                        return;
                    };
                    let values = q.as_array();
                    if values.len() != 4 {
                        return;
                    }
                    let mut quat = [0.; 4];
                    for (i, v) in values.iter().enumerate() {
                        if !matches!(v, Llsd::Integer(_) | Llsd::Real(_)) {
                            return;
                        }
                        quat[i] = v.as_f32();
                    }
                    let norm = quat.iter().map(|v| v * v).sum::<f32>();
                    if quat.iter().any(|v| !v.is_finite() || v.abs() > 1.001) || (norm - 1.).abs() > 0.002 {
                        return;
                    }
                    let len = norm.sqrt();
                    for v in &mut quat {
                        *v /= len;
                    }
                    before.insert(n.clone(), j.clone());
                    changes.insert(n.clone(), j.with_rotation(quat));
                }
                self.edits.push_back(OwnerEdit {
                    native_generation: self.native_generation,
                    peer,
                    session,
                    token,
                    lease,
                    expected,
                    before,
                    changes,
                    typed: true,
                });
            }
            _ => {}
        }
    }
    pub fn confirm_edit(&mut self, edit: OwnerEdit, native_accepted: bool, now: Instant) {
        let accepted = native_accepted
            && edit.session == self.owner.session
            && self
                .owner
                .apply(edit.peer, edit.token, edit.lease, &edit.expected, &edit.changes, now);
        let pose = self.owner.subset(&edit.expected.keys().cloned().collect());
        let jr = edit
            .expected
            .keys()
            .map(|n| (n.clone(), self.owner.revisions.get(n).copied().unwrap_or(0)))
            .collect();
        self.send(edit.peer,llsd_map!{"t"=>"ack","s"=>edit.session,"l"=>edit.lease,"ok"=>accepted,"f"=>true,"r"=>self.owner.revision as i32,"jr"=>revdata(&jr),"p"=>posedata(&pose),"u"=>accepted,"d"=>false},None);
    }
    pub fn acquire(&mut self, target: Uuid, lease: Uuid, expected: &BTreeMap<String, u32>, operation: Uuid) -> bool {
        let Some((session, token)) = self.token(target) else {
            return false;
        };
        self.send(target,llsd_map!{"t"=>"acquire","s"=>session,"g"=>token,"l"=>lease,"a"=>true,"e"=>revdata(expected),"j"=>Llsd::Array(expected.keys().map(|n|Llsd::String(n.clone())).collect())},Some(operation));
        true
    }
    pub fn final_edit(
        &mut self,
        target: Uuid,
        lease: Uuid,
        expected: &BTreeMap<String, u32>,
        rotations: &BTreeMap<String, [f32; 4]>,
        operation: Uuid,
    ) -> bool {
        let Some((session, token)) = self.token(target) else {
            return false;
        };
        let q = Llsd::Map(
            rotations
                .iter()
                .map(|(n, q)| (n.clone(), Llsd::Array(q.iter().map(|v| Llsd::Real(*v as f64)).collect())))
                .collect(),
        );
        self.send(
            target,
            llsd_map! {"t"=>"edit","s"=>session,"g"=>token,"l"=>lease,"a"=>true,"e"=>revdata(expected),"q"=>q,"f"=>true,"n"=>1},
            Some(operation),
        );
        true
    }
    pub fn release(&mut self, target: Uuid, lease: Uuid, operation: Option<Uuid>) {
        if let Some((session, token)) = self.token(target) {
            self.send(target, llsd_map! {"t"=>"release","s"=>session,"g"=>token,"l"=>lease}, operation);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn joint() -> Joint {
        Joint {
            transform: [0., 0., 0., 1., 0.2, 0.3, 0.4, 1., 1., 1.],
            priority: 7,
            enabled: true,
            world_locked: false,
            mirrored: false,
        }
    }
    fn service(now: Instant) -> Service {
        Service {
            available: true,
            identity: Some((Uuid::new_v4(), Uuid::new_v4(), Some(1), Uuid::new_v4())),
            last_packet: now - Duration::from_secs(1),
            ..Default::default()
        }
    }
    #[test]
    fn assistant_stop_retires_queued_publication_and_owner_grants() {
        let now = Instant::now();
        let mut s = service(now);
        let source = Uuid::new_v4();
        assert!(s.publish(Uuid::new_v4(), Pose::from([("mWristLeft".into(), joint())]), Some(source)));
        let editor = Uuid::new_v4();
        let grant = s.owner.grant(editor, now).unwrap();
        let epoch = s.epoch.load(Ordering::Acquire);
        let stop = Uuid::new_v4();
        s.stop_publication(Some(stop));
        assert!(s.epoch.load(Ordering::Acquire) > epoch);
        assert!(!s.owner.active());
        assert!(!s.owner.authorized(editor, grant, now));
        assert!(s.publication.is_none());
        assert_eq!(s.out.len(), 1);
        assert_eq!(s.out.front().unwrap().operation, Some(stop));
        assert_ne!(s.out.front().unwrap().operation, Some(source));
    }
    #[test]
    fn explicit_native_takeover_preserves_owner_pose_and_grants_with_new_provenance() {
        let now = Instant::now();
        let mut s = service(now);
        assert!(s.publish(Uuid::new_v4(), Pose::from([("mWristLeft".into(), joint())]), Some(Uuid::new_v4())));
        let session = s.owner.session;
        let editor = Uuid::new_v4();
        let grant = s.owner.grant(editor, now).unwrap();
        let generation = s.public_generation;
        let epoch = s.epoch.load(Ordering::Acquire);
        s.native_takeover();
        assert_eq!(s.owner.session, session);
        assert!(s.owner.authorized(editor, grant, now));
        assert_ne!(s.public_generation, generation);
        assert!(s.publication.is_none());
        assert!(s.out.iter().all(|p| p.operation.is_none() && p.epoch > epoch));
    }
    #[test]
    fn remote_message_validation_is_atomic_and_cannot_replace_session_with_bad_snapshot() {
        let now = Instant::now();
        let mut s = service(now);
        let peer = Uuid::new_v4();
        let session = Uuid::new_v4();
        let original = Pose::from([("mElbowLeft".into(), joint()), ("mWristLeft".into(), joint())]);
        s.handle(peer, llsd_map! {"t"=>"snapshot","s"=>session,"r"=>0,"p"=>posedata(&original)}, now);
        let generation = s.remotes[&peer].generation;
        let changed: Pose = original
            .iter()
            .map(|(n, j)| (n.clone(), j.with_rotation([0., 0., 1., 0.])))
            .collect();
        let jr = BTreeMap::from([("mElbowLeft".into(), 1), ("mWristLeft".into(), 0)]);
        s.handle(
            peer,
            llsd_map! {"t"=>"update","s"=>session,"r"=>1,"b"=>0,"p"=>posedata(&changed),"jr"=>revdata(&jr)},
            now,
        );
        assert_eq!(s.remotes[&peer].pose, original);
        assert_eq!(s.remotes[&peer].revision, 0);
        s.handle(
            peer,
            llsd_map! {"t"=>"snapshot","s"=>Uuid::new_v4(),"r"=>2,"p"=>Llsd::Binary(vec![255])},
            now,
        );
        assert_eq!(s.remotes[&peer].session, session);
        assert_eq!(s.remotes[&peer].generation, generation);
        let single = Pose::from([("mElbowLeft".into(), changed["mElbowLeft"].clone())]);
        s.handle(
            peer,
            llsd_map! {"t"=>"update","s"=>session,"r"=>1,"b"=>0,"p"=>posedata(&single)},
            now,
        );
        assert_eq!(s.remotes[&peer].pose["mElbowLeft"], changed["mElbowLeft"]);
        assert_eq!(s.remotes[&peer].pose["mWristLeft"], original["mWristLeft"]);
    }
    #[test]
    fn invitations_and_peer_beacons_never_create_editor_grants() {
        let now = Instant::now();
        let mut s = service(now);
        let peer = Uuid::new_v4();
        s.handle(peer, message("hello", Uuid::nil()), now);
        s.request(peer, now);
        assert!(s.owner.editors(now).is_empty());
        assert!(s.grants.is_empty());
        assert!(!s.can_edit(peer, now));
    }
}
