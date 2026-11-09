//! Releasable, source-qualified backgrounds through the shared texture streamer.
//! Up to18 small UI copies (17 saved choices plus one explicit picker preview),
//! no full-resolution allocations, new worker pool, readback or network bypass.
use super::*;
use crate::workspace::{
    controller::{Source, Ticket},
    model::{Arrangement, TextureRef},
};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

pub(super) type Target = Option<(Arrangement, usize)>;
pub(super) struct Choice {
    pub ticket: Ticket,
    pub target: Target,
}
struct Entry {
    texture: Option<egui::TextureHandle>,
    size: [u32; 2],
    complete: bool,
}
impl Entry {
    fn store(&mut self, ctx: &egui::Context, asset: Uuid, w: u32, h: u32, rgba: &[u8]) -> bool {
        if w == 0 || h == 0 || w > 256 || h > 256 || rgba.len() != (w * h * 4) as usize || (w <= self.size[0] && h <= self.size[1]) {
            return false;
        }
        let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], rgba);
        let options = egui::TextureOptions {
            wrap_mode: egui::TextureWrapMode::Repeat,
            ..egui::TextureOptions::LINEAR
        };
        if let Some(texture) = &mut self.texture {
            texture.set(image, options);
        } else {
            self.texture = Some(ctx.load_texture(format!("workspace-picture-{asset}"), image, options));
        }
        self.size = [w, h];
        true
    }
}
pub(super) struct Images {
    source: Option<(Source, u64)>,
    entries: HashMap<Uuid, Entry>,
    pub policy: crate::workspace::images::Policy,
    pub view: ui::workspace_background::Images,
    pub picker: ui::texture_picker::TexturePicker,
    pub choice: Option<Choice>,
    picker_reference: Option<(
        Uuid,
        Option<crate::world::inventory_archive::visibility::VisibilityBinding>,
        Option<TextureRef>,
    )>,
}
impl Default for Images {
    fn default() -> Self {
        Self {
            source: None,
            entries: HashMap::new(),
            policy: Default::default(),
            view: HashMap::new(),
            picker: ui::texture_picker::TexturePicker::workspace(),
            choice: None,
            picker_reference: None,
        }
    }
}
impl Images {
    pub(super) fn cancel_picker(&mut self) {
        self.picker.open = false;
        self.picker.wanted_images.clear();
        self.choice = None;
        self.picker_reference = None;
    }
    pub(super) fn clear(&mut self, scene: &mut Scene) {
        for id in self.entries.keys() {
            scene.textures.release_background_ui_image(id);
        }
        self.entries.clear();
        self.view.clear();
        self.cancel_picker();
        if self.source.take().is_some() {
            self.policy.clear();
        }
    }
    pub(super) fn prepare(
        &mut self,
        ctx: &egui::Context,
        scene: &mut Scene,
        renderer: &mut Renderer,
        world: &World,
        source: Option<&Source>,
        binding: u64,
        mut references: Vec<TextureRef>,
    ) {
        let Some(source) = source else {
            self.clear(scene);
            return;
        };
        if self.source.as_ref().is_some_and(|s| s != &(source.clone(), binding)) {
            self.clear(scene);
        }
        self.source = Some((source.clone(), binding));
        let wanted = std::mem::take(&mut self.picker.wanted_images);
        if self.choice.is_some()
            && let Some(asset) = wanted.into_iter().min()
        {
            let visibility = self.policy.visibility(world, source);
            if self
                .picker_reference
                .as_ref()
                .is_none_or(|(old, v, _)| *old != asset || *v != visibility)
            {
                self.picker_reference = Some((asset, visibility, self.policy.choose(world, source, asset)));
            }
            if let Some((_, _, Some(r))) = &self.picker_reference
                && !references.contains(r)
            {
                references.push(r.clone());
            }
        }
        self.policy.refresh(world, Some(source), binding, &references);
        references.retain(|r| self.policy.allows(r));
        let assets = references.iter().map(|r| r.asset).collect::<HashSet<_>>();
        let removed = self.entries.keys().filter(|id| !assets.contains(id)).copied().collect::<Vec<_>>();
        for id in removed {
            scene.textures.release_background_ui_image(&id);
            self.entries.remove(&id);
        }
        for asset in assets {
            let e = self.entries.entry(asset).or_insert(Entry {
                texture: None,
                size: [0; 2],
                complete: false,
            });
            if e.complete {
                continue;
            }
            scene.textures.want_background_ui_image(renderer, asset, &scene.jobs);
            if let Some((w, h, rgba)) = scene.textures.take_ui_image(&asset) {
                e.store(ctx, asset, w, h, &rgba);
            }
            if e.texture.is_some()
                && scene
                    .textures
                    .ui_image_size(&asset)
                    .is_none_or(|(w, h)| e.size[0] >= w && e.size[1] >= h)
            {
                e.complete = true;
                scene.textures.release_background_ui_image(&asset);
            } else {
                scene.textures.refine_ui_image_size(asset, (e.size[0], e.size[1]), &scene.jobs);
            }
        }
        self.view.clear();
        for r in references {
            if let Some(e) = self.entries.get(&r.asset)
                && let Some(texture) = &e.texture
            {
                let original = scene
                    .textures
                    .full_size(&r.asset)
                    .filter(|(w, h)| *w > 0 && *h > 0 && *w <= 1 << 20 && *h <= 1 << 20)
                    .map_or(e.size, |(w, h)| [w, h]);
                self.view.insert(
                    r,
                    ui::workspace_background::PictureImage {
                        texture: texture.clone(),
                        original,
                    },
                );
            }
        }
    }
    pub(super) fn picker_images(&self) -> HashMap<Uuid, egui::TextureHandle> {
        self.view.iter().map(|(r, image)| (r.asset, image.texture.clone())).collect()
    }
}

impl App {
    pub(super) fn workspace_picture_target(&mut self, target: &Target, window: &Window) -> bool {
        let Some((arrangement, index)) = target else { return true };
        self.workspace_displays.refresh(window, self.egui_ctx.pixels_per_point());
        *index < arrangement.len() && self.workspace_displays.arrangement() == Some(arrangement)
    }
    pub(super) fn workspace_picture_write(&mut self, target: Target, picture: crate::workspace::model::Picture, window: &Window) {
        use crate::workspace::controller::Status;
        if self.workspaces.preview.is_some() || !self.workspace_picture_target(&target, window) {
            return;
        }
        let Some(source) = self.workspace_source() else { return };
        if !self.workspace_images.policy.accepts(&self.world, &source, &picture.texture) {
            self.workspaces.status = Status::PictureUnavailable;
            return;
        }
        let Some(background) = self
            .workspaces
            .background()
            .with_picture(target.as_ref().map(|(a, i)| (a, *i)), picture)
        else {
            return;
        };
        let Some(mut next) = self.workspaces.catalogue.clone() else {
            return;
        };
        next.background = background;
        self.workspace_save_definitions(next);
    }
    pub(super) fn workspace_picture_choose(&mut self, target: Target, window: &Window) {
        use crate::workspace::controller::Status;
        if self.workspaces.preview.is_some() || !self.workspace_picture_target(&target, window) {
            return;
        }
        let Some(ticket) = self.workspaces.ticket() else { return };
        if self.workspace_images.policy.visibility(&self.world, &ticket.source).is_none() {
            self.workspaces.status = Status::PictureUnavailable;
            return;
        }
        let current = self
            .workspaces
            .background()
            .paint(target.as_ref().map(|t| &t.0), target.as_ref().map(|t| t.1))
            .picture()
            .map_or(Uuid::nil(), |p| p.texture.asset);
        self.workspace_images
            .picker
            .open(aurora_i18n::tr!("workspaces-picture-choose"), current);
        self.workspace_images.choice = Some(Choice { ticket, target });
    }
    pub(super) fn workspace_picture_picker(
        &mut self,
        ctx: &egui::Context,
        palette: &crate::theme::Palette,
        renderer: &Renderer,
        window: &Window,
    ) -> Option<crate::workspace::controller::Action> {
        use crate::workspace::{
            controller::{Action, Command, Status},
            model::{ImageMode, Picture},
        };
        if self
            .workspace_images
            .choice
            .as_ref()
            .is_none_or(|c| !self.workspaces.accepts(&c.ticket) || self.workspace_source().as_ref() != Some(&c.ticket.source))
            || !self.workspace_ui.open
            || self.workspaces.preview.is_some()
            || self.panels.settings
            || !self.workspace_permit(renderer, window)
            || self.world.rlva.has("showinv")
            || self.world.rlva.has("viewtexture")
        {
            self.workspace_images.cancel_picker();
            return None;
        }
        // Inventory/privacy can change earlier in this same UI pass. The
        // chooser may not borrow cached pixels from an earlier ordinary read.
        let source = self.workspace_source();
        let images = if self
            .workspace_images
            .policy
            .current(&self.world, source.as_ref(), self.workspaces.binding)
        {
            self.workspace_images.picker_images()
        } else {
            HashMap::new()
        };
        let chosen = self.workspace_images.picker.show(ctx, palette, &mut self.world, &images);
        if let Some(asset) = chosen {
            let choice = self.workspace_images.choice.take()?;
            let reference = self.workspace_images.policy.choose(&self.world, &choice.ticket.source, asset);
            self.workspace_images.cancel_picker();
            let Some(texture) = reference else {
                self.workspaces.status = Status::PictureUnavailable;
                return None;
            };
            let old = self
                .workspaces
                .background()
                .paint(choice.target.as_ref().map(|t| &t.0), choice.target.as_ref().map(|t| t.1));
            let (mode, brightness) = old.picture().map_or((ImageMode::Fill, 255), |p| (p.mode, p.brightness));
            return Some(Action {
                ticket: choice.ticket,
                command: Command::BackgroundPicture(choice.target, Picture { texture, mode, brightness }),
            });
        }
        if !self.workspace_images.picker.open {
            self.workspace_images.cancel_picker();
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn workspace_picture_cache_rejects_oversized_partial_pixels_and_releases_its_owned_repeat_texture() {
        let ctx = egui::Context::default();
        let mut entry = Entry {
            texture: None,
            size: [0; 2],
            complete: false,
        };
        let asset = Uuid::new_v4();
        for (w, h, bytes) in [
            (0, 1, 4),
            (1, 0, 4),
            (257, 1, 1028),
            (1, 257, 1028),
            (2, 2, 15),
            (u32::MAX, u32::MAX, 0),
        ] {
            assert!(!entry.store(&ctx, asset, w, h, &vec![255; bytes]));
            assert!(entry.texture.is_none());
        }
        assert!(entry.store(&ctx, asset, 2, 4, &[255; 32]));
        let id = entry.texture.as_ref().unwrap().id();
        assert_eq!(entry.texture.as_ref().unwrap().size(), [2, 4]);
        assert!(!entry.store(&ctx, asset, 1, 2, &[255; 8]));
        assert!(entry.store(&ctx, asset, 4, 8, &[128; 128]));
        assert_eq!(entry.texture.as_ref().unwrap().id(), id, "refinement reuses one owned allocation");
        drop(entry);
        assert!(ctx.tex_manager().read().meta(id).is_none());
    }
}
