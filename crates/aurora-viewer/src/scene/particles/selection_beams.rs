// SPDX-License-Identifier: LGPL-2.1-or-later
//! LLViewerPartSourceBeam visual path, Folderstorm abbe3cb0.
use super::*;
#[allow(clippy::too_many_arguments)]
pub(super) fn append(
    store: &mut crate::world::selection_beams::Store,
    world: &World,
    dt: f32,
    frozen: bool,
    eye: Vec3,
    draw_distance: f32,
    filters: &crate::world::particle_filters::ParticleFilters,
    max: usize,
    slot: u32,
    out: &mut Vec<ParticleInstance>,
) {
    let now = Instant::now();
    let identity = (world.agent_id, world.login.as_ref().map(|l| l.session_id).unwrap_or_default());
    if store.identity != Some(identity) {
        store.bind(identity);
        return;
    }
    store.effects.retain(|_, b| {
        b.expires > now
            && world.objects.index_of_uuid(&b.source).is_some()
            && (b.target.is_nil() || world.objects.index_of_uuid(&b.target).is_some())
    });
    for beam in store.effects.values_mut() {
        if !frozen {
            beam.elapsed += dt.clamp(0., 0.1);
        }
        let Some(index) = world.objects.index_of_uuid(&beam.source) else {
            continue;
        };
        let Some(source) = world.objects.get(index) else { continue };
        let owner = if source.is_avatar() { source.full_id } else { source.owner_id };
        if filters.hides(source.full_id, owner)
            || world.particles_blocked(&owner)
            || !world.parcel_object_visible(index)
            || !crate::snapshot::isolate::object_visible(world, index)
        {
            continue;
        }
        let Some((mut start, _, hud)) = Scene::object_transform(world, index, now, 0) else {
            continue;
        };
        if hud || start.distance(eye) > draw_distance {
            continue;
        }
        if source.is_avatar() {
            start = Scene::joint_position(world, &beam.source, "mWristLeft", now).unwrap_or(start);
        }
        let end = if beam.target.is_nil() {
            let Some(origin) = world.main_origin() else { continue };
            (beam.position - glam::DVec3::new(origin.0 as f64, origin.1 as f64, 0.)).as_vec3()
        } else {
            let Some(index) = world.objects.index_of_uuid(&beam.target) else {
                continue;
            };
            let Some((end, _, hud)) = Scene::object_transform(world, index, now, 0) else {
                continue;
            };
            if hud {
                continue;
            }
            end
        };
        if !start.is_finite() || !end.is_finite() {
            continue;
        }
        // Native beam emits every .025 s; each point travels from the live
        // wrist/source to the live target over its half-second lifetime.
        let remainder = beam.elapsed.rem_euclid(0.025);
        for n in 0..20 {
            if out.len() >= max {
                return;
            }
            let age = remainder + n as f32 * 0.025;
            if age > beam.elapsed {
                break;
            }
            let t = age / 0.5;
            let mut color = beam.color;
            color[3] = (color[3] as f32 * (1. - 0.6 * t)).round() as u8;
            out.push(ParticleInstance {
                pos: start.lerp(end, t).to_array(),
                texture: slot,
                size: [0.1, 0.1],
                flags: particle_flags::FOLLOW_VELOCITY,
                color,
                axis: (end - start).normalize_or_zero().to_array(),
                glow: 0.,
            });
        }
    }
}
