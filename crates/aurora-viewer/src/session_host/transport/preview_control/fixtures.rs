//! Test-only accepted-control driver. These tokens pass the SAME production
//! ChildPreview/Coordinator gate; no raw permission constructor or OS/GPU proof
//! is supplied. Synthetic control envelopes establish protocol/model scope only.
use super::*;

pub(in crate::session_host) struct AcceptedControl {
    config: WorkerConfig,
    coordinator: Coordinator,
    child: ChildPreview,
    binding: Binding,
    received: u64,
    seen: Option<Instant>,
    active: Option<ActiveViewport>,
    prepared: Option<(Binding, u64, Option<HostedViewport>)>,
    granted: Option<(Binding, u64)>,
}
impl AcceptedControl {
    pub(in crate::session_host) fn new(preview: Option<crate::session_host::preview_bootstrap::PreviewBootstrap>) -> Self {
        let config = WorkerConfig {
            version: PROTOCOL_VERSION,
            identity: WorkerIdentity {
                installation: InstallationId::from_bytes([1; 32]).unwrap(),
                host: HostId::from_bytes([2; 16]).unwrap(),
                worker: WorkerId::from_bytes([3; 16]).unwrap(),
            },
            session: SessionId::from_bytes([4; 16]).unwrap(),
            generation: 1,
            slot: Some(SlotId::try_from(0).unwrap()),
            expected_account: Some(AccountKey::from_bytes([5; 32]).unwrap()),
            expected_grid: Some(GridKey::from_bytes([6; 32]).unwrap()),
            lease_epoch: 7,
            host_pid: 1,
            chat: None,
            preview,
        };
        let mut coordinator = Coordinator::new(config.identity.installation, config.identity.host);
        coordinator.register(config.identity.worker, config.lease_epoch, 0).unwrap();
        let binding = Binding {
            worker: config.identity,
            session: Some(config.session_binding(config.expected_account.unwrap(), config.expected_grid.unwrap())),
        };
        Self {
            config,
            coordinator,
            child: ChildPreview::new(),
            binding,
            received: 0,
            seen: None,
            active: None,
            prepared: None,
            granted: None,
        }
    }
    pub(in crate::session_host) fn config(&self) -> &WorkerConfig {
        &self.config
    }
    pub(in crate::session_host) fn accept_status(&mut self, sequence: u64, seen: Instant) -> bool {
        let envelope = Envelope {
            version: PROTOCOL_VERSION,
            binding: self.binding,
            lease_epoch: self.config.lease_epoch,
            sequence,
            reply_to: 0,
            message: Message::Event(Event::Status(WorkerStatus {
                phase: Phase::Online,
                native_window: 12,
                ..Default::default()
            })),
        };
        if !self.config.accepts(&envelope) || self.coordinator.observe(&envelope, sequence).is_err() {
            return false;
        }
        self.received = sequence;
        self.seen = Some(seen);
        true
    }
    pub(in crate::session_host) fn source(&mut self) -> Option<AcceptedPreviewSource> {
        self.child.observe(
            &self.config,
            self.binding,
            self.received,
            self.seen,
            self.coordinator.workers().next(),
            true,
            Instant::now(),
        )
    }
    pub(in crate::session_host) fn view(&mut self) -> Option<AcceptedViewSource> {
        self.child.observe_view(
            &self.config,
            self.binding,
            self.received,
            self.seen,
            self.coordinator.workers().next(),
            true,
            Instant::now(),
        )
    }
    pub(in crate::session_host) fn activate(&mut self, viewport: HostedViewport) -> u64 {
        use aurora_session::{FrameProof, PrepareBoundary, WorkerGuard};
        let effects = self
            .coordinator
            .activate(self.config.identity.worker, Some(viewport), self.received + 1)
            .unwrap();
        let prepare = effects
            .into_iter()
            .find_map(|effect| {
                if let Effect::Send(envelope) = effect {
                    Some(envelope)
                } else {
                    None
                }
            })
            .unwrap();
        let Message::Command(Command::Prepare {
            activation,
            viewport: prepared_viewport,
        }) = &prepare.message
        else {
            panic!("fresh fixture must prepare once")
        };
        let activation = *activation;
        self.prepared = Some((prepare.binding, activation, *prepared_viewport));
        let mut worker = WorkerGuard::new(self.config.identity, self.config.lease_epoch, 0).unwrap();
        worker.update_session(self.binding.session).unwrap();
        worker.on_command(&prepare, 1).unwrap();
        worker
            .prepare_boundary(
                prepare.sequence,
                PrepareBoundary {
                    device_epoch: 1,
                    surface_epoch: 1,
                    after_frame: 0,
                },
            )
            .unwrap();
        let ready = worker
            .frame_ready(
                prepare.sequence,
                FrameProof {
                    device_epoch: 1,
                    surface_epoch: 1,
                    frame: 1,
                },
            )
            .unwrap()
            .unwrap();
        let response = Envelope {
            version: PROTOCOL_VERSION,
            binding: self.binding,
            lease_epoch: self.config.lease_epoch,
            sequence: self.received + 1,
            reply_to: ready.reply_to,
            message: Message::Event(ready.event),
        };
        let grant = self
            .coordinator
            .observe(&response, self.received + 2)
            .unwrap()
            .into_iter()
            .find_map(|effect| {
                if let Effect::Send(envelope) = effect {
                    Some(envelope)
                } else {
                    None
                }
            })
            .unwrap();
        self.received += 1;
        let reply = worker.on_command(&grant, self.received + 2).unwrap().reply.unwrap();
        let response = Envelope {
            version: PROTOCOL_VERSION,
            binding: self.binding,
            lease_epoch: self.config.lease_epoch,
            sequence: self.received + 1,
            reply_to: reply.reply_to,
            message: Message::Event(reply.event),
        };
        let effects = self.coordinator.observe(&response, self.received + 3).unwrap();
        assert!(
            effects
                .iter()
                .any(|effect| matches!(effect, Effect::Notice(HostNotice::Switched(worker)) if *worker == self.config.identity.worker))
        );
        self.received += 1;
        self.seen = Some(Instant::now());
        self.granted = Some((self.binding, activation));
        self.active = Some(ActiveViewport {
            worker: self.config.identity.worker,
            binding: self.binding,
            viewport,
            activation,
        });
        activation
    }
    pub(in crate::session_host) fn capture(
        &mut self,
        runtime: &BrokerProjection,
        frontdoor: Arc<Frontdoor>,
    ) -> Option<AcceptedPreviewCapture> {
        let source = self.source()?;
        if !runtime.capture_admitted(source.source()) || runtime.exchange_waiting() {
            return None;
        }
        capture_from_accepted(
            &source,
            frontdoor,
            CaptureFacts {
                coordinator: &self.coordinator,
                active: self.active,
                viewport: self.active.map(|active| active.viewport),
                prepared: self.prepared,
                granted: self.granted,
                status: self
                    .coordinator
                    .workers()
                    .find(|entry| entry.binding == self.binding)
                    .map(|entry| &entry.status),
            },
        )
    }
}
