// SPDX-License-Identifier: LGPL-2.1-or-later
//! Strict native v8 absolute-rotation presets, Folderstorm fsposelistener.cpp
//! at abbe3cb0ca66ee2484f277219f6a27706fab7fc6. No position/scale/lock reset.
use glam::Quat;
use std::collections::BTreeMap;
pub(crate) type Rotations = BTreeMap<String, Quat>;
const FILE_LIMIT: usize = 65_536;
pub(crate) fn supported(name: &str) -> bool {
    name.len() > 1
        && name.len() <= 64
        && name.starts_with('m')
        && !matches!(name, "mRoot" | "mAvatarRoot" | "mPelvis")
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

pub(crate) fn parse(bytes: &[u8]) -> Result<Rotations, &'static str> {
    if bytes.is_empty() || bytes.len() > FILE_LIMIT || bytes.windows(2).any(|s| s == b"<!") {
        return Err("bounded_pose_xml_required");
    }
    let doc = aurora_llsd::from_xml(bytes).map_err(|_| "invalid_pose_xml")?;
    if doc["version"].as_map().is_none_or(|m| m.len() != 1)
        || doc["startFromTeePose"].as_map().is_none_or(|m| m.len() != 1)
        || !matches!(doc["version"]["value"], aurora_llsd::Llsd::Integer(8))
        || !matches!(doc["startFromTeePose"]["value"], aurora_llsd::Llsd::Boolean(false))
    {
        return Err("absolute_rotation_v8_required");
    }
    let mut out = Rotations::new();
    for (name, joint) in doc.as_map().ok_or("invalid_pose_xml")? {
        if matches!(name.as_str(), "version" | "startFromTeePose") {
            continue;
        }
        if !supported(name)
            || joint.as_map().ok_or("wider_native_pose_refused")?.keys().any(|key| {
                !matches!(
                    key.as_str(),
                    "rotation" | "enabled" | "jointBaseRotationIsZero" | "userSetBaseRotationToZero"
                )
            })
            || !matches!(joint["jointBaseRotationIsZero"], aurora_llsd::Llsd::Boolean(true))
            || (joint.has("enabled") && !matches!(joint["enabled"], aurora_llsd::Llsd::Boolean(true)))
            || (joint.has("userSetBaseRotationToZero") && !matches!(joint["userSetBaseRotationToZero"], aurora_llsd::Llsd::Boolean(true)))
        {
            return Err("wider_native_pose_refused");
        }
        let values = joint["rotation"].as_array();
        if values.len() != 3
            || values
                .iter()
                .any(|v| !matches!(v, aurora_llsd::Llsd::Integer(_) | aurora_llsd::Llsd::Real(_)))
        {
            return Err("invalid_pose_rotation");
        }
        let angles: Vec<_> = values.iter().map(|n| n.as_f64()).collect();
        if angles.iter().any(|v| !v.is_finite() || v.abs() > f64::from(std::f32::consts::PI)) {
            return Err("invalid_pose_rotation");
        }
        out.insert(
            name.clone(),
            Quat::from_euler(glam::EulerRot::ZYX, angles[2] as f32, angles[1] as f32, angles[0] as f32),
        );
    }
    if out.is_empty() || out.len() > 64 {
        return Err("rotation_joint_limit");
    }
    Ok(out)
}

pub(crate) fn encode(rotations: &Rotations) -> Vec<u8> {
    let mut doc =
        aurora_llsd::llsd_map! {"version"=>aurora_llsd::llsd_map!{"value"=>8},"startFromTeePose"=>aurora_llsd::llsd_map!{"value"=>false}};
    for (name, q) in rotations {
        let (z, y, x) = q.to_euler(glam::EulerRot::ZYX);
        doc.insert(name,aurora_llsd::llsd_map!{"rotation"=>aurora_llsd::Llsd::Array(vec![aurora_llsd::Llsd::Real(x as f64),aurora_llsd::Llsd::Real(y as f64),aurora_llsd::Llsd::Real(z as f64)]),"enabled"=>true,"jointBaseRotationIsZero"=>true,"userSetBaseRotationToZero"=>true});
    }
    aurora_llsd::to_xml(&doc)
}
