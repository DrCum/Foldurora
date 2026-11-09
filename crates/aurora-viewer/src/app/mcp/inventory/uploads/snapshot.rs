//! Exact-target snapshot uploads. Captures share the assistant encoder budget;
//! binary upload, metadata assignment and readback require separate releases.
use super::*;
use crate::app::mcp::world;
#[derive(Default)]
pub(super) struct State {
    proofs: std::cell::RefCell<BTreeMap<Key, Quote>>,
    pending: Option<Job>,
    io: Option<(crossbeam_channel::Sender<Encoded>, crossbeam_channel::Receiver<Encoded>)>,
}
#[derive(Clone)]
struct Quote {
    target: Frozen,
    folder: Frozen,
    size: (u32, u32),
    dimensions: (u32, u32),
    texture: bool,
    cost: i32,
    name: String,
    permissions: Permissions,
}
struct Job {
    id: Uuid,
    capture: Capture,
    login: aurora_net::LoginSource,
    quote: Quote,
    started: Instant,
    phase: u8,
    encoder: Option<Arc<world::ImageEncoderLease>>,
    encoding: bool,
    charged: Option<i32>,
    cost_known: bool,
}
struct Encoded {
    id: Uuid,
    bytes: Result<Vec<u8>, &'static str>,
    _encoder: Arc<world::ImageEncoderLease>,
}
fn ui_allowed(app: &App, c: &Capture) -> bool {
    crate::app::snapshot::assistant_ui_capture(
        c.arguments.get("show_ui").and_then(Value::as_bool).unwrap_or(false),
        app.host_session.is_some(),
        || (),
    )
    .is_ok()
}
fn mutable(app: &App, id: Uuid) -> Result<(), &'static str> {
    let inv = &app.world.inventory;
    ordinary(app, id)?;
    if inv.owner != app.world.agent_id
        || !rules::mutable(inv, id)
        || rules::in_type(inv, id, 14)
        || !rules::under(inv, id, inv.root)
        || inv
            .items
            .get(&id)
            .is_some_and(|i| i.owner != app.world.agent_id || matches!(i.asset_type, 24 | 25))
    {
        return Err("native_permission_denied");
    }
    Ok(())
}
fn quote(app: &App, c: &Capture) -> Result<Quote, &'static str> {
    args(
        c,
        &[
            "id",
            "width",
            "height",
            "viewport_only",
            "destination",
            "name",
            "show_ui",
            "show_hud",
        ],
    )?;
    let id = uuid(&c.arguments, "id")?;
    mutable(app, id)?;
    world::capture_privacy(app)?;
    let texture = match c
        .arguments
        .get("destination")
        .map(|_| string(&c.arguments, "destination", 16))
        .transpose()?
        .unwrap_or("thumbnail")
    {
        "thumbnail" => false,
        "texture" => true,
        _ => return Err("invalid_args"),
    };
    if c.arguments.contains_key("width") != c.arguments.contains_key("height") {
        return Err("invalid_args");
    }
    let size = (
        number(&c.arguments, "width", 1024, 2048)? as u32,
        number(&c.arguments, "height", 1024, 2048)? as u32,
    );
    if size.0 < 64 || size.1 < 64 {
        return Err("invalid_args");
    }
    boolean(&c.arguments, "viewport_only", true)?;
    boolean(&c.arguments, "show_hud", false)?;
    let show = boolean(&c.arguments, "show_ui", false)?;
    if !ui_allowed(app, c) {
        return Err("native_composite_ui_capture_unavailable");
    }
    let gfx = app.gfx.as_ref().ok_or("native_renderer_unavailable")?;
    if gfx.renderer.vr_engaged() {
        return Err("snapshot_vr_unavailable");
    }
    if show && !crate::snapshot::same_proportions(size, gfx.renderer.size()) {
        return Err("snapshot_ui_aspect_mismatch");
    }
    let target = Frozen::capture(&app.world.inventory, id)?;
    let folder_id = if texture {
        rules::system(&app.world.inventory, 15)
            .or_else(|| rules::system(&app.world.inventory, 0))
            .ok_or("native_upload_destination_unavailable")?
    } else {
        target.parent()
    };
    destination(app, folder_id)?;
    let folder = Frozen::capture(&app.world.inventory, folder_id)?;
    let mut options = app.settings.imports.texture();
    options.max_width = if texture { 2048 } else { 256 };
    options.max_height = options.max_width;
    let dimensions = aurora_assets::import_texture::dimensions(size.0, size.1, options);
    let cost = if texture {
        prices(app)
            .cost(Kind::Snapshot, dimensions)
            .ok_or("native_upload_price_unavailable")?
    } else {
        0
    };
    if cost < 0 {
        return Err("native_upload_price_unavailable");
    }
    let default_name = target
        .item
        .as_ref()
        .map(|i| i.name.as_str())
        .or_else(|| target.folder.as_ref().map(|f| f.1.as_str()))
        .unwrap_or("Snapshot");
    let name = c
        .arguments
        .get("name")
        .map(|_| string(&c.arguments, "name", 63))
        .transpose()?
        .unwrap_or(default_name)
        .to_owned();
    if name.is_empty() || name.len() > 63 || name.chars().any(char::is_control) {
        return Err("invalid_asset_name");
    }
    if !app.world.main_has_cap("InventoryAPIv3")
        || !app.world.main_has_cap(if texture {
            "NewFileAgentInventory"
        } else {
            "InventoryThumbnailUpload"
        })
        || (texture && !app.world.main_has_cap("FetchInventory2"))
    {
        return Err("native_upload_capability_unavailable");
    }
    if cost > 0 && app.world.balance.is_none_or(|balance| balance < cost) {
        return Err("native_insufficient_balance");
    }
    Ok(Quote {
        target,
        folder,
        size,
        dimensions,
        texture,
        cost,
        name,
        permissions: app.settings.imports.permissions_for(Kind::Snapshot),
    })
}
fn proof_current(app: &App, q: &Quote, c: &Capture) -> bool {
    q.target.current(&app.world.inventory)
        && target_current(app, &q.folder)
        && mutable(app, q.target.id).is_ok()
        && destination(app, q.folder.id).is_ok()
        && ui_allowed(app, c)
        && world::capture_privacy(app).is_ok()
        && (!q.texture
            || prices(app)
                .cost(Kind::Snapshot, q.dimensions)
                .is_some_and(|cost| cost >= 0 && cost <= q.cost))
}
pub(super) fn prepare(app: &App, state: &super::super::State, c: &Capture) -> Result<Vec<Class>, &'static str> {
    let q = quote(app, c)?;
    if app.photo_busy() || state.uploads.snapshot.pending.is_some() || !world::image_encoder_available() {
        return Err("native_snapshot_busy");
    }
    let mut proofs = state.uploads.snapshot.proofs.borrow_mut();
    if let Some(old) = proofs.get(&key(c)) {
        if !proof_current(app, old, c)
            || old.texture != q.texture
            || old.size != q.size
            || old.name != q.name
            || old.dimensions != q.dimensions
            || q.cost > old.cost
        {
            return Err("native_snapshot_scope_changed");
        }
    } else {
        if proofs.len() >= CAPACITY {
            return Err("native_snapshot_capacity");
        }
        proofs.insert(key(c), q.clone());
    }
    let mut classes = vec![Class::Read, Class::Edit];
    if q.texture {
        classes.push(Class::Create);
    }
    Ok(classes)
}
pub(super) fn force_review(state: &super::super::State, c: &Capture) -> bool {
    state.uploads.snapshot.proofs.borrow().get(&key(c)).is_some_and(|q| q.cost > 0)
}
pub(super) fn approval(state: &super::super::State, c: &Capture) -> Value {
    state.uploads.snapshot.proofs.borrow().get(&key(c)).map_or_else(||failure("native_snapshot_scope_unavailable"),|q|json!({"target_id":q.target.id.to_string(),"target_name":q.target.item.as_ref().map(|i|i.name.as_str()).or_else(||q.target.folder.as_ref().map(|f|f.1.as_str())),"destination":if q.texture{"texture"}else{"thumbnail"},"capture_size":q.size,"encoded_size":q.dimensions,"approved_ceiling":q.cost,"creates_inventory_texture":q.texture,"no_automatic_retry":true}))
}
pub(super) fn execute(app: &mut App, state: &mut super::super::State, c: &Capture) -> Result<Value, &'static str> {
    prepare(app, state, c)?;
    let quote = state
        .uploads
        .snapshot
        .proofs
        .borrow()
        .get(&key(c))
        .cloned()
        .ok_or("native_snapshot_scope_unavailable")?;
    let encoder = world::acquire_image_encoder().ok_or("native_snapshot_busy")?;
    let id = Uuid::new_v4();
    let login = aurora_net::LoginSource::from_login(app.world.login.as_ref().ok_or("native_login_unavailable")?);
    let gfx = app.gfx.take().ok_or("native_renderer_unavailable")?;
    let result = app.photo_assistant_job(
        id,
        quote.size,
        boolean(&c.arguments, "show_ui", false)?,
        boolean(&c.arguments, "show_hud", false)?,
        boolean(&c.arguments, "viewport_only", true)?,
        &gfx.renderer,
    );
    app.gfx = Some(gfx);
    result.map_err(|_| "native_snapshot_busy")?;
    state.uploads.snapshot.pending = Some(Job {
        id,
        capture: c.clone(),
        login,
        quote,
        started: Instant::now(),
        phase: 0,
        encoder: Some(encoder),
        encoding: false,
        charged: None,
        cost_known: false,
    });
    Ok(submitted(c))
}
fn current(app: &App, a: &Assistant, p: &Job) -> bool {
    super::super::super::binding_matches(app, a, &p.capture)
        && a.runtime.capture_current(&p.capture)
        && a.runtime.configuration().as_ref() == Some(&app.settings.local_assistant)
        && app
            .world
            .login
            .as_ref()
            .is_some_and(|l| p.login.matches_login(l, app.world.agent_id))
        && proof_current(app, &p.quote, &p.capture)
        && p.started.elapsed() < Duration::from_secs(65)
}
pub(super) fn valid(app: &App, id: Uuid) -> bool {
    app.character_mcp
        .as_ref()
        .and_then(|a| a.inventory.uploads.snapshot.pending.as_ref().map(|p| (a, p)))
        .is_some_and(|(a, p)| p.id == id && p.phase == 0 && current(app, a, p))
}
pub(super) fn pixels(app: &mut App, id: Uuid, size: (u32, u32), rgba: Vec<u8>) {
    if !valid(app, id) {
        return;
    }
    let Some(a) = app.character_mcp.as_mut() else { return };
    let Some(p) = a.inventory.uploads.snapshot.pending.as_mut() else {
        return;
    };
    if p.encoding || p.quote.size != size || rgba.len() != size.0 as usize * size.1 as usize * 4 {
        return;
    }
    let Some(encoder) = p.encoder.clone() else { return };
    p.encoding = true;
    let mut options = app.settings.imports.texture();
    options.max_width = if p.quote.texture { 2048 } else { 256 };
    options.max_height = options.max_width;
    let maximum = if p.quote.texture { 64 * 1024 * 1024 } else { 2 * 1024 * 1024 };
    let (tx, _) = a.inventory.uploads.snapshot.io.get_or_insert_with(|| crossbeam_channel::bounded(1));
    let tx = tx.clone();
    app.net.runtime().spawn_blocking(move || {
        let bytes = encode_pixels(size, rgba, options, maximum);
        let _ = tx.try_send(Encoded {
            id,
            bytes,
            _encoder: encoder,
        });
    });
}
fn encode_pixels(
    size: (u32, u32),
    rgba: Vec<u8>,
    options: aurora_assets::import_texture::Options,
    maximum: usize,
) -> Result<Vec<u8>, &'static str> {
    if size.0 < 64 || size.1 < 64 || size.0 > 2048 || size.1 > 2048 || rgba.len() != size.0 as usize * size.1 as usize * 4 {
        return Err("snapshot_pixel_limit");
    }
    let source = aurora_assets::DecodedImage {
        width: size.0,
        height: size.1,
        components: 4,
        data: rgba,
        discard: 0,
    };
    aurora_assets::import_texture::prepare(&source, options)
        .and_then(|pixels| aurora_assets::import_texture::encode(&pixels, options))
        .map_err(|_| "snapshot_encode_failed")
        .and_then(|bytes| {
            if bytes.is_empty() || bytes.len() > maximum {
                Err("snapshot_encoded_limit")
            } else {
                Ok(bytes)
            }
        })
}
fn finish(app: &App, a: &mut Assistant, p: Job, value: Value) {
    let _ = receipts::record(app, &mut a.inventory, &p.capture, &value);
    outcome(&mut a.inventory, &p.capture, p.capture.request_id.clone(), value.clone());
    let _ = a.runtime.complete(&p.capture, value);
}
pub(super) fn tick(app: &mut App, a: &mut Assistant) {
    a.inventory
        .uploads
        .snapshot
        .proofs
        .borrow_mut()
        .retain(|k, _| a.runtime.has_operation(&k.2));
    let replies = a
        .inventory
        .uploads
        .snapshot
        .io
        .as_ref()
        .map(|(_, rx)| rx.try_iter().take(1).collect::<Vec<_>>())
        .unwrap_or_default();
    for encoded in replies {
        let Some(mut p) = a.inventory.uploads.snapshot.pending.take() else {
            continue;
        };
        if p.id != encoded.id {
            a.inventory.uploads.snapshot.pending = Some(p);
            continue;
        }
        p.encoder = None;
        if !current(app, a, &p) {
            finish(app, a, p, failure("native_snapshot_authority_retired"));
            continue;
        }
        let result = encoded.bytes.and_then(|data| {
            let command = if p.quote.texture {
                NetCommand::PrepareAssistantUpload {
                    source: p.login.clone(),
                    request: p.id,
                    upload: UploadRequest {
                        kind: Kind::Snapshot,
                        folder: p.quote.folder.id,
                        name: p.quote.name.clone(),
                        description: String::new(),
                        permissions: p.quote.permissions,
                        expected_cost: p.quote.cost,
                        data,
                    },
                }
            } else {
                NetCommand::PrepareAssistantThumbnail {
                    source: p.login.clone(),
                    request: p.id,
                    item: p.quote.target.id,
                    folder: p.quote.target.folder.is_some(),
                    parent: p.quote.target.parent(),
                    data,
                }
            };
            let runtime = a.runtime.clone();
            runtime.continue_action(&p.capture, || app.send(command))
        });
        if let Err(code) = result {
            finish(app, a, p, failure(code));
        } else {
            p.phase = 1;
            a.inventory.uploads.snapshot.pending = Some(p);
        }
    }
    if a.inventory.uploads.snapshot.pending.as_ref().is_some_and(|p| !current(app, a, p))
        && let Some(p) = a.inventory.uploads.snapshot.pending.take()
    {
        app.send(NetCommand::FinishAssistantUpload {
            source: p.login.clone(),
            request: p.id,
            approved: false,
        });
        finish(
            app,
            a,
            p,
            json!({"status":"uncertain_or_stopped","retry_safe":false,"new_phases_stopped":true}),
        );
    }
}
pub(super) fn owns(app: &App, request: Uuid) -> bool {
    app.character_mcp
        .as_ref()
        .and_then(|a| a.inventory.uploads.snapshot.pending.as_ref())
        .is_some_and(|p| p.id == request)
}
pub(super) fn native_event(app: &mut App, event: &NetEvent) -> bool {
    let request = match event {
        NetEvent::AssistantUploadReady { request, .. }
        | NetEvent::AssistantUploadCommitted { request, .. }
        | NetEvent::AssistantUploadVerified { request, .. }
        | NetEvent::AssistantThumbnailPatched { request, .. }
        | NetEvent::AssistantThumbnailVerified { request, .. } => *request,
        _ => return false,
    };
    // Accepted facts may update only their original native login; no continuation
    // effect or private MCP body is derived from this cache reconciliation.
    if let NetEvent::AssistantThumbnailVerified {
        source,
        result: Ok(receipt),
        ..
    } = event
        && app
            .world
            .login
            .as_ref()
            .is_some_and(|l| source.matches_login(l, app.world.agent_id))
    {
        match receipt {
            aurora_net::inventory::upload::assistant::ThumbnailReceipt::Item(i) => app.world.inventory.add_items(vec![(**i).clone()]),
            aurora_net::inventory::upload::assistant::ThumbnailReceipt::Folder(f) => app.world.inventory.created_folders(vec![f.clone()]),
        }
    }
    if !owns(app, request) {
        return matches!(
            event,
            NetEvent::AssistantThumbnailPatched { .. } | NetEvent::AssistantThumbnailVerified { .. }
        );
    }
    let Some(mut a) = app.character_mcp.take() else { return true };
    let Some(mut p) = a.inventory.uploads.snapshot.pending.take() else {
        app.character_mcp = Some(a);
        return true;
    };
    let runtime = a.runtime.clone();
    let authority = current(app, &a, &p);
    let mut done = None;
    let mut next = None;
    match event {
        NetEvent::AssistantUploadReady { result, .. } if p.phase == 1 => {
            if authority && result.as_ref().is_ok_and(|cost| *cost >= 0 && *cost <= p.quote.cost) {
                next = Some(NetCommand::FinishAssistantUpload {
                    source: p.login.clone(),
                    request,
                    approved: true,
                });
                p.phase = 2;
            } else {
                app.send(NetCommand::FinishAssistantUpload {
                    source: p.login.clone(),
                    request,
                    approved: false,
                });
                done = Some(failure("native_snapshot_upload_not_released"));
            }
        }
        NetEvent::AssistantUploadCommitted { result, .. } if p.phase == 2 => match result {
            Ok(receipt) if receipt.cost > p.quote.cost => {
                done = Some(
                    json!({"status":"unexpected_charge_assignment_stopped","charge_known":receipt.cost_known,"charged_cost":receipt.cost_known.then_some(receipt.cost),"approved_ceiling":p.quote.cost,"retry_safe":false,"assignment_confirmed":false}),
                );
            }
            Ok(receipt) if receipt.thumbnail != p.quote.texture && receipt.cost <= p.quote.cost => {
                p.charged = receipt.cost_known.then_some(receipt.cost);
                p.cost_known = receipt.cost_known;
                if authority {
                    next = Some(if p.quote.texture {
                        NetCommand::VerifyAssistantSnapshotTexture {
                            source: p.login.clone(),
                            request,
                            approved: true,
                        }
                    } else {
                        NetCommand::PatchAssistantThumbnail {
                            source: p.login.clone(),
                            request,
                            approved: true,
                        }
                    });
                    p.phase = if p.quote.texture { 3 } else { 4 };
                } else {
                    done = Some(
                        json!({"status":"uploaded_assignment_not_released","retry_safe":false,"charge_known":p.cost_known,"charged_cost":p.charged}),
                    );
                }
            }
            _ => done = Some(json!({"status":"upload_uncertain","retry_safe":false,"assignment_confirmed":false})),
        },
        NetEvent::AssistantUploadVerified { result, .. } if p.phase == 3 => {
            // The upload result has already been applied by its source-bound hook.
            if authority
                && result.as_ref().is_ok_and(|r| {
                    r.item.owner == app.world.agent_id
                        && r.item.parent == p.quote.folder.id
                        && crate::ui::inventory::thumbnail::image_source_allowed(&app.world.inventory, &r.item.asset_id, false, false)
                })
            {
                next = Some(NetCommand::AssignAssistantUploadThumbnail {
                    source: p.login.clone(),
                    request,
                    item: p.quote.target.id,
                    folder: p.quote.target.folder.is_some(),
                    parent: p.quote.target.parent(),
                    approved: true,
                });
                p.phase = 4;
            } else {
                done = Some(failure("native_snapshot_texture_unverified"));
            }
        }
        NetEvent::AssistantThumbnailPatched { result, .. } if p.phase == 4 => {
            if authority && result.is_ok() {
                next = Some(NetCommand::VerifyAssistantThumbnail {
                    source: p.login.clone(),
                    request,
                    approved: true,
                });
                p.phase = 5;
            } else {
                done = Some(
                    json!({"status":"assignment_unconfirmed","retry_safe":false,"charge_known":p.cost_known,"charged_cost":p.charged}),
                );
            }
        }
        NetEvent::AssistantThumbnailVerified { result, .. } if p.phase == 5 => {
            done = Some(
                json!({"status":if result.is_ok(){"confirmed"}else{"assignment_unconfirmed"},"assignment_confirmed":result.is_ok(),"inventory_texture_created":p.quote.texture,"charge_known":p.cost_known,"charged_cost":p.charged,"approved_ceiling":p.quote.cost,"retry_safe":false}),
            );
        }
        _ => {}
    }
    if let Some(command) = next
        && runtime.continue_action(&p.capture, || app.send(command)).is_err()
    {
        done = Some(failure("native_snapshot_authority_retired"));
    }
    if let Some(value) = done {
        finish(app, &mut a, p, value);
    } else {
        a.inventory.uploads.snapshot.pending = Some(p);
    }
    app.character_mcp = Some(a);
    true
}

pub(super) fn retire(app: &mut App, state: &mut State) {
    if let Some(p) = state.pending.take() {
        app.send(NetCommand::FinishAssistantUpload {
            source: p.login,
            request: p.id,
            approved: false,
        });
    }
    // A running encoder retains its own permit; dropping the receiver cannot
    // refund its live buffers and prevents an abandoned queued output lease.
    state.io = None;
    state.proofs.borrow_mut().clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn thumbnail_uses_native_jpeg2000_with_bounded_dimensions_and_validates_pixel_shape() {
        let options = aurora_assets::import_texture::Options {
            max_width: 256,
            max_height: 256,
            ..Default::default()
        };
        let bytes = encode_pixels((256, 1024), vec![128; 256 * 1024 * 4], options, 2 * 1024 * 1024).unwrap();
        let decoded = aurora_assets::decode_j2k(&bytes, 0).unwrap();
        assert_eq!((decoded.width, decoded.height), (64, 256));
        assert!(bytes.len() <= 2 * 1024 * 1024);
        assert_eq!(
            encode_pixels((64, 64), vec![0; 10], options, 2 * 1024 * 1024),
            Err("snapshot_pixel_limit")
        );
        assert_eq!(
            encode_pixels((64, 64), vec![128; 64 * 64 * 4], options, 1),
            Err("snapshot_encoded_limit")
        );
    }
}
