use super::*;
#[derive(Default)]
pub(super) struct State {
    pending: Option<Job>,
    io: Option<(crossbeam_channel::Sender<Loaded>, crossbeam_channel::Receiver<Loaded>)>,
}
struct Job {
    id: Uuid,
    capture: Capture,
    login: aurora_net::LoginSource,
    folder: Frozen,
    started: Instant,
}
struct Loaded {
    id: Uuid,
    entries: ReaderResult<Result<Vec<crate::imports::Entry>, &'static str>>,
}
pub(super) fn prepare(app: &App, state: &super::super::State, c: &Capture) -> Result<Vec<Class>, &'static str> {
    args(c, &["path", "folder_id"])?;
    string(&c.arguments, "path", 4096)?;
    destination(app, uuid(&c.arguments, "folder_id")?)?;
    if !app.imports.assistant_mesh_idle() || state.uploads.mesh.pending.is_some() {
        return Err("native_mesh_importer_occupied");
    }
    if READER_BUSY.load(Ordering::Acquire) {
        return Err("native_upload_preparation_busy");
    }
    Ok(vec![Class::Read, Class::Create])
}
pub(super) fn execute(app: &mut App, state: &mut super::super::State, c: &Capture) -> Result<Value, &'static str> {
    prepare(app, state, c)?;
    let lease = ReaderLease::acquire().ok_or("native_upload_preparation_busy")?;
    let id = Uuid::new_v4();
    let folder = Frozen::capture(&app.world.inventory, uuid(&c.arguments, "folder_id")?)?;
    let login = aurora_net::LoginSource::from_login(app.world.login.as_ref().ok_or("native_login_unavailable")?);
    let path = string(&c.arguments, "path", 4096)?.to_owned();
    let options = app.settings.imports.texture();
    let prefs = app.settings.imports.clone();
    let wake = app.egui_ctx.clone();
    let (tx, _) = state.uploads.mesh.io.get_or_insert_with(|| crossbeam_channel::bounded(1));
    let tx = tx.clone();
    app.imports.assistant_mesh_begin(id, &app.egui_ctx)?;
    state.uploads.mesh.pending = Some(Job {
        id,
        capture: c.clone(),
        login,
        folder,
        started: Instant::now(),
    });
    app.scene.jobs.execute(move || {
        let entries = files::mesh(&path)
            .map_err(|_| "native_mesh_input_rejected")
            .and_then(|(path, bytes)| crate::imports::assistant::mesh(&path, bytes, options, &prefs));
        let _ = tx.try_send(Loaded {
            id,
            entries: ReaderResult {
                value: entries,
                _lease: lease,
            },
        });
        wake.request_repaint();
    });
    Ok(json!({"status":"submitted","operation_id":c.request_id,"upload_submitted":false,"manual_upload_required":true}))
}
fn current(app: &App, assistant: &Assistant, p: &Job) -> bool {
    super::super::super::binding_matches(app, assistant, &p.capture)
        && app
            .world
            .login
            .as_ref()
            .is_some_and(|login| p.login.matches_login(login, app.world.agent_id))
        && assistant.runtime.capture_current(&p.capture)
        && assistant.runtime.configuration().as_ref() == Some(&app.settings.local_assistant)
        && super::super::super::original_scope_current(app, assistant, &p.capture)
}
pub(super) fn tick(app: &mut App, assistant: &mut Assistant) {
    if assistant
        .inventory
        .uploads
        .mesh
        .pending
        .as_ref()
        .is_some_and(|p| p.started.elapsed() > Duration::from_secs(45) || !current(app, assistant, p))
        && let Some(p) = assistant.inventory.uploads.mesh.pending.take()
    {
        app.imports.assistant_mesh_cancel(p.id);
        let _ = assistant.runtime.complete(&p.capture, failure("native_mesh_review_scope_retired"));
    }
    let replies = assistant
        .inventory
        .uploads
        .mesh
        .io
        .as_ref()
        .map(|(_, rx)| rx.try_iter().take(1).collect::<Vec<_>>())
        .unwrap_or_default();
    for reply in replies {
        let Some(p) = assistant.inventory.uploads.mesh.pending.take() else {
            continue;
        };
        if p.id != reply.id {
            assistant.inventory.uploads.mesh.pending = Some(p);
            continue;
        }
        let result = if current(app, assistant, &p) && target_current(app, &p.folder) && destination(app, p.folder.id).is_ok() {
            match reply.entries.value {
                Ok(entries) => {
                    let runtime = assistant.runtime.clone();
                    runtime
                        .continue_action(&p.capture, || {
                            app.imports
                                .assistant_mesh_finish(p.id, p.folder.id, entries, &app.settings.imports, &app.settings.inventory)
                        })
                        .and_then(|result| result)
                }
                Err(code) => Err(code),
            }
        } else {
            Err("native_mesh_review_scope_retired")
        };
        let value = match result {
            Ok(()) => {
                json!({"status":"native_review_opened","manual_upload_required":true,"upload_submitted":false,"operation_id":p.capture.request_id})
            }
            Err(code) => {
                app.imports.assistant_mesh_cancel(p.id);
                failure(code)
            }
        };
        let _ = receipts::record(app, &mut assistant.inventory, &p.capture, &value);
        let _ = assistant.runtime.complete(&p.capture, value);
    }
}
pub(super) fn retire(app: &mut App, state: &mut State) {
    state.io = None;
    if let Some(p) = state.pending.take() {
        app.imports.assistant_mesh_cancel(p.id);
    }
}
