//! UI layout kept between sessions (the Legacy Viewer saves floater rects):
//! egui's memory (window positions and sizes, collapsed floaters) is written
//! to `ui_layout.ron` shortly after each change (`Autosave`) and on exit, and
//! restored at startup. Which panels are open lives in the settings.
//!
//! Each window retains one of eight invisible perimeter anchors and its
//! signed offset in the room where windows may move. Existing default rects
//! initialize these points and offsets directly; a window may supply an
//! explicit default point. First opening and completed moves otherwise choose
//! the closest corresponding corners or side midpoints, retained on resize.
//! `Slot` shares this placement with floaters, toolstrips and the mini-map.
//!
//! The slots also make the windows of the frame known to the gestures
//! (`gesture`): a window moved with Alt or resized snaps to the others and to the
//! work area (`snap`), and windows snapped side by side are glued: they
//! move as one block, saved with the layout too.

mod anchor;
mod gesture;

use super::snap;
use crate::theme::Palette;
use anchor::Anchor;
use egui::{Align2, Id, Pos2, Rect, Vec2};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

fn path() -> PathBuf {
    crate::settings::config_dir().join("ui_layout.ron")
}

/// Restore the saved layout. The style and options set by the skin are
/// kept (only positions, sizes and per-window state come from the file).
///
/// Call it before the theme and `fonts::apply`: egui keeps the transient
/// font scale and the font definitions that wait
/// for the next pass in the memory this replaces, so fonts installed before
/// it are dropped and the interface starts with egui's own.
pub fn load(ctx: &egui::Context) {
    let Ok(bytes) = std::fs::read(path()) else {
        return;
    };
    if let Err(e) = std::str::from_utf8(&bytes).map_err(|e| e.to_string()).and_then(|s| restore(ctx, s)) {
        log::warn!("ui layout ignored: {e}");
    }
}

fn restore(ctx: &egui::Context, saved: &str) -> Result<(), String> {
    let mem = ron::from_str::<egui::Memory>(saved).map_err(|e| e.to_string())?;
    let options = ctx.options(|o| o.clone());
    ctx.memory_mut(|m| *m = mem);
    ctx.options_mut(|o| *o = options);
    Ok(())
}

/// Save the layout now (on exit, after a restored backup).
pub fn save(ctx: &egui::Context) {
    let (generation, mem) = snapshot(ctx);
    write(generation, &mem);
}

/// The layout as `ui_layout.ron` holds it (and a settings backup).
pub fn text(ctx: &egui::Context) -> Option<String> {
    let (_, mem) = snapshot(ctx);
    ron::to_string(&mem).inspect_err(|e| log::warn!("ui layout not saved: {e}")).ok()
}

/// Order of the saves: each snapshot takes the next number, and the file
/// only takes a snapshot newer than the last one written (or than a reset),
/// so a background save finishing after the exit's never brings an older
/// layout back.
static ISSUED: AtomicU64 = AtomicU64::new(0);
static WORKSPACE_BARRIER: AtomicU64 = AtomicU64::new(0);
static WRITTEN: Mutex<Written> = Mutex::new(Written { generation: 0, text: None });

struct Written {
    generation: u64,
    /// What the file holds, to skip writing the same layout again.
    text: Option<String>,
}

/// The current layout, with the windows' anchors up to date. Cheap enough
/// for the frame (a clone); serializing it is left to `write`.
fn snapshot(ctx: &egui::Context) -> (u64, egui::Memory) {
    let room = move_area(ctx);
    windows(ctx, |w| w.refresh_anchors(room));
    let mem = ctx.memory(|m| m.clone());
    if let Some(baseline) = ctx.data(|d| d.get_temp::<WorkspacePreview>(Id::unique("workspace_preview_baseline"))) {
        // Only the existing legacy layout file serializes Memory. Named records
        // contain typed geometry; a temporary preview never leaks into this file.
        let saved = egui::Context::default();
        saved.memory_mut(|m| *m = mem);
        workspace_restore_at(&saved, &baseline, room, ctx.content_rect(), false);
        (next_generation(), saved.memory(|m| m.clone()))
    } else {
        (next_generation(), mem)
    }
}

pub(crate) type WorkspaceGeometry = [Option<crate::workspace::model::Geometry>; 4];
#[derive(Clone)]
struct WorkspacePreview {
    geometry: WorkspaceGeometry,
    initial: [super::widgets::WorkspaceGeometryState; 4],
    touched: [bool; 4],
}

/// Capture the four native geometry roles, including known closed windows.
pub(crate) fn workspace_capture(ctx: &egui::Context) -> WorkspaceGeometry {
    let room = move_area(ctx);
    windows(ctx, |w| w.refresh_anchors(room));
    std::array::from_fn(|i| workspace_named_capture_at(ctx, crate::workspace::model::Role::ALL[i].id()))
}
/// Native fixed-role callers only; serialized workspace data never chooses IDs.
pub(crate) fn workspace_named_capture(ctx: &egui::Context, name: &str) -> Option<crate::workspace::model::Geometry> {
    let room = move_area(ctx);
    windows(ctx, |w| w.refresh_anchors(room));
    workspace_named_capture_at(ctx, name)
}
fn workspace_named_capture_at(ctx: &egui::Context, name: &str) -> Option<crate::workspace::model::Geometry> {
    let id = Id::unique(name);
    let placed = windows(ctx, |w| w.placed.get(&id.value()).copied())?;
    let size = super::widgets::workspace_expanded_size(ctx, name).unwrap_or(placed.rect.size());
    let anchor = placed
        .anchor
        .unwrap_or_else(|| Anchor::nearest(placed.room.unwrap_or(placed.screen), placed.rect));
    Some(anchor.workspace(size))
}

/// Geometry only: no visibility, minimization, keyboard focus or editor data.
pub(crate) fn workspace_apply(ctx: &egui::Context, geometry: &WorkspaceGeometry) {
    workspace_touch(ctx, std::array::from_fn(|i| geometry[i].is_some()));
    let room = move_area(ctx);
    let screen = ctx.content_rect();
    workspace_apply_at(ctx, geometry, room, screen, true);
}
fn workspace_touch(ctx: &egui::Context, roles: [bool; 4]) {
    ctx.data_mut(|d| {
        let key = Id::unique("workspace_preview_baseline");
        if let Some(mut p) = d.get_temp::<WorkspacePreview>(key) {
            for (t, on) in p.touched.iter_mut().zip(roles) {
                *t |= on;
            }
            d.insert_temp(key, p);
        }
    });
}
fn workspace_clear(ctx: &egui::Context, index: usize, old: super::widgets::WorkspaceGeometryState, projection: bool) {
    let role = crate::workspace::model::Role::ALL[index];
    let key = Id::unique(role.id()).value();
    windows(ctx, |w| {
        w.placed.remove(&key);
        w.live.follow.remove(&key);
        w.live.places.remove(&key);
        w.live.settle.remove(&key);
        w.live.reanchor.remove(&key);
        w.live.reanchor_after_show.remove(&key);
        w.live.followed = None;
        w.live.moved = None;
        w.live.resized = None;
    });
    super::widgets::workspace_restore_geometry_state(ctx, role.id(), old, projection);
}
fn workspace_restore_at(ctx: &egui::Context, p: &WorkspacePreview, room: Rect, screen: Rect, constrain: bool) {
    let affected = std::array::from_fn(|i| p.touched[i].then_some(p.geometry[i]).flatten());
    workspace_apply_at(ctx, &affected, room, screen, constrain);
    for i in 0..4 {
        if p.touched[i] && p.geometry[i].is_none() {
            workspace_clear(ctx, i, p.initial[i], !constrain);
        }
    }
}
/// Restores absence as well as known geometry; no window is opened or focused.
pub(crate) fn workspace_restore(ctx: &egui::Context) {
    if let Some(p) = ctx.data(|d| d.get_temp::<WorkspacePreview>(Id::unique("workspace_preview_baseline"))) {
        workspace_restore_at(ctx, &p, move_area(ctx), ctx.content_rect(), true);
    }
}
/// Previous is a full live geometry capture. Its absent roles restore absence,
/// unlike an omitted role in a saved record, which is left alone.
pub(crate) fn workspace_previous(ctx: &egui::Context, geometry: &WorkspaceGeometry) {
    workspace_touch(ctx, [true; 4]);
    workspace_apply(ctx, geometry);
    for (i, g) in geometry.iter().enumerate() {
        if g.is_none() {
            workspace_clear(ctx, i, Default::default(), false);
        }
    }
}
/// Pointer/drag quarantine has no keyboard-focus, text, Undo or IME effect.
pub(crate) fn workspace_cancel_gesture(ctx: &egui::Context) {
    ctx.stop_dragging();
    ctx.input_mut(|i| i.pointer = Default::default());
}
fn workspace_apply_at(ctx: &egui::Context, geometry: &WorkspaceGeometry, room: Rect, screen: Rect, constrain: bool) {
    for (i, g) in geometry.iter().enumerate() {
        if let Some(g) = g {
            workspace_named_apply_at(ctx, crate::workspace::model::Role::ALL[i].id(), *g, room, screen, constrain);
        }
    }
}
pub(crate) fn workspace_named_apply(ctx: &egui::Context, name: &str, g: crate::workspace::model::Geometry) {
    if !g.valid() {
        return;
    }
    workspace_named_apply_at(ctx, name, g, move_area(ctx), ctx.content_rect(), true);
}
fn workspace_named_apply_at(
    ctx: &egui::Context,
    name: &str,
    g: crate::workspace::model::Geometry,
    room: Rect,
    screen: Rect,
    constrain: bool,
) {
    let Some(anchor) = Anchor::from_workspace(g) else { return };
    let id = Id::unique(name);
    let size = Vec2::from(g.size);
    let requested = Rect::from_min_size(anchor.position(room, size), size);
    let rect = if constrain {
        let size = size.min(room.size()).max(Vec2::splat(1.0));
        Rect::from_min_size(keep_title_in(room, requested.min, size, window_edges(ctx)), size)
    } else {
        requested
    };
    windows(ctx, |w| {
        w.placed.insert(
            id.value(),
            Placed {
                rect,
                screen,
                room: Some(room),
                anchor: Some(anchor),
            },
        );
        w.live.followed = None;
        w.live.moved = None;
        w.live.resized = None;
        w.live.places.insert(id.value(), rect.min);
        w.live.settle.remove(&id.value());
        w.live.reanchor.remove(&id.value());
        w.live.reanchor_after_show.remove(&id.value());
    });
    super::widgets::workspace_place(ctx, name, rect.min, size);
}

/// A generation barrier supersedes snapshots already queued before a preview.
/// Generic save/backup snapshots project the baseline while this hold is set.
pub(crate) fn workspace_preview(ctx: &egui::Context, baseline: Option<WorkspaceGeometry>) {
    let key = Id::unique("workspace_preview_baseline");
    match baseline {
        Some(geometry) => {
            if ctx.data(|d| d.get_temp::<WorkspacePreview>(key).is_none()) {
                let initial =
                    std::array::from_fn(|i| super::widgets::workspace_geometry_state(ctx, crate::workspace::model::Role::ALL[i].id()));
                ctx.data_mut(|d| {
                    d.insert_temp(
                        key,
                        WorkspacePreview {
                            geometry,
                            initial,
                            touched: [false; 4],
                        },
                    )
                });
            }
        }
        None => ctx.data_mut(|d| d.remove::<WorkspacePreview>(key)),
    }
    // Never wait for the legacy writer's mutex/fsync on the App thread.
    WORKSPACE_BARRIER.fetch_max(next_generation(), Ordering::AcqRel);
}

fn next_generation() -> u64 {
    ISSUED.fetch_add(1, Ordering::Relaxed) + 1
}

fn write(generation: u64, mem: &egui::Memory) {
    write_to_guarded(&WRITTEN, &path(), generation, mem, Some(&WORKSPACE_BARRIER));
}

#[cfg(test)]
fn write_to(state: &Mutex<Written>, p: &Path, generation: u64, mem: &egui::Memory) {
    write_to_guarded(state, p, generation, mem, None);
}
fn write_to_guarded(state: &Mutex<Written>, p: &Path, generation: u64, mem: &egui::Memory, barrier: Option<&AtomicU64>) {
    if barrier.is_some_and(|b| generation <= b.load(Ordering::Acquire)) {
        return;
    }
    let text = match ron::to_string(mem) {
        Ok(text) => text,
        Err(e) => {
            log::warn!("ui layout not saved: {e}");
            return;
        }
    };
    let mut written = state.lock().unwrap_or_else(PoisonError::into_inner);
    if generation <= written.generation {
        return;
    }
    written.generation = generation;
    if written.text.as_deref() == Some(text.as_str()) {
        return;
    }
    if let Some(dir) = p.parent()
        && let Err(e) = std::fs::create_dir_all(dir)
    {
        log::warn!("ui layout not saved: {e}");
        return;
    }
    // write then rename: never leave a half-written file
    let result = match barrier {
        Some(b) => crate::atomic_file::write_if(p, text.as_bytes(), || generation > b.load(Ordering::Acquire)),
        None => crate::atomic_file::write(p, text.as_bytes()).map(|()| true),
    };
    match result {
        Ok(true) => {
            if barrier.is_some() {
                crate::disk_io::written(crate::disk_io::Origin::Other, text.len());
            }
            written.text = Some(text);
        }
        Ok(false) => {}
        Err(e) => log::warn!("ui layout not saved: {e}"),
    }
}

/// How long the layout must stay still before it is saved: a window moved
/// several times in a row is written once.
const AUTOSAVE_DELAY: Duration = Duration::from_secs(2);

/// Saves the layout shortly after the user changes it (a window moved,
/// resized or collapsed, a column widened, the viewer window resized), on a
/// background thread: a crash or a forced close no longer loses the
/// arrangement of the session (#1319). The Legacy Viewer keeps the floater
/// rects in its settings and writes them at exit only
/// (LLFloater::storeRectControl, indra/llui/llfloater.cpp;
/// LLAppViewer::cleanup, indra/newview/llappviewer.cpp): a clean exit gives
/// the same file, an abrupt one no longer loses it.
#[derive(Default)]
pub struct Autosave {
    due: Option<Instant>,
    room: Option<Rect>,
}

impl Autosave {
    /// Explicit workspace completion uses the ordinary debounced saver, never
    /// a synchronous file flush from an egui gesture.
    pub(crate) fn request(&mut self, now: Instant) {
        self.due = Some(now + AUTOSAVE_DELAY);
    }
    /// Call it after each egui pass.
    pub fn after_pass(&mut self, ctx: &egui::Context, now: Instant) {
        if ctx.data(|d| d.get_temp::<WorkspacePreview>(Id::unique("workspace_preview_baseline")).is_some()) {
            self.due = None;
            return;
        }
        let room = move_area(ctx);
        let released = ctx.input(|i| i.pointer.any_released()) && (ctx.is_pointer_over_egui() || ctx.egui_is_using_pointer());
        let resized = self.room.is_some_and(|r| r != room);
        self.room = Some(room);
        if self.mark(released || resized, now) {
            let (generation, mem) = snapshot(ctx);
            let spawned = std::thread::Builder::new()
                .name("aurora-layout-save".into())
                .spawn(move || write(generation, &mem));
            if let Err(e) = spawned {
                log::warn!("ui layout not saved: {e}");
            }
        }
    }

    /// Whether the layout is due for a save this frame: `changed` pushes the
    /// save back by `AUTOSAVE_DELAY`.
    fn mark(&mut self, changed: bool, now: Instant) -> bool {
        if changed {
            self.due = Some(now + AUTOSAVE_DELAY);
        }
        if self.due.is_some_and(|due| now >= due) {
            self.due = None;
            return true;
        }
        false
    }
}

/// Takes a layout from a settings backup, at once and for the next
/// launches.
pub fn apply(ctx: &egui::Context, saved: &str) -> Result<(), String> {
    restore(ctx, saved)?;
    save(ctx);
    Ok(())
}

/// Forget the saved layout ("Reset everything").
pub fn forget(ctx: &egui::Context) {
    discard(&WRITTEN, &path(), next_generation());
    let options = ctx.options(|o| o.clone());
    ctx.memory_mut(|m| *m = egui::Memory::default());
    ctx.options_mut(|o| *o = options);
}

/// Remove the file; a background save still on its way, older than
/// `generation`, does not write it back.
fn discard(state: &Mutex<Written>, p: &Path, generation: u64) {
    let mut written = state.lock().unwrap_or_else(PoisonError::into_inner);
    written.generation = written.generation.max(generation);
    written.text = None;
    let _ = std::fs::remove_file(p);
}

/// Room left between a window and the bars or the edges of the screen, and
/// between two windows that stand side by side, for the windows of a first
/// launch as for the ones the user snaps (`snap`): none, their borders
/// touch, as Firestorm's snapped floaters do (findSnapRect is called
/// without padding).
pub(crate) const GAP: f32 = 0.0;

fn canvas_id() -> Id {
    Id::unique("layout_interface_canvas")
}

fn canvas_bounds_id() -> Id {
    Id::unique("layout_interface_bounds")
}

pub(crate) fn canvas_matches(ctx: &egui::Context, expected: Rect) -> bool {
    let actual = ctx.viewport_rect();
    let tolerance = 1.0 / ctx.pixels_per_point();
    (actual.min - expected.min).abs().max_elem() <= tolerance && (actual.max - expected.max).abs().max_elem() <= tolerance
}

/// Called at the start of the root UI pass, after egui adopts its logical
/// canvas. Existing perimeter anchors follow it without resetting layouts.
/// The last frame's bar room cannot place early floaters on the old monitor.
pub(crate) fn begin_canvas(ctx: &egui::Context, restricted: bool) -> bool {
    let screen = ctx.content_rect();
    let previous = ctx.data(|d| d.get_temp::<(Rect, bool)>(canvas_id()));
    let changed = previous != Some((screen, restricted)) && (restricted || previous.is_some_and(|(_, on)| on));
    ctx.data_mut(|d| {
        d.insert_temp(canvas_id(), (screen, restricted));
        d.insert_temp(canvas_bounds_id(), restricted.then_some(screen));
    });
    if changed {
        // Hit testing at begin_pass still sees the previous canvas's widget
        // rectangles. A held click/resize must not act on a window that has
        // just moved to the other monitor. Keyboard focus remains intact.
        ctx.stop_dragging();
        ctx.input_mut(|input| input.pointer = Default::default());
        let room = default_work_area(screen);
        set_work_area(ctx, room);
        // Login/loading floaters have no bars pass to report movement room.
        // Keep their existing default inset; World and photo mode replace
        // these defaults with their own bar/view geometry.
        set_move_area(ctx, room);
    }
    changed
}

pub(crate) fn restricted_canvas(ctx: &egui::Context) -> Option<Rect> {
    ctx.data(|d| d.get_temp::<Option<Rect>>(canvas_bounds_id())).flatten()
}

fn world_canvas_id() -> Id {
    Id::unique("layout_world_canvas")
}

/// The live scene has its own rectangle even when controls use the full
/// workspace. Snapshot output sizes do not change this logical rectangle.
pub(crate) fn set_world_canvas(ctx: &egui::Context, bounds: Option<Rect>) {
    ctx.data_mut(|data| data.insert_temp(world_canvas_id(), bounds));
}

pub(crate) fn world_rect(ctx: &egui::Context) -> Rect {
    ctx.data(|data| data.get_temp::<Option<Rect>>(world_canvas_id()))
        .flatten()
        .unwrap_or_else(|| ctx.content_rect())
}

fn toolbar_canvas_id() -> Id {
    Id::unique("layout_toolbar_canvas")
}

/// Chrome has its own placement rectangle; egui's root canvas and ordinary
/// windows always keep the full native workspace.
pub(crate) fn set_toolbar_canvas(ctx: &egui::Context, bounds: Option<Rect>) -> bool {
    let previous = toolbar_canvas(ctx);
    ctx.data_mut(|d| d.insert_temp(toolbar_canvas_id(), bounds));
    if previous == bounds {
        return false;
    }
    let layers = ctx.data(|data| data.get_temp::<(u64, Vec<egui::LayerId>)>(Id::unique("toolbar_layers")));
    let pressed_layer = ctx
        .input(|input| input.pointer.press_origin())
        .and_then(|point| ctx.layer_id_at(point));
    let dragged_layer = ctx
        .dragged_id()
        .and_then(|id| ctx.read_response(id))
        .map(|response| response.layer_id);
    if layers.is_some_and(|(_, layers)| pressed_layer.into_iter().chain(dragged_layer).any(|layer| layers.contains(&layer))) {
        // Only controls that actually moved lose a held gesture. Settings
        // and ordinary floaters can keep adjusting the viewport continuously.
        ctx.stop_dragging();
        ctx.input_mut(|input| input.pointer = Default::default());
    }
    windows(ctx, |windows| {
        windows.control_rooms.retain(|_, room| {
            if let Some(bounds) = bounds {
                *room = bounds;
                true
            } else {
                false
            }
        });
        windows.live.followed = None;
    });
    true
}

pub(crate) fn toolbar_canvas(ctx: &egui::Context) -> Option<Rect> {
    ctx.data(|d| d.get_temp::<Option<Rect>>(toolbar_canvas_id())).flatten()
}

pub(crate) fn toolbar_rect(ctx: &egui::Context) -> Rect {
    toolbar_canvas(ctx).unwrap_or_else(|| ctx.content_rect())
}

pub(crate) fn toolbar_layer(ctx: &egui::Context, layer: egui::LayerId) {
    let frame = ctx.cumulative_frame_nr();
    ctx.data_mut(|data| {
        let layers = data.get_temp_mut_or_default::<(u64, Vec<egui::LayerId>)>(Id::unique("toolbar_layers"));
        if layers.0 != frame {
            *layers = (frame, Vec::new());
        }
        if !layers.1.contains(&layer) {
            layers.1.push(layer);
        }
    });
}

pub(crate) fn toolbar_layer_bounds(ctx: &egui::Context, layer: egui::LayerId) -> Option<Rect> {
    let bounds = toolbar_canvas(ctx)?;
    ctx.data(|data| data.get_temp::<(u64, Vec<egui::LayerId>)>(Id::unique("toolbar_layers")))
        .filter(|(frame, layers)| *frame == ctx.cumulative_frame_nr() && layers.contains(&layer))
        .map(|_| bounds)
}

pub(crate) fn unrestricted_rect(ctx: &egui::Context, id: Id, size: Vec2) -> Option<Rect> {
    let room = move_area(ctx);
    windows(ctx, |windows| {
        windows.placed.get(&id.value()).map(|placed| {
            let position = placed.anchor.map_or(placed.rect.min, |anchor| anchor.position(room, size));
            Rect::from_min_size(position, size)
        })
    })
}

pub(crate) fn anchored_rect(rect: Rect, old_room: Rect, room: Rect) -> Rect {
    Rect::from_min_size(Anchor::nearest(old_room, rect).position(room, rect.size()), rect.size())
}

/// Bars can consume the whole height of a very small world rectangle.
/// Keep a finite, positive placement area even then; egui clips to the canvas.
fn bounded_area(ctx: &egui::Context, area: Rect) -> Rect {
    let Some(bounds) = restricted_canvas(ctx) else { return area };
    if !area.is_finite() {
        return bounds;
    }
    let minimum = Vec2::splat(1.0).min(bounds.size());
    let min = area.min.clamp(bounds.min, bounds.max - minimum);
    let max = area.max.clamp(min + minimum, bounds.max);
    Rect::from_min_max(min, max)
}

/// A resize can bypass egui's normal movement constraint. Keep it inside
/// the chosen interface canvas as well, including fixed-aspect floaters.
pub(crate) fn fit_window(ctx: &egui::Context, rect: Rect) -> Rect {
    if restricted_canvas(ctx).is_none() {
        return rect;
    }
    let room = move_area(ctx);
    let size = rect.size().max(Vec2::splat(1.0)).min(room.size());
    let min = rect.min.clamp(room.min, (room.max - size).max(room.min));
    Rect::from_min_size(min, size)
}

/// A fixed-aspect body shrinks uniformly inside the available room, keeping
/// its title/frame chrome unchanged. Capping each axis separately would
/// distort it after `Drag::keep_aspect` resolved the gesture.
pub(crate) fn fit_aspect_window(ctx: &egui::Context, rect: Rect, ratio: f32, chrome: Vec2) -> Rect {
    if restricted_canvas(ctx).is_none() {
        return rect;
    }
    let room = move_area(ctx);
    if !ratio.is_finite() || ratio <= 0.0 || !chrome.is_finite() || chrome.x >= room.width() || chrome.y >= room.height() {
        return fit_window(ctx, rect);
    }
    let chrome = chrome.max(Vec2::ZERO);
    let available = room.size() - chrome;
    let body = (rect.size() - chrome).max(Vec2::ZERO);
    let width = body.x.min(body.y * ratio).min(available.x).min(available.y * ratio);
    let size = Vec2::new(width, width / ratio) + chrome;
    let min = rect.min.clamp(room.min, (room.max - size).max(room.min));
    Rect::from_min_size(min, size)
}

/// An automatically reduced floater is returning to its wide-canvas size.
/// Follow its existing anchor using that size before egui places it.
pub(crate) fn restore_window_size(ctx: &egui::Context, id: Id, size: Vec2) {
    let room = move_area(ctx);
    windows(ctx, |windows| {
        if let Some(placed) = windows.placed.get_mut(&id.value())
            && placed.rect.size() != size
        {
            placed.rect = Rect::from_min_size(placed.rect.min, size);
            if let Some(pos) = placed
                .anchor
                .map(|anchor| anchor.position(room, size))
                .filter(|pos| pos.is_finite())
            {
                windows.live.places.entry(id.value()).or_insert(pos);
            }
            windows.live.followed = None;
        }
    });
}

/// The part of `screen` the windows are laid out in: between the navigation
/// bar and the bottom bar of the default skin. For what must be known before
/// the bars are drawn; a frame then reports the real one (`set_work_area`).
pub(crate) fn default_work_area(screen: Rect) -> Rect {
    let bars = super::skin::Layout::default();
    Rect::from_min_max(
        Pos2::new(screen.left(), screen.top() + bars.nav_bar_height),
        Pos2::new(screen.right(), screen.bottom() - bars.bottom_bar_height),
    )
}

fn work_area_id() -> Id {
    Id::unique("layout_work_area")
}

/// The room left by the bars this frame: call it once they are drawn,
/// before the windows are shown.
pub(crate) fn set_work_area(ctx: &egui::Context, area: Rect) {
    let area = bounded_area(ctx, area);
    ctx.data_mut(|d| d.insert_temp(work_area_id(), area));
}

/// The room the bars leave this frame, from `left` (what the panels of
/// Firestorm's bars leave) and `bottom_top` (the top of what the bottom
/// bar keeps): with the islands, below the top islands and above the dock,
/// 8 px from the window's sides (`bars::islands::work_area`).
pub(crate) fn bars_room(left: Rect, bottom_top: f32, classic_bars: bool) -> Rect {
    if classic_bars {
        left
    } else {
        super::bars::islands::work_area(left, bottom_top)
    }
}

/// Where the windows are laid out: the screen without the bars.
pub(crate) fn work_area(ctx: &egui::Context) -> Rect {
    bounded_area(
        ctx,
        ctx.data(|d| d.get_temp::<Rect>(work_area_id()))
            .unwrap_or_else(|| default_work_area(ctx.content_rect())),
    )
}

fn move_area_id() -> Id {
    Id::unique("layout_move_area")
}

/// Where the windows can be moved this frame: with the islands the whole
/// screen, since the islands leave room between them, above and below;
/// with the Legacy Viewer's full-width bars, the room between them. Call
/// it with `set_work_area`.
pub(crate) fn set_move_area(ctx: &egui::Context, area: Rect) {
    let area = bounded_area(ctx, area);
    ctx.data_mut(|d| d.insert_temp(move_area_id(), area));
}

/// Where the windows can be moved (`set_move_area`); the work area when
/// nothing set it.
pub(crate) fn move_area(ctx: &egui::Context) -> Rect {
    bounded_area(
        ctx,
        ctx.data(|d| d.get_temp::<Rect>(move_area_id())).unwrap_or_else(|| work_area(ctx)),
    )
}

/// Height of a floater's title bar, and the width of it that stays in the
/// room the windows move in at least: enough to grab it and bring the
/// window back.
pub(crate) const TITLE_BAR_H: f32 = 38.0;
pub(crate) const TITLE_GRIP: f32 = 96.0;

/// Which edges of the room stop the windows (`Settings::windows_stop_*`,
/// all by default): a window stays whole on the screen on that side; off,
/// it may cross that edge, its title bar kept within reach. The top edge
/// always stops them: a title bar above it could not be grabbed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Edges {
    pub left: bool,
    pub right: bool,
    pub bottom: bool,
}

impl Default for Edges {
    fn default() -> Self {
        Edges {
            left: true,
            right: true,
            bottom: true,
        }
    }
}

impl Edges {
    pub const OPEN: Self = Self {
        left: false,
        right: false,
        bottom: false,
    };
}

fn edges_id() -> Id {
    Id::unique("layout_window_edges")
}

/// The edges that stop the windows this frame (the user's settings): call
/// it with `set_move_area`.
pub(crate) fn set_window_edges(ctx: &egui::Context, edges: Edges) {
    ctx.data_mut(|d| d.insert_temp(edges_id(), edges));
}

/// The edges that stop the windows (`set_window_edges`); all of them when
/// nothing set it.
pub(crate) fn window_edges(ctx: &egui::Context) -> Edges {
    if restricted_canvas(ctx).is_some() {
        return Edges::default();
    }
    ctx.data(|d| d.get_temp::<Edges>(edges_id())).unwrap_or_default()
}

/// Per-window exception to the edge stops, retaining the shared title grip.
pub(crate) fn set_partial_outside(ctx: &egui::Context, id: Id, allowed: bool) {
    ctx.data_mut(|d| d.insert_temp(id.with("partial_outside"), allowed));
}

fn partial_outside(ctx: &egui::Context, id: Id) -> bool {
    ctx.data(|d| d.get_temp::<bool>(id.with("partial_outside"))).unwrap_or(false)
}

pub(crate) fn edges_for(ctx: &egui::Context, id: Id) -> Edges {
    if restricted_canvas(ctx).is_none() && partial_outside(ctx, id) {
        Edges::OPEN
    } else {
        window_edges(ctx)
    }
}

/// The rect a window of `size` is kept in (`egui::Window::constrain_to`):
/// whole in `area` on the sides whose edge stops it (`edges`); past the
/// left or right edge otherwise but `TITLE_GRIP` of its title bar, past the
/// bottom one but its whole title bar. Never above the top.
pub(crate) fn reach(area: Rect, size: Vec2, edges: Edges) -> Rect {
    let grip = TITLE_GRIP.min(size.x);
    let over_x = (size.x - grip).max(0.0);
    let over_y = (size.y - TITLE_BAR_H).max(0.0);
    Rect::from_min_max(
        Pos2::new(area.left() - if edges.left { 0.0 } else { over_x }, area.top()),
        Pos2::new(
            area.right() + if edges.right { 0.0 } else { over_x },
            area.bottom() + if edges.bottom { 0.0 } else { over_y },
        ),
    )
}

/// `pos` of a window of `size`, moved within `reach`; a window larger than
/// the room keeps its top left corner in it.
pub(crate) fn keep_title_in(area: Rect, pos: Pos2, size: Vec2, edges: Edges) -> Pos2 {
    let limits = reach(area, size, edges);
    let right = (limits.right() - size.x).max(limits.left());
    let bottom = (limits.bottom() - size.y).max(limits.top());
    Pos2::new(pos.x.clamp(limits.left(), right), pos.y.clamp(limits.top(), bottom))
}

fn islands_id() -> Id {
    Id::unique("layout_bar_islands")
}

/// An island of the bars was drawn this frame at `rect` (`bars::islands`):
/// the windows keep their title clear of them (`clear_of_islands`).
pub(crate) fn island_drawn(ctx: &egui::Context, rect: Rect) {
    let frame = ctx.cumulative_frame_nr();
    ctx.data_mut(|d| {
        let islands = d.get_temp_mut_or_default::<(u64, Vec<Rect>)>(islands_id());
        if islands.0 != frame {
            *islands = (frame, Vec::new());
        }
        islands.1.push(rect);
    });
}

/// The islands drawn this frame.
pub(crate) fn islands(ctx: &egui::Context) -> Vec<Rect> {
    let frame = ctx.cumulative_frame_nr();
    ctx.data(|d| d.get_temp::<(u64, Vec<Rect>)>(islands_id()))
        .filter(|(drawn, _)| *drawn == frame)
        .map(|(_, rects)| rects)
        .unwrap_or_default()
}

/// How much of `title` (a title bar) shows in `area` beside the islands
/// that cross its height.
fn uncovered(title: Rect, area: Rect, islands: &[Rect]) -> f32 {
    let (left, right) = (title.left().max(area.left()), title.right().min(area.right()));
    if right <= left {
        return 0.0;
    }
    let mut covered: Vec<(f32, f32)> = islands
        .iter()
        .filter(|i| i.top() < title.bottom() && i.bottom() > title.top())
        .map(|i| (i.left().max(left), i.right().min(right)))
        .filter(|(a, b)| b > a)
        .collect();
    covered.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut hidden = 0.0;
    let mut end = left;
    for (a, b) in covered {
        if b > end {
            hidden += b - a.max(end);
            end = b;
        }
    }
    (right - left) - hidden
}

/// Room kept between a window brought out from under the islands and them.
const ISLAND_CLEARANCE: f32 = 8.0;

/// `pos` of a window of `size`, moved off the islands when its title bar is
/// hidden by them all but `TITLE_GRIP` (the islands and the windows share a
/// layer: the last one clicked comes on top, and a title left under an
/// island could not be grabbed again). It goes below the islands of the
/// upper half of `area`, above those of the lower half, and stays within
/// reach (`keep_title_in`).
pub(crate) fn clear_of_islands(area: Rect, pos: Pos2, size: Vec2, edges: Edges, islands: &[Rect]) -> Pos2 {
    let width = size.x;
    let title = Rect::from_min_size(pos, Vec2::new(width, TITLE_BAR_H));
    if uncovered(title, area, islands) >= TITLE_GRIP.min(width) {
        return pos;
    }
    let crossing: Vec<&Rect> = islands
        .iter()
        .filter(|i| i.top() < title.bottom() && i.bottom() > title.top() && i.left() < title.right() && i.right() > title.left())
        .collect();
    let upper = crossing.iter().filter(|i| i.center().y < area.center().y);
    let lower = crossing.iter().filter(|i| i.center().y >= area.center().y);
    let y = if let Some(bottom) = upper.map(|i| i.bottom()).reduce(f32::max) {
        bottom + ISLAND_CLEARANCE
    } else if let Some(top) = lower.map(|i| i.top()).reduce(f32::min) {
        top - ISLAND_CLEARANCE - TITLE_BAR_H
    } else {
        pos.y
    };
    keep_title_in(area, Pos2::new(pos.x, y), size, edges)
}

/// A screen smaller than this is not one to lay windows out in: the content
/// rect of a minimized viewer window, or of the first frames before the
/// surface has its size. Windows neither follow it nor record it.
const MIN_SCREEN: Vec2 = Vec2::new(320.0, 240.0);

fn usable(screen: Rect) -> bool {
    screen.is_finite() && screen.width() >= MIN_SCREEN.x && screen.height() >= MIN_SCREEN.y
}

/// Where a window stood the last time it was shown, and in which screen.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
struct Placed {
    rect: Rect,
    screen: Rect,
    /// Absent in older layouts, which recorded only the full screen.
    #[serde(default)]
    room: Option<Rect>,
    #[serde(default)]
    anchor: Option<Anchor>,
}

/// A window shown this frame or the last one.
#[derive(Clone, Copy, Debug)]
struct Shown {
    id: Id,
    rect: Rect,
    /// The frame (`Live::frame`) it was last shown in.
    frame: u64,
    minimized: bool,
    partial_outside: bool,
}

/// What is known of the windows for the session only.
#[derive(Clone, Debug, Default)]
struct Live {
    /// Frames counted by `frame` (the gestures); stays 0 without it.
    frame: u64,
    shown: BTreeMap<u64, Shown>,
    /// The screen the targets of `follow` were computed for, and where each
    /// window recorded in another screen goes in it.
    followed: Option<(Rect, Rect)>,
    follow: BTreeMap<u64, Pos2>,
    /// Positions the gestures give this frame: the windows glued to the one
    /// dragged, a snap applied.
    places: BTreeMap<u64, Pos2>,
    /// Windows egui measures this frame, and where each belongs: egui keeps
    /// a window it has no size for inside the screen as if it had its
    /// default size, which pushes away from the bottom and right edges one
    /// that was left smaller. The next frame puts it back.
    settle: BTreeMap<u64, Pos2>,
    /// Where the dragged window of a group can go (`snap::block_limits`).
    limits: Option<(u64, Rect)>,
    /// The setting (`Settings::snap_windows`), as of this frame.
    snap: bool,
    moved: Option<gesture::Move>,
    resized: Option<gesture::Resize>,
    /// Completed gestures or explicit placements, recorded after every
    /// member has drawn its final rect, including any applied snap.
    reanchor: BTreeSet<u64>,
    /// Explicit placements may be requested before layout::frame runs;
    /// wait until Slot::shown has recorded their actual result.
    reanchor_after_show: BTreeSet<u64>,
}

/// The windows the user places, by `Id::value`; persisted with egui's
/// memory, so written to `ui_layout.ron` with the rest of the layout.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
struct Windows {
    #[serde(default)]
    placed: BTreeMap<u64, Placed>,
    /// Windows snapped side by side: they move together (`snap::group`).
    #[serde(default)]
    links: BTreeSet<snap::Link>,
    #[serde(skip)]
    live: Live,
    /// Only camera/photo controls use these explicit rooms. Never replace
    /// the shared workspace while following or saving another window.
    #[serde(skip)]
    control_rooms: BTreeMap<u64, Rect>,
}

impl Windows {
    fn anchor_group(&mut self, window: u64, room: Rect) {
        let members = snap::group(&self.links, window);
        let block = members
            .iter()
            .filter_map(|id| self.placed.get(id))
            .fold(Rect::NOTHING, |all, placed| all.union(placed.rect));
        if !block.is_finite() {
            return;
        }
        let group_room = self.control_rooms.get(&window).copied().unwrap_or(room);
        let point = Anchor::nearest(group_room, block).point;
        for id in members {
            if let Some(placed) = self.placed.get_mut(&id) {
                let room = self.control_rooms.get(&id).copied().unwrap_or(room);
                placed.anchor = Some(Anchor::new(point, room, placed.rect));
            }
        }
        self.live.followed = None;
    }

    fn refresh_anchors(&mut self, room: Rect) {
        for id in std::mem::take(&mut self.live.reanchor) {
            self.anchor_group(id, room);
        }
    }

    /// Reconstruct positions from saved anchors, including closed windows.
    /// Compute once before any window is recorded in the new room.
    fn follow(&mut self, window: u64, screen: Rect, room: Rect) -> Option<Pos2> {
        if self.live.followed == Some((screen, room)) {
            return self.live.follow.remove(&window);
        }
        // Migrate old layouts before any member moves to the current room.
        let missing: Vec<_> = self
            .placed
            .iter()
            .filter(|(_, p)| p.anchor.is_none() && usable(p.screen) && p.rect.is_finite())
            .map(|(id, p)| (*id, *p))
            .collect();
        for (id, placed) in missing {
            if self.placed[&id].anchor.is_none() {
                // Old layouts knew the screen but not the movement room.
                // Transfer the current bar insets to that screen so migration
                // does not add the bars' height to an existing window offset.
                let old_room = placed.room.unwrap_or_else(|| {
                    Rect::from_min_max(
                        placed.screen.min + (room.min - screen.min),
                        placed.screen.max + (room.max - screen.max),
                    )
                });
                self.anchor_group(id, old_room);
            }
        }
        self.live.followed = Some((screen, room));
        self.live.follow = self
            .placed
            .iter()
            .filter(|(_, placed)| placed.screen != screen || placed.room != Some(room))
            .filter_map(|(id, placed)| {
                let pos = placed.anchor?.position(room, placed.rect.size());
                pos.is_finite().then_some((*id, pos))
            })
            .collect();
        self.live.follow.remove(&window)
    }
}

fn windows<R>(ctx: &egui::Context, read: impl FnOnce(&mut Windows) -> R) -> R {
    ctx.data_mut(|d| read(d.get_persisted_mut_or_default::<Windows>(Id::unique("window_layout"))))
}

/// One window of the layout for one frame: ask for it before the window is
/// shown, give `pos` and `limits` (when there are) to
/// `egui::Window::current_pos` and `constrain_to`, and report the rect once
/// shown.
#[must_use = "report the window with Slot::shown"]
pub(crate) struct Slot {
    id: Id,
    screen: Rect,
    room: Rect,
    /// egui knew the size of the window before this frame. It does not on
    /// the first frame of a window, shown or restored from the layout: that
    /// frame only measures it, and its rect is not one to record.
    sized: bool,
    preserve_anchor: bool,
    initial_anchor: Option<Align2>,
    /// The position the window takes this frame: it follows the window it
    /// is glued to, a snap is applied, or the screen changed since it was
    /// last shown and it follows its edges.
    pub pos: Option<Pos2>,
    /// Where the window can be dragged this frame instead of the whole
    /// screen: it carries a group that must stay inside with it.
    pub limits: Option<Rect>,
}

/// The slot of window `id` (the id given to `egui::Window::id`).
pub(crate) fn slot(ctx: &egui::Context, id: Id) -> Slot {
    slot_in(ctx, id, None)
}

pub(crate) fn slot_in(ctx: &egui::Context, id: Id, bounds: Option<Rect>) -> Slot {
    let screen = bounds.unwrap_or_else(|| ctx.content_rect());
    let room = bounds.unwrap_or_else(|| move_area(ctx));
    let sized = egui::AreaState::load(ctx, id).is_some_and(|s| s.size.is_some());
    let key = id.value();
    let (pos, limits, preserve_anchor) = windows(ctx, |w| {
        if let Some(bounds) = bounds {
            w.control_rooms.insert(key, bounds);
        } else {
            w.control_rooms.remove(&key);
        }
        let followed = if bounds.is_some() {
            w.placed
                .get(&key)
                .filter(|old| old.screen != screen || old.room != Some(room))
                .and_then(|old| {
                    let anchor = old
                        .anchor
                        .unwrap_or_else(|| Anchor::nearest(old.room.unwrap_or(old.screen), old.rect));
                    let position = anchor.position(room, old.rect.size());
                    position.is_finite().then_some(position)
                })
        } else {
            usable(screen).then(|| w.follow(key, screen, room)).flatten()
        };
        let preserve_anchor = followed.is_some() || !sized || w.live.settle.contains_key(&key);
        let limits = w.live.limits.filter(|(window, _)| *window == key).map(|(_, limits)| limits);
        let mut pos = w.live.places.remove(&key).or(followed);
        if sized {
            // measured last frame: back where it belongs, unless it is
            // given another place now
            let settled = w.live.settle.remove(&key);
            pos = pos.or(settled);
        } else if let Some(target) = pos.or_else(|| w.placed.get(&key).map(|placed| placed.rect.min)) {
            w.live.settle.insert(key, target);
        }
        (pos, limits, preserve_anchor)
    });
    Slot {
        id,
        screen,
        room,
        sized,
        preserve_anchor,
        initial_anchor: None,
        pos,
        limits: bounds.or_else(|| limits.map(|limits| bounded_area(ctx, limits))),
    }
}

impl Slot {
    /// An automatic viewport constraint can change size/position while the
    /// same canvas remains active (for example expanding a minimized bar).
    /// Keep the user's offset until a deliberate move/resize replaces it.
    pub(crate) fn keep_anchor(mut self) -> Self {
        self.preserve_anchor = true;
        self
    }

    /// An explicit default is applied only after measurement and before the
    /// first recorded placement. Saved layouts and requested moves take priority.
    pub(crate) fn default_anchor(mut self, ctx: &egui::Context, point: Align2, at: Pos2) -> Self {
        let key = self.id.value();
        let fresh = windows(ctx, |w| {
            !w.placed.contains_key(&key) && !w.live.reanchor_after_show.contains(&key) && !w.live.reanchor.contains(&key)
        });
        if fresh {
            self.initial_anchor = Some(point);
            if self.pos.is_none()
                && let Some(size) = egui::AreaState::load(ctx, self.id).and_then(|s| s.size)
            {
                self.pos = Some(point.anchor_size(at, size).min);
            }
        }
        self
    }

    /// The window was shown at `rect`; `minimized`: reduced to its title.
    pub(crate) fn shown(self, ctx: &egui::Context, rect: Rect, minimized: bool) {
        let screen = self.screen;
        let room = self.room;
        let partial_outside = self.screen == ctx.content_rect() && partial_outside(ctx, self.id);
        windows(ctx, |w| {
            let key = self.id.value();
            let recorded = w.placed.get(&key).copied();
            let rect = match (self.sized, recorded) {
                // measured this frame: it keeps the size it was recorded
                // with, where it belongs (`Live::settle`)
                (false, Some(old)) => {
                    let min = w.live.settle.get(&key).copied().unwrap_or(rect.min);
                    Rect::from_min_size(min, old.rect.size())
                }
                _ => rect,
            };
            let anchor = recorded
                .and_then(|old| {
                    old.anchor.map(|anchor| {
                        // Content growth and collapse change the window's own
                        // anchor point. Keep its current position and chosen
                        // point; viewer resize constraints never rewrite offsets.
                        if !self.preserve_anchor && old.room == Some(room) && old.screen == screen && old.rect.size() != rect.size() {
                            Anchor::new(anchor.point, room, rect)
                        } else {
                            anchor
                        }
                    })
                })
                .or_else(|| {
                    (self.sized && usable(screen)).then(|| {
                        self.initial_anchor
                            .map_or_else(|| Anchor::nearest(room, rect), |point| Anchor::new(point, room, rect))
                    })
                });
            let new = Placed {
                rect,
                screen,
                room: Some(room),
                anchor,
            };
            if usable(screen) && (self.sized || recorded.is_some()) && recorded != Some(new) {
                w.placed.insert(key, new);
            }
            if usable(screen) && self.sized && w.live.reanchor_after_show.remove(&key) {
                w.live.reanchor.insert(key);
            }
            let frame = w.live.frame;
            w.live.shown.insert(
                key,
                Shown {
                    id: self.id,
                    rect,
                    frame,
                    minimized,
                    partial_outside,
                },
            );
        });
    }
}

/// An explicit placement follows the same rule as a completed mouse move.
pub(crate) fn reanchor(ctx: &egui::Context, id: Id) {
    windows(ctx, |w| {
        w.live.reanchor_after_show.insert(id.value());
    });
}

/// The gestures of the frame on the windows: call it once per frame, after
/// the bars (`set_work_area`) and before the windows are shown. A window
/// dragged by its title takes the windows glued to it along, is offered a
/// snap to the others and to the work area only while Alt is held (`snap`;
/// `snap_enabled` is the user's setting), shown by a guide and applied when
/// the mouse is released; Alt also takes it out of its current group.
pub(crate) fn frame(ctx: &egui::Context, p: &Palette, snap_enabled: bool) {
    let screen = ctx.content_rect();
    if !usable(screen) {
        return;
    }
    let pointer = gesture::Pointer {
        dragged: ctx.dragged_id(),
        stopped: ctx.drag_stopped_id(),
        total: ctx.input(|i| i.pointer.total_drag_delta()).unwrap_or_default(),
        alt: ctx.input(|i| i.modifiers.alt),
        time: ctx.input(|i| i.time),
    };
    let (area, room, edges, pixels_per_point) = (work_area(ctx), move_area(ctx), window_edges(ctx), ctx.pixels_per_point());
    let overlay = windows(ctx, |w| w.drive(area, room, edges, pixels_per_point, pointer, snap_enabled));
    if let Some(overlay) = overlay {
        snap::overlay::paint(ctx, p, &overlay);
    }
}

/// Window `id` is resized by its `edges` and the pointer has it at `rect`:
/// the rect to give it, its moving edges snapped when they come near
/// another window or the work area. Unlike a move, a resize snaps under the
/// pointer: the window is laid out again at each size, and its content may
/// refuse one (`min_size` grows then, and the snap is no longer offered).
pub(crate) fn resizing(ctx: &egui::Context, p: &Palette, id: Id, rect: Rect, edges: snap::Edges, min_size: Vec2) -> Rect {
    let (alt, time, released) = ctx.input(|i| (i.modifiers.alt, i.time, !i.pointer.primary_down()));
    let area = windows(ctx, |windows| windows.control_rooms.get(&id.value()).copied()).unwrap_or_else(|| work_area(ctx));
    let request = gesture::ResizeRequest {
        window: id.value(),
        rect,
        edges,
        min_size,
        alt,
        time,
        released,
    };
    let (rect, overlay) = windows(ctx, |w| w.resize(request, area));
    if let Some(overlay) = overlay {
        snap::overlay::paint(ctx, p, &overlay);
    }
    fit_window(ctx, rect)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace_native_frame(ctx: &egui::Context, n: usize, draft: &mut String) {
        workspace_native_frame_events(ctx, n, draft, Vec::new());
    }
    fn workspace_native_frame_events(ctx: &egui::Context, n: usize, draft: &mut String, events: Vec<egui::Event>) {
        let p = crate::theme::Theme::default().palette();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(screen(1280.0, 800.0)),
                time: Some(n as f64 / 60.0),
                events,
                ..Default::default()
            },
            |ui| {
                frame(ui.ctx(), &p, false);
                super::super::widgets::Floater::new("inventory", "Inventory", egui::pos2(70.0, 90.0), egui::vec2(320.0, 220.0))
                    .silent()
                    .show(ui.ctx(), &p, &mut true, |ui| {
                        ui.add(egui::TextEdit::singleline(draft).id(egui::Id::unique("workspace_untouched_editor")));
                    });
                super::super::widgets::bare_window(
                    ui.ctx(),
                    &p,
                    "minimap",
                    "Map",
                    egui::Frame::NONE,
                    Rect::from_min_size(egui::pos2(900.0, 90.0), egui::vec2(200.0, 200.0)),
                    egui::vec2(64.0, 64.0),
                    false,
                    |ui| {
                        ui.label("map");
                    },
                );
            },
        );
        output.textures_delta.clear();
    }
    #[test]
    fn workspace_geometry_preserves_native_minimization_editor_focus_and_closed_conversations() {
        let ctx = egui::Context::default();
        let mut draft = "unsent text".to_owned();
        for n in 1..5 {
            workspace_native_frame(&ctx, n, &mut draft);
        }
        let baseline = workspace_capture(&ctx);
        assert!(baseline[0].is_some());
        assert!(baseline[1].is_some());
        assert!(baseline[3].is_none());
        super::super::widgets::minimize_floater(&ctx, "inventory");
        workspace_native_frame(&ctx, 5, &mut draft);
        let before = workspace_capture(&ctx);
        assert_eq!(before[0].unwrap().size, baseline[0].unwrap().size);
        ctx.memory_mut(|m| m.request_focus(Id::unique("workspace_untouched_editor")));
        let mut target = before;
        target[0].as_mut().unwrap().offset[0] += 200.0;
        target[0].as_mut().unwrap().size = [420.0, 300.0];
        target[1].as_mut().unwrap().size = [240.0, 180.0];
        workspace_apply(&ctx, &target);
        assert!(ctx.memory(|m| m.has_focus(Id::unique("workspace_untouched_editor"))));
        assert!(super::super::widgets::floater_minimized(&ctx, "inventory"));
        for n in 6..9 {
            workspace_native_frame(&ctx, n, &mut draft);
        }
        assert_eq!(draft, "unsent text");
        assert!(super::super::widgets::floater_minimized(&ctx, "inventory"));
        assert_eq!(workspace_capture(&ctx)[0].unwrap().size, [420.0, 300.0]);
        assert!(workspace_capture(&ctx)[3].is_none());
        let map = ctx.memory(|m| m.area_rect(Id::unique("minimap"))).unwrap();
        assert!(map.width() > 200.0);
    }
    #[test]
    fn workspace_preview_generic_backup_projects_baseline_without_mutating_live_context() {
        let ctx = egui::Context::default();
        let mut draft = "keep editor".to_owned();
        for n in 1..5 {
            workspace_native_frame(&ctx, n, &mut draft);
        }
        let baseline = workspace_capture(&ctx);
        let original = ctx.memory(|m| m.area_rect(Id::unique("inventory"))).unwrap();
        workspace_preview(&ctx, Some(baseline));
        let mut target = baseline;
        target[0].as_mut().unwrap().offset[0] += 260.0;
        workspace_apply(&ctx, &target);
        for n in 5..8 {
            workspace_native_frame(&ctx, n, &mut draft);
        }
        let live = ctx.memory(|m| m.area_rect(Id::unique("inventory"))).unwrap();
        assert!((live.left() - original.left()).abs() > 100.0);
        let saved = text(&ctx).unwrap();
        assert_eq!(ctx.memory(|m| m.area_rect(Id::unique("inventory"))).unwrap(), live);
        let restored = egui::Context::default();
        restore(&restored, &saved).unwrap();
        for n in 10..14 {
            workspace_native_frame(&restored, n, &mut draft);
        }
        let restored_rect = restored.memory(|m| m.area_rect(Id::unique("inventory"))).unwrap();
        assert!((restored_rect.left() - original.left()).abs() < 1.0);
        let mut autosave = Autosave {
            due: Some(Instant::now()),
            ..Default::default()
        };
        autosave.after_pass(&ctx, Instant::now());
        assert!(autosave.due.is_none());
        workspace_apply(&ctx, &baseline);
        workspace_preview(&ctx, None);
        assert_eq!(draft, "keep editor");
    }
    #[test]
    fn workspace_atomic_generation_barrier_rejects_old_layout_without_taking_writer_mutex() {
        let ctx = egui::Context::default();
        let held = WRITTEN.lock().unwrap_or_else(PoisonError::into_inner);
        // This call must not try to acquire WRITTEN, including when the
        // background writer is stalled in its disk flush.
        workspace_preview(&ctx, Some([None; 4]));
        drop(held);
        workspace_preview(&ctx, None);
        let dir = crate::settings::config_dir().join(format!("workspace-barrier-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("layout.ron");
        std::fs::write(&p, b"original").unwrap();
        let state = Mutex::new(Written { generation: 0, text: None });
        let barrier = AtomicU64::new(3);
        write_to_guarded(&state, &p, 2, &egui::Memory::default(), Some(&barrier));
        assert_eq!(std::fs::read(&p).unwrap(), b"original");
        write_to_guarded(&state, &p, 4, &egui::Memory::default(), Some(&barrier));
        assert_ne!(std::fs::read(&p).unwrap(), b"original");
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn workspace_closed_geometry_is_deferred_and_cancel_backup_restore_only_introduced_state() {
        let ctx = egui::Context::default();
        let mut draft = "keep unsent editor".to_owned();
        for n in 1..5 {
            workspace_native_frame(&ctx, n, &mut draft);
        }
        let role = Id::unique("conversations");
        let unrelated = Id::unique("workspace_unrelated_geometry");
        let historical = Vec2::new(360.0, 240.0);
        ctx.data_mut(|d| {
            d.insert_persisted(role.with("expanded_size"), historical);
            d.insert_persisted(unrelated, Vec2::new(11.0, 19.0));
        });
        let baseline = workspace_capture(&ctx);
        assert!(baseline[3].is_none());
        assert!(egui::AreaState::load(&ctx, role).is_none());
        ctx.memory_mut(|m| m.request_focus(Id::unique("workspace_untouched_editor")));
        let mut target = baseline;
        target[3] = Some(crate::workspace::model::Geometry {
            point: 0,
            offset: [600.0, 100.0],
            size: [420.0, 260.0],
        });
        workspace_preview(&ctx, Some(baseline));
        workspace_apply(&ctx, &target);
        assert_eq!(workspace_capture(&ctx)[3], target[3]);
        assert!(egui::AreaState::load(&ctx, role).is_none(), "no opening/draw to capture geometry");
        assert!(
            ctx.data_mut(|d| d.get_persisted::<[f32; 4]>(role.with("workspace_geometry")))
                .is_some()
        );
        let saved = text(&ctx).unwrap();
        let restored = egui::Context::default();
        restore(&restored, &saved).unwrap();
        assert!(workspace_capture(&restored)[3].is_none());
        assert!(egui::AreaState::load(&restored, role).is_none());
        assert!(
            restored
                .data_mut(|d| d.get_persisted::<[f32; 4]>(role.with("workspace_geometry")))
                .is_none()
        );
        assert_eq!(
            restored.data_mut(|d| d.get_persisted::<Vec2>(role.with("expanded_size"))),
            Some(historical)
        );
        workspace_cancel_gesture(&ctx);
        workspace_restore(&ctx);
        workspace_preview(&ctx, None);
        assert!(workspace_capture(&ctx)[3].is_none());
        assert!(egui::AreaState::load(&ctx, role).is_none());
        assert!(
            ctx.data_mut(|d| d.get_persisted::<[f32; 4]>(role.with("workspace_geometry")))
                .is_none()
        );
        assert_eq!(
            ctx.data_mut(|d| d.get_persisted::<Vec2>(role.with("expanded_size"))),
            Some(historical)
        );
        assert_eq!(ctx.data_mut(|d| d.get_persisted::<Vec2>(unrelated)), Some(Vec2::new(11.0, 19.0)));
        assert!(ctx.memory(|m| m.has_focus(Id::unique("workspace_untouched_editor"))));
        assert_eq!(draft, "keep unsent editor");
        // A→B must retain the initial absent-role checkpoint, not adopt A's
        // introduced deferred geometry as B's new rollback state.
        workspace_preview(&ctx, Some(baseline));
        workspace_apply(&ctx, &target);
        workspace_restore(&ctx);
        workspace_preview(&ctx, Some(baseline));
        workspace_apply(&ctx, &baseline);
        workspace_restore(&ctx);
        workspace_preview(&ctx, None);
        assert!(workspace_capture(&ctx)[3].is_none());
        assert!(egui::AreaState::load(&ctx, role).is_none());
    }
    #[test]
    fn workspace_committed_closed_geometry_applies_on_ordinary_open_and_previous_can_restore_absence() {
        let ctx = egui::Context::default();
        let mut draft = String::new();
        for n in 1..5 {
            workspace_native_frame(&ctx, n, &mut draft);
        }
        let baseline = workspace_capture(&ctx);
        let mut target = baseline;
        target[3] = Some(crate::workspace::model::Geometry {
            point: 0,
            offset: [600.0, 100.0],
            size: [420.0, 260.0],
        });
        workspace_preview(&ctx, Some(baseline));
        workspace_apply(&ctx, &target);
        workspace_preview(&ctx, None);
        assert!(egui::AreaState::load(&ctx, Id::unique("conversations")).is_none());
        let pending = ctx.memory(|m| m.clone());
        let expected = Anchor::from_workspace(target[3].unwrap())
            .unwrap()
            .position(move_area(&ctx), Vec2::from(target[3].unwrap().size));
        let p = crate::theme::Theme::default().palette();
        for n in 6..10 {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen(1280.0, 800.0)),
                    time: Some(n as f64 / 60.0),
                    ..Default::default()
                },
                |ui| {
                    frame(ui.ctx(), &p, false);
                    super::super::widgets::Floater::new("conversations", "Conversations", Pos2::new(80.0, 450.0), Vec2::new(300.0, 180.0))
                        .silent()
                        .show(ui.ctx(), &p, &mut true, |ui| {
                            ui.label("ordinary opening");
                        });
                },
            );
            out.textures_delta.clear();
        }
        let actual = ctx.memory(|m| m.area_rect(Id::unique("conversations"))).unwrap();
        assert!((actual.min - expected).abs().max_elem() < 1.0);
        let closed = egui::Context::default();
        closed.memory_mut(|m| *m = pending);
        workspace_preview(&closed, Some(target));
        workspace_previous(&closed, &baseline);
        workspace_preview(&closed, None);
        assert!(workspace_capture(&closed)[3].is_none());
        assert!(egui::AreaState::load(&closed, Id::unique("conversations")).is_none());
        assert!(
            closed
                .data_mut(|d| d.get_persisted::<[f32; 4]>(Id::unique("conversations").with("workspace_geometry")))
                .is_none()
        );
    }
    #[test]
    fn workspace_cancel_after_user_opens_preview_role_resets_geometry_only_on_ordinary_draw() {
        let ctx = egui::Context::default();
        let role = Id::unique("conversations");
        let mut draft = "untouched".to_owned();
        for n in 1..5 {
            workspace_native_frame(&ctx, n, &mut draft);
        }
        let baseline = workspace_capture(&ctx);
        assert!(baseline[3].is_none());
        let mut target = baseline;
        target[3] = Some(crate::workspace::model::Geometry {
            point: 0,
            offset: [650.0, 80.0],
            size: [400.0, 220.0],
        });
        workspace_preview(&ctx, Some(baseline));
        workspace_apply(&ctx, &target);
        let p = crate::theme::Theme::default().palette();
        let show = |n: usize| {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen(1280.0, 800.0)),
                    time: Some(n as f64 / 60.0),
                    ..Default::default()
                },
                |ui| {
                    frame(ui.ctx(), &p, false);
                    super::super::widgets::Floater::new("conversations", "Conversations", Pos2::new(80.0, 450.0), Vec2::new(300.0, 180.0))
                        .silent()
                        .show(ui.ctx(), &p, &mut true, |ui| {
                            ui.label("opened by the user");
                        });
                },
            );
            out.textures_delta.clear();
        };
        for n in 5..9 {
            show(n);
        }
        let candidate = ctx.memory(|m| m.area_rect(role)).unwrap();
        assert!(candidate.left() > 600.0);
        let saved = text(&ctx).unwrap();
        let restored = egui::Context::default();
        restore(&restored, &saved).unwrap();
        assert!(workspace_capture(&restored)[3].is_none());
        assert!(
            egui::AreaState::load(&restored, role).unwrap().pivot_pos.is_none(),
            "save clone projects the old absent Area geometry"
        );
        ctx.memory_mut(|m| m.request_focus(Id::unique("workspace_untouched_editor")));
        workspace_cancel_gesture(&ctx);
        workspace_restore(&ctx);
        workspace_preview(&ctx, None);
        assert!(workspace_capture(&ctx)[3].is_none());
        assert_eq!(
            ctx.memory(|m| m.area_rect(role)).unwrap(),
            candidate,
            "live cancellation does not set Area visibility/state"
        );
        assert!(ctx.memory(|m| m.has_focus(Id::unique("workspace_untouched_editor"))));
        assert_eq!(draft, "untouched");
        for n in 9..13 {
            show(n);
        }
        let normal = ctx.memory(|m| m.area_rect(role)).unwrap();
        assert!((normal.min - Pos2::new(80.0, 450.0)).abs().max_elem() < 1.0);
    }
    #[test]
    fn workspace_cancel_quarantines_an_actual_held_native_drag_without_touching_editor_focus() {
        let ctx = egui::Context::default();
        let mut draft = "keep draft".to_owned();
        for n in 1..5 {
            workspace_native_frame(&ctx, n, &mut draft);
        }
        let baseline = workspace_capture(&ctx);
        let original = ctx.memory(|m| m.area_rect(Id::unique("inventory"))).unwrap();
        workspace_preview(&ctx, Some(baseline));
        let mut target = baseline;
        target[0].as_mut().unwrap().offset[0] += 80.0;
        workspace_apply(&ctx, &target);
        for n in 5..8 {
            workspace_native_frame(&ctx, n, &mut draft);
        }
        let preview = ctx.memory(|m| m.area_rect(Id::unique("inventory"))).unwrap();
        assert!(
            (preview.left() - original.left()).abs() > 50.0,
            "actual Preview placement precedes the drag"
        );
        let from = preview.min + Vec2::new(170.0, 16.0);
        let to = from + Vec2::new(240.0, 60.0);
        let press = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        workspace_native_frame_events(&ctx, 8, &mut draft, vec![egui::Event::PointerMoved(from), press(from, true)]);
        for n in 9..13 {
            workspace_native_frame_events(&ctx, n, &mut draft, vec![egui::Event::PointerMoved(to)]);
        }
        assert!(ctx.dragged_id().is_some());
        assert!(ctx.input(|i| i.pointer.primary_down()));
        assert!((ctx.memory(|m| m.area_rect(Id::unique("inventory"))).unwrap().left() - original.left()).abs() > 100.0);
        assert!(
            (ctx.memory(|m| m.area_rect(Id::unique("inventory"))).unwrap().left() - preview.left()).abs() > 100.0,
            "held drag moves beyond the settled Preview"
        );
        ctx.memory_mut(|m| m.request_focus(Id::unique("workspace_untouched_editor")));
        workspace_cancel_gesture(&ctx);
        workspace_restore(&ctx);
        workspace_preview(&ctx, None);
        assert!(ctx.dragged_id().is_none());
        assert!(ctx.memory(|m| m.has_focus(Id::unique("workspace_untouched_editor"))));
        for n in 13..17 {
            workspace_native_frame_events(&ctx, n, &mut draft, vec![egui::Event::PointerMoved(to + Vec2::new(150.0, 0.0))]);
        }
        let actual = ctx.memory(|m| m.area_rect(Id::unique("inventory"))).unwrap();
        assert!((actual.min - original.min).abs().max_elem() < 1.0);
        workspace_native_frame_events(&ctx, 17, &mut draft, vec![press(to, false)]);
        assert_eq!(draft, "keep draft");
    }

    fn screen(width: f32, height: f32) -> Rect {
        Rect::from_min_size(Pos2::ZERO, Vec2::new(width, height))
    }

    /// Frames of the windows of a session: a floater against the left edge,
    /// one in the bottom right corner, and a bare `egui::Window` in the top
    /// right one, like the mini-map.
    struct Session {
        ctx: egui::Context,
        frame: usize,
    }

    const LEFT: &str = "layout_left";
    const CORNER: &str = "layout_corner";
    const MAP: &str = "layout_map";

    impl Session {
        fn new() -> Self {
            Self {
                ctx: egui::Context::default(),
                frame: 0,
            }
        }

        /// One frame in a `width` x `height` screen; the windows open at
        /// their place in a 1280 x 720 one.
        fn frame(&mut self, width: f32, height: f32) {
            self.frame += 1;
            let palette = crate::theme::Theme::default().palette();
            let input = egui::RawInput {
                screen_rect: Some(screen(width, height)),
                time: Some(self.frame as f64 / 60.0),
                ..Default::default()
            };
            let mut output = self.ctx.run_ui(input, |ui| {
                let ctx = ui.ctx();
                frame(ctx, &palette, false);
                let floater = |id, pos| {
                    crate::ui::widgets::Floater::new(id, "Window", pos, Vec2::new(300.0, 200.0))
                        .silent()
                        .show(ctx, &palette, &mut true, |ui| {
                            ui.label("content");
                        });
                };
                floater(LEFT, Pos2::new(8.0, 56.0));
                floater(CORNER, Pos2::new(972.0, 480.0));
                let slot = slot(ctx, Id::unique(MAP));
                let mut map = egui::Window::new("Map")
                    .id(Id::unique(MAP))
                    .title_bar(false)
                    .default_pos([1064.0, 56.0])
                    .default_size([200.0, 200.0]);
                if let Some(pos) = slot.pos {
                    map = map.current_pos(pos);
                }
                if let Some(shown) = map.show(ctx, |ui| ui.allocate_space(ui.available_size())) {
                    slot.shown(ctx, shown.response.rect, false);
                }
            });
            output.textures_delta.clear();
        }

        fn frames(&mut self, n: usize, width: f32, height: f32) {
            for _ in 0..n {
                self.frame(width, height);
            }
        }

        fn rect(&self, id: &str) -> Rect {
            self.ctx.memory(|m| m.area_rect(Id::unique(id))).expect("the window was shown")
        }

        /// (left gap, top gap, right gap, bottom gap) of a window.
        fn gaps(&self, id: &str, width: f32, height: f32) -> [f32; 4] {
            let r = self.rect(id);
            [r.left(), r.top(), width - r.right(), height - r.bottom()]
        }

        /// The session that follows: a new context restored from the layout
        /// this one would write on exit.
        fn restart(&self) -> Session {
            let saved = ron::to_string(&self.ctx.memory(|m| m.clone())).expect("serialize");
            let next = Session::new();
            restore(&next.ctx, &saved).expect("restore");
            next
        }
    }

    #[test]
    fn windows_follow_their_edges_when_the_viewer_window_is_resized() {
        let mut s = Session::new();
        s.frames(3, 1280.0, 720.0);
        let left = s.gaps(LEFT, 1280.0, 720.0);
        let corner = s.gaps(CORNER, 1280.0, 720.0);
        let map = s.gaps(MAP, 1280.0, 720.0);
        assert_eq!((left[0], left[1]), (8.0, 56.0));
        assert_eq!((map[1], map[2]), (56.0, 16.0), "{:?}", s.rect(MAP));
        for (width, height) in [(1600.0, 900.0), (1920.0, 1080.0), (1400.0, 760.0), (1280.0, 720.0)] {
            s.frames(2, width, height);
            let now = s.gaps(LEFT, width, height);
            assert_eq!((now[0], now[1]), (left[0], left[1]), "{width} x {height}");
            let now = s.gaps(CORNER, width, height);
            assert_eq!((now[2], now[3]), (corner[2], corner[3]), "{width} x {height}");
            let now = s.gaps(MAP, width, height);
            assert_eq!((now[1], now[2]), (map[1], map[2]), "{width} x {height}");
        }
    }

    #[test]
    fn first_opening_initializes_anchors_without_a_manual_move() {
        let mut s = Session::new();
        s.frames(3, 1280.0, 720.0);
        for (id, point) in [
            (LEFT, egui::Align2::LEFT_TOP),
            (CORNER, egui::Align2::RIGHT_BOTTOM),
            (MAP, egui::Align2::RIGHT_TOP),
        ] {
            let placed = windows(&s.ctx, |w| w.placed[&Id::unique(id).value()]);
            assert_eq!(placed.anchor.expect("initialized at measured opening").point, point);
            assert_eq!(placed.room, Some(move_area(&s.ctx)));
        }
        let anchors = windows(&s.ctx, |w| {
            w.placed.iter().map(|(id, p)| (*id, p.anchor)).collect::<BTreeMap<_, _>>()
        });
        for (width, height) in [(640.0, 480.0), (1920.0, 1080.0), (1280.0, 720.0)] {
            s.frames(3, width, height);
            assert_eq!(
                windows(&s.ctx, |w| w
                    .placed
                    .iter()
                    .map(|(id, p)| (*id, p.anchor))
                    .collect::<BTreeMap<_, _>>()),
                anchors
            );
        }
    }

    #[test]
    fn factory_rects_initialize_eight_point_anchors_and_offsets_without_changing_opening_positions() {
        for (width, height) in [(1280.0, 720.0), (1600.0, 900.0), (1920.0, 1080.0)] {
            let screen = screen(width, height);
            let work = Rect::from_min_max(Pos2::new(8.0, 53.0), Pos2::new(width - 8.0, height - 57.0));
            let people = Rect::from_min_size(super::super::people::default_pos(work), super::super::people::AURORA_SIZE);
            let map = super::super::minimap::default_rect(work);
            let chat = super::super::chat::default_rect(work);
            let (people_point, people_travel) = if height == 720.0 {
                (Align2::LEFT_CENTER, Vec2::new(0.0, 90.0))
            } else {
                (Align2::LEFT_TOP, Vec2::ZERO)
            };
            for (rect, point, travel) in [
                (people, people_point, people_travel),
                (map, Align2::RIGHT_TOP, Vec2::new(320.0, 0.0)),
                (chat, Align2::RIGHT_BOTTOM, Vec2::new(320.0, 180.0)),
            ] {
                let anchor = Anchor::nearest(screen, rect);
                assert_eq!(anchor.point, point, "{width}x{height}: {rect:?}");
                assert_eq!(anchor.position(screen, rect.size()), rect.min);
                let larger = Rect::from_min_size(Pos2::ZERO, Vec2::new(width + 320.0, height + 180.0));
                assert_eq!(anchor.position(larger, rect.size()), rect.min + travel);
            }
        }
    }

    #[test]
    fn an_explicit_placement_is_anchored_after_drawing_and_survives_temporary_clamping() {
        let mut s = Session::new();
        s.frames(3, 1600.0, 900.0);
        crate::ui::widgets::move_floater(&s.ctx, CORNER, Pos2::new(900.0, 70.0));
        s.frames(3, 1600.0, 900.0);
        let rect = s.rect(CORNER);
        assert_eq!(rect.min, Pos2::new(900.0, 70.0));
        let anchor = windows(&s.ctx, |w| {
            w.placed[&Id::unique(CORNER).value()].anchor.expect("explicit placement")
        });
        assert_eq!(anchor.point, egui::Align2::CENTER_TOP);
        s.frames(3, 640.0, 480.0);
        assert_ne!(
            s.rect(CORNER).min,
            anchor.position(move_area(&s.ctx), rect.size()),
            "temporarily clamped"
        );
        assert_eq!(windows(&s.ctx, |w| w.placed[&Id::unique(CORNER).value()].anchor), Some(anchor));
        s.frames(3, 1600.0, 900.0);
        assert_eq!(s.rect(CORNER), rect, "the original offset was retained");
    }

    #[test]
    fn older_layouts_initialize_an_anchor_and_keep_their_rects() {
        #[derive(serde::Serialize)]
        struct LegacyPlaced {
            rect: Rect,
            screen: Rect,
        }
        #[derive(serde::Serialize)]
        struct LegacyWindows {
            placed: BTreeMap<u64, LegacyPlaced>,
        }
        let screen = screen(1280.0, 720.0);
        let rect = Rect::from_min_size(Pos2::new(1000.0, 70.0), Vec2::new(240.0, 200.0));
        let saved = ron::to_string(&LegacyWindows {
            placed: BTreeMap::from([(1, LegacyPlaced { rect, screen })]),
        })
        .expect("legacy serialization");
        let mut windows: Windows = ron::from_str(&saved).expect("backward-compatible layout");
        assert!(windows.placed[&1].anchor.is_none());
        assert_eq!(
            windows.follow(1, screen, default_work_area(screen)),
            Some(rect.min),
            "bar insets must not shift an old layout"
        );
        let anchor = windows.placed[&1].anchor.expect("migration initialized the anchor");
        assert_eq!(anchor.point, egui::Align2::RIGHT_TOP);
        let wider = Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 900.0));
        assert_eq!(
            windows.follow(1, wider, default_work_area(wider)),
            Some(rect.min + Vec2::new(320.0, 0.0))
        );
    }

    #[test]
    fn the_allowed_movement_room_drives_anchors_and_clamping_does_not_rewrite_them() {
        let screen = screen(1600.0, 900.0);
        let old = Rect::from_min_max(Pos2::new(40.0, 60.0), Pos2::new(1500.0, 800.0));
        let rect = Rect::from_min_size(Pos2::new(900.0, 570.0), Vec2::new(300.0, 200.0));
        let anchor = Anchor::nearest(old, rect);
        assert_eq!(anchor.point, egui::Align2::CENTER_BOTTOM);
        let mut w = Windows::default();
        w.placed.insert(
            1,
            Placed {
                rect,
                screen,
                room: Some(old),
                anchor: Some(anchor),
            },
        );
        let small = Rect::from_min_size(Pos2::new(100.0, 80.0), Vec2::new(400.0, 300.0));
        let asked = w.follow(1, screen, small).expect("room changed without a native resize");
        let constrained = keep_title_in(small, asked, rect.size(), Edges::default());
        assert_ne!(constrained, asked);
        // Model the actual constrained rect being recorded by Slot::shown.
        w.placed.get_mut(&1).expect("placed").rect = Rect::from_min_size(constrained, rect.size());
        w.placed.get_mut(&1).expect("placed").room = Some(small);
        assert_eq!(w.follow(1, screen, old), Some(rect.min));
        assert_eq!(w.placed[&1].anchor, Some(anchor));
    }

    /// A screen too small to lay windows out in (egui itself squeezes them
    /// into it) leaves no trace in the layout: what is recorded is still
    /// where the windows stood in the last real screen. A minimized viewer
    /// window does not even get there: egui-winit keeps the previous screen
    /// rect when the window has no size.
    #[test]
    fn an_unusable_screen_is_not_recorded() {
        let mut s = Session::new();
        s.frames(3, 1600.0, 900.0);
        let before = windows(&s.ctx, |w| w.placed.clone());
        assert_eq!(before.len(), 3);
        assert_eq!(before[&Id::unique(MAP).value()].screen, screen(1600.0, 900.0));
        s.frames(2, 0.0, 0.0);
        s.frames(2, 160.0, 28.0);
        assert_eq!(windows(&s.ctx, |w| w.placed.clone()), before);
    }

    /// The layout saved in a small viewer window, opened in a larger one:
    /// the first frames of the next session put each window back against
    /// its edges, whatever the screen of the very first frame is.
    #[test]
    fn a_saved_layout_follows_the_edges_of_the_next_session() {
        let mut first = Session::new();
        first.frames(3, 1280.0, 720.0);
        let corner = first.gaps(CORNER, 1280.0, 720.0);
        let map = first.gaps(MAP, 1280.0, 720.0);
        let sizes = [first.rect(LEFT).size(), first.rect(CORNER).size(), first.rect(MAP).size()];
        for first_frame in [None, Some((800.0, 600.0)), Some((640.0, 480.0)), Some((2560.0, 1440.0))] {
            let mut next = first.restart();
            if let Some((width, height)) = first_frame {
                next.frame(width, height);
            }
            next.frames(3, 1600.0, 900.0);
            assert_eq!(next.rect(LEFT).min, Pos2::new(8.0, 56.0), "{first_frame:?}");
            let now = next.gaps(CORNER, 1600.0, 900.0);
            assert_eq!((now[2], now[3]), (corner[2], corner[3]), "{first_frame:?}");
            let now = next.gaps(MAP, 1600.0, 900.0);
            assert_eq!((now[1], now[2]), (map[1], map[2]), "{first_frame:?}");
            assert_eq!(
                [next.rect(LEFT).size(), next.rect(CORNER).size(), next.rect(MAP).size()],
                sizes,
                "{first_frame:?}"
            );
        }
    }

    /// A window left smaller than its default size, in the bottom right
    /// corner: the next session, in a larger viewer window where its default
    /// size is larger still (Conversations), it is back in its corner with
    /// its size. egui alone measures it on its first frame and keeps it
    /// inside the screen as if it had its default size, away from the corner.
    #[test]
    fn a_window_smaller_than_its_default_size_keeps_its_corner() {
        let palette = crate::theme::Theme::default().palette();
        let run = |ctx: &egui::Context, frame: usize, width: f32, height: f32, default_height: f32| {
            let input = egui::RawInput {
                screen_rect: Some(screen(width, height)),
                time: Some(frame as f64 / 60.0),
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                let size = Vec2::new(300.0, default_height);
                let pos = Pos2::new(width - 8.0, height - 38.0) - size;
                crate::ui::widgets::Floater::new(CORNER, "Window", pos, size)
                    .silent()
                    .show(ui.ctx(), &palette, &mut true, |ui| {
                        ui.label("content");
                    });
            });
            output.textures_delta.clear();
            ctx.memory(|m| m.area_rect(Id::unique(CORNER))).expect("shown")
        };
        let first = egui::Context::default();
        let mut rect = Rect::NOTHING;
        for frame in 0..3 {
            rect = run(&first, frame, 1280.0, 720.0, 252.0);
        }
        assert_eq!(rect, Rect::from_min_max(Pos2::new(972.0, 430.0), Pos2::new(1272.0, 682.0)));
        let saved = ron::to_string(&first.memory(|m| m.clone())).expect("serialize");
        // the same viewer window, then a larger one
        for (width, height) in [(1280.0, 720.0), (1600.0, 900.0)] {
            let next = egui::Context::default();
            restore(&next, &saved).expect("restore");
            for frame in 0..4 {
                rect = run(&next, frame, width, height, 432.0);
            }
            assert_eq!(rect.size(), Vec2::new(300.0, 252.0), "{width} x {height}");
            assert_eq!(rect.right_bottom(), Pos2::new(width - 8.0, height - 38.0), "{width} x {height}");
            let recorded = windows(&next, |w| w.placed[&Id::unique(CORNER).value()]);
            assert_eq!((recorded.rect, recorded.screen), (rect, screen(width, height)));
        }
    }

    /// A window closed while the viewer window changed size follows its
    /// edges when it opens again.
    #[test]
    fn a_window_reopened_after_a_resize_follows_its_edges() {
        let ctx = egui::Context::default();
        let palette = crate::theme::Theme::default().palette();
        let mut time = 0.0;
        let mut frame = |width: f32, open: bool| {
            time += 1.0 / 60.0;
            let input = egui::RawInput {
                screen_rect: Some(screen(width, 720.0)),
                time: Some(time),
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                crate::ui::widgets::Floater::new(CORNER, "Window", Pos2::new(972.0, 100.0), Vec2::new(300.0, 200.0))
                    .silent()
                    .show(ui.ctx(), &palette, &mut { open }, |ui| {
                        ui.label("content");
                    });
            });
            output.textures_delta.clear();
        };
        frame(1280.0, true);
        frame(1280.0, true);
        frame(1600.0, false);
        frame(1600.0, false);
        frame(1600.0, true);
        frame(1600.0, true);
        let rect = ctx.memory(|m| m.area_rect(Id::unique(CORNER))).expect("shown");
        assert_eq!(1600.0 - rect.right(), 8.0, "{rect:?}");
    }

    #[test]
    fn layout_round_trip_keeps_window_rects() {
        let ctx = egui::Context::default();
        let run = |ctx: &egui::Context| {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                egui::Window::new("t")
                    .id(egui::Id::unique("floater_t"))
                    .default_pos([123.0, 45.0])
                    .show(ui.ctx(), |ui| {
                        ui.label("x");
                    });
                ui.ctx().data_mut(|d| d.insert_persisted(egui::Id::unique("min"), true));
            });
            out.textures_delta.clear();
        };
        run(&ctx);
        let text = ron::to_string(&ctx.memory(|m| m.clone())).expect("serialize");
        let back: egui::Memory = ron::from_str(&text).expect("deserialize");
        let ctx2 = egui::Context::default();
        ctx2.memory_mut(|m| *m = back);
        assert_eq!(ctx2.data_mut(|d| d.get_persisted::<bool>(egui::Id::unique("min"))), Some(true));
        let rect = ctx2.memory(|m| m.area_rect(egui::Id::unique("floater_t")));
        assert!(rect.is_some_and(|r| (r.min.x - 123.0).abs() < 1.0), "{rect:?}");
    }

    /// The order of `App::new`: the interface font chosen in Preferences is
    /// installed after the saved layout. The other way round, a session with
    /// a saved layout started with egui's own fonts (and without the system
    /// fallback faces, #127), whatever the setting.
    #[test]
    fn fonts_installed_after_the_layout_are_kept() {
        let saved = ron::to_string(&egui::Context::default().memory(|m| m.clone())).expect("serialize");
        let first_font = |ctx: &egui::Context| {
            let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
            out.textures_delta.clear();
            ctx.fonts(|f| f.definitions().families[&egui::FontFamily::Proportional][0].clone())
        };
        let ctx = egui::Context::default();
        restore(&ctx, &saved).expect("restore");
        crate::theme::Theme::default().apply(&ctx, 1.4);
        crate::ui::fonts::apply(&ctx, crate::ui::fonts::DEFAULT);
        assert_eq!(first_font(&ctx), "aurora-main");
        assert_eq!(crate::ui::fonts::scale(&ctx), 1.4);
        ctx.fonts(|fonts| assert_eq!(fonts.definitions().font_data["aurora-main"].tweak.scale, 1.4));

        let ctx = egui::Context::default();
        crate::ui::fonts::apply(&ctx, crate::ui::fonts::DEFAULT);
        super::restore(&ctx, &saved).expect("restore");
        assert_ne!(first_font(&ctx), "aurora-main", "egui keeps pending fonts now: the order is free");
    }

    const ALL: Edges = Edges {
        left: true,
        right: true,
        bottom: true,
    };
    const NONE: Edges = Edges {
        left: false,
        right: false,
        bottom: false,
    };

    /// By default every edge stops a window: it stays whole on the screen,
    /// anywhere on it, never above the top.
    #[test]
    fn by_default_a_window_stays_whole_on_the_screen() {
        let room = screen(1280.0, 720.0);
        let size = Vec2::new(400.0, 300.0);
        assert_eq!(Edges::default(), ALL);
        assert_eq!(reach(room, size, ALL), room);
        let keep = |x: f32, y: f32| keep_title_in(room, Pos2::new(x, y), size, ALL);
        assert_eq!(keep(5000.0, 5000.0), Pos2::new(880.0, 420.0));
        assert_eq!(keep(-5000.0, -5000.0), Pos2::new(0.0, 0.0));
        assert_eq!(keep(300.0, 200.0), Pos2::new(300.0, 200.0));
    }

    /// An edge let open: a window crosses it, by the sides but `TITLE_GRIP`
    /// of its title bar, by the bottom but its whole title bar; the other
    /// edges still stop it, the top always does.
    #[test]
    fn an_open_edge_lets_a_window_cross_it_within_reach() {
        let room = screen(1280.0, 720.0);
        let size = Vec2::new(400.0, 300.0);
        let fits = |edges: Edges, pos: Pos2| reach(room, size, edges).contains_rect(Rect::from_min_size(pos, size));
        let only = |left, right, bottom| Edges { left, right, bottom };
        // right open
        assert!(fits(only(true, false, true), Pos2::new(1280.0 - TITLE_GRIP, 300.0)));
        assert!(!fits(only(true, false, true), Pos2::new(1281.0 - TITLE_GRIP, 300.0)));
        assert!(!fits(only(true, false, true), Pos2::new(-1.0, 300.0)), "the left still stops it");
        assert!(!fits(only(true, false, true), Pos2::new(300.0, 421.0)), "the bottom still stops it");
        // left open
        assert!(fits(only(false, true, true), Pos2::new(-(400.0 - TITLE_GRIP), 300.0)));
        assert!(!fits(only(false, true, true), Pos2::new(-(401.0 - TITLE_GRIP), 300.0)));
        assert!(!fits(only(false, true, true), Pos2::new(881.0, 300.0)), "the right still stops it");
        // bottom open: down to its title bar
        assert!(fits(only(true, true, false), Pos2::new(300.0, 720.0 - TITLE_BAR_H)));
        assert!(!fits(only(true, true, false), Pos2::new(300.0, 721.0 - TITLE_BAR_H)));
        // all open: never above the top
        assert!(!fits(NONE, Pos2::new(300.0, -1.0)));
        // a saved place comes back by the same rules
        for edges in [ALL, NONE, only(false, true, true), only(true, false, false)] {
            for pos in [Pos2::new(5000.0, -300.0), Pos2::new(-5000.0, 5000.0), Pos2::new(640.0, 360.0)] {
                assert!(fits(edges, keep_title_in(room, pos, size, edges)), "{edges:?} {pos:?}");
            }
        }
        // a window narrower than the grip never crosses a side
        assert_eq!(reach(room, Vec2::new(60.0, TITLE_BAR_H), NONE), room);
    }

    /// Islands of the top row at 8..44 px: one 600 px wide on the left, one
    /// 200 px wide on the right, a gap between them.
    fn top_islands() -> Vec<Rect> {
        vec![
            Rect::from_min_max(Pos2::new(8.0, 8.0), Pos2::new(608.0, 44.0)),
            Rect::from_min_max(Pos2::new(1072.0, 8.0), Pos2::new(1272.0, 44.0)),
        ]
    }

    /// A window can sit in the gap between two islands of the top row; one
    /// whose title would be hidden under an island comes out below the row,
    /// one under the bottom islands above them, the place it had across.
    #[test]
    fn a_title_left_under_the_islands_comes_out_of_them() {
        let room = screen(1280.0, 720.0);
        let islands = top_islands();
        let window = Vec2::new(300.0, 200.0);
        // in the gap, its title over the row: left as it is
        let gap = Pos2::new(700.0, 10.0);
        assert_eq!(clear_of_islands(room, gap, window, ALL, &islands), gap);
        // straddling an island but with a grip in the gap: left as it is
        let straddle = Pos2::new(500.0, 10.0);
        assert_eq!(clear_of_islands(room, straddle, window, ALL, &islands), straddle);
        // hidden under the left island: below the row, 8 px under it
        let hidden = Pos2::new(100.0, 10.0);
        assert_eq!(clear_of_islands(room, hidden, window, ALL, &islands), Pos2::new(100.0, 52.0));
        // a minimized window wider than the room left beside the island
        let minimized = Vec2::new(180.0, TITLE_BAR_H);
        assert_eq!(clear_of_islands(room, Pos2::new(1080.0, 20.0), minimized, ALL, &islands).y, 52.0);
        // a minimized one under the dock at the bottom: above it
        let dock = [Rect::from_min_max(Pos2::new(400.0, 668.0), Pos2::new(880.0, 712.0))];
        assert_eq!(
            clear_of_islands(room, Pos2::new(500.0, 670.0), minimized, ALL, &dock),
            Pos2::new(500.0, 668.0 - 8.0 - TITLE_BAR_H)
        );
        // a window let past the bottom, its title on the dock: above it too
        assert_eq!(
            clear_of_islands(room, Pos2::new(500.0, 670.0), window, NONE, &dock),
            Pos2::new(500.0, 668.0 - 8.0 - TITLE_BAR_H)
        );
        // no island: nothing moves
        assert_eq!(clear_of_islands(room, hidden, window, ALL, &[]), hidden);
    }

    /// What shows of a title bar beside the islands crossing its height.
    #[test]
    fn the_uncovered_width_of_a_title_bar() {
        let room = screen(1280.0, 720.0);
        let islands = top_islands();
        let title = |x: f32, y: f32, w: f32| Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, TITLE_BAR_H));
        assert_eq!(uncovered(title(700.0, 10.0, 300.0), room, &islands), 300.0);
        assert_eq!(uncovered(title(500.0, 10.0, 300.0), room, &islands), 192.0);
        assert_eq!(uncovered(title(100.0, 10.0, 300.0), room, &islands), 0.0);
        // below the row: nothing hides it
        assert_eq!(uncovered(title(100.0, 60.0, 300.0), room, &islands), 300.0);
        // partly off the screen: only what is on it counts
        assert_eq!(uncovered(title(1200.0, 300.0, 300.0), room, &islands), 80.0);
    }

    /// The save waits for the layout to stay still, each change pushing it
    /// back, and comes once.
    #[test]
    fn the_autosave_waits_for_the_layout_to_settle() {
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let mut autosave = Autosave::default();
        assert!(!autosave.mark(false, at(0)), "nothing changed, nothing to save");
        assert!(!autosave.mark(true, at(0)));
        assert!(!autosave.mark(false, at(1_000)));
        // moved again: the delay starts over
        assert!(!autosave.mark(true, at(1_500)));
        assert!(!autosave.mark(false, at(3_000)));
        assert!(autosave.mark(false, at(3_500)));
        assert!(!autosave.mark(false, at(10_000)), "saved once");
    }

    /// A layout with the window `id` at `x`.
    fn layout_at(id: &str, x: f32) -> egui::Memory {
        let ctx = egui::Context::default();
        // the second pass records the window's place
        for _ in 0..2 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                egui::Window::new(id)
                    .id(Id::unique(id))
                    .default_pos(Pos2::new(x, 50.0))
                    .show(ui.ctx(), |ui| ui.label("x"));
            });
            out.textures_delta.clear();
        }
        ctx.memory(|m| m.clone())
    }

    fn saved_x(p: &Path, id: &str) -> f32 {
        let saved = std::fs::read_to_string(p).expect("saved layout");
        let ctx = egui::Context::default();
        restore(&ctx, &saved).expect("readable layout");
        ctx.memory(|m| m.area_rect(Id::unique(id))).expect("saved window").min.x
    }

    /// A background save that ends after a newer one (the exit's) or after a
    /// reset never brings its older layout back.
    #[test]
    fn an_older_layout_never_replaces_a_newer_one() {
        let dir = crate::settings::config_dir().join(format!("layout-autosave-{}", uuid::Uuid::new_v4()));
        let p = dir.join("ui_layout.ron");
        let state = Mutex::new(Written { generation: 0, text: None });
        let moved = layout_at("autosave", 300.0);
        write_to(&state, &p, 2, &moved);
        write_to(&state, &p, 1, &layout_at("autosave", 100.0));
        assert_eq!(saved_x(&p, "autosave"), 300.0);
        // the same layout again: the file is left as it is
        let modified = std::fs::metadata(&p).and_then(|m| m.modified()).expect("modified");
        write_to(&state, &p, 3, &moved);
        assert_eq!(std::fs::metadata(&p).and_then(|m| m.modified()).expect("modified"), modified);
        // reset, then a save taken before it arrives
        discard(&state, &p, 5);
        write_to(&state, &p, 4, &layout_at("autosave", 100.0));
        assert!(!p.exists(), "the reset layout stays forgotten");
        write_to(&state, &p, 6, &layout_at("autosave", 200.0));
        assert_eq!(saved_x(&p, "autosave"), 200.0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
