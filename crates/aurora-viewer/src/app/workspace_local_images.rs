//! Managed PNG lifetime and explicit native import consent. No frame-thread IO.
use super::*;
use crate::{
    session_host::profile::{NativeDirectory, NativeStoreCancellation, NativeStoreKind},
    workspace::{
        controller::{Source, Status},
        local_images::{
            intent::{Intent, Target},
            model::Image,
            store::{Lane, Operation, Outcome, Request},
        },
        model::ManagedRef,
    },
};
use std::collections::HashSet;

enum Kind {
    Load(Vec<ManagedRef>),
    Choose,
    Publish,
}
struct Pending {
    source: Source,
    binding: u64,
    operation: u64,
    cancel: NativeStoreCancellation,
    kind: Kind,
}
enum Stage {
    Choosing,
    Prepared(Image),
    Publishing,
    Published(Image),
}
struct Import {
    scope: Intent,
    window: WindowId,
    stage: Stage,
}
#[derive(Default)]
pub(super) struct Runtime {
    lane: Option<Lane>,
    pending: Option<Pending>,
    source: Option<(Source, u64)>,
    operation: u64,
    nonce: u64,
    import: Option<Import>,
    known: HashSet<ManagedRef>,
    pub view: ui::workspace_background::LocalImages,
}
impl Runtime {
    pub(super) fn busy(&self) -> bool {
        self.import.is_some() || self.pending.is_some()
    }
    pub(super) fn cancel_import(&mut self) {
        self.import = None;
        if let Some(p) = &self.pending
            && !matches!(p.kind, Kind::Load(_))
        {
            p.cancel.cancel();
        }
    }
    pub(super) fn retire(&mut self) {
        self.cancel_import();
        if let Some(p) = &self.pending {
            p.cancel.cancel();
        }
        self.source = None;
        self.known.clear();
        self.view.clear();
        // Keep the original busy lane until its old reply drains. A native
        // dialog can block one worker, never spawn replacements or block App.
    }
    fn issue(&mut self, source: &Source, binding: u64, action: Operation, kind: Kind) -> bool {
        if self.pending.is_some() {
            return false;
        }
        let Some(operation) = self.operation.checked_add(1) else {
            return false;
        };
        self.operation = operation;
        let Ok(directory) = NativeDirectory::select(NativeStoreKind::WorkspaceImages, source.agent, &source.grid) else {
            return false;
        };
        if self.lane.is_none() {
            self.lane = Lane::new().ok();
        }
        let Some(lane) = &self.lane else {
            return false;
        };
        let cancel = NativeStoreCancellation::new();
        if !lane.submit(Request {
            source: source.clone(),
            binding,
            operation,
            directory,
            cancellation: cancel.clone(),
            action,
        }) {
            return false;
        }
        self.pending = Some(Pending {
            source: source.clone(),
            binding,
            operation,
            cancel,
            kind,
        });
        true
    }
    fn poll(&mut self, app: &mut App, source: Option<&Source>, wanted: &[ManagedRef]) {
        let result = match self.lane.as_ref().map(Lane::poll) {
            Some(Ok(Some(r))) => r,
            Some(Err(())) => {
                self.pending = None;
                self.cancel_import();
                return;
            }
            _ => return,
        };
        if self
            .pending
            .as_ref()
            .is_none_or(|p| p.source != result.source || p.binding != result.binding || p.operation != result.operation)
        {
            return;
        }
        let p = self.pending.take().expect("matched managed image reply");
        if p.cancel.is_cancelled() || source != Some(&p.source) || app.workspaces.binding != p.binding {
            return;
        }
        match (p.kind, result.result) {
            (Kind::Load(refs), Ok(Outcome::Loaded(images))) => {
                self.known.extend(refs.iter().cloned());
                for image in images {
                    let Some(r) = refs.iter().find(|r| r.image == image.id && wanted.contains(r)) else {
                        continue;
                    };
                    if image.size[0] == 0
                        || image.size[1] == 0
                        || image.size[0] > 256
                        || image.size[1] > 256
                        || image.rgba.len() != (image.size[0] * image.size[1] * 4) as usize
                    {
                        continue;
                    }
                    let pixels = egui::ColorImage::from_rgba_unmultiplied([image.size[0] as usize, image.size[1] as usize], &image.rgba);
                    let texture = app.egui_ctx.load_texture(
                        format!("workspace-managed-{}", image.id),
                        pixels,
                        egui::TextureOptions {
                            wrap_mode: egui::TextureWrapMode::Repeat,
                            ..egui::TextureOptions::LINEAR
                        },
                    );
                    self.view.insert(
                        r.clone(),
                        ui::workspace_background::PictureImage {
                            texture,
                            original: image.original,
                        },
                    );
                }
            }
            (Kind::Load(refs), Err(_)) => {
                self.known.extend(refs);
            }
            (Kind::Choose, Ok(Outcome::Prepared(Some(image)))) => {
                if let Some(i) = &mut self.import {
                    i.scope.normalized(result.finished);
                    i.stage = Stage::Prepared(image);
                }
            }
            (Kind::Publish, Ok(Outcome::Published(image))) => {
                if let Some(i) = &mut self.import {
                    i.scope.published(result.finished);
                    i.stage = Stage::Published(image);
                }
            }
            (_, Ok(Outcome::Prepared(None))) => self.cancel_import(),
            _ => {
                self.cancel_import();
                app.workspaces.status = Status::LocalImageUnavailable;
            }
        }
    }
    fn tick(&mut self, app: &mut App, renderer: &Renderer, window: &Window, mut wanted: Vec<ManagedRef>) {
        let actual = app
            .workspace_source()
            .filter(|s| crate::workspace::images::source_current(&app.world, s));
        if self
            .source
            .as_ref()
            .is_some_and(|s| Some(&s.0) != actual.as_ref() || s.1 != app.workspaces.binding)
        {
            self.retire();
        }
        if let Some(s) = &actual {
            self.source = Some((s.clone(), app.workspaces.binding));
        } else {
            self.retire();
        }
        wanted.retain(|r| actual.as_ref().is_some_and(|s| r.account == s.agent && r.grid == s.grid));
        if wanted.len() > 18 {
            wanted.clear();
        }
        self.view.retain(|r, _| wanted.contains(r));
        self.known.retain(|r| wanted.contains(r));
        self.poll(app, actual.as_ref(), &wanted);
        if let Some(i) = &self.import
            && (!app.workspace_ui.open
                || app.panels.settings
                || i.window != window.id()
                || !i.scope.matches(&app.workspaces, actual.as_ref(), self.nonce, Instant::now()))
        {
            self.cancel_import();
        }
        if let Some(mut i) = self.import.take() {
            let allowed = app.workspaces.ready() && app.workspace_permit(renderer, window) && !app.panels.settings;
            match &i.stage {
                Stage::Prepared(image) if allowed => {
                    if app.workspace_picture_target(&i.scope.target, window)
                        && actual
                            .as_ref()
                            .is_some_and(|s| self.issue(s, app.workspaces.binding, Operation::Publish(image.clone()), Kind::Publish))
                    {
                        i.scope.publishing();
                        i.stage = Stage::Publishing;
                    }
                    self.import = Some(i);
                }
                Stage::Published(image) if allowed => {
                    if app.workspace_picture_target(&i.scope.target, window)
                        && let Some(s) = &actual
                        && let Ok(reference) = ManagedRef::new(image.id, s.agent, s.grid.clone())
                    {
                        let target = i.scope.target.as_ref().map(|(a, n)| (a, *n));
                        let old = app
                            .workspaces
                            .background()
                            .paint(target.map(|t| t.0), target.map(|t| t.1))
                            .draw_image();
                        let picture = crate::workspace::model::LocalPicture {
                            reference,
                            mode: old.as_ref().map_or(crate::workspace::model::ImageMode::Fill, |p| p.mode()),
                            brightness: old.as_ref().map_or(255, |p| p.brightness()),
                        };
                        if let Some(background) = app.workspaces.background().with_local(target, picture)
                            && let Some(mut catalogue) = app.workspaces.catalogue.clone()
                        {
                            catalogue.background = background;
                            app.workspace_save_definitions(catalogue);
                        }
                    }
                }
                _ => self.import = Some(i),
            }
        }
        if self.import.is_none()
            && self.pending.is_none()
            && let Some(source) = &actual
        {
            let missing = wanted.into_iter().filter(|r| !self.known.contains(r)).collect::<Vec<_>>();
            if !missing.is_empty()
                && !self.issue(
                    source,
                    app.workspaces.binding,
                    Operation::Load(missing.iter().map(|r| r.image).collect()),
                    Kind::Load(missing.clone()),
                )
            {
                self.known.extend(missing);
                app.workspaces.status = Status::LocalImageUnavailable;
            }
        }
    }
    pub(super) fn current(&self, world: &World, source: Option<&Source>, binding: u64) -> bool {
        source.is_some_and(|s| {
            crate::workspace::images::source_current(world, s) && self.source.as_ref().is_some_and(|(old, b)| old == s && *b == binding)
        })
    }
}
impl App {
    pub(super) fn workspace_local_picture_write(
        &mut self,
        target: Target,
        picture: crate::workspace::model::LocalPicture,
        window: &Window,
    ) {
        let Some(s) = self
            .workspace_source()
            .filter(|s| crate::workspace::images::source_current(&self.world, s))
        else {
            return;
        };
        if picture.reference.account != s.agent
            || picture.reference.grid != s.grid
            || self.workspaces.preview.is_some()
            || !self.workspace_picture_target(&target, window)
        {
            return;
        }
        if let Some(background) = self
            .workspaces
            .background()
            .with_local(target.as_ref().map(|(a, n)| (a, *n)), picture)
            && let Some(mut catalogue) = self.workspaces.catalogue.clone()
        {
            catalogue.background = background;
            self.workspace_save_definitions(catalogue);
        }
    }
    pub(super) fn workspace_local_prepare(&mut self, renderer: &Renderer, window: &Window, refs: Vec<ManagedRef>) {
        let mut runtime = std::mem::take(&mut self.workspace_local_images);
        runtime.tick(self, renderer, window, refs);
        self.workspace_local_images = runtime;
    }
    pub(super) fn workspace_local_begin(&mut self, target: Target, window: &Window) {
        if !self.workspace_picture_target(&target, window) {
            return;
        }
        let Some(source) = self
            .workspace_source()
            .filter(|s| crate::workspace::images::source_current(&self.world, s))
        else {
            return;
        };
        let mut r = std::mem::take(&mut self.workspace_local_images);
        r.cancel_import();
        if let Some(nonce) = r.nonce.checked_add(1)
            && let Some(scope) = Intent::new(&self.workspaces, nonce, target)
        {
            r.nonce = nonce;
            if r.issue(
                &source,
                self.workspaces.binding,
                Operation::Choose(aurora_i18n::tr!("workspaces-local-choose").to_owned()),
                Kind::Choose,
            ) {
                r.import = Some(Import {
                    scope,
                    window: window.id(),
                    stage: Stage::Choosing,
                });
            } else {
                self.workspaces.status = Status::LocalImageUnavailable;
            }
        }
        self.workspace_local_images = r;
    }
}
