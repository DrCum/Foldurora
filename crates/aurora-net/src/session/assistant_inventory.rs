//! Bounded native inventory reads with a separate assistant callback namespace.
//! The owner supplies a captured native account; callers never supply URLs.
use super::*;
use std::collections::HashSet;
const MAX_BODY: usize = 2 * 1024 * 1024;
struct Lease {
    reads: Arc<Mutex<HashSet<Uuid>>>,
    request: Uuid,
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.reads.lock().remove(&self.request);
    }
}
async fn read(request: reqwest::RequestBuilder) -> Result<Option<Llsd>, &'static str> {
    let mut response = request
        .header("Accept", "application/llsd+xml")
        .send()
        .await
        .map_err(|_| "native_inventory_read_failed")?;
    if matches!(response.status().as_u16(), 404 | 410) {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err("native_inventory_read_refused");
    }
    if response.content_length().is_some_and(|n| n > MAX_BODY as u64) {
        return Err("native_inventory_response_limit");
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "native_inventory_read_failed")? {
        if bytes.len().saturating_add(chunk.len()) > MAX_BODY {
            return Err("native_inventory_response_limit");
        }
        bytes.extend_from_slice(&chunk);
    }
    aurora_llsd::from_xml(&bytes)
        .map(Some)
        .map_err(|_| "native_inventory_response_invalid")
}
impl Session<'_> {
    fn assistant_read(&self, request: Uuid) -> Result<Lease, &'static str> {
        let reads = self.asset_imports.assistant_reads.clone();
        let mut active = reads.lock();
        if request.is_nil() || active.len() >= 4 || !active.insert(request) {
            return Err("native_inventory_read_capacity");
        }
        drop(active);
        Ok(Lease { reads, request })
    }
    pub(super) fn create_assistant_inventory_link(&mut self, request: Uuid, parent: Uuid, link: crate::outfits::OutfitLink) {
        if request.is_nil()
            || parent.is_nil()
            || link.target.is_nil()
            || link.name.len() > 255
            || link.desc.len() > 1024
            || self.assistant_creates.len() >= 128
        {
            emit(
                self.sh,
                NetEvent::AssistantInventoryLinkCreated {
                    request,
                    result: Err("native_link_not_submitted".into()),
                },
            );
            return;
        }
        if let Some(cap) = self.main_cap("InventoryAPIv3") {
            let lease = self.assistant_read(request);
            if lease.is_err() {
                emit(
                    self.sh,
                    NetEvent::AssistantInventoryLinkCreated {
                        request,
                        result: Err("native_link_not_submitted".into()),
                    },
                );
                return;
            }
            let http = self.sh.caps_http.clone();
            let events = self.sh.events.clone();
            tokio::spawn(async move {
                let _lease = lease;
                let result = tokio::time::timeout(Duration::from_secs(20), async {
                    let body = aurora_llsd::llsd_map! {"links"=>crate::outfits::links_body(&[link])};
                    let url = format!("{}/category/{parent}?tid={}", cap.trim_end_matches('/'), Uuid::new_v4());
                    let doc = read(
                        http.post(url)
                            .header("Content-Type", "application/llsd+xml")
                            .body(aurora_llsd::to_xml(&body)),
                    )
                    .await?
                    .ok_or("native_link_outcome_unknown")?;
                    let ids = doc["_created_items"].as_array();
                    if ids.len() != 1 || ids[0].as_uuid().is_nil() {
                        return Err("native_link_outcome_unknown");
                    }
                    Ok(ids[0].as_uuid())
                })
                .await
                .map_err(|_| "native_link_outcome_unknown")
                .and_then(|r| r)
                .map_err(str::to_owned);
                let _ = events.send(NetEvent::AssistantInventoryLinkCreated { request, result });
            });
        } else {
            let callback = self.inventory_callback(false);
            self.assistant_creates.insert(
                callback,
                super::asset_imports::AssistantCreate {
                    request,
                    started: Instant::now(),
                    parent,
                    asset_type: if link.folder { 25 } else { 24 },
                    asset: Some(link.target),
                    subtype: None,
                    link: true,
                },
            );
            let mut message = LinkInventoryItem::default();
            message.agent_data.agent_id = self.agent_id();
            message.agent_data.session_id = self.session_id();
            message.inventory_block.callback_id = callback;
            message.inventory_block.folder_id = parent;
            message.inventory_block.transaction_id = Uuid::new_v4();
            message.inventory_block.old_item_id = link.target;
            message.inventory_block.type_ = if link.folder { 25 } else { 24 };
            message.inventory_block.inv_type = link.inv_type as i8;
            message.inventory_block.name = str_field(&link.name);
            message.inventory_block.description = str_field(&link.desc);
            self.send_main(&message, true);
        }
    }
    pub(super) fn read_assistant_landmark(&mut self, request: Uuid, item: crate::inventory::InvItem) {
        let lease = self.assistant_read(request);
        let cap = self.main_cap("ViewerAsset");
        if item.asset_type != 3 || item.id.is_nil() || item.asset_id.is_nil() || lease.is_err() || cap.is_none() {
            emit(
                self.sh,
                NetEvent::AssistantLandmark {
                    request,
                    item: item.id,
                    asset: item.asset_id,
                    result: Err("native_landmark_read_unavailable".into()),
                },
            );
            return;
        }
        let base = cap.unwrap();
        let url = if base.contains('?') {
            format!("{base}&landmark_id={}", item.asset_id)
        } else {
            format!("{}/?landmark_id={}", base.trim_end_matches('/'), item.asset_id)
        };
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        tokio::spawn(async move {
            let _lease = lease;
            let result = tokio::time::timeout(Duration::from_secs(20), async {
                let mut response = http.get(url).send().await.map_err(|_| "native_landmark_read_failed")?;
                if !response.status().is_success() {
                    return Err("native_landmark_read_refused");
                }
                let mut bytes = Vec::new();
                while let Some(chunk) = response.chunk().await.map_err(|_| "native_landmark_read_failed")? {
                    if bytes.len().saturating_add(chunk.len()) > 4096 {
                        return Err("native_landmark_response_limit");
                    }
                    bytes.extend_from_slice(&chunk);
                }
                Ok(bytes)
            })
            .await
            .map_err(|_| "native_landmark_read_timeout")
            .and_then(|r| r)
            .map_err(str::to_owned);
            let _ = events.send(NetEvent::AssistantLandmark {
                request,
                item: item.id,
                asset: item.asset_id,
                result,
            });
        });
    }
    pub(super) fn fetch_assistant_inventory_folder(&mut self, request: Uuid, parent: Uuid, owner: Uuid, library: bool) {
        let lease = self.assistant_read(request);
        if parent.is_nil() || owner.is_nil() || (!library && owner != self.agent_id()) || lease.is_err() {
            emit(
                self.sh,
                NetEvent::AssistantInventoryFolder {
                    request,
                    parent,
                    result: Err("native_inventory_read_unavailable".into()),
                },
            );
            return;
        }
        let ais = self.main_cap(if library { "LibraryAPIv3" } else { "InventoryAPIv3" });
        let legacy = self.main_cap(if library {
            "FetchLibDescendents2"
        } else {
            "FetchInventoryDescendents2"
        });
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        tokio::spawn(async move {
            let _lease = lease;
            let result = tokio::time::timeout(Duration::from_secs(20), async {
                if let Some(cap) = ais {
                    let url = format!("{}/category/{parent}/children?depth=0", cap.trim_end_matches('/'));
                    let doc = read(http.get(url)).await?.ok_or("native_inventory_not_found")?;
                    return crate::inventory::ais::parse_folder(&doc, parent, owner)
                        .map(|c| vec![c])
                        .map_err(|_| "native_inventory_response_invalid");
                }
                let cap = legacy.ok_or("native_inventory_capability_unavailable")?;
                let body = crate::inventory::fetch_request_body(&[parent], owner);
                let doc = read(
                    http.post(cap)
                        .header("Content-Type", "application/llsd+xml")
                        .body(aurora_llsd::to_xml(&body)),
                )
                .await?
                .ok_or("native_inventory_not_found")?;
                let contents = crate::inventory::parse_fetch_response(&doc);
                if contents.len() != 1 || contents[0].folder_id != parent || contents[0].owner_id != owner {
                    return Err("native_inventory_response_invalid");
                }
                Ok(contents)
            })
            .await
            .map_err(|_| "native_inventory_read_timeout")
            .and_then(|r| r)
            .map_err(str::to_owned);
            let _ = events.send(NetEvent::AssistantInventoryFolder { request, parent, result });
        });
    }
    pub(super) fn fetch_assistant_inventory_items(&mut self, request: Uuid, items: Vec<Uuid>, owner: Uuid) {
        let lease = self.assistant_read(request);
        if items.is_empty() || items.len() > 50 || items.iter().any(Uuid::is_nil) || owner != self.agent_id() || lease.is_err() {
            emit(
                self.sh,
                NetEvent::AssistantInventoryItems {
                    request,
                    result: Err("native_inventory_read_unavailable".into()),
                },
            );
            return;
        }
        let ais = self.main_cap("InventoryAPIv3");
        let legacy = self.main_cap("FetchInventory2");
        let http = self.sh.caps_http.clone();
        let events = self.sh.events.clone();
        tokio::spawn(async move {
            let _lease = lease;
            let result = tokio::time::timeout(Duration::from_secs(20), async {
                if let Some(cap) = ais {
                    let mut reads = crate::inventory::ais::ItemReads::default();
                    for id in items {
                        let url = format!("{}/item/{id}", cap.trim_end_matches('/'));
                        match read(http.get(url)).await {
                            Ok(Some(doc)) => match crate::inventory::ais::parse_item_reply(&doc, id, owner) {
                                Ok(item) => reads.items.push(item),
                                Err(_) => reads.failed.push(id),
                            },
                            Ok(None) => reads.missing.push(id),
                            Err(_) => reads.failed.push(id),
                        }
                    }
                    return Ok(reads);
                }
                let cap = legacy.ok_or("native_inventory_capability_unavailable")?;
                let body = crate::inventory::fetch_items_body(&items, owner);
                let doc = read(
                    http.post(cap)
                        .header("Content-Type", "application/llsd+xml")
                        .body(aurora_llsd::to_xml(&body)),
                )
                .await?
                .ok_or("native_inventory_not_found")?;
                Ok(crate::inventory::item_reads(&doc, &items))
            })
            .await
            .map_err(|_| "native_inventory_read_timeout")
            .and_then(|r| r)
            .map_err(str::to_owned);
            let _ = events.send(NetEvent::AssistantInventoryItems { request, result });
        });
    }
}
