//! One-shot private upload phases. Viewer authority releases payment and Read
//! separately; no uploader URL or login secret appears in a native event.
use super::*;
use crate::inventory::upload::{
    Error, Request,
    assistant::{self, Committed, Offer},
};
enum Phase {
    Preparing,
    Offered(Offer),
    Posting,
    Committed(Committed),
    Verifying,
    Patched(Committed),
}
#[derive(Clone, Copy)]
struct Thumbnail {
    item: Uuid,
    folder: bool,
    parent: Uuid,
}

pub(super) struct Pending {
    started: Instant,
    owner: Uuid,
    fetch: String,
    phase: Phase,
    thumbnail: Option<Thumbnail>,
}
impl Pending {
    pub(super) fn expired(&self) -> bool {
        self.started.elapsed()
            > Duration::from_secs(if matches!(self.phase, Phase::Committed(_) | Phase::Patched(_)) {
                600
            } else {
                90
            })
    }
}
impl Session<'_> {
    pub(super) fn retire_assistant_uploads(&mut self) {
        self.asset_imports
            .assistant_uploads
            .lock()
            .retain(|_, p| !matches!(p.phase, Phase::Preparing | Phase::Offered(_)));
    }
    pub(super) fn create_assistant_upload_folder(&mut self, request: Uuid, folder: crate::inventory::InvFolder) {
        use crate::inventory::operations::{self, Mutation, Operation};
        if self.finished
            || self.logout_started.is_some()
            || request.is_nil()
            || folder.id.is_nil()
            || folder.parent.is_nil()
            || folder.type_default != -1
            || folder.name.is_empty()
            || folder.name.len() > 63
            || folder.name.chars().any(char::is_control)
            || self.asset_imports.assistant_upload_ids.len() >= 4096
            || !self.asset_imports.assistant_upload_ids.insert(request)
        {
            emit(
                self.sh,
                NetEvent::AssistantUploadFolderCreated {
                    request,
                    result: Err("invalid_upload_folder".into()),
                },
            );
            return;
        }
        let cap = self.main_cap("InventoryAPIv3");
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        let owner = self.agent_id();
        tokio::spawn(async move {
            let result = if let Some(cap) = cap {
                operations::mutate(
                    &http,
                    &cap,
                    "",
                    owner,
                    Mutation {
                        operations: vec![Operation::Folder {
                            id: folder.id,
                            parent: folder.parent,
                            name: folder.name.clone(),
                            type_default: -1,
                        }],
                        refresh: vec![folder.parent],
                        defer_folder_contents: true,
                        ..Default::default()
                    },
                )
                .await
            } else {
                Err("upload_folder_capability_unavailable".into())
            };
            let result = result.and_then(|reply| {
                let actual = reply
                    .created
                    .get(&folder.id)
                    .copied()
                    .ok_or_else(|| "upload_folder_unconfirmed".to_owned())?;
                reply
                    .folders
                    .into_iter()
                    .find(|f| f.id == actual && f.parent == folder.parent && f.name == folder.name && f.type_default == -1)
                    .ok_or_else(|| "upload_folder_unconfirmed".into())
            });
            let _ = events.send(NetEvent::AssistantUploadFolderCreated { request, result });
        });
    }
    pub(super) fn prepare_assistant_upload(&mut self, request: Uuid, upload: Request) {
        let error = if self.finished || self.logout_started.is_some() {
            Some(Error::Unavailable)
        } else if request.is_nil()
            || upload.body().is_err()
            || self.asset_imports.assistant_upload_ids.contains(&request)
            || self.asset_imports.assistant_upload_ids.len() >= 4096
            || self.asset_imports.assistant_uploads.lock().len() >= 32
        {
            Some(Error::Invalid)
        } else if self
            .asset_imports
            .assistant_uploads
            .lock()
            .values()
            .filter(|p| !matches!(p.phase, Phase::Committed(_)))
            .count()
            >= 4
        {
            Some(Error::Unavailable)
        } else {
            None
        };
        if let Some(error) = error {
            emit(
                self.sh,
                NetEvent::AssistantUploadReady {
                    request,
                    result: Err(error),
                },
            );
            return;
        }
        let Some((cap, fetch)) = self.main_cap("NewFileAgentInventory").zip(self.main_cap("FetchInventory2")) else {
            emit(
                self.sh,
                NetEvent::AssistantUploadReady {
                    request,
                    result: Err(Error::Unavailable),
                },
            );
            return;
        };
        self.asset_imports.assistant_upload_ids.insert(request);
        self.asset_imports.assistant_uploads.lock().insert(
            request,
            Pending {
                started: Instant::now(),
                owner: self.agent_id(),
                fetch,
                phase: Phase::Preparing,
                thumbnail: None,
            },
        );
        let pending = self.asset_imports.assistant_uploads.clone();
        let events = self.sh.events.clone();
        let http = self.sh.caps_http.clone();
        tokio::spawn(async move {
            let result = assistant::prepare(&http, &cap, upload).await;
            let result = match result {
                Ok(offer) => {
                    let cost = offer.cost();
                    let mut map = pending.lock();
                    if let Some(p) = map.get_mut(&request) {
                        if p.expired() || !matches!(p.phase, Phase::Preparing) {
                            Err(Error::Unavailable)
                        } else {
                            p.started = Instant::now();
                            p.phase = Phase::Offered(offer);
                            Ok(cost)
                        }
                    } else {
                        Err(Error::Unavailable)
                    }
                }
                Err(error) => {
                    pending.lock().remove(&request);
                    Err(error)
                }
            };
            let _ = events.send(NetEvent::AssistantUploadReady { request, result });
        });
    }
    pub(super) fn prepare_assistant_thumbnail(&mut self, request: Uuid, item: Uuid, folder: bool, parent: Uuid, data: Vec<u8>) {
        let valid = !self.finished
            && self.logout_started.is_none()
            && !request.is_nil()
            && !item.is_nil()
            && !parent.is_nil()
            && !data.is_empty()
            && data.len() <= 2 * 1024 * 1024
            && self.asset_imports.assistant_upload_ids.len() < 4096
            && !self.asset_imports.assistant_upload_ids.contains(&request)
            && self.asset_imports.assistant_uploads.lock().len() < 32
            && self
                .asset_imports
                .assistant_uploads
                .lock()
                .values()
                .filter(|p| !matches!(p.phase, Phase::Committed(_) | Phase::Patched(_)))
                .count()
                < 4;
        let caps = self.main_cap("InventoryThumbnailUpload").zip(self.main_cap("InventoryAPIv3"));
        let Some((cap, fetch)) = caps.filter(|_| valid) else {
            emit(
                self.sh,
                NetEvent::AssistantUploadReady {
                    request,
                    result: Err(Error::Unavailable),
                },
            );
            return;
        };
        self.asset_imports.assistant_upload_ids.insert(request);
        self.asset_imports.assistant_uploads.lock().insert(
            request,
            Pending {
                started: Instant::now(),
                owner: self.agent_id(),
                fetch,
                phase: Phase::Preparing,
                thumbnail: Some(Thumbnail { item, folder, parent }),
            },
        );
        let pending = self.asset_imports.assistant_uploads.clone();
        let events = self.sh.events.clone();
        let http = self.sh.caps_http.clone();
        tokio::spawn(async move {
            let result = assistant::prepare_thumbnail(&http, &cap, item, folder, data).await;
            let result = match result {
                Ok(offer) => {
                    let mut map = pending.lock();
                    if let Some(p) = map.get_mut(&request) {
                        if p.expired() || !matches!(p.phase, Phase::Preparing) {
                            Err(Error::Unavailable)
                        } else {
                            p.started = Instant::now();
                            p.phase = Phase::Offered(offer);
                            Ok(0)
                        }
                    } else {
                        Err(Error::Unavailable)
                    }
                }
                Err(e) => {
                    pending.lock().remove(&request);
                    Err(e)
                }
            };
            let _ = events.send(NetEvent::AssistantUploadReady { request, result });
        });
    }
    pub(super) fn assign_assistant_upload_thumbnail(&mut self, request: Uuid, item: Uuid, folder: bool, parent: Uuid, approved: bool) {
        if !approved || item.is_nil() || parent.is_nil() {
            return;
        }
        {
            let mut map = self.asset_imports.assistant_uploads.lock();
            let Some(p) = map.get_mut(&request) else { return };
            if p.expired() || p.owner != self.agent_id() || p.thumbnail.is_some() {
                return;
            }
            let Phase::Committed(done) = &p.phase else { return };
            if done.thumbnail || done.kind != crate::inventory::upload::Kind::Snapshot || done.cost > done.approved_ceiling {
                return;
            }
            p.thumbnail = Some(Thumbnail { item, folder, parent });
        }
        self.patch_assistant_thumbnail(request, true);
    }
    pub(super) fn patch_assistant_thumbnail(&mut self, request: Uuid, approved: bool) {
        if !approved || self.finished || self.logout_started.is_some() {
            return;
        }
        let next = {
            let mut map = self.asset_imports.assistant_uploads.lock();
            let Some(p) = map.get_mut(&request) else { return };
            if p.expired() || p.owner != self.agent_id() {
                return;
            }
            let Some(target) = p.thumbnail else { return };
            let Phase::Committed(done) = &p.phase else { return };
            if (done.thumbnail && (done.item != target.item || done.cost > 0))
                || (!done.thumbnail && done.cost > done.approved_ceiling)
                || done.asset.is_nil()
            {
                return;
            }
            let done = done.clone();
            p.phase = Phase::Posting;
            p.started = Instant::now();
            (done, target, p.fetch.clone())
        };
        let pending = self.asset_imports.assistant_uploads.clone();
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        tokio::spawn(async move {
            let result = assistant::patch_thumbnail(&http, &next.2, next.1.item, next.1.folder, next.0.asset)
                .await
                .map_err(|_| "native_thumbnail_patch_unconfirmed".to_owned());
            if result.is_ok() {
                if let Some(p) = pending.lock().get_mut(&request) {
                    p.phase = Phase::Patched(next.0);
                    p.started = Instant::now();
                }
            } else {
                pending.lock().remove(&request);
            }
            let _ = events.send(NetEvent::AssistantThumbnailPatched { request, result });
        });
    }
    pub(super) fn verify_assistant_thumbnail(&mut self, request: Uuid, approved: bool) {
        if !approved || self.finished || self.logout_started.is_some() {
            return;
        }
        let next = {
            let mut map = self.asset_imports.assistant_uploads.lock();
            let Some(p) = map.get_mut(&request) else { return };
            if p.expired() || p.owner != self.agent_id() {
                return;
            }
            let Some(target) = p.thumbnail else { return };
            let Phase::Patched(done) = &p.phase else { return };
            let done = done.clone();
            p.phase = Phase::Verifying;
            p.started = Instant::now();
            (done, target, p.fetch.clone(), p.owner)
        };
        let pending = self.asset_imports.assistant_uploads.clone();
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        let source = LoginSource::from_login(&self.login);
        tokio::spawn(async move {
            let result = assistant::verify_thumbnail(&http, &next.2, next.3, next.1.item, next.1.folder, next.1.parent, next.0.asset)
                .await
                .map_err(|_| "native_thumbnail_readback_unconfirmed".to_owned());
            pending.lock().remove(&request);
            let _ = events.send(NetEvent::AssistantThumbnailVerified { source, request, result });
        });
    }
    pub(super) fn finish_assistant_upload(&mut self, request: Uuid, approved: bool) {
        if self.finished || self.logout_started.is_some() {
            self.retire_assistant_uploads();
            emit(
                self.sh,
                NetEvent::AssistantUploadCommitted {
                    request,
                    result: Err(Error::Unavailable),
                },
            );
            return;
        }
        let offer = {
            let mut map = self.asset_imports.assistant_uploads.lock();
            let Some(p) = map.get_mut(&request) else {
                return;
            };
            if !matches!(p.phase, Phase::Offered(_)) {
                return;
            }
            if !approved || p.expired() || p.owner != self.agent_id() {
                map.remove(&request);
                return;
            }
            p.started = Instant::now();
            match std::mem::replace(&mut p.phase, Phase::Posting) {
                Phase::Offered(offer) => offer,
                _ => return,
            }
        };
        let pending = self.asset_imports.assistant_uploads.clone();
        let events = self.sh.events.clone();
        let http = self.sh.caps_http.clone();
        tokio::spawn(async move {
            let result = assistant::commit(&http, offer).await;
            if let Ok(committed) = &result
                && let Some(p) = pending.lock().get_mut(&request)
            {
                p.started = Instant::now();
                p.phase = Phase::Committed(committed.clone());
            } else if result.is_err() {
                pending.lock().remove(&request);
            }
            let _ = events.send(NetEvent::AssistantUploadCommitted { request, result });
        });
    }
    pub(super) fn verify_assistant_upload(&mut self, request: Uuid, approved: bool, retain_thumbnail: bool) {
        if self.finished || self.logout_started.is_some() {
            emit(
                self.sh,
                NetEvent::AssistantUploadVerified {
                    source: LoginSource::from_login(&self.login),
                    request,
                    result: Err(Error::Unavailable),
                },
            );
            return;
        }
        if !approved {
            return;
        }
        let next = {
            let mut map = self.asset_imports.assistant_uploads.lock();
            let Some(p) = map.get_mut(&request) else {
                return;
            };
            if p.expired() || p.owner != self.agent_id() || !matches!(p.phase, Phase::Committed(_)) {
                return;
            }
            p.started = Instant::now();
            match std::mem::replace(&mut p.phase, Phase::Verifying) {
                Phase::Committed(done) => (done, p.fetch.clone(), p.owner),
                _ => return,
            }
        };
        let pending = self.asset_imports.assistant_uploads.clone();
        let events = self.sh.events.clone();
        let http = self.sh.caps_http.clone();
        let source = LoginSource::from_login(&self.login);
        tokio::spawn(async move {
            let done = next.0.clone();
            let result = assistant::verify(&http, &next.1, next.2, next.0).await;
            if retain_thumbnail && result.is_ok() {
                if let Some(p) = pending.lock().get_mut(&request) {
                    p.phase = Phase::Committed(done);
                    p.started = Instant::now();
                }
            } else {
                pending.lock().remove(&request);
            }
            let _ = events.send(NetEvent::AssistantUploadVerified { source, request, result });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    fn offer(owner: Uuid, url: String) -> Pending {
        Pending {
            started: Instant::now(),
            owner,
            fetch: String::new(),
            thumbnail: None,
            phase: Phase::Offered(assistant::test_offer(
                Request {
                    kind: crate::inventory::upload::Kind::Animation,
                    folder: Uuid::from_u128(8800),
                    name: "Synthetic".into(),
                    description: String::new(),
                    permissions: Default::default(),
                    expected_cost: 10,
                    data: vec![1, 2, 3, 4],
                },
                url,
            )),
        }
    }
    #[tokio::test]
    async fn cancelled_expired_and_logout_queued_offers_cannot_start_payment() {
        for case in 0..4 {
            let sock = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
            sock.writable().await.unwrap();
            let simulator = UdpSocket::bind("127.0.0.1:0").await.unwrap();
            let main = simulator.local_addr().unwrap();
            let (events, received) = crossbeam_channel::unbounded();
            let shared = super::super::event_queue_tests::shared(crate::EventSender(events));
            let mut session = super::super::event_queue_tests::two_regions(&shared, sock, main, "127.0.0.1:9".parse().unwrap());
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let request = Uuid::new_v4();
            let mut pending = offer(session.agent_id(), format!("http://{}/paid", listener.local_addr().unwrap()));
            if case == 1 {
                pending.started = Instant::now() - Duration::from_secs(91);
            }
            session.asset_imports.assistant_uploads.lock().insert(request, pending);
            if case == 2 {
                session.on_command(NetCommand::Logout);
            }
            if case == 3 {
                session.finished = true;
            }
            session.on_command(NetCommand::FinishAssistantUpload {
                source: LoginSource::from_login(&session.login),
                request,
                approved: case != 0,
            });
            assert!(session.asset_imports.assistant_uploads.lock().is_empty());
            assert!(
                tokio::time::timeout(Duration::from_millis(40), listener.accept()).await.is_err(),
                "retired offer started payment, case {case}"
            );
            assert!(
                !received
                    .try_iter()
                    .any(|(_, event)| matches!(event, NetEvent::AssistantUploadCommitted { result: Ok(_), .. }))
            );
        }
    }
    #[tokio::test]
    async fn duplicate_native_release_consumes_the_offer_once_without_implicit_readback() {
        let sock = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let simulator = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let main = simulator.local_addr().unwrap();
        let (events, received) = crossbeam_channel::unbounded();
        let shared = super::super::event_queue_tests::shared(crate::EventSender(events));
        let mut session = super::super::event_queue_tests::two_regions(&shared, sock, main, "127.0.0.1:9".parse().unwrap());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let request = Uuid::new_v4();
        session.asset_imports.assistant_uploads.lock().insert(
            request,
            offer(session.agent_id(), format!("http://{}/paid", listener.local_addr().unwrap())),
        );
        session.finish_assistant_upload(request, true);
        session.finish_assistant_upload(request, true);
        let (mut stream, _) = tokio::time::timeout(Duration::from_secs(3), listener.accept())
            .await
            .unwrap()
            .unwrap();
        let mut bytes = Vec::new();
        loop {
            let mut part = [0; 4096];
            let n = stream.read(&mut part).await.unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&part[..n]);
            if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n")
                && bytes.len() >= end + 8
            {
                assert_eq!(&bytes[end + 4..end + 8], [1, 2, 3, 4]);
                break;
            }
        }
        let body = aurora_llsd::to_xml(
            &aurora_llsd::llsd_map! {"state"=>"complete","new_inventory_item"=>Uuid::from_u128(8801),"new_asset"=>Uuid::from_u128(8802)},
        );
        stream
            .write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes())
            .await
            .unwrap();
        stream.write_all(&body).await.unwrap();
        crate::test_http::finish(stream);
        let done = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Ok((_, NetEvent::AssistantUploadCommitted { result, .. })) = received.try_recv() {
                    break result.unwrap();
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(!done.cost_known);
        assert_eq!(done.approved_ceiling, 10);
        assert!(matches!(
            session.asset_imports.assistant_uploads.lock()[&request].phase,
            Phase::Committed(_)
        ));
        assert!(tokio::time::timeout(Duration::from_millis(40), listener.accept()).await.is_err());
    }
}
