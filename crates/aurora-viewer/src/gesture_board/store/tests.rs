use super::*;
use crate::gesture_board::model::BoardId;
use aurora_session::GridKey;
use uuid::Uuid;
fn facts() -> Facts {
    Facts {
        agent: Uuid::from_u128(1),
        login: Uuid::from_u128(2),
        grid: GridKey::from_bytes([3; 32]).unwrap(),
        logical_generation: 1,
        profile_generation: 1,
        managed: None,
    }
}
fn create() -> Change {
    Change::Create {
        id: BoardId::try_from(Uuid::from_u128(4)).unwrap(),
        name: "Test".into(),
    }
}
#[test]
fn board_store_actual_decode_never_reinterprets_unknown_or_wrong_identity_as_missing() {
    let facts = facts();
    let empty = decode(None, facts).unwrap();
    assert!(empty.boards.is_empty());
    let bytes = empty.apply(create()).unwrap().encode().unwrap();
    let saved = bytes.clone();
    let mut unknown: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    unknown["version"] = 2.into();
    let unknown = serde_json::to_vec(&unknown).unwrap();
    assert!(matches!(
        decode(Some(&unknown), facts),
        Err(Error::Model(model::Error::Unsupported))
    ));
    let mut other = facts;
    other.agent = Uuid::from_u128(9);
    assert!(matches!(decode(Some(&bytes), other), Err(Error::Model(model::Error::Identity))));
    other = facts;
    other.grid = GridKey::from_bytes([9; 32]).unwrap();
    assert!(decode(Some(&bytes), other).is_err());
    assert!(decode(Some(b""), facts).is_err());
    assert!(decode(Some(b"{"), facts).is_err());
    assert_eq!(bytes, saved);
}
#[test]
fn board_store_actual_edit_preparation_requires_exact_original_and_keeps_prior_model() {
    let data = decode(None, facts()).unwrap();
    let (next, bytes) = prepare(&data, None, None, &create()).unwrap();
    assert!(data.boards.is_empty());
    assert_eq!(decode(Some(&bytes), facts()).unwrap(), next);
    let edit = Change::Rename {
        board: next.boards[0].id,
        name: "Next".into(),
    };
    assert!(matches!(prepare(&next, Some(&bytes), None, &edit), Err(Error::Conflict)));
    let mut changed = bytes.clone();
    changed.push(b' ');
    assert!(matches!(prepare(&next, Some(&changed), Some(&bytes), &edit), Err(Error::Conflict)));
    let (saved, encoded) = prepare(&next, Some(&bytes), Some(&bytes), &edit).unwrap();
    assert_eq!(saved.revision, 2);
    assert_eq!(next.boards[0].name, "Test");
    assert_eq!(decode(Some(&encoded), facts()).unwrap(), saved);
}
#[test]
fn board_store_cancelled_or_wrong_captured_directory_exits_before_private_io() {
    let facts = facts();
    let cancellation = NativeStoreCancellation::new();
    cancellation.cancel();
    // select derives a fixed private path only. perform returns before lock/read;
    // this fixture never creates or opens a real selected profile directory.
    let mut request = Request {
        owner: Owner {
            instance: 1,
            epoch: 1,
            facts,
        },
        operation: 1,
        directory: NativeDirectory::select(NativeStoreKind::Boards, facts.agent, &facts.grid.to_string()).unwrap(),
        cancellation,
        edit: None,
    };
    assert!(matches!(perform(&request), Err(Error::Cancelled)));
    request.directory = NativeDirectory::select(NativeStoreKind::Boards, Uuid::from_u128(7), &facts.grid.to_string()).unwrap();
    assert!(matches!(perform(&request), Err(Error::Invalid)));
    request.directory = NativeDirectory::select(NativeStoreKind::Workspaces, facts.agent, &facts.grid.to_string()).unwrap();
    assert!(matches!(perform(&request), Err(Error::Invalid)));
    request.owner.instance = 0;
    assert!(matches!(perform(&request), Err(Error::Invalid)));
}
#[test]
fn board_store_real_lane_methods_keep_one_inflight_exact_reply_and_no_backlog() {
    let facts = facts();
    let request = |operation| {
        let cancellation = NativeStoreCancellation::new();
        cancellation.cancel();
        Request {
            owner: Owner {
                instance: 1,
                epoch: 1,
                facts,
            },
            operation,
            directory: NativeDirectory::select(NativeStoreKind::Boards, facts.agent, &facts.grid.to_string()).unwrap(),
            cancellation,
            edit: None,
        }
    };
    let (send, requests) = mpsc::sync_channel(1);
    let (results, receive) = mpsc::sync_channel(1);
    let lane = Lane {
        send,
        receive,
        busy: Cell::new(false),
    };
    assert!(lane.try_submit(request(1)));
    assert!(!lane.try_submit(request(2)));
    let first = requests.try_recv().unwrap();
    assert_eq!(first.operation, 1);
    assert!(requests.try_recv().is_err());
    assert!(lane.poll().unwrap().is_none());
    results
        .try_send(Reply {
            owner: first.owner,
            operation: first.operation,
            result: Err(Error::Cancelled),
        })
        .unwrap_or_else(|_| panic!("empty result slot"));
    assert!(!lane.try_submit(request(3)));
    let reply = lane.poll().unwrap().unwrap();
    assert_eq!(reply.operation, 1);
    assert!(matches!(reply.result, Err(Error::Cancelled)));
    assert!(lane.try_submit(request(4)));
    assert_eq!(requests.try_recv().unwrap().operation, 4);
}
