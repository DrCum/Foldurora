//! Genuine accepted-control read/cache fixtures. No endpoint, World or native
//! window is created; these tests establish view scope, not image permission.
use super::*;
use crate::session_host::monitor_model::{MonitorSet, Source};
use crate::session_host::monitor_reads::ViewReads;
use crate::session_host::transport::preview_control::fixtures::AcceptedControl;
use aurora_session::{HostId, InstallationId};

fn view_fixture(seen: Instant) -> (AcceptedControl, BrokerProjection, PreviewReader, ViewDescriptor, MonitorSet) {
    let reader = PreviewReader::new(Arc::new(Frontdoor::default()));
    let mut control = AcceptedControl::new(None);
    assert!(control.accept_status(1, seen));
    let proof = control.view().unwrap();
    let source = proof.source();
    let slot = SlotId::try_from(0).unwrap();
    let mut runtime = BrokerProjection::new(
        reader.clone(),
        InstallationId::from_bytes([1; 32]).unwrap(),
        HostId::from_bytes([2; 16]).unwrap(),
        false,
    );
    runtime
        .publish_control(
            vec![(
                ViewSource {
                    slot,
                    source,
                    available: true,
                },
                proof,
            )],
            Vec::new(),
            None,
        )
        .unwrap();
    let mut model = MonitorSet::new().unwrap();
    let local = Source::new(
        slot,
        aurora_session::chat::ChatSource::try_from((
            aurora_session::Binding {
                worker: source.worker(),
                session: Some(source.session()),
            },
            source.lease_epoch(),
        ))
        .unwrap(),
        true,
    )
    .unwrap();
    model
        .sync(&[local], std::array::from_fn(|at| SlotId::try_from(at as u8).unwrap()), None)
        .unwrap();
    let view = model.open_fixed(local).unwrap();
    let descriptor = ViewDescriptor {
        view,
        namespace: ViewNamespace::fresh().unwrap(),
        ticket: model.capture_view(view).unwrap(),
        source,
        visible: true,
        options: Options::default(),
    };
    let generation = reader.0.frontdoor.views_intent().unwrap();
    runtime.queue(
        1,
        RequestKind::Views {
            generation,
            views: vec![descriptor],
        },
        reader.reserve().unwrap(),
    );
    runtime.poll(&[]);
    (control, runtime, reader, descriptor, model)
}

#[test]
fn preview_host_none_bootstrap_and_image_mutex_busy_keep_genuine_image_free_view_without_command_fallback() {
    let (_control, _runtime, reader, descriptor, _model) = view_fixture(Instant::now());
    let read = reader.try_read(descriptor.view, descriptor.namespace).unwrap();
    assert!(read.view_current());
    assert_eq!(read.image_status(), ImageStatus::Unsupported);
    assert!(read.prepared().is_none());
    let mut cache = ViewReads::default();
    cache.remember(&read);
    let lock = reader.0.store.lock().unwrap();
    assert!(matches!(
        reader.try_read(descriptor.view, descriptor.namespace),
        Err(PreviewError::Busy)
    ));
    let body = cache.fallback(descriptor.view, descriptor.namespace).unwrap();
    assert!(body.view_current());
    assert!(!body.commands_current());
    assert!(!body.image_current());
    assert!(body.prepared().is_none());
    assert!(body.image_guard.is_none());
    assert!(body.settings_receipts().is_empty() && body.exchange_receipts().is_empty());
    drop(lock);
    let io_lock = reader.0.io.lock().unwrap();
    assert!(reader.try_read(descriptor.view, descriptor.namespace).unwrap().view_current());
    drop(io_lock);
}

#[test]
fn preview_host_stable_cached_view_survives_unrelated_head_renewal_but_not_source_owner_revoke() {
    let (mut control, runtime, reader, descriptor, _model) = view_fixture(Instant::now());
    let read = reader.try_read(descriptor.view, descriptor.namespace).unwrap();
    let mut cache = ViewReads::default();
    cache.remember(&read);
    assert!(control.accept_status(2, Instant::now()));
    let proof = control.view().unwrap();
    runtime
        .publish_control(
            vec![(
                ViewSource {
                    slot: SlotId::try_from(0).unwrap(),
                    source: proof.source(),
                    available: true,
                },
                proof,
            )],
            Vec::new(),
            None,
        )
        .unwrap();
    assert!(read.view_current(), "same genuine source keeps its original view witness");
    assert!(!read.commands_current(), "old generic head is never backend command authority");
    assert!(cache.fallback(descriptor.view, descriptor.namespace).unwrap().view_current());
    drop(control); // Actual source-owner Drop permanently revokes its witness.
    assert!(!read.view_current());
    assert!(cache.fallback(descriptor.view, descriptor.namespace).is_none());
}

#[test]
fn preview_host_cached_view_uses_original_control_expiry_even_after_fresh_accepted_heartbeat() {
    let seen = Instant::now() - std::time::Duration::from_secs(4);
    let (mut control, runtime, reader, descriptor, _model) = view_fixture(seen);
    let old = reader.try_read(descriptor.view, descriptor.namespace).unwrap();
    let mut cache = ViewReads::default();
    cache.remember(&old);
    assert!(control.accept_status(2, Instant::now()));
    let proof = control.view().unwrap();
    runtime
        .publish_control(
            vec![(
                ViewSource {
                    slot: SlotId::try_from(0).unwrap(),
                    source: proof.source(),
                    available: true,
                },
                proof,
            )],
            Vec::new(),
            None,
        )
        .unwrap();
    let fresh = reader.try_read(descriptor.view, descriptor.namespace).unwrap();
    let deadline = Instant::now() + std::time::Duration::from_secs(3);
    while old.view_current() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(fresh.view_current());
    assert!(
        cache.fallback(descriptor.view, descriptor.namespace).is_none(),
        "cache access never renews old proof"
    );
}

#[test]
fn preview_host_manager_recreation_cannot_reuse_local_binding_incarnation_as_view_namespace() {
    let (_control, mut runtime, reader, old, model) = view_fixture(Instant::now());
    let before = reader.try_read(old.view, old.namespace).unwrap();
    let old_binding = model.binding(old.view).unwrap();
    let mut replacement = MonitorSet::new().unwrap();
    replacement
        .sync(
            &[old.ticket.source()],
            std::array::from_fn(|at| SlotId::try_from(at as u8).unwrap()),
            None,
        )
        .unwrap();
    let view = replacement.open_fixed(old.ticket.source()).unwrap();
    assert_eq!(
        replacement.binding(view).unwrap().incarnation,
        old_binding.incarnation,
        "raw local serial can repeat"
    );
    let new = ViewDescriptor {
        namespace: ViewNamespace::fresh().unwrap(),
        ticket: replacement.capture_view(view).unwrap(),
        ..old
    };
    let generation = reader.0.frontdoor.views_intent().unwrap();
    runtime.queue(
        2,
        RequestKind::Views {
            generation,
            views: vec![new],
        },
        reader.reserve().unwrap(),
    );
    runtime.poll(&[]);
    assert!(!before.view_current());
    assert!(reader.try_read(view, old.namespace).is_err());
    assert!(reader.try_read(view, new.namespace).unwrap().view_current());
    let generation = reader.0.frontdoor.views_intent().unwrap();
    runtime.queue(
        3,
        RequestKind::Views {
            generation,
            views: vec![old],
        },
        reader.reserve().unwrap(),
    );
    runtime.poll(&[]);
    assert!(
        reader.try_read(view, old.namespace).is_err(),
        "even stale caller cannot resurrect retired namespace"
    );
}

#[test]
fn preview_host_process_ui_pool_is_shared_across_reader_recreation_and_actual_slot_admission() {
    let a = PreviewReader::new(Arc::new(Frontdoor::default()));
    let b = PreviewReader::new(Arc::new(Frontdoor::default()));
    let namespace = ViewNamespace::fresh().unwrap();
    let image = PrivateImage::from_rgba(&a.ui_budget(), [1, 1], &[0, 0, 0, 255]).unwrap();
    let mut slot =
        crate::session_host::private_image::PrivateImageSlot::new(egui::Context::default(), namespace.get(), b.ui_budget()).unwrap();
    slot.stage(namespace.get(), image).unwrap();
    // Actual stage rejects foreign pool Arcs; successful admission above proves
    // both independent readers inject the same process-wide credit owner.
    slot.retire();
}
