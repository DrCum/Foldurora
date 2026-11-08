use super::*;
use std::{
    collections::BTreeSet,
    time::{Duration, Instant},
};
use uuid::Uuid;

pub(super) struct DropObservation {
    used: Arc<AtomicUsize>,
    observed: Arc<AtomicUsize>,
    payload: std::sync::Weak<[u8]>,
}
impl Drop for DropObservation {
    fn drop(&mut self) {
        // Runs after the actual Resources channels/queued payloads drop,
        // immediately BEFORE Job's sole physical permit may be returned.
        assert_eq!(self.payload.strong_count(), 1);
        self.observed.store(self.used.load(Ordering::Acquire), Ordering::Release);
    }
}

fn source() -> Source {
    Source {
        account: Uuid::from_u128(100),
        session: Uuid::from_u128(101),
        root: Uuid::from_u128(1),
        grid: super::super::super::account_metadata_origin::GridDigest::from_uri("https://archive-store.example.invalid/login").unwrap(),
    }
}
fn intent(source: Source, expected: Document, folders: &[u128]) -> SaveIntent {
    SaveIntent {
        instance: 1,
        source,
        operation: 1,
        next: Document {
            account: source.account,
            grid: source.grid.bytes(),
            publication: expected.publication + 1,
            folders: Arc::new(folders.iter().copied().map(Uuid::from_u128).collect::<BTreeSet<_>>()),
        },
        expected,
    }
}
fn load(source: Source) -> Request {
    Request::Load(LoadTicket { instance: 1, source })
}
fn fixture(source: Source, pool: &WorkerPool) -> (Worker, Job) {
    let permit = pool.admit().unwrap();
    let directory = NativeDirectory::select(NativeStoreKind::InventoryArchive, source.account(), &source.grid().namespace()).unwrap();
    let (requests, incoming) = mpsc::sync_channel(1);
    let (outgoing, replies) = mpsc::sync_channel(1);
    let cancellation = NativeStoreCancellation::new();
    (
        Worker {
            requests,
            replies: Mutex::new(replies),
            cancellation: cancellation.clone(),
        },
        Job {
            resources: Resources {
                directory,
                source,
                cancellation,
                requests: incoming,
                replies: outgoing,
                entering: None,
                _dropping: None,
            },
            _permit: permit,
        },
    )
}
fn released(pool: &WorkerPool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while pool.0.load(Ordering::Acquire) != 0 && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert_eq!(pool.0.load(Ordering::Acquire), 0);
}

#[test]
fn inventory_archive_store_missing_is_distinct_from_empty_corrupt_unknown_or_wrong_source() {
    let source = source();
    assert!(decode(None, source).unwrap().is_none());
    for bytes in [b"".as_slice(), b"{", b"null"] {
        assert!(decode(Some(bytes), source).is_err());
    }
    let next = intent(source, Document::empty(source), &[2]).next;
    let original = next.encode().unwrap();
    assert_eq!(decode(Some(&original), source).unwrap(), Some(next));
    let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
    value["version"] = 2.into();
    assert!(decode(Some(&serde_json::to_vec(&value).unwrap()), source).is_err());
    value["version"] = 1.into();
    value["publication"] = 0.into();
    assert!(decode(Some(&serde_json::to_vec(&value).unwrap()), source).is_err());
    let mut other = source;
    other.account = Uuid::from_u128(999);
    assert_eq!(decode(Some(&original), other), Err(Error::Scope));
    assert!(decode(Some(&vec![0; MAX_BYTES + 1]), source).is_err());
    assert_eq!(original, decode(Some(&original), source).unwrap().unwrap().encode().unwrap());
}

#[test]
fn inventory_archive_store_preparation_checks_original_bytes_document_and_publication_aba() {
    let source = source();
    let first = intent(source, Document::empty(source), &[2]);
    let request = Request::Save {
        intent: first.clone(),
        original: None,
    };
    let bytes = prepare(&request, None).unwrap();
    assert_eq!(Document::decode(&bytes, source).unwrap(), first.next);
    assert_eq!(prepare(&request, Some(&bytes)), Err(Error::Stale));
    let second = intent(source, first.next.clone(), &[]);
    let request = Request::Save {
        intent: second.clone(),
        original: Some(Arc::from(bytes.clone())),
    };
    let b = prepare(&request, Some(&bytes)).unwrap();
    let third = intent(source, second.next.clone(), &[2]);
    let a_again = third.next.encode().unwrap();
    assert_eq!(first.next.folders, third.next.folders);
    assert_ne!(first.next.publication, third.next.publication);
    assert_eq!(prepare(&request, Some(&a_again)), Err(Error::Stale));
    let mut whitespace = bytes.clone();
    whitespace.push(b' ');
    assert_eq!(prepare(&request, Some(&whitespace)), Err(Error::Stale));
    let mut wrong_expected = second;
    wrong_expected.expected.folders = Arc::new(BTreeSet::new());
    assert_eq!(
        prepare(
            &Request::Save {
                intent: wrong_expected,
                original: Some(Arc::from(bytes.clone()))
            },
            Some(&bytes)
        ),
        Err(Error::Stale)
    );
    assert_eq!(Document::decode(&b, source).unwrap().publication(), 2);
    assert_eq!(bytes, first.next.encode().unwrap());
}

#[test]
fn inventory_archive_store_wrong_bound_kind_source_and_cancel_stop_before_private_io() {
    let source = source();
    let pool = WorkerPool(Arc::new(AtomicUsize::new(0)));
    let (worker, mut job) = fixture(source, &pool);
    let mut other = source;
    other.session = Uuid::from_u128(999);
    assert!(matches!(job.resources.perform(&load(other)), Err(Error::Scope)));
    job.resources.directory = NativeDirectory::select(NativeStoreKind::Boards, source.account(), &source.grid().namespace()).unwrap();
    assert!(matches!(job.resources.perform(&load(source)), Err(Error::Scope)));
    job.resources.directory =
        NativeDirectory::select(NativeStoreKind::InventoryArchive, source.account(), &source.grid().namespace()).unwrap();
    drop(worker);
    assert!(matches!(job.resources.perform(&load(source)), Err(Error::Unavailable)));
    assert_eq!(pool.0.load(Ordering::Acquire), 1);
    drop(job);
    released(&pool);
}

#[test]
fn inventory_archive_store_unstarted_owning_closure_drops_queued_storage_before_credit_refund() {
    let source = source();
    let pool = WorkerPool(Arc::new(AtomicUsize::new(0)));
    let (worker, mut job) = fixture(source, &pool);
    let bytes: Arc<[u8]> = Arc::from(vec![0; MAX_BYTES]);
    let observed = Arc::new(AtomicUsize::new(0));
    job.resources._dropping = Some(DropObservation {
        used: pool.0.clone(),
        observed: observed.clone(),
        payload: Arc::downgrade(&bytes),
    });
    worker
        .submit(Request::Save {
            intent: intent(source, Document::empty(source), &[2]),
            original: Some(bytes.clone()),
        })
        .unwrap();
    assert_eq!(Arc::strong_count(&bytes), 2);
    assert!(worker.submit(load(source)).is_err(), "the one request slot is already occupied");
    let run = move || job.run();
    // Same owning closure drop path used when thread spawn fails.
    drop(run);
    assert_eq!(pool.0.load(Ordering::Acquire), 0);
    assert_eq!(observed.load(Ordering::Acquire), 1);
    assert_eq!(Arc::strong_count(&bytes), 1);
    assert!(worker.submit(load(source)).is_err());
    assert!(worker.poll().is_err());
}

#[test]
fn inventory_archive_store_unwind_drops_actual_resources_while_physical_permit_is_still_charged() {
    let source = source();
    let pool = WorkerPool(Arc::new(AtomicUsize::new(0)));
    let (worker, mut job) = fixture(source, &pool);
    let bytes: Arc<[u8]> = Arc::from(vec![0; MAX_BYTES]);
    let observed = Arc::new(AtomicUsize::new(0));
    job.resources._dropping = Some(DropObservation {
        used: pool.0.clone(),
        observed: observed.clone(),
        payload: Arc::downgrade(&bytes),
    });
    worker
        .submit(Request::Save {
            intent: intent(source, Document::empty(source), &[2]),
            original: Some(bytes.clone()),
        })
        .unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let _owned = job;
        panic!("synthetic worker unwind before IO");
    }));
    assert!(result.is_err());
    assert_eq!(observed.load(Ordering::Acquire), 1);
    assert_eq!(pool.0.load(Ordering::Acquire), 0);
    assert_eq!(Arc::strong_count(&bytes), 1);
    assert!(worker.poll().is_err());
}

#[test]
fn inventory_archive_store_two_physical_retiring_workers_block_hundreds_of_fresh_starts() {
    let source = source();
    let pool = WorkerPool(Arc::new(AtomicUsize::new(0)));
    let order = IO_ORDER.lock().unwrap();
    let mut handles = Vec::new();
    let mut owners = Vec::new();
    for _ in 0..MAX_WORKERS {
        let (worker, mut job) = fixture(source, &pool);
        let (entered, receive) = mpsc::sync_channel(1);
        job.resources.entering = Some(entered);
        worker.submit(load(source)).unwrap();
        handles.push(std::thread::spawn(move || job.run()));
        receive.recv_timeout(Duration::from_secs(5)).unwrap();
        owners.push(worker);
    }
    assert_eq!(pool.0.load(Ordering::Acquire), 2);
    drop(owners); // Actual jobs remain blocked on IO_ORDER, not logically alive.
    for _ in 0..512 {
        assert!(Worker::start_in(source, &pool).unwrap().is_none());
        assert_eq!(pool.0.load(Ordering::Acquire), 2);
    }
    drop(order);
    for handle in handles {
        handle.join().unwrap();
    }
    released(&pool);
    let (worker, job) = fixture(source, &pool);
    drop(worker);
    drop(job);
    released(&pool);
}

#[test]
fn inventory_archive_store_idle_disconnect_releases_actual_worker_without_gui_join() {
    let source = source();
    let pool = WorkerPool(Arc::new(AtomicUsize::new(0)));
    let worker = Worker::start_in(source, &pool).unwrap().unwrap();
    assert_eq!(pool.0.load(Ordering::Acquire), 1);
    assert!(worker.poll().unwrap().is_none());
    drop(worker);
    released(&pool);
}

#[test]
fn inventory_archive_store_full_or_disconnected_reply_sink_is_terminal_without_blocking() {
    // perform rejects this mismatched source before IO; the real Job loop and
    // bounded try_send exit path still run, rather than a fake worker loop.
    for full in [false, true] {
        let source = source();
        let pool = WorkerPool(Arc::new(AtomicUsize::new(0)));
        let (worker, mut job) = fixture(source, &pool);
        if full {
            job.resources
                .replies
                .try_send(Reply::Loaded {
                    ticket: LoadTicket { instance: 9, source },
                    result: Err(Error::Invalid),
                })
                .unwrap_or_else(|_| panic!("empty reply slot"));
        } else {
            let (send, receive) = mpsc::sync_channel(1);
            drop(receive);
            job.resources.replies = send;
        }
        let mut other = source;
        other.session = Uuid::from_u128(999);
        worker.submit(load(other)).unwrap();
        let handle = std::thread::spawn(move || job.run());
        handle.join().unwrap();
        if full {
            assert!(matches!(
                worker.poll().unwrap(),
                Some(Reply::Loaded {
                    ticket: LoadTicket { instance: 9, .. },
                    ..
                })
            ));
            assert!(worker.poll().is_err());
        } else {
            assert!(worker.poll().is_err());
        }
        released(&pool);
    }
}

#[test]
fn inventory_archive_store_sync_receiver_try_poll_preserves_queued_reply_under_real_held_mutex() {
    fn assert_sync<T: Sync>() {}
    assert_sync::<Worker>();
    assert_sync::<crate::world::World>();
    let source = source();
    let ticket = LoadTicket { instance: 1, source };
    let (requests, _incoming) = mpsc::sync_channel(1);
    let (outgoing, replies) = mpsc::sync_channel(1);
    let worker = Worker {
        requests,
        replies: Mutex::new(replies),
        cancellation: NativeStoreCancellation::new(),
    };
    assert!(
        outgoing
            .try_send(Reply::Loaded {
                ticket,
                result: Err(Error::Invalid)
            })
            .is_ok()
    );
    let held = worker.replies.try_lock().unwrap();
    for _ in 0..64 {
        assert!(
            worker.poll().unwrap().is_none(),
            "contended real receiver is a nonconsuming try-only miss"
        );
    }
    assert!(matches!(
        outgoing.try_send(Reply::Loaded {
            ticket,
            result: Err(Error::Stale)
        }),
        Err(mpsc::TrySendError::Full(_))
    ));
    drop(held);
    assert!(matches!(worker.poll().unwrap(), Some(Reply::Loaded { ticket: original, result: Err(Error::Invalid) }) if original == ticket));
    assert!(worker.poll().unwrap().is_none());
    drop(outgoing);
    assert_eq!(worker.poll().err(), Some(Error::Unavailable));
}
