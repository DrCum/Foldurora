use super::*;
use crate::preview_image::{ImageBudget, PreviewAccess, PreviewEpochs, PreviewGeometry, SourceImages};
use crate::{AccountKey, GridKey, HostId, InstallationId, SessionId, SessionIdentity, WorkerId, WorkerIdentity};

fn source() -> PreviewSource {
    PreviewSource::new(
        WorkerIdentity {
            installation: InstallationId::from_bytes([255; 32]).unwrap(),
            host: HostId::from_bytes([255; 16]).unwrap(),
            worker: WorkerId::from_bytes([255; 16]).unwrap(),
        },
        SessionIdentity {
            session: SessionId::from_bytes([255; 16]).unwrap(),
            account: AccountKey::from_bytes([255; 32]).unwrap(),
            grid: GridKey::from_bytes([255; 32]).unwrap(),
            generation: u64::MAX,
        },
        u64::MAX,
    )
    .unwrap()
}
fn policy() -> PreviewPolicy {
    PreviewPolicy::new(
        source(),
        PreviewEpochs::new(u64::MAX, u64::MAX, u64::MAX, u64::MAX, u64::MAX).unwrap(),
        PreviewAccess {
            ready: true,
            active: true,
            activation: u64::MAX,
            capture_active: true,
            retain_last_active: true,
        },
    )
    .unwrap()
}
fn claim(size: PreviewSize) -> CaptureClaim {
    let mut images = SourceImages::new(source(), ImageBudget::new(4 * MAX_IMAGE_BYTES).unwrap()).unwrap();
    images.sync(policy(), 1).unwrap();
    images
        .request(
            u64::MAX,
            u64::MAX,
            PreviewGeometry::new([1920, 1080], [0, 0, 1920, 1080], size).unwrap(),
            2,
        )
        .unwrap()
}
fn header(lane: Lane, direction: Direction, metadata: &[u8], pixels: usize) -> Header {
    Header::new(lane, direction, u64::MAX, u64::MAX, metadata.len(), pixels).unwrap()
}

#[test]
fn preview_wire_maximal_closed_metadata_and_full_stamp_fit_fixed_storage_without_pixels_serde() {
    let claim = claim(PreviewSize::Large);
    let messages = [
        Metadata::Policy {
            connection: u64::MAX,
            policy: policy(),
        },
        Metadata::Invalidate {
            source: source(),
            connection: u64::MAX,
        },
        Metadata::Demand {
            source: source(),
            connection: u64::MAX,
            demand: u64::MAX,
            size: PreviewSize::Large,
            rate: PreviewRate::Five,
        },
        Metadata::Capture(claim),
        Metadata::Stop {
            source: source(),
            connection: u64::MAX,
            demand: u64::MAX,
        },
        Metadata::Stopped {
            source: source(),
            connection: u64::MAX,
            demand: u64::MAX,
        },
        Metadata::Unavailable {
            source: source(),
            connection: u64::MAX,
            request: u64::MAX,
            reason: Unavailable::Unsupported,
        },
    ];
    for message in messages {
        let bytes = encode_metadata(message).unwrap();
        assert!(bytes.as_slice().len() <= MAX_METADATA_BYTES);
        let header = header(Lane::Metadata, message.direction(), bytes.as_slice(), 0);
        assert_eq!(decode_metadata(header, bytes.as_slice()).unwrap(), message);
        assert_eq!(Header::decode(&header.encode()).unwrap(), header);
    }
    let stamp = PreviewStamp::new(claim, u64::MAX, u64::MAX, u64::MAX, claim.geometry()).unwrap();
    let bytes = encode_stamp(stamp).unwrap();
    let header = header(Lane::Pixels, Direction::WorkerToHost, bytes.as_slice(), MAX_IMAGE_BYTES);
    assert_eq!(decode_stamp(header, bytes.as_slice()).unwrap(), stamp);
    assert!(header.frame_bytes() <= MAX_FRAME_BYTES);
    assert!(!String::from_utf8_lossy(bytes.as_slice()).contains("pixels"));
}

#[test]
fn preview_wire_rejects_oversized_versions_reserved_roles_directions_and_noncanonical_lengths() {
    let base = Header::new(Lane::Pixels, Direction::WorkerToHost, 1, 1, 100, MAX_IMAGE_BYTES)
        .unwrap()
        .encode();
    for mutation in 0..9 {
        let mut bytes = base;
        match mutation {
            0 => bytes[4] = 1,
            1 => bytes[32] = 1,
            2 => bytes[6] = 3,
            3 => bytes[7] = 1,
            4 => bytes[8..16].fill(0),
            5 => bytes[16..24].fill(0),
            6 => bytes[24..28].copy_from_slice(&u32::MAX.to_be_bytes()),
            7 => bytes[28..32].copy_from_slice(&u32::MAX.to_be_bytes()),
            8 => bytes[28..32].copy_from_slice(&65u32.to_be_bytes()),
            _ => unreachable!(),
        }
        assert!(Header::decode(&bytes).is_err(), "mutation {mutation}");
    }
    for prefix in 0..HEADER_BYTES {
        assert!(Header::decode(&base[..prefix]).is_err());
    }
    let mut extra = base.to_vec();
    extra.push(0);
    assert!(Header::decode(&extra).is_err());
    assert!(Header::new(Lane::Metadata, Direction::WorkerToHost, 1, 1, 100, 64).is_err());
}

#[test]
fn preview_wire_metadata_rejects_unknown_generic_payloads_binding_directions_and_zero_ids() {
    let message = Metadata::Invalidate {
        source: source(),
        connection: u64::MAX,
    };
    let bytes = encode_metadata(message).unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(bytes.as_slice()).unwrap();
    value["data"]["url"] = "ignored".into();
    let bad = serde_json::to_vec(&value).unwrap();
    assert_eq!(
        decode_metadata(header(Lane::Metadata, Direction::WorkerToHost, &bad, 0), &bad),
        Err(WireError::Invalid)
    );
    assert!(
        decode_metadata(
            header(Lane::Metadata, Direction::HostToWorker, bytes.as_slice(), 0),
            bytes.as_slice()
        )
        .is_err()
    );
    let wrong_connection = Header::new(Lane::Metadata, Direction::WorkerToHost, 1, 1, bytes.as_slice().len(), 0).unwrap();
    assert_eq!(decode_metadata(wrong_connection, bytes.as_slice()), Err(WireError::Binding));
    assert!(
        encode_metadata(Metadata::Stop {
            source: source(),
            connection: 1,
            demand: 0
        })
        .is_err()
    );
    assert!(
        encode_metadata(Metadata::Unavailable {
            source: source(),
            connection: 1,
            request: 0,
            reason: Unavailable::Busy
        })
        .is_err()
    );
}

#[test]
fn preview_wire_all_fit_caps_check_layout_exact_payload_and_forbidden_stamp_fields() {
    for cap in [PreviewSize::Small, PreviewSize::Medium, PreviewSize::Large] {
        let claim = claim(cap);
        let stamp = PreviewStamp::new(claim, 1, u64::MAX, 1, claim.geometry()).unwrap();
        let bytes = encode_stamp(stamp).unwrap();
        let good = header(
            Lane::Pixels,
            Direction::WorkerToHost,
            bytes.as_slice(),
            stamp.geometry().payload_bytes(),
        );
        assert_eq!(decode_stamp(good, bytes.as_slice()).unwrap(), stamp);
        let mismatch = Header::new(
            Lane::Pixels,
            Direction::WorkerToHost,
            u64::MAX,
            1,
            bytes.as_slice().len(),
            stamp.geometry().payload_bytes() - 4,
        )
        .unwrap();
        assert_eq!(decode_stamp(mismatch, bytes.as_slice()), Err(WireError::Invalid));
        for field in ["row_bytes", "pixels", "path", "url", "input"] {
            let mut value: serde_json::Value = serde_json::from_slice(bytes.as_slice()).unwrap();
            value[field] = 0.into();
            let invalid = serde_json::to_vec(&value).unwrap();
            assert!(
                decode_stamp(
                    header(Lane::Pixels, Direction::WorkerToHost, &invalid, stamp.geometry().payload_bytes()),
                    &invalid
                )
                .is_err()
            );
        }
        let mut value: serde_json::Value = serde_json::from_slice(bytes.as_slice()).unwrap();
        value["geometry"]["size"] = serde_json::json!([u32::MAX, 360]);
        let invalid = serde_json::to_vec(&value).unwrap();
        assert!(
            decode_stamp(
                header(Lane::Pixels, Direction::WorkerToHost, &invalid, stamp.geometry().payload_bytes()),
                &invalid
            )
            .is_err()
        );
    }
}

#[test]
fn preview_wire_maximally_escaped_bounded_metadata_decodes_and_overflow_rejects_before_serde() {
    let message = Metadata::Invalidate {
        source: source(),
        connection: u64::MAX,
    };
    let bytes = encode_metadata(message).unwrap();
    // Escaping every hex character exercises the largest identity strings;
    // punctuation/keys remain canonical so this valid fixture fits the bound.
    let text = std::str::from_utf8(bytes.as_slice()).unwrap();
    let escaped = text.replace('f', "\\u0066");
    assert!(escaped.len() <= MAX_METADATA_BYTES);
    assert_eq!(
        decode_metadata(
            header(Lane::Metadata, Direction::WorkerToHost, escaped.as_bytes(), 0),
            escaped.as_bytes()
        )
        .unwrap(),
        message
    );
    let huge = vec![b' '; MAX_METADATA_BYTES + 1];
    assert!(decode_metadata_value::<Metadata>(&huge).is_err());
    assert!(!format!("{bytes:?} {message:?}").contains(&"ff".repeat(32)));
}
