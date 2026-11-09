// SPDX-License-Identifier: LGPL-2.1-or-later
//! Wire-compatible bounded shared posing. Port of Folderstorm
//! fssharedposemodel.h at abbe3cb0ca66ee2484f277219f6a27706fab7fc6.
//! Copyright (c) 2026 Folderstorm contributors.
use std::{
    collections::{BTreeMap, BTreeSet},
    time::{Duration, Instant},
};
use uuid::Uuid;

pub const CHANNEL: i32 = -1_847_659_327;
pub const MAX_JOINTS: usize = 512;
pub const MAX_PACKET: usize = 800;
const CHUNK: usize = 600;
const MAX_FRAGMENTS: usize = 128;
pub const LEASE_SECONDS: u64 = 5;
pub const PEER_SECONDS: u64 = 10;

/// Only validated packets can enter the query transport. Packet syntax never
/// introduces the query body's separator or arbitrary script commands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packet(String);
impl Packet {
    pub fn parse(raw: &str) -> Option<Self> {
        parts(raw)?;
        Some(Self(raw.into()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
fn parts(raw: &str) -> Option<(Uuid, usize, usize, &str)> {
    if raw.len() > MAX_PACKET || !raw.starts_with("FSP1:") {
        return None;
    }
    let mut p = raw.splitn(5, ':');
    p.next()?;
    let id = p.next()?;
    if id.len() != 36 {
        return None;
    }
    let id = Uuid::parse_str(id).ok().filter(|u| !u.is_nil())?;
    let number = |s: &str| {
        if s.is_empty() || s.len() > 3 || !s.bytes().all(|b| b.is_ascii_digit()) {
            None
        } else {
            s.parse::<usize>().ok()
        }
    };
    let index = number(p.next()?)?;
    let count = number(p.next()?)?;
    let data = p.next()?;
    if count == 0
        || count > MAX_FRAGMENTS
        || index >= count
        || data.is_empty()
        || data.len() > CHUNK
        || !data.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='))
    {
        return None;
    }
    Some((id, index, count, data))
}
pub fn fragment(id: Uuid, payload: &str) -> Option<Vec<Packet>> {
    if id.is_nil() || payload.is_empty() || payload.len() > CHUNK * MAX_FRAGMENTS || !payload.is_ascii() {
        return None;
    }
    let count = payload.len().div_ceil(CHUNK);
    payload
        .as_bytes()
        .chunks(CHUNK)
        .enumerate()
        .map(|(i, c)| Packet::parse(&format!("FSP1:{id}:{i}:{count}:{}", std::str::from_utf8(c).ok()?)))
        .collect()
}
struct Frame {
    parts: Vec<Option<String>>,
    expires: Instant,
}
#[derive(Default)]
pub struct Assembler {
    frames: BTreeMap<(Uuid, Uuid), Frame>,
    complete: BTreeMap<(Uuid, Uuid), Instant>,
}
impl Assembler {
    pub fn receive(&mut self, owner: Uuid, packet: &Packet, now: Instant) -> Option<String> {
        self.frames.retain(|_, f| f.expires > now);
        self.complete.retain(|_, at| *at > now);
        if owner.is_nil() {
            return None;
        }
        let (id, index, count, data) = parts(packet.as_str())?;
        let key = (owner, id);
        if self.complete.contains_key(&key) || (!self.frames.contains_key(&key) && self.frames.len() >= 64) {
            return None;
        }
        let frame = self.frames.entry(key).or_insert_with(|| Frame {
            parts: vec![None; count],
            expires: now + Duration::from_secs(15),
        });
        if frame.parts.len() != count || frame.parts[index].as_ref().is_some_and(|s| s != data) {
            self.frames.remove(&key);
            return None;
        }
        frame.parts[index] = Some(data.into());
        let result: Option<String> = frame.parts.iter().map(|p| p.as_deref()).collect();
        if let Some(result) = result {
            self.frames.remove(&key);
            if self.complete.len() >= 256
                && let Some(key) = self.complete.keys().next().copied()
            {
                self.complete.remove(&key);
            }
            self.complete.insert(key, now + Duration::from_secs(30));
            Some(result)
        } else {
            None
        }
    }
    pub fn clear(&mut self) {
        self.frames.clear();
        self.complete.clear();
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Joint {
    pub transform: [f32; 10],
    pub priority: u8,
    pub enabled: bool,
    pub world_locked: bool,
    pub mirrored: bool,
}
pub type Pose = BTreeMap<String, Joint>;
pub fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 64 && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}
impl Joint {
    pub fn valid(&self) -> bool {
        let norm = self.transform[..4].iter().map(|v| v * v).sum::<f32>();
        self.priority <= 7
            && norm > 0.5
            && norm < 1.5
            && self
                .transform
                .iter()
                .enumerate()
                .all(|(i, v)| v.is_finite() && (i < 4 || v.abs() <= 1024.))
    }
    pub fn with_rotation(&self, q: [f32; 4]) -> Self {
        let mut next = self.clone();
        next.transform[..4].copy_from_slice(&q);
        next
    }
}
pub fn encode_pose(pose: &Pose) -> Option<Vec<u8>> {
    if pose.is_empty() || pose.len() > MAX_JOINTS {
        return None;
    }
    let mut out = Vec::new();
    for (name, joint) in pose {
        if !valid_name(name) || !joint.valid() {
            return None;
        }
        out.push(name.len() as u8);
        out.extend_from_slice(name.as_bytes());
        out.push(joint.priority);
        out.push(u8::from(joint.enabled) | (u8::from(joint.world_locked) << 1) | (u8::from(joint.mirrored) << 2));
        for value in joint.transform {
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
    Some(out)
}
pub fn decode_pose(bytes: &[u8]) -> Option<Pose> {
    if bytes.is_empty() || bytes.len() > 50_000 {
        return None;
    }
    let mut offset = 0;
    let mut pose = Pose::new();
    while offset < bytes.len() {
        let len = *bytes.get(offset)? as usize;
        offset += 1;
        if len == 0 || len > 64 || offset.checked_add(len + 42)? > bytes.len() {
            return None;
        }
        let name = std::str::from_utf8(&bytes[offset..offset + len]).ok()?.to_owned();
        offset += len;
        let priority = bytes[offset];
        let flags = bytes[offset + 1];
        offset += 2;
        if flags > 7 {
            return None;
        }
        let mut transform = [0.; 10];
        for value in &mut transform {
            *value = f32::from_le_bytes(bytes[offset..offset + 4].try_into().ok()?);
            offset += 4;
        }
        let joint = Joint {
            transform,
            priority,
            enabled: flags & 1 != 0,
            world_locked: flags & 2 != 0,
            mirrored: flags & 4 != 0,
        };
        if !valid_name(&name) || !joint.valid() || pose.insert(name, joint).is_some() || pose.len() > MAX_JOINTS {
            return None;
        }
    }
    Some(pose)
}

struct Grant {
    token: Uuid,
    expires: Instant,
}
struct Lease {
    editor: Uuid,
    joints: BTreeSet<String>,
    expires: Instant,
    before: Pose,
}
#[derive(Clone)]
struct History {
    before: Pose,
    after: Pose,
    expected: BTreeMap<String, u32>,
}
/// Native owner authority. Tokens, grants and leases never enter public DTOs.
#[derive(Default)]
pub struct Owner {
    pub owner: Uuid,
    pub session: Uuid,
    pub generation: Uuid,
    pub pose: Pose,
    pub revision: u32,
    pub revisions: BTreeMap<String, u32>,
    grants: BTreeMap<Uuid, Grant>,
    leases: BTreeMap<Uuid, Lease>,
    history: BTreeMap<Uuid, Vec<History>>,
}
impl Owner {
    pub fn start(&mut self, pose: Pose) -> bool {
        if encode_pose(&pose).is_none() {
            return false;
        }
        *self = Self {
            session: Uuid::new_v4(),
            generation: Uuid::new_v4(),
            pose,
            ..Default::default()
        };
        true
    }
    pub fn active(&self) -> bool {
        !self.session.is_nil()
    }
    pub fn start_for(&mut self, owner: Uuid, pose: Pose) -> bool {
        if owner.is_nil() || !self.start(pose) {
            return false;
        }
        self.owner = owner;
        true
    }
    pub fn grant(&mut self, editor: Uuid, now: Instant) -> Option<Uuid> {
        if !self.active() || editor.is_nil() || self.grants.len() >= 64 {
            return None;
        }
        let token = Uuid::new_v4();
        self.grants.insert(
            editor,
            Grant {
                token,
                expires: now + Duration::from_secs(PEER_SECONDS),
            },
        );
        Some(token)
    }
    pub fn authorized(&self, editor: Uuid, token: Uuid, now: Instant) -> bool {
        self.active()
            && ((!self.owner.is_nil() && editor == self.owner && token.is_nil())
                || self.grants.get(&editor).is_some_and(|g| g.token == token && g.expires > now))
    }
    pub fn heartbeat(&mut self, editor: Uuid, token: Uuid, now: Instant) -> bool {
        if !self.authorized(editor, token, now) {
            return false;
        }
        if let Some(g) = self.grants.get_mut(&editor) {
            g.expires = now + Duration::from_secs(PEER_SECONDS);
        }
        true
    }
    pub fn editors(&self, now: Instant) -> Vec<Uuid> {
        self.grants.iter().filter(|(_, g)| g.expires > now).map(|(id, _)| *id).collect()
    }
    pub fn revoke(&mut self, editor: Uuid) {
        self.grants.remove(&editor);
        self.leases.retain(|_, l| l.editor != editor);
    }
    pub fn acquire(&mut self, editor: Uuid, token: Uuid, lease: Uuid, expected: &BTreeMap<String, u32>, now: Instant) -> Option<Pose> {
        self.leases.retain(|_, l| l.expires > now);
        if !self.authorized(editor, token, now) || lease.is_nil() || expected.is_empty() || expected.len() > 64 {
            return None;
        }
        if expected
            .iter()
            .any(|(n, r)| !self.pose.contains_key(n) || self.revisions.get(n).copied().unwrap_or(0) != *r)
            || self
                .leases
                .iter()
                .any(|(id, l)| *id != lease && l.joints.iter().any(|n| expected.contains_key(n)))
        {
            return None;
        }
        let before: Pose = expected
            .keys()
            .filter_map(|n| self.pose.get(n).map(|j| (n.clone(), j.clone())))
            .collect();
        if let Some(old) = self.leases.get(&lease)
            && (old.editor != editor || old.joints != expected.keys().cloned().collect())
        {
            return None;
        }
        if self.leases.len() >= 64 && !self.leases.contains_key(&lease) {
            return None;
        }
        self.leases.insert(
            lease,
            Lease {
                editor,
                joints: expected.keys().cloned().collect(),
                expires: now + Duration::from_secs(LEASE_SECONDS),
                before: before.clone(),
            },
        );
        Some(before)
    }
    pub fn renew(&mut self, editor: Uuid, token: Uuid, lease: Uuid, now: Instant) -> bool {
        if !self.authorized(editor, token, now) {
            return false;
        }
        if let Some(l) = self.leases.get_mut(&lease).filter(|l| l.editor == editor && l.expires > now) {
            l.expires = now + Duration::from_secs(LEASE_SECONDS);
            true
        } else {
            false
        }
    }
    pub fn release(&mut self, editor: Uuid, lease: Uuid) {
        if self.leases.get(&lease).is_some_and(|l| l.editor == editor) {
            self.leases.remove(&lease);
        }
    }
    pub fn lease_matches(&self, editor: Uuid, token: Uuid, lease: Uuid, expected: &BTreeMap<String, u32>, now: Instant) -> bool {
        self.authorized(editor, token, now)
            && self.leases.get(&lease).is_some_and(|l| {
                l.editor == editor
                    && l.expires > now
                    && l.joints == expected.keys().cloned().collect()
                    && expected.iter().all(|(n, r)| self.revisions.get(n).copied().unwrap_or(0) == *r)
            })
    }
    pub fn apply(
        &mut self,
        editor: Uuid,
        token: Uuid,
        lease: Uuid,
        expected: &BTreeMap<String, u32>,
        changes: &Pose,
        now: Instant,
    ) -> bool {
        if !self.can_apply(editor, token, lease, expected, changes, now) {
            return false;
        }
        let Some(reservation) = self.leases.remove(&lease) else {
            return false;
        };
        self.revision += 1;
        let mut history = History {
            before: reservation.before,
            after: changes.clone(),
            expected: BTreeMap::new(),
        };
        for (n, j) in changes {
            self.pose.insert(n.clone(), j.clone());
            self.revisions.insert(n.clone(), self.revision);
            history.expected.insert(n.clone(), self.revision);
        }
        let entries = self.history.entry(editor).or_default();
        entries.push(history);
        if entries.len() > 20 {
            entries.remove(0);
        }
        true
    }
    /// Final bounded owner check, called immediately before the native poser
    /// mutation and again when committing its matching authority revision.
    pub fn can_apply(
        &self,
        editor: Uuid,
        token: Uuid,
        lease: Uuid,
        expected: &BTreeMap<String, u32>,
        changes: &Pose,
        now: Instant,
    ) -> bool {
        self.revision < i32::MAX as u32
            && self.lease_matches(editor, token, lease, expected, now)
            && changes.len() == expected.len()
            && encode_pose(changes).is_some()
            && changes.keys().all(|n| expected.contains_key(n))
    }
    pub fn observe(&mut self, pose: Pose) {
        if !self.active() || encode_pose(&pose).is_none() {
            return;
        }
        let changes: Vec<_> = pose
            .iter()
            .filter(|(n, j)| self.pose.get(*n) != Some(*j))
            .map(|(n, j)| (n.clone(), j.clone()))
            .collect();
        if changes.is_empty() && pose.len() == self.pose.len() {
            return;
        }
        if self.revision == i32::MAX as u32 {
            *self = Self::default();
            return;
        }
        self.revision += 1;
        for (n, _) in &changes {
            self.revisions.insert(n.clone(), self.revision);
        }
        self.pose = pose;
    }
    /// Restore only this editor's latest unchanged joint edit. Another editor's
    /// intervening write, a manual native change, or a live reservation refuses
    /// the undo rather than restoring a stale whole-avatar snapshot.
    pub fn undo(&mut self, editor: Uuid, token: Uuid, now: Instant) -> Option<Pose> {
        if !self.authorized(editor, token, now) || self.revision == i32::MAX as u32 {
            return None;
        }
        let history = self.history.get(&editor)?.last()?.clone();
        if history
            .expected
            .iter()
            .any(|(n, r)| self.revisions.get(n) != Some(r) || self.pose.get(n) != history.after.get(n))
            || self
                .leases
                .values()
                .any(|l| l.expires > now && l.joints.iter().any(|n| history.before.contains_key(n)))
        {
            return None;
        }
        self.revision += 1;
        for (n, j) in &history.before {
            self.pose.insert(n.clone(), j.clone());
            self.revisions.insert(n.clone(), self.revision);
        }
        let entries = self.history.get_mut(&editor)?;
        entries.pop();
        if let Some(previous) = entries.last_mut() {
            for (n, j) in &previous.after {
                if self.pose.get(n) == Some(j) {
                    previous.expected.insert(n.clone(), self.revisions.get(n).copied().unwrap_or(0));
                }
            }
        }
        Some(history.before)
    }
    pub fn subset(&self, names: &BTreeSet<String>) -> Pose {
        names
            .iter()
            .filter_map(|n| self.pose.get(n).map(|j| (n.clone(), j.clone())))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn joint() -> Joint {
        Joint {
            transform: [0., 0., 0., 1., 0.1, 0.2, 0.3, 1., 2., 3.],
            priority: 7,
            enabled: true,
            world_locked: false,
            mirrored: false,
        }
    }
    #[test]
    fn wire_pose_roundtrip_rejects_duplicates_nonfinite_and_unknown_flags() {
        let pose = Pose::from([("mWristLeft".into(), joint())]);
        let bytes = encode_pose(&pose).unwrap();
        assert_eq!(decode_pose(&bytes), Some(pose));
        let mut duplicate = bytes.clone();
        duplicate.extend_from_slice(&bytes);
        assert!(decode_pose(&duplicate).is_none());
        let mut invalid = bytes;
        invalid[1 + 10 + 1] = 8;
        assert!(decode_pose(&invalid).is_none());
    }
    #[test]
    fn fragments_are_authenticated_per_owner_conflict_checked_and_replay_bounded() {
        let now = Instant::now();
        let packets = fragment(Uuid::new_v4(), &"A".repeat(1400)).unwrap();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let mut assembler = Assembler::default();
        assert!(assembler.receive(a, &packets[0], now).is_none());
        assert!(assembler.receive(b, &packets[1], now).is_none());
        assert!(assembler.receive(a, &packets[1], now).is_none());
        assert_eq!(assembler.receive(a, &packets[2], now).unwrap().len(), 1400);
        assert!(assembler.receive(a, &packets[2], now).is_none());
        assert!(Packet::parse("FSP1:bad:0:1:A").is_none());
    }
    #[test]
    fn owner_reservations_require_grant_and_exact_revision_preserve_other_channels() {
        let now = Instant::now();
        let mut owner = Owner::default();
        assert!(owner.start(Pose::from([("mWristLeft".into(), joint())])));
        let editor = Uuid::new_v4();
        let token = owner.grant(editor, now).unwrap();
        let lease = Uuid::new_v4();
        let expected = BTreeMap::from([("mWristLeft".into(), 0)]);
        assert!(owner.acquire(editor, Uuid::nil(), lease, &expected, now).is_none());
        let before = owner.acquire(editor, token, lease, &expected, now).unwrap();
        let changed = Pose::from([("mWristLeft".into(), before["mWristLeft"].with_rotation([0., 0., 1., 0.]))]);
        assert!(owner.apply(editor, token, lease, &expected, &changed, now));
        assert_eq!(&owner.pose["mWristLeft"].transform[4..], &joint().transform[4..]);
        assert!(owner.acquire(editor, token, Uuid::new_v4(), &expected, now).is_none());
        assert!(!owner.authorized(editor, token, now + Duration::from_secs(11)));
    }
    #[test]
    fn revocation_and_session_restart_retire_leases_and_editor_history() {
        let now = Instant::now();
        let editor = Uuid::new_v4();
        let mut owner = Owner::default();
        assert!(owner.start(Pose::from([("mWristLeft".into(), joint())])));
        let token = owner.grant(editor, now).unwrap();
        let expected = BTreeMap::from([("mWristLeft".into(), 0)]);
        let lease = Uuid::new_v4();
        let before = owner.acquire(editor, token, lease, &expected, now).unwrap();
        owner.revoke(editor);
        assert!(!owner.apply(editor, token, lease, &expected, &before, now));
        assert!(owner.start(before.clone()));
        assert!(!owner.authorized(editor, token, now));
        assert!(owner.undo(editor, token, now).is_none());
    }
    #[test]
    fn editor_undo_restores_own_channels_and_refuses_intervening_native_change() {
        let now = Instant::now();
        let editor = Uuid::new_v4();
        let mut owner = Owner::default();
        let original = Pose::from([("mWristLeft".into(), joint())]);
        assert!(owner.start(original.clone()));
        let token = owner.grant(editor, now).unwrap();
        let expected = BTreeMap::from([("mWristLeft".into(), 0)]);
        let lease = Uuid::new_v4();
        let before = owner.acquire(editor, token, lease, &expected, now).unwrap();
        let changed = Pose::from([("mWristLeft".into(), before["mWristLeft"].with_rotation([0., 0., 1., 0.]))]);
        assert!(owner.apply(editor, token, lease, &expected, &changed, now));
        assert_eq!(owner.undo(editor, token, now), Some(original.clone()));
        assert_eq!(owner.pose, original);
        let expected = BTreeMap::from([("mWristLeft".into(), 2)]);
        let lease = Uuid::new_v4();
        owner.acquire(editor, token, lease, &expected, now).unwrap();
        assert!(owner.apply(editor, token, lease, &expected, &changed, now));
        owner.observe(original.clone());
        assert!(owner.undo(editor, token, now).is_none());
        assert_eq!(owner.pose, original);
    }
}
