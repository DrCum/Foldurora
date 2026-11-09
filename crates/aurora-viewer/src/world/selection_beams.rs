// SPDX-License-Identifier: LGPL-2.1-or-later
//! Typed native selection effects, after Folderstorm llhudeffecttrail.cpp
//! at abbe3cb0ca66ee2484f277219f6a27706fab7fc6. Aurora bounds this visual
//! cache to 128 effects and sixty seconds; it grants no interaction rights.
use glam::DVec3;
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};
use uuid::Uuid;
pub(crate) struct Beam {
    pub source: Uuid,
    pub target: Uuid,
    pub position: DVec3,
    pub color: [u8; 4],
    pub expires: Instant,
    pub elapsed: f32,
}
pub(crate) struct Store {
    pub effects: BTreeMap<Uuid, Beam>,
    pub identity: Option<(Uuid, Uuid)>,
    pub own: Uuid,
    pub target: Option<Uuid>,
    pub sent: Instant,
}
impl Default for Store {
    fn default() -> Self {
        Self {
            effects: BTreeMap::new(),
            identity: None,
            own: Uuid::new_v4(),
            target: None,
            sent: Instant::now() - Duration::from_secs(1),
        }
    }
}
impl Store {
    pub fn bind(&mut self, identity: (Uuid, Uuid)) {
        if self.identity != Some(identity) {
            self.effects.clear();
            self.identity = Some(identity);
            self.own = Uuid::new_v4();
            self.target = None;
            self.sent = Instant::now() - Duration::from_secs(1);
        }
    }
    pub fn receive(&mut self, id: Uuid, source: Uuid, target: Uuid, position: [f64; 3], color: [u8; 4], duration: f32) {
        if id.is_nil()
            || source.is_nil()
            || !duration.is_finite()
            || !(0.0..=60.).contains(&duration)
            || position.iter().any(|v| !v.is_finite())
        {
            return;
        }
        if duration == 0. {
            self.effects.remove(&id);
            return;
        }
        if self.effects.len() >= 128 && !self.effects.contains_key(&id) {
            return;
        }
        let position = DVec3::from_array(position);
        if target.is_nil() && position == DVec3::ZERO {
            return;
        }
        let elapsed = self
            .effects
            .get(&id)
            .filter(|b| b.source == source && b.target == target)
            .map_or(0., |b| b.elapsed);
        self.effects.insert(
            id,
            Beam {
                source,
                target,
                position,
                color,
                expires: Instant::now() + Duration::from_secs_f32(duration),
                elapsed,
            },
        );
    }
}
