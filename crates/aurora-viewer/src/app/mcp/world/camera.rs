use super::*;
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicUsize, Ordering},
};
const MAX_ENCODERS: usize = 2;
#[derive(Clone, Default)]
struct EncoderPool(Arc<AtomicUsize>);
pub(in crate::app::mcp) struct EncoderLease(Arc<AtomicUsize>);
impl EncoderPool {
    fn available(&self) -> bool {
        self.0.load(Ordering::Acquire) < MAX_ENCODERS
    }
    fn acquire(&self) -> Option<Arc<EncoderLease>> {
        self.0
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| (n < MAX_ENCODERS).then_some(n + 1))
            .ok()?;
        Some(Arc::new(EncoderLease(self.0.clone())))
    }
}
impl Drop for EncoderLease {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
fn encoders() -> &'static EncoderPool {
    static POOL: OnceLock<EncoderPool> = OnceLock::new();
    POOL.get_or_init(EncoderPool::default)
}
pub(super) fn acquire_encoder() -> Option<Arc<EncoderLease>> {
    encoders().acquire()
}
pub(super) fn encoder_available() -> bool {
    encoders().available()
}
fn ui_allowed(app: &App, c: &Capture) -> bool {
    crate::app::snapshot::assistant_ui_capture(
        c.arguments.get("show_ui").and_then(Value::as_bool).unwrap_or(false),
        app.host_session.is_some(),
        || (),
    )
    .is_ok()
}

#[derive(Default)]
pub(super) struct State {
    pending: BTreeMap<Uuid, Pending>,
    encoded: Option<(crossbeam_channel::Sender<Encoded>, crossbeam_channel::Receiver<Encoded>)>,
}
#[derive(Clone)]
struct Pending {
    capture: Capture,
    started: Instant,
    size: (u32, u32),
    encoder: Arc<EncoderLease>,
}
struct Encoded {
    id: Uuid,
    size: (u32, u32),
    bytes: Result<Vec<u8>, &'static str>,
    preview: Option<Preview>,
    // Fields drop in declaration order: refund after image buffers, including
    // when a retired source drops a queued result without reading it.
    _encoder: Arc<EncoderLease>,
}
struct Preview {
    width: u32,
    height: u32,
    jpeg: Vec<u8>,
}
fn preview(size: (u32, u32), rgba: &[u8]) -> Option<Preview> {
    for (edge, quality) in [(192, 65), (128, 55), (64, 45)] {
        let (width, height, pixels) = crate::snapshot::save::downscale(size.0, size.1, rgba, edge);
        let jpeg = crate::snapshot::save::encode_jpeg(width, height, &pixels, quality).ok()?;
        if jpeg.len() <= aurora_mcp::assistant::runtime::MAX_CAMERA_PREVIEW_BYTES {
            return Some(Preview { width, height, jpeg });
        }
    }
    None
}
pub(super) fn supports(t: Tool) -> bool {
    matches!(
        t,
        Tool::CameraGet | Tool::CameraSetPose | Tool::CameraSet | Tool::CameraReset | Tool::CameraSnapshot
    )
}
fn movable(app: &App) -> Result<(), &'static str> {
    if app.world.rlva.camera_preset_locked(Uuid::nil())
        || app.world.rlva.camera_owner().is_some()
        || app.camera.flycam.is_some()
        || app.snapshots.ui.free.on
    {
        Err("native_camera_restriction")
    } else {
        Ok(())
    }
}
fn size(app: &App, c: &Capture) -> Result<(u32, u32), &'static str> {
    let a = &c.arguments;
    let max = count(a, "max_edge", 1024, 2048)?;
    if max < 64 {
        return Err("invalid_args");
    }
    let exact = match (a.get("width"), a.get("height")) {
        (None, None) => None,
        (Some(_), Some(_)) => {
            let w = count(a, "width", 0, 2048)?;
            let h = count(a, "height", 0, 2048)?;
            if w < 64 || h < 64 {
                return Err("invalid_args");
            }
            Some((w as u32, h as u32))
        }
        _ => return Err("invalid_args"),
    };
    let gfx = app.gfx.as_ref().ok_or("native_renderer_unavailable")?;
    let canvas = gfx.renderer.size();
    let viewport = boolean(a, "viewport_only", false)?;
    let source = if viewport {
        app.photo_scene_for_size(canvas, gfx.renderer.vr_engaged()).view.size()
    } else {
        canvas
    };
    if let Some(exact) = exact {
        Ok(exact)
    } else {
        let scale = (max as f64 / source.0.max(source.1).max(1) as f64).min(1.);
        Ok((
            ((source.0 as f64 * scale).round() as u32).max(64),
            ((source.1 as f64 * scale).round() as u32).max(64),
        ))
    }
}
pub(super) fn prepare(app: &App, state: &super::State, c: &Capture) -> Result<Vec<Class>, &'static str> {
    world_read(app)?;
    match c.tool {
        Tool::CameraGet => args(c, &[])?,
        Tool::CameraReset => {
            args(c, &[])?;
            movable(app)?;
        }
        Tool::CameraSet => {
            args(c, &["position", "focus"])?;
            movable(app)?;
            let p = vector(&c.arguments, "position")?.as_vec3();
            let f = vector(&c.arguments, "focus")?.as_vec3();
            let max = app.settings.draw_distance.max(64.);
            if !p.is_finite()
                || !f.is_finite()
                || p.distance(f) < 0.05
                || p.distance(app.world.agent.position) > max
                || f.distance(app.world.agent.position) > max
            {
                return Err("camera_pose_out_of_range");
            }
        }
        Tool::CameraSetPose => {
            args(c, &["preset"])?;
            movable(app)?;
            if !matches!(
                string(&c.arguments, "preset", 32)?,
                "portrait" | "full_body" | "front" | "back" | "left" | "right"
            ) {
                return Err("invalid_args");
            }
        }
        Tool::CameraSnapshot => {
            args(c, &["max_edge", "width", "height", "viewport_only", "show_ui", "show_hud"])?;
            let exact = size(app, c)?;
            boolean(&c.arguments, "show_ui", false)?;
            if !ui_allowed(app, c) {
                return Err("native_composite_ui_capture_unavailable");
            }
            boolean(&c.arguments, "show_hud", false)?;
            if app.photo_busy() || state.camera.pending.len() >= 2 || !encoders().available() {
                return Err("native_snapshot_busy");
            }
            if app.gfx.as_ref().is_some_and(|g| g.renderer.vr_engaged()) {
                return Err("snapshot_vr_unavailable");
            }
            if boolean(&c.arguments, "show_ui", false)? {
                let window = app.gfx.as_ref().ok_or("native_renderer_unavailable")?.renderer.size();
                if !crate::snapshot::same_proportions(exact, window) {
                    return Err("snapshot_ui_aspect_mismatch");
                }
            }
        }
        _ => return Err("unsupported_capability"),
    };
    Ok(vec![])
}
pub(super) fn evidence(app: &App, _: &super::State, c: &Capture) -> Result<Value, &'static str> {
    Ok(
        if matches!(
            c.tool,
            Tool::CameraSet | Tool::CameraSetPose | Tool::CameraReset | Tool::CameraSnapshot
        ) {
            json!({"agent":xyz(app.world.agent.position),"yaw":app.world.agent.yaw,"position":xyz(app.camera.position),"focus":xyz(app.camera.target),"fov":app.camera.fov_y,"viewport":app.gfx.as_ref().map(|g|g.renderer.size()),"snapshot_busy":app.photo_busy()})
        } else {
            Value::Null
        },
    )
}
pub(super) fn execute(app: &mut App, assistant: &mut Assistant, c: &Capture) -> Result<Value, &'static str> {
    match c.tool {
        Tool::CameraGet => Ok(
            json!({"position":xyz(app.camera.position),"focus":xyz(app.camera.target),"distance":app.camera.position.distance(app.camera.target),"fov":app.camera.fov_y,"mode":format!("{:?}",app.camera.mode).to_ascii_lowercase()}),
        ),
        Tool::CameraReset => {
            app.reset_camera_view();
            Ok(json!({"reset":true,"observed_final_pose":false}))
        }
        Tool::CameraSet | Tool::CameraSetPose => {
            let (position, focus) = if c.tool == Tool::CameraSet {
                (
                    vector(&c.arguments, "position")?.as_vec3(),
                    vector(&c.arguments, "focus")?.as_vec3(),
                )
            } else {
                let preset = string(&c.arguments, "preset", 32)?;
                let yaw = app.world.agent.yaw;
                let forward = Vec3::new(yaw.cos(), yaw.sin(), 0.);
                let left = Vec3::new(-yaw.sin(), yaw.cos(), 0.);
                let focus = app.world.agent.position + Vec3::Z * if preset == "portrait" { app.pelvis_to_foot() * 0.7 } else { 0. };
                let distance = if preset == "portrait" { 1.6 } else { 3.5 };
                let direction = match preset {
                    "portrait" | "full_body" | "front" => forward,
                    "back" => -forward,
                    "left" => left,
                    "right" => -left,
                    _ => return Err("invalid_args"),
                };
                (focus + direction * distance + Vec3::Z * 0.15, focus)
            };
            movable(app)?;
            let was = app.camera.mouselook();
            app.joystick.active = false;
            app.camera.clear_flycam();
            app.camera.set_view(position, focus, &mut app.world.agent, &app.settings.camera);
            if was {
                app.set_mouselook_grab(false);
            }
            Ok(json!({"requested_position":xyz(position),"requested_focus":xyz(focus),"submitted":true,"observed_final_pose":false}))
        }
        Tool::CameraSnapshot => {
            let size = size(app, c)?;
            // This lease belongs to actual work and its output, not its Actor
            // descriptor. Retiring a source cannot reopen a live encoder slot.
            let encoder = encoders().acquire().ok_or("native_snapshot_busy")?;
            let id = Uuid::new_v4();
            assistant.world.camera.pending.insert(
                id,
                Pending {
                    capture: c.clone(),
                    started: Instant::now(),
                    size,
                    encoder,
                },
            );
            let Some(gfx) = app.gfx.take() else {
                assistant.world.camera.pending.remove(&id);
                return Err("native_renderer_unavailable");
            };
            let result = app.photo_assistant_job(
                id,
                size,
                boolean(&c.arguments, "show_ui", false)?,
                boolean(&c.arguments, "show_hud", false)?,
                boolean(&c.arguments, "viewport_only", false)?,
                &gfx.renderer,
            );
            app.gfx = Some(gfx);
            if result.is_err() {
                assistant.world.camera.pending.remove(&id);
                return Err("native_snapshot_busy");
            }
            Ok(submitted(c))
        }
        _ => Err("unsupported_capability"),
    }
}
pub(super) fn valid(app: &App, id: Uuid) -> bool {
    app.character_mcp
        .as_ref()
        .and_then(|a| a.world.camera.pending.get(&id))
        .is_some_and(|p| {
            super::super::continuation_current(app, &p.capture)
                && ui_allowed(app, &p.capture)
                && world_read(app).is_ok()
                && p.started.elapsed() < Duration::from_secs(45)
        })
}
pub(super) fn pixels(app: &mut App, id: Uuid, size: (u32, u32), rgba: Vec<u8>) {
    if !valid(app, id) {
        return;
    }
    let Some(assistant) = app.character_mcp.as_mut() else { return };
    let Some(pending) = assistant.world.camera.pending.get(&id) else {
        return;
    };
    if pending.size != size || rgba.len() != size.0 as usize * size.1 as usize * 4 {
        return;
    }
    let encoder = pending.encoder.clone();
    let (tx, _) = assistant.world.camera.encoded.get_or_insert_with(|| crossbeam_channel::bounded(2));
    let tx = tx.clone();
    app.net.runtime().spawn_blocking(move || {
        let preview = preview(size, &rgba);
        let bytes = crate::snapshot::save::encode_jpeg(size.0, size.1, &rgba, 85)
            .map_err(|_| "snapshot_encode_failed")
            .and_then(|b| {
                if b.len() <= 8 * 1024 * 1024 {
                    Ok(b)
                } else {
                    Err("snapshot_size_limit")
                }
            });
        drop(rgba);
        let _ = tx.try_send(Encoded {
            _encoder: encoder,
            id,
            size,
            bytes,
            preview,
        });
    });
}
pub(super) fn tick(app: &mut App, assistant: &mut Assistant) {
    let done: Vec<_> = assistant
        .world
        .camera
        .encoded
        .as_ref()
        .map(|(_, rx)| rx.try_iter().take(2).collect())
        .unwrap_or_default();
    for encoded in done {
        let Some(p) = assistant.world.camera.pending.remove(&encoded.id) else {
            continue;
        };
        if !current(app, assistant, &p.capture) || !ui_allowed(app, &p.capture) || world_read(app).is_err() {
            continue;
        }
        let result = match encoded.bytes {
            Ok(bytes) => {
                let digest = hash(&bytes);
                let count = bytes.len();
                match assistant.runtime.install_artifact(&p.capture, encoded.id, bytes) {
                    Ok(()) => {
                        let inline = encoded.preview.is_some_and(|preview| {
                            assistant
                                .runtime
                                .install_camera_preview(&p.capture, encoded.id, preview.width, preview.height, preview.jpeg)
                                .is_ok()
                        });
                        json!({"artifact_id":encoded.id.to_string(),"mime_type":"image/jpeg","width":encoded.size.0,"height":encoded.size.1,"bytes":count,"sha256":digest,"private_path_included":false,"inline_preview_available":inline})
                    }
                    Err(code) => failure(code),
                }
            }
            Err(code) => failure(code),
        };
        let _ = assistant.runtime.complete(&p.capture, result);
    }
    let expired: Vec<_> = assistant
        .world
        .camera
        .pending
        .iter()
        .filter(|(_, p)| {
            p.started.elapsed() > Duration::from_secs(45) || !ui_allowed(app, &p.capture) || !current(app, assistant, &p.capture)
        })
        .map(|(id, _)| *id)
        .collect();
    for id in expired {
        if let Some(p) = assistant.world.camera.pending.remove(&id) {
            let _ = assistant.runtime.complete(&p.capture, failure("snapshot_expired"));
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inline_preview_is_a_real_bounded_native_jpeg() {
        let pixels = vec![128; 512 * 256 * 4];
        let preview = preview((512, 256), &pixels).unwrap();
        assert!(preview.width <= 192 && preview.height <= 192);
        assert!(preview.jpeg.len() <= aurora_mcp::assistant::runtime::MAX_CAMERA_PREVIEW_BYTES);
        assert!(preview.jpeg.starts_with(&[255, 216]) && preview.jpeg.ends_with(&[255, 217]));
    }
    #[test]
    fn retired_camera_descriptors_cannot_refund_live_or_queued_encoder_memory() {
        let pool = EncoderPool::default();
        let first = pool.acquire().unwrap();
        let second = pool.acquire().unwrap();
        let mut pending = BTreeMap::new();
        let id = Uuid::new_v4();
        pending.insert(
            id,
            Pending {
                capture: super::super::tests::capture(Tool::CameraSnapshot),
                started: Instant::now(),
                size: (64, 64),
                encoder: first.clone(),
            },
        );
        let worker = first.clone();
        drop(first);
        pending.clear();
        assert!(
            pool.acquire().is_none(),
            "retired Actor descriptor cannot refund worker-owned pixels"
        );
        let (tx, rx) = crossbeam_channel::bounded(1);
        assert!(
            tx.send(Encoded {
                _encoder: worker,
                id,
                size: (64, 64),
                bytes: Ok(vec![1; 1024]),
                preview: None,
            })
            .is_ok()
        );
        assert!(pool.acquire().is_none(), "queued output retains the same resource lease");
        drop(second);
        let reconnect = pool.acquire().unwrap();
        assert!(pool.acquire().is_none(), "one freed slot never refunds both jobs");
        drop(rx.recv().unwrap());
        let next = pool.acquire().unwrap();
        assert!(!pool.available());
        drop(next);
        drop(reconnect);
        assert!(pool.available());
    }
}
