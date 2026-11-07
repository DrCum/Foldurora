//! The photo window, anchored on the right of the world (Firestorm's
//! snapshot floater, floater_snapshot.xml / panel_snapshot_local.xml, laid
//! out for photographers: preview with its guide, proportions and sizes in
//! one click, file, what the image holds, focal length, latest photos), and
//! the full-frame view: the interface gone but a small command strip, the
//! world outside the photo's proportions darkened, a composition guide.

use super::widgets::{self, ActionKind, Floater};
use crate::snapshot::inventory::TextureSize;
use crate::snapshot::profile::ProfileSize;
use crate::snapshot::{Aspect, FileFormat, Guide, Resolution, Settings};
use crate::theme::Palette;
use egui::{Color32, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use std::path::{Path, PathBuf};

pub const WINDOW: &str = "photo";
/// Firestorm's floater (Preferences › Aurora › Photo window): its own
/// place and size, kept apart from the panel's.
pub const CLASSIC_WINDOW: &str = "photo_classic";
/// Side of the classic window's preview (floater_snapshot.xml's
/// thumbnail_placeholder, 420 x 420).
const CLASSIC_PREVIEW: f32 = 420.0;
const WIDTH: f32 = 340.0;
/// Focal lengths of the slider and its presets (mm, full frame).
pub(crate) const FOCAL_RANGE: std::ops::RangeInclusive<f32> = 10.0..=135.0;
const FOCAL_PRESETS: [f32; 5] = [24.0, 35.0, 50.0, 85.0, 135.0];

#[derive(Default)]
pub struct SnapshotUi {
    /// The full-frame view instead of the interface.
    pub full_frame: bool,
    /// The photo is taken and shown still until saved or released.
    pub freeze: bool,
    /// The world holds still while the camera moves (never saved: the
    /// photo mode's end thaws it).
    pub freeze_world: bool,
    /// Where "Take the photo" sends it (the disk each time the viewer
    /// starts: a photo that costs L$ is never one by surprise).
    pub destination: Destination,
    /// The caption of a profile post (not kept, like Firestorm's editor).
    pub caption: String,
    /// The e-mail of a postcard (not kept either).
    pub postcard: PostcardForm,
    /// The quick light, for the photo only (never saved), set in the
    /// viewfinder's light panel (`light_only`).
    pub light: crate::snapshot::light::PhotoLight,
    /// The free camera's switch and flight (never saved).
    pub free: crate::camera::free::FreeSettings,
    /// The looks editor's state (`ui::looks_editor`).
    pub looks: super::looks_editor::LooksUi,
    /// The user's 3D tables, by file name (`snapshot::luts`).
    pub lut_names: Vec<String>,
}

/// Where a photo goes (Firestorm's snapshot panels; the profile feed
/// comes later).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Destination {
    #[default]
    Disk,
    Inventory,
    Profile,
    /// A postcard (LLPanelSnapshotPostcard).
    Email,
}

/// The e-mail of a postcard (panel_postcard_message.xml).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PostcardForm {
    /// The recipients, separated by commas.
    pub to: String,
    /// The sender's name; the app writes ours while it is empty.
    pub name: String,
    pub subject: String,
    pub message: String,
}

/// panel_postcard_message.xml's max_length_bytes / max_length.
const POSTCARD_NAME_MAX: usize = 100;
const POSTCARD_SUBJECT_MAX: usize = 100;
const POSTCARD_MESSAGE_MAX: usize = 700;

/// A photo sent by e-mail.
pub struct EmailView {
    /// The region takes postcards (SendPostcard), or the demo.
    pub available: bool,
    pub size: (u32, u32),
    /// Our name, sent while "From" is empty (shown as its hint).
    pub sender: String,
}

/// A photo for the profile feed.
pub struct ProfileView {
    /// Logged in (or in demo).
    pub connected: bool,
    /// The web profile knows who we are (the login's OpenID cookie came).
    pub ready: bool,
    pub size: (u32, u32),
}

/// The environment under the photo's light, for its Light section.
pub struct EnvView<'a> {
    /// The sky, water and day cycle lists (the environment window's).
    pub catalog: &'a crate::world::env_select::SettingsCatalog,
    /// What the user's own environment shows in each list.
    pub own: crate::world::eep_env::LocalView,
    /// The sun of the sky shown before the photo turns it: azimuth and
    /// elevation (degrees), what the sun sliders show and restore.
    pub sun: (f32, f32),
    /// RLVa holds the environment (@setenv): nothing to change.
    pub locked: bool,
    /// An item the photo picked is still loading.
    pub loading: bool,
}

/// A photo for the inventory: the texture it becomes and its price.
pub struct InventoryView {
    pub texture: (u32, u32),
    /// The upload cost in L$; None while it is not known.
    pub cost: Option<i32>,
    /// Logged in (or in demo): the upload is possible.
    pub connected: bool,
}

/// What the window shows besides the settings.
pub struct View<'a> {
    pub window: (u32, u32),
    pub photo: (u32, u32),
    /// Parts the photo is drawn in (1: one pass).
    pub tiles: usize,
    pub preview: Option<&'a egui::TextureHandle>,
    /// Work in progress: (0..1, what).
    pub busy: Option<(f32, String)>,
    /// Last outcome: (text, is an error).
    pub message: Option<&'a (String, bool)>,
    pub last_saved: Option<&'a Path>,
    pub gallery: &'a [(PathBuf, Option<egui::TextureHandle>)],
    /// The session's photos, shown instead of the folder's once there is
    /// one (None in the Legacy style).
    pub reel: Option<crate::ui::photo_reel::ReelView<'a>>,
    pub folder: PathBuf,
    /// The destination is the inventory.
    pub inventory: Option<InventoryView>,
    /// The message tells of a photo saved to the inventory ("Show").
    pub item_saved: bool,
    /// The destination is the profile feed.
    pub profile: Option<ProfileView>,
    /// The message tells of a post to the profile feed ("View").
    pub profile_posted: bool,
    /// The destination is a postcard.
    pub email: Option<EmailView>,
    /// Preferences' exposure, under the photo's own.
    pub exposure: f32,
    /// HDR and Emissive is on: without it no exposure applies, the row is
    /// greyed.
    pub hdr: bool,
    pub env: EnvView<'a>,
    /// The key that brings the free camera back (Escape by default).
    pub back_key: Option<crate::keybinds::Binding>,
    /// The photo quality is lowered to stay within the video memory.
    pub quality_reduced: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Refresh,
    Take,
    SaveAs,
    ChooseFolder,
    OpenFolder,
    OpenPhoto(PathBuf),
    /// Freeze frame switched (on: take it now).
    Freeze(bool),
    /// A setting that changes the photo changed: the preview follows.
    Changed,
    /// The settings changed (saved, the preview unchanged).
    Saved,
    /// Show the photo saved to the inventory.
    ShowItem,
    /// Open our profile (its feed shows the post).
    ShowProfile,
    /// Open the personal lighting window.
    PersonalLighting,
    /// "Keep this light": the photo's environment becomes the user's.
    KeepLight,
    /// Open the Controls window (it drives the free camera).
    Controls,
    /// The upload's price was confirmed (true) or not.
    Confirm(bool),
    /// The "cannot afford" message was closed.
    CloseAfford,
    /// The first "Photo quality" warning was answered: go on (true) or not.
    QualityWarning(bool),
    /// Choose a .cube file to add to the tables of the photo's look.
    ImportLut,
    /// Delete one of the tables (its file name).
    RemoveLut(String),
    /// Show the folder of the tables.
    OpenLutFolder,
    /// The session's photos (`ui::photo_reel`).
    Reel(crate::ui::photo_reel::ReelAction),
}

impl SnapshotUi {
    /// The window; `fov` is the camera's vertical field of view (radians),
    /// `default_fov` the viewer's.
    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        open: &mut bool,
        s: &mut Settings,
        fov: &mut f32,
        default_fov: f32,
        view: &View,
        classic: bool,
    ) -> Vec<Action> {
        let mut out = Vec::new();
        self.looks.hover = None;
        let screen = super::layout::toolbar_rect(ctx);
        let mut full = false;
        let full_tip = aurora_i18n::tr!("photo-full-frame");
        if classic {
            // a window of its own, placed and moved like Firestorm's floater
            let size = Vec2::new(WIDTH + CLASSIC_PREVIEW + 8.0, CLASSIC_PREVIEW + 160.0);
            Floater::new(CLASSIC_WINDOW, aurora_i18n::tr!("photo-title"), screen.center() - size * 0.5, size)
                .viewport_control()
                .auto_height()
                .action("maximize", full_tip, &mut full)
                .show(ctx, p, open, |ui| self.body_classic(ui, p, s, fov, default_fov, view, &mut out));
        } else {
            Floater::new(
                WINDOW,
                aurora_i18n::tr!("photo-title"),
                Pos2::new(screen.right() - WIDTH - 24.0, screen.top() + 64.0),
                Vec2::new(WIDTH, 640.0),
            )
            .viewport_control()
            .fixed()
            .auto_height()
            .action("maximize", full_tip, &mut full)
            .show(ctx, p, open, |ui| self.body(ui, p, s, fov, default_fov, view, &mut out));
        }
        if full {
            self.full_frame = true;
        }
        if !*open {
            self.full_frame = false;
            if self.freeze {
                self.freeze = false;
                out.push(Action::Freeze(false));
            }
        }
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn body(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        s: &mut Settings,
        fov: &mut f32,
        default_fov: f32,
        view: &View,
        out: &mut Vec<Action>,
    ) {
        ui.set_width(WIDTH - 24.0);
        ui.spacing_mut().item_spacing.y = 6.0;
        if preview(ui, p, s, view) {
            out.push(Action::Refresh);
        }

        let before = s.clone();
        // the settings scroll between the preview and the shutter, which
        // stay in sight on a short screen
        let room = (super::layout::toolbar_rect(ui.ctx()).height() - 470.0).max(150.0);
        egui::ScrollArea::vertical()
            .id_salt("photo_settings")
            .max_height(room)
            .show(ui, |ui| self.settings(ui, p, s, fov, default_fov, view, out));

        self.actions(ui, p, view, out);

        changed(s, &before, out);
    }

    /// The viewfinder's drawer: every setting of the panel, its shutter,
    /// progress, last outcome and latest photos, without the preview (the
    /// viewfinder is one); the settings scroll in `room` points.
    #[allow(clippy::too_many_arguments)]
    pub fn drawer(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        s: &mut Settings,
        fov: &mut f32,
        default_fov: f32,
        view: &View,
        room: f32,
        out: &mut Vec<Action>,
    ) {
        ui.spacing_mut().item_spacing.y = 6.0;
        let before = s.clone();
        let freeze = self.freeze;
        egui::ScrollArea::vertical()
            .id_salt("photo_drawer")
            .max_height(room)
            .show(ui, |ui| self.settings(ui, p, s, fov, default_fov, view, out));
        self.actions(ui, p, view, out);
        changed(s, &before, out);
        if self.freeze != freeze && !out.contains(&Action::Changed) {
            out.push(Action::Changed);
        }
    }

    /// The viewfinder's light panel, opened by the dock's sun button: the
    /// only place of the photo's light.
    pub fn light_only(&mut self, ui: &mut egui::Ui, p: &Palette, view: &View, out: &mut Vec<Action>) {
        ui.spacing_mut().item_spacing.y = 6.0;
        let before = self.light;
        light_rows(ui, p, &mut self.light, view.exposure, view.hdr, &view.env, out);
        if self.light != before && !out.contains(&Action::Changed) {
            out.push(Action::Changed);
        }
    }

    /// The shutter (or the destination's button), the progress or the last
    /// outcome, and the latest photos.
    fn actions(&mut self, ui: &mut egui::Ui, p: &Palette, view: &View, out: &mut Vec<Action>) {
        ui.add_space(4.0);
        let busy = view.busy.is_some();
        ui.horizontal(|ui| match (&view.email, &view.inventory, &view.profile) {
            (Some(mail), _, _) => {
                let ready = !busy && mail.available;
                if widgets::action_button_if(
                    ui,
                    p,
                    "mail-filled",
                    aurora_i18n::tr!("photo-email-send"),
                    ActionKind::Primary,
                    Vec2::new(ui.available_width(), 32.0),
                    ready,
                )
                .clicked()
                {
                    out.push(Action::Take);
                }
            }
            (None, _, Some(prof)) => {
                let ready = !busy && prof.connected && prof.ready;
                if widgets::action_button_if(
                    ui,
                    p,
                    "send-filled",
                    aurora_i18n::tr!("photo-profile-post"),
                    ActionKind::Primary,
                    Vec2::new(ui.available_width(), 32.0),
                    ready,
                )
                .clicked()
                {
                    out.push(Action::Take);
                }
            }
            (None, Some(inv), None) => {
                let ready = !busy && inv.connected && inv.cost.is_some();
                if widgets::action_button_if(
                    ui,
                    p,
                    "file-upload-filled",
                    aurora_i18n::tr!("photo-save-inventory"),
                    ActionKind::Primary,
                    Vec2::new(ui.available_width(), 32.0),
                    ready,
                )
                .clicked()
                {
                    out.push(Action::Take);
                }
            }
            (None, None, None) => {
                let take_w = ui.available_width() - 34.0;
                ui.add_enabled_ui(!busy, |ui| {
                    if widgets::action_button(
                        ui,
                        p,
                        "camera-filled",
                        aurora_i18n::tr!("photo-take"),
                        ActionKind::Primary,
                        Vec2::new(take_w, 32.0),
                    )
                    .clicked()
                    {
                        out.push(Action::Take);
                    }
                    if widgets::action_button(ui, p, "folder-filled", "", ActionKind::Flat, Vec2::new(30.0, 32.0))
                        .on_hover_text(aurora_i18n::tr!("photo-save-as"))
                        .clicked()
                    {
                        out.push(Action::SaveAs);
                    }
                });
            }
        });
        if let Some((fraction, what)) = &view.busy {
            widgets::progress(ui, p, "photo_progress", *fraction, ui.available_width(), what, None);
        } else if let Some((text, error)) = view.message {
            // the button on the right, the text cut before it (a long file
            // name never widens the window)
            let line = Vec2::new(ui.available_width(), 26.0);
            ui.allocate_ui_with_layout(line, egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let button = match () {
                    _ if *error => None,
                    _ if view.item_saved => Some((aurora_i18n::tr!("photo-show-in-inventory"), Action::ShowItem)),
                    _ if view.profile_posted => Some((aurora_i18n::tr!("photo-profile-show"), Action::ShowProfile)),
                    _ => view
                        .last_saved
                        .map(|path| (aurora_i18n::tr!("photo-open"), Action::OpenPhoto(path.to_path_buf()))),
                };
                if let Some((label, action)) = button
                    && widgets::flat_button(ui, p, label).clicked()
                {
                    out.push(action);
                }
                let color = if *error { p.danger } else { p.success };
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add(egui::Label::new(RichText::new(text).size(12.0).color(color)).truncate());
                });
            });
        }
        gallery(ui, p, view, out);
    }

    /// The postcard's e-mail (panel_postcard_message.xml) and its JPEG
    /// quality (SnapshotQuality, shared with the disk's JPEG). An empty
    /// subject or message is sent with the default shown in the field
    /// (PromptMissingSubjMsg's choice, without the question).
    pub(crate) fn postcard_form(&mut self, ui: &mut egui::Ui, p: &Palette, s: &mut Settings, mail: &EmailView) {
        if !mail.available {
            ui.label(RichText::new(aurora_i18n::tr!("photo-email-unavailable")).color(p.muted));
        }
        let form = &mut self.postcard;
        let fields: [(&str, &mut String, &str, usize); 3] = [
            (
                aurora_i18n::tr!("photo-email-to"),
                &mut form.to,
                aurora_i18n::tr!("photo-email-to-hint"),
                usize::MAX,
            ),
            (
                aurora_i18n::tr!("photo-email-from"),
                &mut form.name,
                &mail.sender,
                POSTCARD_NAME_MAX,
            ),
            (
                aurora_i18n::tr!("photo-email-subject"),
                &mut form.subject,
                aurora_i18n::tr!("photo-email-default-subject"),
                POSTCARD_SUBJECT_MAX,
            ),
        ];
        for (label, text, hint, max) in fields {
            ui.label(RichText::new(label).color(p.ink));
            ui.add(
                egui::TextEdit::singleline(text)
                    .hint_text(hint)
                    .char_limit(max)
                    .desired_width(f32::INFINITY),
            );
        }
        ui.label(RichText::new(aurora_i18n::tr!("photo-email-message")).color(p.ink));
        let mut message = egui::TextEdit::multiline(&mut form.message)
            .hint_text(aurora_i18n::tr!("photo-email-default-message"))
            .char_limit(POSTCARD_MESSAGE_MAX)
            .desired_rows(3)
            .desired_width(f32::INFINITY)
            .show(ui);
        // spell checked, as panel_postcard_message.xml
        crate::ui::spelling::field(ui, p, &mut message, &mut form.message, false);
        row(ui, p, aurora_i18n::tr!("photo-jpeg-quality"), None, |ui| {
            ui.spacing_mut().slider_width = 130.0;
            ui.add(egui::Slider::new(&mut s.jpeg_quality, 1..=100));
        });
    }

    /// Firestorm's snapshot floater (floater_snapshot.xml, Preferences ›
    /// Aurora › Photo window › Classic window): a floating window, the
    /// controls in a column on the left (Refresh first, then the options and
    /// the destination), the preview on the right; the same controls.
    #[allow(clippy::too_many_arguments)]
    fn body_classic(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        s: &mut Settings,
        fov: &mut f32,
        default_fov: f32,
        view: &View,
        out: &mut Vec<Action>,
    ) {
        let before = s.clone();
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(WIDTH - 24.0);
                ui.spacing_mut().item_spacing.y = 6.0;
                // new_snapshot_btn
                if widgets::action_button(
                    ui,
                    p,
                    "refresh",
                    aurora_i18n::tr!("photo-refresh"),
                    ActionKind::Flat,
                    Vec2::new(ui.available_width(), 28.0),
                )
                .clicked()
                {
                    out.push(Action::Refresh);
                }
                egui::ScrollArea::vertical()
                    .id_salt("photo_settings_classic")
                    .max_height(CLASSIC_PREVIEW + 40.0)
                    .show(ui, |ui| self.settings(ui, p, s, fov, default_fov, view, out));
                self.actions(ui, p, view, out);
            });
            ui.add_space(8.0);
            ui.vertical(|ui| {
                ui.set_width(CLASSIC_PREVIEW);
                ui.spacing_mut().item_spacing.y = 6.0;
                if preview(ui, p, s, view) {
                    out.push(Action::Refresh);
                }
            });
        });
        changed(s, &before, out);
    }

    #[allow(clippy::too_many_arguments)]
    fn settings(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        s: &mut Settings,
        fov: &mut f32,
        default_fov: f32,
        view: &View,
        out: &mut Vec<Action>,
    ) {
        ui.set_width(WIDTH - 36.0);
        section(ui, p, "aspect-ratio-filled", aurora_i18n::tr!("photo-section-format"));
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
            for a in Aspect::ALL {
                let label = a.short().unwrap_or(aurora_i18n::tr!("photo-aspect-window"));
                if chip(ui, p, label, None, s.aspect == a, true).clicked() {
                    s.set_aspect(a, view.window);
                }
            }
        });

        section(ui, p, "dimensions", aurora_i18n::tr!("photo-section-resolution"));
        match (&view.email, &view.inventory, &view.profile) {
            (Some(mail), _, _) => long_sides(ui, p, &mut s.postcard_size, mail.size),
            (None, _, Some(prof)) => long_sides(ui, p, &mut s.profile_size, prof.size),
            (None, Some(inv), None) => texture_sizes(ui, p, s, inv),
            (None, None, None) => resolutions(ui, p, s, view),
        }
        // the photo quality lowered for the video memory (the switch is
        // further down, often out of sight): with how the photo is drawn
        if view.quality_reduced && s.quality {
            ui.add(egui::Label::new(RichText::new(aurora_i18n::tr!("photo-quality-reduced")).size(11.5).color(p.warn)).wrap());
        }

        section(ui, p, "send-filled", aurora_i18n::tr!("photo-section-destination"));
        let mut dest = match self.destination {
            Destination::Disk => 0,
            Destination::Inventory => 1,
            Destination::Profile => 2,
            Destination::Email => 3,
        };
        if widgets::segmented(
            ui,
            p,
            &mut dest,
            &[
                (aurora_i18n::tr!("photo-destination-disk"), true),
                (aurora_i18n::tr!("photo-destination-inventory"), true),
                (aurora_i18n::tr!("photo-destination-profile"), true),
                (aurora_i18n::tr!("photo-destination-email"), true),
            ],
        ) {
            self.destination = match dest {
                1 => Destination::Inventory,
                2 => Destination::Profile,
                3 => Destination::Email,
                _ => Destination::Disk,
            };
            out.push(Action::Changed);
        }
        match (&view.email, &view.inventory, &view.profile) {
            (Some(mail), _, _) => self.postcard_form(ui, p, s, mail),
            (None, _, Some(prof)) => {
                if !prof.connected {
                    ui.label(RichText::new(aurora_i18n::tr!("photo-profile-offline")).color(p.muted));
                } else if !prof.ready {
                    ui.label(RichText::new(aurora_i18n::tr!("photo-profile-not-ready")).color(p.muted));
                }
                ui.label(RichText::new(aurora_i18n::tr!("photo-profile-caption")).color(p.ink));
                let mut caption = egui::TextEdit::multiline(&mut self.caption)
                    .hint_text(aurora_i18n::tr!("photo-profile-caption-hint"))
                    .char_limit(crate::snapshot::profile::MAX_CAPTION)
                    .desired_rows(2)
                    .desired_width(f32::INFINITY)
                    .show(ui);
                // spell checked, as panel_snapshot_profile.xml
                crate::ui::spelling::field(ui, p, &mut caption, &mut self.caption, false);
                row(ui, p, aurora_i18n::tr!("photo-profile-location"), None, |ui| {
                    widgets::switch(ui, p, &mut s.profile_location);
                });
            }
            (None, Some(inv), None) => {
                let cost = match (inv.connected, inv.cost) {
                    (false, _) => aurora_i18n::tr!("photo-inventory-offline").to_string(),
                    (true, Some(cost)) => aurora_i18n::tr!("photo-upload-cost", cost = cost).to_string(),
                    (true, None) => aurora_i18n::tr!("photo-upload-cost-unknown").to_string(),
                };
                ui.label(RichText::new(cost).color(p.ink));
            }
            (None, None, None) => {
                row(ui, p, aurora_i18n::tr!("photo-file-format"), None, |ui| {
                    let mut i = FileFormat::ALL.iter().position(|f| *f == s.format).unwrap_or(0);
                    let options: Vec<(&str, bool)> = FileFormat::ALL.iter().map(|f| (f.label(), true)).collect();
                    if widgets::segmented(ui, p, &mut i, &options) {
                        s.format = FileFormat::ALL[i];
                    }
                });
                if s.format == FileFormat::Jpeg {
                    row(ui, p, aurora_i18n::tr!("photo-jpeg-quality"), None, |ui| {
                        ui.spacing_mut().slider_width = 130.0;
                        ui.add(egui::Slider::new(&mut s.jpeg_quality, 1..=100));
                    });
                }
            }
        }

        section(ui, p, "adjustments-horizontal-filled", aurora_i18n::tr!("photo-section-in-image"));
        let interface_possible = crate::snapshot::same_proportions(view.photo, view.window);
        row(
            ui,
            p,
            aurora_i18n::tr!("photo-interface"),
            (!interface_possible).then(|| aurora_i18n::tr!("photo-interface-hint")),
            |ui| {
                let mut on = s.interface;
                ui.add_enabled_ui(interface_possible || on, |ui| {
                    if widgets::switch(ui, p, &mut on).changed() {
                        s.interface = on;
                    }
                });
            },
        );
        row(ui, p, aurora_i18n::tr!("photo-huds"), None, |ui| {
            widgets::switch(ui, p, &mut s.huds);
        });
        row(
            ui,
            p,
            aurora_i18n::tr!("photo-quality"),
            Some(aurora_i18n::tr!("photo-quality-hint")),
            |ui| {
                widgets::switch(ui, p, &mut s.quality);
            },
        );
        row(
            ui,
            p,
            aurora_i18n::tr!("photo-freeze"),
            Some(aurora_i18n::tr!("photo-freeze-hint")),
            |ui| {
                let mut on = self.freeze;
                if widgets::switch(ui, p, &mut on).changed() {
                    self.freeze = on;
                    out.push(Action::Freeze(on));
                }
            },
        );
        row(
            ui,
            p,
            aurora_i18n::tr!("photo-freeze-world"),
            Some(aurora_i18n::tr!("photo-freeze-world-hint")),
            |ui| {
                widgets::switch(ui, p, &mut self.freeze_world);
            },
        );
        row(
            ui,
            p,
            aurora_i18n::tr!("photo-auto-refresh"),
            Some(aurora_i18n::tr!("photo-auto-refresh-hint")),
            |ui| {
                widgets::switch(ui, p, &mut s.auto_refresh);
            },
        );

        section(ui, p, "camera-filled", aurora_i18n::tr!("photo-section-camera"));
        if focal(ui, p, fov, default_fov) {
            out.push(Action::Saved);
        }
        row(ui, p, aurora_i18n::tr!("photo-free-camera"), None, |ui| {
            widgets::switch(ui, p, &mut self.free.on);
        });
        free_camera_hint(ui, p, view.back_key.as_ref());
        if widgets::action_button(
            ui,
            p,
            "video-filled",
            aurora_i18n::tr!("photo-free-camera-controls"),
            ActionKind::Flat,
            Vec2::new(0.0, 26.0),
        )
        .on_hover_text(aurora_i18n::tr!("photo-free-camera-controls-tip"))
        .clicked()
        {
            out.push(Action::Controls);
        }

        section(ui, p, "focus-centered", aurora_i18n::tr!("photo-section-dof"));
        dof_rows(ui, p, &mut s.dof);
        // (the photo's light: the viewfinder dock's sun button, `light_only`)

        let filter = ui.cursor();
        section(ui, p, "color-filter", aurora_i18n::tr!("photo-looks"));
        let opts = super::looks_editor::Options {
            list_height: 220.0,
            compare: false,
            luts: &self.lut_names,
        };
        self.looks.show(ui, p, s, opts, out);
        if std::mem::take(&mut self.looks.reveal) {
            ui.scroll_to_rect(filter.with_max_y(ui.cursor().top()), Some(egui::Align::Center));
        }

        ui.add_space(2.0);
        widgets::collapsing(ui, p, "photo_file", aurora_i18n::tr!("photo-section-file"), false, |ui| {
            row(ui, p, aurora_i18n::tr!("photo-base-name"), None, |ui| {
                ui.add(egui::TextEdit::singleline(&mut s.base_name).desired_width(150.0));
            });
            row(ui, p, aurora_i18n::tr!("photo-timestamp"), None, |ui| {
                widgets::switch(ui, p, &mut s.timestamp_names);
            });
            row(
                ui,
                p,
                aurora_i18n::tr!("photo-folder"),
                Some(&view.folder.display().to_string()),
                |ui| {
                    if widgets::flat_button(ui, p, aurora_i18n::tr!("photo-folder-change")).clicked() {
                        out.push(Action::ChooseFolder);
                    }
                },
            );
            row(ui, p, aurora_i18n::tr!("photo-remember-folder"), None, |ui| {
                widgets::switch(ui, p, &mut s.remember_folder);
            });
            row(ui, p, aurora_i18n::tr!("photo-sound"), None, |ui| {
                widgets::switch(ui, p, &mut s.sound);
            });
        });
    }

    /// The warning before "Photo quality" is first turned on: it raises
    /// every graphics setting, which a modest PC or graphics card may not
    /// survive (a frozen window or a crash). Some(true): go on.
    pub fn quality_warning(&self, ctx: &egui::Context, p: &Palette) -> Option<bool> {
        use widgets::{Modal, action_button};
        let size = Vec2::new(0.0, 26.0);
        let mut answer = None;
        let response = Modal::new(egui::Id::unique("photo_quality_warning")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.label(
                RichText::new(aurora_i18n::tr!("photo-quality-warning-title"))
                    .size(17.0)
                    .color(p.ink),
            );
            ui.add_space(8.0);
            ui.label(aurora_i18n::tr!("photo-quality-warning-text"));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let ok = aurora_i18n::tr!("photo-quality-warning-ok");
                if action_button(ui, p, "sparkles-filled", ok, ActionKind::Primary, size).clicked() {
                    answer = Some(true);
                }
                if action_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                    answer = Some(false);
                }
            });
        });
        if answer.is_none() && response.should_close() {
            answer = Some(false);
        }
        answer
    }

    /// The price to confirm before a paid upload (`confirm`), or the price
    /// the balance cannot pay (`short`: LLPanelSnapshotInventory::onSend
    /// and its ErrorPhotoCannotAfford).
    pub fn dialogs(&self, ctx: &egui::Context, p: &Palette, confirm: Option<i32>, short: Option<i32>) -> Vec<Action> {
        use widgets::{Modal, action_button};
        let mut out = Vec::new();
        let size = Vec2::new(0.0, 26.0);
        if let Some(cost) = confirm {
            let mut answer = None;
            let response = Modal::new(egui::Id::unique("photo_upload_confirm")).show(ctx, |ui| {
                ui.set_width(350.0);
                ui.label(RichText::new(aurora_i18n::tr!("photo-confirm-title")).size(17.0).color(p.ink));
                ui.add_space(8.0);
                ui.label(aurora_i18n::tr!("photo-confirm-text", cost = cost));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let ok = aurora_i18n::tr!("photo-confirm-ok", cost = cost);
                    if action_button(ui, p, "file-upload-filled", &ok, ActionKind::Primary, size).clicked() {
                        answer = Some(true);
                    }
                    if action_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                        answer = Some(false);
                    }
                });
            });
            // (above the reel's quick view, which raises its layers every frame)
            ctx.move_to_top(response.backdrop_response.layer_id);
            ctx.move_to_top(response.response.layer_id);
            if answer.is_none() && response.should_close() {
                answer = Some(false);
            }
            out.extend(answer.map(Action::Confirm));
        }
        if let Some(cost) = short {
            let mut close = false;
            let response = Modal::new(egui::Id::unique("photo_cannot_afford")).show(ctx, |ui| {
                ui.set_width(350.0);
                ui.label(aurora_i18n::tr!("photo-cannot-afford", cost = cost));
                ui.add_space(8.0);
                if action_button(ui, p, "check", aurora_i18n::tr!("common-ok"), ActionKind::Primary, size).clicked() {
                    close = true;
                }
            });
            ctx.move_to_top(response.backdrop_response.layer_id);
            ctx.move_to_top(response.response.layer_id);
            if close || response.should_close() {
                out.push(Action::CloseAfford);
            }
        }
        out
    }

    /// The full-frame view (instead of the interface); `photo` the photo's
    /// size. Escape or the exit button leaves it.
    #[allow(clippy::too_many_arguments)]
    pub fn full_frame(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        s: &mut Settings,
        fov: &mut f32,
        default_fov: f32,
        photo: (u32, u32),
        window: (u32, u32),
    ) -> Vec<Action> {
        let mut out = Vec::new();
        let screen = super::layout::world_rect(ctx);
        let include_interface = s.interface && crate::snapshot::same_proportions(photo, window) && !self.freeze;
        let frame = scene_frame(screen, photo, include_interface);
        let controls = super::layout::toolbar_rect(ctx);
        let painter = ctx
            .layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::unique("photo_full_frame")))
            .with_clip_rect(screen);
        let mask = p.navy.gamma_multiply(0.62);
        for r in outside(screen, frame) {
            painter.rect_filled(r, 0.0, mask);
        }
        painter.rect_stroke(frame, 0.0, Stroke::new(1.0, p.ink.gamma_multiply(0.7)), egui::StrokeKind::Inside);
        guide_lines(&painter, frame, s.guide, p.ink.gamma_multiply(0.55));

        let tag = |id: &str, pos: Pos2, align: egui::Align2, icon: &str, text: String| {
            widgets::viewport_area(ctx, egui::Id::unique(id))
                .order(egui::Order::Foreground)
                .pivot(align)
                .fixed_pos(pos)
                .interactable(false)
                .show(ctx, |ui| {
                    widgets::overlay(ui, p, egui::Margin::symmetric(8, 4), |ui| {
                        ui.horizontal(|ui| {
                            icon_image(ui, icon, 14.0, p.teal);
                            ui.label(RichText::new(text).size(12.0).color(p.ink));
                        });
                    });
                });
        };
        let label = s.aspect.short().unwrap_or(aurora_i18n::tr!("photo-aspect-window"));
        tag(
            "photo_full_size",
            controls.left_top() + Vec2::new(12.0, 12.0),
            egui::Align2::LEFT_TOP,
            "aspect-ratio-filled",
            format!("{label} · {} × {}", photo.0, photo.1),
        );
        tag(
            "photo_full_exit",
            controls.right_top() + Vec2::new(-12.0, 12.0),
            egui::Align2::RIGHT_TOP,
            "x",
            aurora_i18n::tr!("photo-exit-full-frame").to_string(),
        );

        let before = s.clone();
        widgets::viewport_area(ctx, egui::Id::unique("photo_full_strip"))
            .order(egui::Order::Foreground)
            .pivot(egui::Align2::CENTER_BOTTOM)
            .fixed_pos(Pos2::new(controls.center().x, controls.bottom() - 14.0))
            .constrain_to(controls)
            .show(ctx, |ui| {
                ui.set_max_width((controls.width() - 24.0).max(1.0));
                ui.set_clip_rect(ui.clip_rect().intersect(controls));
                widgets::overlay(ui, p, egui::Margin::symmetric(10, 8), |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        for a in [
                            Aspect::Window,
                            Aspect::R16x9,
                            Aspect::R3x2,
                            Aspect::R1x1,
                            Aspect::R4x5,
                            Aspect::R9x16,
                        ] {
                            let label = a.short().unwrap_or(aurora_i18n::tr!("photo-aspect-window"));
                            if chip(ui, p, label, None, s.aspect == a, true).clicked() {
                                s.aspect = a;
                            }
                        }
                        ui.add_space(8.0);
                        for g in Guide::ALL {
                            let (icon, tip) = guide_face(g);
                            if widgets::icon_toggle(ui, p, icon, tip, s.guide == g) {
                                s.guide = if s.guide == g { Guide::None } else { g };
                            }
                        }
                        ui.add_space(8.0);
                        if shutter(ui, p).clicked() {
                            out.push(Action::Take);
                        }
                        ui.add_space(8.0);
                        if focal(ui, p, fov, default_fov) {
                            out.push(Action::Saved);
                        }
                        ui.add_space(4.0);
                        strip_aperture(ui, p, &mut s.dof);
                        ui.add_space(4.0);
                        if widgets::icon_button(ui, p, "minimize", aurora_i18n::tr!("photo-exit-full-frame"), true) {
                            self.full_frame = false;
                        }
                    });
                });
            });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.full_frame = false;
        }
        if *s != before {
            out.push(if s.aspect != before.aspect || s.dof != before.dof {
                Action::Changed
            } else {
                Action::Saved
            });
        }
        out
    }
}

/// The disk's sizes: presets, the window's, or typed, with how the photo
/// is drawn.
fn resolutions(ui: &mut egui::Ui, p: &Palette, s: &mut Settings, view: &View) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
        for r in Resolution::ALL {
            let (label, sub) = match r {
                Resolution::Window => (aurora_i18n::tr!("photo-resolution-window"), None),
                Resolution::Custom => (aurora_i18n::tr!("photo-resolution-custom"), None),
                _ => (r.short().unwrap_or(""), r.long_side().map(|v| v.to_string())),
            };
            if chip(ui, p, label, sub.as_deref(), s.resolution == r, true).clicked() {
                if r == Resolution::Custom && s.resolution != Resolution::Custom {
                    // the typed size starts from the photo of the moment
                    let (w, h) = s.size(view.window);
                    s.custom = [w, h];
                }
                s.resolution = r;
            }
        }
    });
    ui.horizontal(|ui| {
        if s.resolution == Resolution::Custom {
            let (mut w, mut h) = (s.custom[0], s.custom[1]);
            let range = crate::snapshot::MIN_SIDE..=crate::snapshot::MAX_SIDE;
            let wc = ui.add(egui::DragValue::new(&mut w).range(range.clone()).speed(4.0)).changed();
            let lock = if s.keep_ratio { "lock-filled" } else { "lock-open" };
            if widgets::icon_toggle(ui, p, lock, aurora_i18n::tr!("photo-keep-ratio"), s.keep_ratio) {
                s.keep_ratio = !s.keep_ratio;
            }
            let hc = ui.add(egui::DragValue::new(&mut h).range(range).speed(4.0)).changed();
            if wc || hc {
                s.set_custom(wc.then_some(w), hc.then_some(h), view.window);
            }
        } else {
            let (w, h) = view.photo;
            ui.label(RichText::new(aurora_i18n::tr!("photo-size", width = w, height = h)).color(p.ink));
        }
        let passes = if view.tiles <= 1 {
            aurora_i18n::tr!("photo-one-pass").to_string()
        } else {
            aurora_i18n::tr!("photo-tiles", count = view.tiles).to_string()
        };
        ui.label(RichText::new(passes).size(11.5).color(p.muted));
    });
}

/// The window's depth of field (the mockup's card): on, aperture, focus by
/// click, pointer or a distance set by hand. It drives the view's effect
/// while the window is open; Preferences › Graphics keeps its own.
fn dof_rows(ui: &mut egui::Ui, p: &Palette, d: &mut crate::dof::PhotoDof) {
    use crate::dof::PhotoFocus;
    row(
        ui,
        p,
        aurora_i18n::tr!("photo-dof-enable"),
        Some(aurora_i18n::tr!("photo-dof-enable-hint")),
        |ui| {
            widgets::switch(ui, p, &mut d.enabled);
        },
    );
    ui.add_enabled_ui(d.enabled, |ui| {
        row(ui, p, aurora_i18n::tr!("photo-dof-aperture"), None, |ui| {
            // the slider, then its value, like the focal length's
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.spacing_mut().slider_width = (ui.available_width() - 72.0).max(60.0);
                ui.add(
                    egui::Slider::new(&mut d.f_number, 1.0..=22.0)
                        .logarithmic(true)
                        .prefix("f/")
                        .fixed_decimals(1)
                        .clamping(egui::SliderClamping::Edits)
                        .custom_parser(aurora_i18n::parse_number),
                );
            });
        });
        row(ui, p, aurora_i18n::tr!("photo-dof-focus"), None, |ui| {
            let modes = [PhotoFocus::Click, PhotoFocus::Pointer, PhotoFocus::Manual];
            let mut i = modes.iter().position(|m| *m == d.focus).unwrap_or(0);
            let options = [
                (aurora_i18n::tr!("photo-dof-focus-click"), true),
                (aurora_i18n::tr!("photo-dof-focus-pointer"), true),
                (aurora_i18n::tr!("photo-dof-focus-manual"), true),
            ];
            if widgets::segmented(ui, p, &mut i, &options) {
                d.focus = modes[i];
            }
        });
        if d.focus == PhotoFocus::Manual {
            row(ui, p, aurora_i18n::tr!("photo-dof-distance"), None, |ui| {
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.spacing_mut().slider_width = (ui.available_width() - 72.0).max(60.0);
                    ui.add(
                        egui::Slider::new(&mut d.distance, 0.3..=200.0)
                            .logarithmic(true)
                            .suffix(" m")
                            .fixed_decimals(1)
                            .clamping(egui::SliderClamping::Edits)
                            .custom_parser(aurora_i18n::parse_number),
                    );
                });
            });
        }
    });
}

/// The photo's quick light (Phototools' Quick Environments, the sky, water
/// and day cycle lists of its Env tab, Pause Clouds, Attached Lights and
/// Exposure, and the sun of "Personal lighting"), for the photo only: closing
/// the photo mode gives the user's own back, unless "Keep this
/// light".
fn light_rows(
    ui: &mut egui::Ui,
    p: &Palette,
    l: &mut crate::snapshot::light::PhotoLight,
    exposure: f32,
    hdr: bool,
    env: &EnvView,
    out: &mut Vec<Action>,
) {
    use crate::snapshot::light::EXPOSURE;
    ui.add_enabled_ui(!env.locked, |ui| env_rows(ui, p, l, env))
        .response
        .on_disabled_hover_text(aurora_i18n::tr!("rlva-blocked"));
    row(ui, p, aurora_i18n::tr!("photo-light-exposure"), None, |ui| {
        // (no exposure without HDR: LLPipeline::gammaCorrect)
        ui.add_enabled_ui(hdr, |ui| {
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                // (the value and the reset button after it)
                ui.spacing_mut().slider_width = (ui.available_width() - 130.0).max(40.0);
                let mut value = l.exposure.unwrap_or(exposure);
                // (shown, the sky's exposure must not turn into a fixed one)
                let slider = egui::Slider::new(&mut value, EXPOSURE)
                    .logarithmic(true)
                    .fixed_decimals(2)
                    .clamping(egui::SliderClamping::Edits)
                    .custom_parser(aurora_i18n::parse_number);
                if ui.add(slider).changed() {
                    l.exposure = Some(value);
                }
                if widgets::icon_button(
                    ui,
                    p,
                    "restore",
                    aurora_i18n::tr!("photo-light-exposure-reset"),
                    l.exposure.is_some(),
                ) {
                    l.exposure = None;
                }
            })
        })
        .response
        .on_disabled_hover_text(aurora_i18n::tr!("ui-options-needs-hdr-emissive"));
    });
    row(
        ui,
        p,
        aurora_i18n::tr!("photo-light-pause-clouds"),
        Some(aurora_i18n::tr!("photo-light-pause-clouds-hint")),
        |ui| {
            widgets::switch(ui, p, &mut l.clouds_paused);
        },
    );
    row(
        ui,
        p,
        aurora_i18n::tr!("photo-light-attached"),
        Some(aurora_i18n::tr!("photo-light-attached-hint")),
        |ui| {
            widgets::switch(ui, p, &mut l.attached_lights);
        },
    );
    ui.horizontal(|ui| {
        let keep = !l.env.is_empty() && !env.loading && !env.locked;
        if ui
            .add_enabled_ui(keep, |ui| widgets::flat_button(ui, p, aurora_i18n::tr!("photo-light-keep")))
            .inner
            .on_hover_text(aurora_i18n::tr!("photo-light-keep-tip"))
            .on_disabled_hover_text(aurora_i18n::tr!("photo-light-keep-tip"))
            .clicked()
        {
            out.push(Action::KeepLight);
        }
        if widgets::flat_button(ui, p, aurora_i18n::tr!("common-personal-lighting")).clicked() {
            out.push(Action::PersonalLighting);
        }
    });
}

/// Width of a list's label in the Light section.
const ENV_LABEL_W: f32 = 86.0;

/// The photo's time of day, sky, water, day cycle and sun over the
/// environment shown (`world::eep_photo`); each row's restore button gives
/// the user's own back.
fn env_rows(ui: &mut egui::Ui, p: &Palette, l: &mut crate::snapshot::light::PhotoLight, env: &EnvView) {
    use crate::world::env_select::SettingsKind;
    ui.label(RichText::new(aurora_i18n::tr!("photo-light-time")).color(p.ink));
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
        let times = [
            (None, aurora_i18n::tr!("photo-light-time-current")),
            (Some(1), aurora_i18n::tr!("ui-environment-sunrise")),
            (Some(2), aurora_i18n::tr!("common-midday")),
            (Some(3), aurora_i18n::tr!("ui-environment-lie-down")),
            (Some(4), aurora_i18n::tr!("common-midnight")),
        ];
        for (time, label) in times {
            if chip(ui, p, label, None, l.env.time == time, true).clicked() {
                l.env.set_time(time);
            }
        }
    });
    ui.add_space(2.0);
    let shown = l.env.shown(env.own);
    let lists = [
        (
            SettingsKind::Sky,
            aurora_i18n::tr!("photo-light-sky"),
            aurora_i18n::tr!("ui-environment-sky-settings"),
            shown.sky,
        ),
        (
            SettingsKind::Water,
            aurora_i18n::tr!("common-water"),
            aurora_i18n::tr!("ui-environment-water-settings"),
            shown.water,
        ),
        (
            SettingsKind::Day,
            aurora_i18n::tr!("photo-light-day"),
            aurora_i18n::tr!("ui-environment-day-cycle-settings"),
            shown.day,
        ),
    ];
    for (kind, label, tip, shown) in lists {
        ui.horizontal(|ui| {
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(ENV_LABEL_W, 20.0), Sense::hover());
            ui.painter().text(
                rect.left_center(),
                egui::Align2::LEFT_CENTER,
                label,
                egui::FontId::proportional(12.5),
                p.ink,
            );
            resp.on_hover_text(tip);
            // the two arrows and the restore button after the list
            let gap = ui.spacing().item_spacing.x;
            let width = (ui.available_width() - 3.0 * (20.0 + gap) - 12.0).max(60.0);
            if let Some(id) = super::environment::settings_list(ui, p, env.catalog, kind, shown, ("photo_env_list", kind as u8), width) {
                l.env.pick(kind, Some(id));
            }
            let own = l.env.of(kind).is_some() || (kind == SettingsKind::Sky && l.env.time.is_some());
            if widgets::icon_button(ui, p, "restore", aurora_i18n::tr!("photo-light-own"), own) {
                if kind == SettingsKind::Sky {
                    l.env.set_time(None);
                }
                l.env.pick(kind, None);
            }
        });
    }
    // the sun: LLFloaterEnvironmentAdjust's azimuth and elevation, over any
    // sky; each row's restore button gives back the sky's own value of its
    // slider and keeps the other's
    let own = env.sun;
    let (mut az, mut el) = l.env.sun.map_or(own, super::environment::azimuth_elevation);
    let (mut moved, mut restored) = (false, false);
    for (label, tip, value, range, own_value, wraps) in [
        (
            aurora_i18n::tr!("photo-light-sun-azimuth"),
            aurora_i18n::tr!("photo-light-sun-azimuth-reset"),
            &mut az,
            0.0..=359.99,
            own.0,
            true,
        ),
        (
            aurora_i18n::tr!("photo-light-sun-elevation"),
            aurora_i18n::tr!("photo-light-sun-elevation-reset"),
            &mut el,
            -90.0..=90.0,
            own.1,
            false,
        ),
    ] {
        row(ui, p, label, None, |ui| {
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.spacing_mut().slider_width = (ui.available_width() - 100.0).max(40.0);
                // (shown, the sky's sun must not turn into a fixed one)
                let slider = egui::Slider::new(value, range)
                    .fixed_decimals(0)
                    .suffix("°")
                    .clamping(egui::SliderClamping::Edits)
                    .custom_parser(aurora_i18n::parse_number);
                moved |= ui.add(slider).changed();
                let off = l.env.sun.is_some() && sun_differs(*value, own_value, wraps);
                if widgets::icon_button(ui, p, "restore", tip, off) {
                    *value = own_value;
                    restored = true;
                }
            });
        });
    }
    if restored {
        // both back: the sky's own sun again, moving with its day
        let fixed = sun_differs(az, own.0, true) || sun_differs(el, own.1, false);
        l.env.sun = fixed.then(|| super::environment::rotation_from(az, el));
    } else if moved {
        l.env.sun = Some(super::environment::rotation_from(az, el));
    }
}

/// A sun slider's value is off the sky's own (degrees; an azimuth wraps
/// at 360°), past what its whole-degree display shows.
fn sun_differs(value: f32, own: f32, wraps: bool) -> bool {
    let mut d = (value - own).abs();
    if wraps {
        d = d.min(360.0 - d);
    }
    d >= 0.5
}

/// The full-frame strip's depth of field: its switch and the aperture.
fn strip_aperture(ui: &mut egui::Ui, p: &Palette, d: &mut crate::dof::PhotoDof) {
    if widgets::icon_toggle(ui, p, "focus-centered", aurora_i18n::tr!("photo-section-dof"), d.enabled) {
        d.enabled = !d.enabled;
    }
    ui.add_enabled_ui(d.enabled, |ui| {
        ui.label(RichText::new(aurora_i18n::tr!("photo-dof-aperture")).size(12.0).color(p.muted));
        ui.add(
            egui::DragValue::new(&mut d.f_number)
                .range(1.0..=22.0)
                .speed(0.05)
                .prefix("f/")
                .fixed_decimals(1),
        );
    });
}

/// The profile feed's and the postcard's sizes (panel_snapshot_profile.xml's
/// and panel_snapshot_postcard.xml's combos): long sides, or the window.
fn long_sides(ui: &mut egui::Ui, p: &Palette, current: &mut ProfileSize, shown: (u32, u32)) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
        for size in ProfileSize::ALL {
            let label = match size.side() {
                Some(side) => side.to_string(),
                None => aurora_i18n::tr!("photo-resolution-window").to_string(),
            };
            if chip(ui, p, &label, None, *current == size, true).clicked() {
                *current = size;
            }
        }
    });
    let (w, h) = shown;
    ui.label(RichText::new(aurora_i18n::tr!("photo-size", width = w, height = h)).color(p.ink));
}

/// The inventory's sizes (panel_snapshot_inventory.xml's combo) and the
/// texture the photo becomes.
fn texture_sizes(ui: &mut egui::Ui, p: &Palette, s: &mut Settings, inv: &InventoryView) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
        for t in TextureSize::ALL {
            let label = match t.side() {
                Some(side) => side.to_string(),
                None => aurora_i18n::tr!("photo-resolution-window").to_string(),
            };
            if chip(ui, p, &label, None, s.inventory_size == t, true).clicked() {
                s.inventory_size = t;
            }
        }
    });
    let (w, h) = inv.texture;
    ui.label(RichText::new(aurora_i18n::tr!("photo-texture-size", width = w, height = h)).color(p.ink));
    if w != h {
        ui.label(RichText::new(aurora_i18n::tr!("photo-texture-hint")).size(11.5).color(p.muted));
    }
}

/// What a change of the settings asks: a new preview when the photo
/// changes, a save otherwise.
fn changed(s: &Settings, before: &Settings, out: &mut Vec<Action>) {
    if s != before {
        let photo_changed = s.aspect != before.aspect
            || s.resolution != before.resolution
            || s.custom != before.custom
            || s.inventory_size != before.inventory_size
            || s.profile_size != before.profile_size
            || s.dof != before.dof
            || s.interface != before.interface
            || s.huds != before.huds
            || s.looks != before.looks
            || s.look_settings != before.look_settings;
        out.push(if photo_changed { Action::Changed } else { Action::Saved });
    }
}

/// The photo held by "Freeze frame", over the whole world, in its
/// proportions.
pub fn frozen(ctx: &egui::Context, p: &Palette, texture: &egui::TextureHandle) {
    let screen = super::layout::world_rect(ctx);
    let size = texture.size_vec2();
    let rect = fit(screen, size.x / size.y.max(1.0));
    let painter = ctx
        .layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::unique("photo_frozen")))
        .with_clip_rect(screen);
    painter.rect_filled(screen, 0.0, p.navy);
    painter.image(
        texture.id(),
        rect,
        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
        widgets::PICTURE_TINT,
    );
}

/// A clean photo frames its output proportions within the live scene. A
/// photo with the interface keeps the entire scene inside its output canvas.
pub(crate) fn scene_frame(area: Rect, photo: (u32, u32), include_interface: bool) -> Rect {
    if include_interface {
        area
    } else {
        fit(area, photo.0 as f32 / photo.1.max(1) as f32)
    }
}

/// The largest rectangle of `ratio` (width / height) centred in `area`.
pub fn fit(area: Rect, ratio: f32) -> Rect {
    let ratio = crate::world_viewport::pixel_aspect(ratio);
    let size = if area.width() / area.height() > ratio {
        Vec2::new(area.height() * ratio, area.height())
    } else {
        Vec2::new(area.width(), area.width() / ratio)
    };
    Rect::from_center_size(area.center(), size)
}

/// The parts of `screen` outside `frame` (at most four bands).
pub fn outside(screen: Rect, frame: Rect) -> Vec<Rect> {
    let mut v = Vec::new();
    if frame.left() > screen.left() {
        v.push(Rect::from_min_max(screen.left_top(), Pos2::new(frame.left(), screen.bottom())));
    }
    if frame.right() < screen.right() {
        v.push(Rect::from_min_max(Pos2::new(frame.right(), screen.top()), screen.right_bottom()));
    }
    if frame.top() > screen.top() {
        v.push(Rect::from_min_max(
            Pos2::new(frame.left(), screen.top()),
            Pos2::new(frame.right(), frame.top()),
        ));
    }
    if frame.bottom() < screen.bottom() {
        v.push(Rect::from_min_max(
            Pos2::new(frame.left(), frame.bottom()),
            Pos2::new(frame.right(), screen.bottom()),
        ));
    }
    v
}

/// Icon and name of a guide.
fn guide_face(g: Guide) -> (&'static str, &'static str) {
    match g {
        Guide::Thirds => ("grid-3x3", aurora_i18n::tr!("photo-guide-thirds")),
        Guide::Golden => ("spiral", aurora_i18n::tr!("photo-guide-golden")),
        Guide::Diagonals => ("slash", aurora_i18n::tr!("photo-guide-diagonals")),
        Guide::Center => ("focus-centered", aurora_i18n::tr!("photo-guide-center")),
        Guide::None => ("x", aurora_i18n::tr!("photo-guide-off")),
    }
}

pub(crate) fn guide_lines(painter: &egui::Painter, frame: Rect, guide: Guide, color: Color32) {
    let at = |v: [f32; 2]| Pos2::new(frame.left() + v[0] * frame.width(), frame.top() + v[1] * frame.height());
    for [a, b] in guide.lines() {
        painter.line_segment([at(a), at(b)], Stroke::new(1.0, color));
    }
}

/// Preview with its guide, size badge and refresh button; true when the
/// refresh was clicked.
fn preview(ui: &mut egui::Ui, p: &Palette, s: &Settings, view: &View) -> bool {
    let width = ui.available_width();
    let ratio = view.photo.0 as f32 / view.photo.1.max(1) as f32;
    let height = (width / ratio).min(300.0);
    let (outer, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let rect = fit(outer, ratio);
    let radius = ui.visuals().window_corner_radius;
    ui.painter().rect_filled(outer, radius, p.field);
    match view.preview {
        Some(t) => {
            ui.painter().image(
                t.id(),
                rect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                widgets::PICTURE_TINT,
            );
        }
        None => {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                aurora_i18n::tr!("photo-preview-pending"),
                egui::FontId::proportional(12.0),
                p.muted,
            );
        }
    }
    guide_lines(ui.painter(), rect, s.guide, p.ink.gamma_multiply(0.55));
    // the size badge
    let badge = match (&view.email, &view.inventory, &view.profile) {
        (Some(mail), _, _) => format!("{} × {} · JPEG", mail.size.0, mail.size.1),
        (None, _, Some(prof)) => format!("{} × {} · PNG", prof.size.0, prof.size.1),
        (None, Some(inv), None) => aurora_i18n::tr!("photo-texture-size", width = inv.texture.0, height = inv.texture.1).to_string(),
        (None, None, None) => format!("{} × {} · {}", view.photo.0, view.photo.1, s.format.label()),
    };
    let galley = ui.painter().layout_no_wrap(badge, egui::FontId::proportional(11.0), p.ink);
    let badge_rect = Rect::from_min_size(
        rect.right_bottom() - galley.size() - Vec2::new(14.0, 10.0),
        galley.size() + Vec2::new(10.0, 4.0),
    );
    ui.painter().rect_filled(badge_rect, radius, p.navy.gamma_multiply(0.72));
    ui.painter().galley(badge_rect.min + Vec2::new(5.0, 2.0), galley, p.ink);
    // the refresh button
    let button = Rect::from_min_size(rect.left_bottom() + Vec2::new(6.0, -28.0), Vec2::splat(22.0));
    let resp = ui.interact(button, ui.scope_id().with("photo_refresh"), Sense::click());
    let fill = if resp.hovered() { p.raised } else { p.navy.gamma_multiply(0.72) };
    ui.painter().rect_filled(button, radius, fill);
    if let Some(t) = super::icons::global("refresh") {
        ui.painter().image(
            t.id(),
            Rect::from_center_size(button.center(), Vec2::splat(14.0)),
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            p.ink,
        );
    }
    resp.on_hover_text(aurora_i18n::tr!("photo-refresh")).clicked()
}

/// The latest photos and "Open the folder": the session's reel once it
/// has a photo (its stack, the older ones after it; a click opens the
/// quick view), the folder's latest before that and in the Legacy style.
fn gallery(ui: &mut egui::Ui, p: &Palette, view: &View, out: &mut Vec<Action>) {
    ui.add_space(2.0);
    if let Some(reel) = view.reel.filter(|r| !r.photos.is_empty()) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let mut asked = Vec::new();
            let stack = crate::ui::photo_reel::stack(ui, p, &reel, 34.0, reel.photos.len());
            if stack.clicked() {
                asked.push(crate::ui::photo_reel::ReelAction::Show(reel.photos[0].id));
            }
            // the room left of the folder's button
            let width = ui.available_width() - 30.0 - ui.spacing().item_spacing.x;
            crate::ui::photo_reel::strip(ui, p, &reel, 1, width, &mut asked);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::action_button(ui, p, "folder-open-filled", "", ActionKind::Flat, Vec2::new(30.0, 26.0))
                    .on_hover_text(aurora_i18n::tr!("photo-open-folder"))
                    .clicked()
                {
                    out.push(Action::OpenFolder);
                }
            });
            out.extend(asked.into_iter().map(Action::Reel));
        });
        return;
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let radius = ui.visuals().window_corner_radius;
        for (path, tex) in view.gallery {
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(54.0, 34.0), Sense::click());
            // the middle of the photo, filling the thumbnail
            crate::ui::photo_reel::picture(ui.painter(), p, rect, radius, tex.as_ref());
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if resp.on_hover_text(name).clicked() {
                out.push(Action::OpenPhoto(path.clone()));
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::action_button(
                ui,
                p,
                "folder-open-filled",
                aurora_i18n::tr!("photo-open-folder"),
                ActionKind::Flat,
                Vec2::new(0.0, 26.0),
            )
            .clicked()
            {
                out.push(Action::OpenFolder);
            }
        });
    });
}

/// The focal length slider and presets (mm, full frame) on the camera's
/// field of view; true when it changed.
fn focal(ui: &mut egui::Ui, p: &Palette, fov: &mut f32, default_fov: f32) -> bool {
    let mut changed = false;
    let mut mm = crate::snapshot::focal_of_fov(*fov).clamp(*FOCAL_RANGE.start(), *FOCAL_RANGE.end());
    ui.horizontal(|ui| {
        ui.label(RichText::new(aurora_i18n::tr!("photo-focal")).color(p.ink))
            .on_hover_text(aurora_i18n::tr!("photo-focal-hint"));
        ui.spacing_mut().slider_width = 110.0;
        if ui
            .add(
                // clamped and rounded only when the user moves it: egui's
                // default rounds the shown value back into `mm` (60° is
                // 20.8 mm, shown 21), which changed the view on opening
                egui::Slider::new(&mut mm, FOCAL_RANGE)
                    .logarithmic(true)
                    .suffix(" mm")
                    .fixed_decimals(0)
                    .clamping(egui::SliderClamping::Edits),
            )
            .changed()
        {
            *fov = crate::snapshot::fov_of_focal(mm);
            changed = true;
        }
        if widgets::icon_button(
            ui,
            p,
            "restore",
            aurora_i18n::tr!("photo-focal-reset"),
            (*fov - default_fov).abs() > 1e-3,
        ) {
            *fov = default_fov;
            changed = true;
        }
    });
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for preset in FOCAL_PRESETS {
            let on = (mm - preset).abs() < 0.5;
            if chip(ui, p, &format!("{preset:.0}"), None, on, true).clicked() {
                *fov = crate::snapshot::fov_of_focal(preset);
                changed = true;
            }
        }
    });
    changed
}

/// Under the free camera's switch: the avatar stays, and what brings the
/// camera back, its key drawn as a keycap in the sentence.
fn free_camera_hint(ui: &mut egui::Ui, p: &Palette, key: Option<&crate::keybinds::Binding>) {
    const MARK: &str = "\u{1}";
    let text = aurora_i18n::tr!("photo-free-camera-hint", key = MARK);
    let (before, after) = text.split_once(MARK).unwrap_or((text.as_str(), ""));
    let style = |t: &str| RichText::new(t).size(11.0).color(p.muted);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.label(style(before.trim_end()));
        if let Some(b) = key {
            let caps = widgets::keycap::caps(ui, b, p.panel);
            let (rect, _) = ui.allocate_exact_size(
                Vec2::new(widgets::keycap::row_width(&caps), widgets::keycap::HEIGHT),
                Sense::hover(),
            );
            widgets::keycap::paint_row(ui, p, &caps, egui::pos2(rect.left(), rect.center().y), p.panel);
        }
        ui.label(style(after.trim_start()));
    });
}

fn section(ui: &mut egui::Ui, p: &Palette, icon: &str, title: &str) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        icon_image(ui, icon, 14.0, p.violet_light);
        ui.label(RichText::new(title).size(12.5).strong().color(p.ink));
    });
}

fn icon_image(ui: &mut egui::Ui, icon: &str, side: f32, tint: Color32) {
    let (r, _) = ui.allocate_exact_size(Vec2::splat(side), Sense::hover());
    if let Some(t) = super::icons::global(icon) {
        ui.painter()
            .image(t.id(), r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), tint);
    }
}

/// A setting: its label (and a hint under it) on the left, its control on
/// the right.
fn row(ui: &mut egui::Ui, p: &Palette, label: &str, hint: Option<&str>, control: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        let width = ui.available_width();
        ui.vertical(|ui| {
            ui.set_width(width * 0.55);
            ui.label(RichText::new(label).color(p.ink));
            if let Some(h) = hint {
                ui.add(egui::Label::new(RichText::new(h).size(11.0).color(p.muted)).wrap());
            }
        });
        // (a long hint: the control stays next to its label)
        let align = if hint.is_some() { egui::Align::Min } else { egui::Align::Center };
        ui.with_layout(egui::Layout::right_to_left(align), control);
    });
}

/// A choice among several shown side by side: the accent when chosen.
pub(crate) fn chip(ui: &mut egui::Ui, p: &Palette, label: &str, sub: Option<&str>, on: bool, enabled: bool) -> egui::Response {
    let font = egui::FontId::proportional(12.0);
    let main = ui.painter().layout_no_wrap(label.to_owned(), font, p.ink);
    let sub = sub.map(|t| ui.painter().layout_no_wrap(t.to_owned(), egui::FontId::proportional(10.0), p.ink));
    let width = main.size().x + sub.as_ref().map_or(0.0, |g| g.size().x + 4.0) + 16.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 24.0), if enabled { Sense::click() } else { Sense::hover() });
    let fill = match (on, resp.hovered() && enabled) {
        (true, _) => p.violet,
        (false, true) => p.raised,
        (false, false) => p.bar,
    };
    ui.painter().rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, fill);
    let text = if on {
        p.on_accent
    } else if enabled {
        p.muted
    } else {
        p.muted_dim
    };
    let mut x = rect.left() + 8.0;
    let main_w = main.size().x;
    let at = crate::ui::widgets::row_text_origin(ui.painter(), &main, x, rect.center().y);
    ui.painter().galley(at, main, if on { text } else { p.ink });
    x += main_w + 4.0;
    if let Some(g) = sub {
        let at = crate::ui::widgets::row_text_origin(ui.painter(), &g, x, rect.center().y);
        ui.painter().galley(at, g, text.gamma_multiply(0.8));
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, enabled, on, label));
    resp
}

/// The round shutter button of the full-frame strip.
fn shutter(ui: &mut egui::Ui, p: &Palette) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(40.0), Sense::click());
    let c = rect.center();
    ui.painter().circle_filled(c, 21.0, p.violet.gamma_multiply(0.35));
    ui.painter()
        .circle_filled(c, 18.0, if resp.hovered() { p.violet_light } else { p.violet });
    if let Some(t) = super::icons::global("camera-filled") {
        ui.painter().image(
            t.id(),
            Rect::from_center_size(c, Vec2::splat(20.0)),
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            p.on_accent,
        );
    }
    resp.on_hover_text(aurora_i18n::tr!("photo-take"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum PhotoScene {
        Panel,
        FullFrame,
        Viewfinder,
        Frozen,
    }

    struct ViewportPhotoFixture {
        ctx: egui::Context,
        ui: SnapshotUi,
        settings: Settings,
        viewport: crate::world_viewport::ViewportSettings,
        frame: usize,
        open: bool,
        fov: f32,
        classic: bool,
        scene: PhotoScene,
        viewfinder: crate::ui::viewfinder::Viewfinder,
        frozen: egui::TextureHandle,
        native_scale: f32,
        native_size: (u32, u32),
        edge_subjects: bool,
    }

    impl ViewportPhotoFixture {
        fn new(classic: bool) -> Self {
            Self::new_at_scale(classic, 1.25, 1.2)
        }

        fn new_at_scale(classic: bool, native_scale: f32, zoom: f32) -> Self {
            let ctx = egui::Context::default();
            crate::theme::Theme::default().apply(&ctx, 1.0);
            // Queue only the requested zoom. Queuing 1.2 and then requesting
            // the still-current 1.0 leaves egui's pending 1.2 unchanged.
            ctx.set_zoom_factor(zoom);
            ctx.all_styles_mut(|style| style.animation_time = 0.0);
            let frozen = ctx.load_texture(
                "held photo fixture",
                egui::ColorImage::filled([4, 5], widgets::PICTURE_TINT),
                Default::default(),
            );
            Self {
                ctx,
                ui: SnapshotUi::default(),
                settings: Settings {
                    resolution: Resolution::Window,
                    aspect: Aspect::Window,
                    quality: false,
                    ..Default::default()
                },
                viewport: crate::world_viewport::ViewportSettings {
                    enabled: true,
                    restrict_ui: true,
                    left: 25.0,
                    right: 10.0,
                    top: 15.0,
                    bottom: 20.0,
                },
                frame: 0,
                open: true,
                fov: crate::camera::DEFAULT_FOV,
                classic,
                scene: PhotoScene::Panel,
                viewfinder: Default::default(),
                frozen,
                native_scale,
                native_size: (3200, 2000),
                edge_subjects: false,
            }
        }

        fn native(&self) -> Rect {
            Rect::from_min_size(
                egui::pos2(7.0, 11.0),
                Vec2::new(self.native_size.0 as f32, self.native_size.1 as f32) / (self.native_scale * self.ctx.zoom_factor()),
            )
        }

        fn bounds(&self) -> Rect {
            let configured = crate::world_viewport::ViewportSettings {
                restrict_ui: true,
                ..self.viewport.clone()
            };
            configured
                .toolbar_rect(self.native_size, self.native())
                .unwrap_or_else(|| self.native())
        }

        fn control_bounds(&self) -> Rect {
            self.viewport
                .toolbar_rect(self.native_size, self.native())
                .unwrap_or_else(|| self.native())
        }

        fn id(&self) -> &'static str {
            if self.classic { CLASSIC_WINDOW } else { WINDOW }
        }

        fn rect(&self) -> Rect {
            self.ctx
                .memory(|memory| memory.area_rect(egui::Id::unique(self.id())))
                .expect("actual snapshot window")
        }

        fn frames(&mut self, count: usize) {
            for _ in 0..count {
                self.show(Vec::new());
            }
        }

        fn show(&mut self, events: Vec<egui::Event>) -> Vec<egui::epaint::ClippedShape> {
            use super::super::layout;
            self.frame += 1;
            let native = self.native();
            let world = self.viewport.enabled.then(|| self.bounds());
            let catalog = crate::world::env_select::SettingsCatalog::default();
            let view = View {
                window: self.native_size,
                photo: self.settings.size(self.native_size),
                tiles: 1,
                preview: None,
                busy: None,
                message: None,
                last_saved: None,
                gallery: &[],
                reel: None,
                folder: PathBuf::new(),
                inventory: None,
                item_saved: false,
                profile: None,
                profile_posted: false,
                email: None,
                exposure: 0.0,
                hdr: true,
                env: EnvView {
                    catalog: &catalog,
                    own: crate::world::eep_env::LocalView::SHARED,
                    sun: (120.0, 20.0),
                    locked: false,
                    loading: false,
                },
                back_key: None,
                quality_reduced: false,
            };
            let p = crate::theme::Theme::default().palette();
            let isolation = crate::snapshot::isolate::Isolation::default();
            let thumbs = std::collections::HashMap::new();
            let full = std::collections::HashMap::new();
            let time = |_: i64| String::new();
            let reel = crate::ui::photo_reel::ReelView {
                photos: &[],
                thumbs: &thumbs,
                full: &full,
                can_send: &[],
                time: &time,
                arriving: false,
            };
            let mut frame = crate::ui::viewfinder::Frame {
                still: true,
                now: self.frame as f64 / 60.0,
                photo: view.photo,
                window: view.window,
                fps: 60.0,
                focal_mm: crate::snapshot::focal_of_fov(self.fov),
                ev: 0.0,
                base_exposure: 0.0,
                roll: 0.0,
                sharp: (1.0, 100.0),
                point: world.unwrap_or(native).center(),
                face: None,
                group: Vec::new(),
                points: Vec::new(),
                zone: None,
                pitch: 0.0,
                histogram: None,
                review: None,
                first_person: false,
                first_person_allowed: true,
                free_camera: false,
                keys: crate::ui::viewfinder::Keys {
                    menu: "Tab",
                    af_mode: "N",
                    first_person: Some("M"),
                },
                isolation: &isolation,
                isolate_rows: &[],
                isolate_hover: None,
                presets: &[],
                preset_object: None,
                posing: false,
                reel: &reel,
            };
            if self.edge_subjects {
                let world = world.unwrap_or(native);
                frame.point = world.right_top() + Vec2::new(-4.0, 3.0);
                frame.face = Some((
                    Rect::from_center_size(world.right_top() + Vec2::new(-30.0, 60.0), Vec2::new(120.0, 100.0)),
                    true,
                    "Edge face".to_owned(),
                ));
                frame.group.push((
                    Rect::from_center_size(world.left_bottom() + Vec2::new(10.0, -20.0), Vec2::new(100.0, 100.0)),
                    "Edge group".to_owned(),
                ));
                frame.points.push(world.right_top() + Vec2::new(-3.0, 4.0));
                frame.zone = Some((2.0, 8.0));
            }
            let mut raw = egui::RawInput {
                screen_rect: Some(native),
                time: Some(self.frame as f64 / 60.0),
                events,
                ..Default::default()
            };
            raw.viewports.entry(raw.viewport_id).or_default().native_pixels_per_point = Some(self.native_scale);
            let mut output = self.ctx.run_ui(raw, |ui| {
                layout::begin_canvas(ui.ctx(), false);
                layout::set_world_canvas(ui.ctx(), world);
                layout::set_toolbar_canvas(ui.ctx(), self.viewport.toolbar_rect(self.native_size, native));
                layout::set_work_area(ui.ctx(), native);
                layout::set_move_area(ui.ctx(), native);
                layout::set_window_edges(ui.ctx(), layout::Edges::OPEN);
                layout::frame(ui.ctx(), &p, false);
                // The UI only emits actions; no renderer, file service or
                // network consumer is installed in this offline fixture.
                match self.scene {
                    PhotoScene::Panel => {
                        self.ui.show(
                            ui.ctx(),
                            &p,
                            &mut self.open,
                            &mut self.settings,
                            &mut self.fov,
                            crate::camera::DEFAULT_FOV,
                            &view,
                            self.classic,
                        );
                    }
                    PhotoScene::FullFrame => {
                        self.ui.full_frame(
                            ui.ctx(),
                            &p,
                            &mut self.settings,
                            &mut self.fov,
                            crate::camera::DEFAULT_FOV,
                            view.photo,
                            view.window,
                        );
                    }
                    PhotoScene::Viewfinder => {
                        self.viewfinder.show(
                            ui.ctx(),
                            &p,
                            &mut self.settings,
                            &mut self.ui,
                            &mut self.fov,
                            crate::camera::DEFAULT_FOV,
                            &frame,
                            &view,
                        );
                    }
                    PhotoScene::Frozen => frozen(ui.ctx(), &p, &self.frozen),
                }
            });
            output.textures_delta.clear();
            output.shapes
        }

        fn click(&mut self, position: Pos2) {
            self.show(vec![
                egui::Event::ModifiersChanged(egui::Modifiers::NONE),
                egui::Event::PointerMoved(position),
            ]);
            for pressed in [true, false] {
                self.show(vec![egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                }]);
            }
        }
    }

    #[test]
    fn actual_viewfinder_current_native_pointer_blocks_scene_before_the_next_ui_frame() {
        use crate::snapshot::viewfinder::{Display, OnScreen};
        for (native_scale, zoom) in [(1.0, 1.0), (1.25, 1.2)] {
            for restricted in [false, true] {
                let mut f = ViewportPhotoFixture::new_at_scale(false, native_scale, zoom);
                f.viewport.left = 50.0;
                f.viewport.right = 0.0;
                f.viewport.top = 0.0;
                f.viewport.bottom = 0.0;
                f.viewport.restrict_ui = restricted;
                f.scene = PhotoScene::Viewfinder;
                f.settings.viewfinder.display = Display::Full;
                f.settings.viewfinder.controls = OnScreen::Always;
                f.viewfinder.hinted = true;
                f.fov = crate::snapshot::fov_of_focal(50.0);
                f.frames(5);
                for name in ["vf_top_left", "vf_ring"] {
                    let scene = f.bounds().center();
                    f.show(vec![egui::Event::PointerMoved(scene)]);
                    f.frames(2);
                    assert!(
                        !widgets::pointer_over_ui(&f.ctx, scene),
                        "ordinary scene input must remain available"
                    );
                    let rect = f
                        .ctx
                        .memory(|m| m.area_rect(egui::Id::unique(name)))
                        .expect("actual viewfinder control");
                    let position = if name == "vf_ring" {
                        Pos2::new(rect.right() - 16.0, rect.bottom() - 23.0)
                    } else {
                        rect.center()
                    };
                    assert_eq!(
                        f.ctx.layer_id_at(position),
                        Some(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique(name))),
                        "current native position must hit the actual interactive control"
                    );
                    assert!(!f.ctx.is_pointer_over_egui(), "egui still has the preceding scene position");
                    assert!(!f.ctx.egui_wants_pointer_input(), "no preceding UI press owns capture");
                    // A native move and press can both arrive before egui's
                    // next frame. Test the event position without advancing it.
                    assert!(widgets::pointer_over_ui(&f.ctx, position), "{name} leaks its press into the scene");
                    let guide = f.settings.guide;
                    let fov = f.fov;
                    f.show(vec![
                        egui::Event::PointerMoved(position),
                        egui::Event::PointerButton {
                            pos: position,
                            button: egui::PointerButton::Primary,
                            pressed: true,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ]);
                    f.show(vec![egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: egui::Modifiers::NONE,
                    }]);
                    if name == "vf_ring" {
                        assert!(f.fov < fov, "the real focal + button must still take the click");
                    } else {
                        assert_ne!(f.settings.guide, guide, "the real guide button must still take the click");
                    }
                }
            }
        }
    }

    #[test]
    fn actual_viewfinder_dock_leaves_scene_wheel_input_and_ring_buttons_available() {
        use crate::snapshot::viewfinder::{Display, OnScreen};
        for (native_scale, zoom) in [(1.0, 1.0), (1.25, 1.2), (2.0, 1.0)] {
            for restricted in [false, true] {
                let mut f = ViewportPhotoFixture::new_at_scale(false, native_scale, zoom);
                f.native_size = (3200, 2000);
                f.viewport.left = 5.0;
                f.viewport.right = 45.0;
                f.viewport.top = 0.0;
                f.viewport.bottom = 0.0;
                f.viewport.restrict_ui = restricted;
                f.scene = PhotoScene::Viewfinder;
                f.settings.viewfinder.display = Display::Full;
                f.settings.viewfinder.controls = OnScreen::Always;
                f.viewfinder.hinted = true;
                f.fov = crate::snapshot::fov_of_focal(50.0);
                f.frames(5);
                let area = |f: &ViewportPhotoFixture, name| {
                    f.ctx
                        .memory(|m| m.area_rect(egui::Id::unique(name)))
                        .expect("actual viewfinder area")
                };
                let bounds = f.control_bounds();
                assert!(area(&f, "vf_top_right").height() <= 40.0, "top controls must not cover the scene");
                let dock = area(&f, "vf_dock");
                assert!(dock.width() <= 90.0, "dock owns an oversized hit region: {dock:?}");
                assert!(
                    (dock.right() - (bounds.right() - 20.0)).abs() < 2.0,
                    "dock must follow the selected control scope's right edge"
                );
                let ring = area(&f, "vf_ring");
                assert!(
                    (ring.center().x - bounds.center().x).abs() < 2.0,
                    "ring must follow the selected control scope's centre"
                );
                assert!(bounds.contains_rect(ring));
                assert!(!dock.intersects(ring), "dock must not intercept the lens ring");
                for modifiers in [egui::Modifiers::NONE, egui::Modifiers::ALT] {
                    let point = bounds.center();
                    f.show(vec![egui::Event::ModifiersChanged(modifiers), egui::Event::PointerMoved(point)]);
                    assert!(
                        !widgets::pointer_over_ui(&f.ctx, point),
                        "scene wheel is blocked before the camera handler"
                    );
                }
                for cycle in 0..3 {
                    // Open and close the real settings drawer: this raises its
                    // foreground layer, the same state transition as the dock.
                    let dock = area(&f, "vf_dock");
                    let settings_button = Pos2::new(dock.center().x, dock.top() + 54.0 + 22.0);
                    f.click(settings_button);
                    assert!(f.viewfinder.drawer);
                    f.frames(2);
                    f.click(settings_button);
                    assert!(!f.viewfinder.drawer);
                    f.frames(2);
                    let ring = area(&f, "vf_ring");
                    let before = f.fov;
                    f.click(Pos2::new(ring.right() - 16.0, ring.bottom() - 23.0));
                    assert!(f.fov < before, "focal + stopped responding after drawer cycle {cycle}");
                    f.click(Pos2::new(ring.left() + 16.0, ring.bottom() - 23.0));
                    assert!((f.fov - before).abs() < 1e-5, "focal - must restore the preceding stop");
                    let point = Pos2::new(ring.center().x, ring.bottom() - 23.0);
                    f.show(vec![egui::Event::PointerMoved(point)]);
                    f.show(vec![egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Line,
                        phase: egui::TouchPhase::Move,
                        source: egui::MouseWheelSource::Unknown,
                        delta: Vec2::new(0.0, 1.0),
                        modifiers: egui::Modifiers::NONE,
                    }]);
                    assert!(f.fov < before, "ring wheel stopped responding after drawer cycle {cycle}");
                    f.click(Pos2::new(ring.left() + 16.0, ring.bottom() - 23.0));
                }
            }
        }
    }

    #[test]
    fn actual_viewfinder_controls_fit_narrow_scenes_and_keep_lens_input_after_other_dials() {
        use crate::snapshot::viewfinder::{Display, OnScreen};
        use crate::ui::viewfinder::Dial;
        for (size, native_scale, zoom, enabled) in [
            ((960, 1080), 1.0, 1.0, true),
            ((2048, 1080), 1.0, 1.0, true),
            ((2880, 2160), 2.0, 1.5, true),
            ((1280, 900), 1.0, 1.0, false),
        ] {
            let mut f = ViewportPhotoFixture::new_at_scale(false, native_scale, zoom);
            f.native_size = size;
            f.viewport.enabled = enabled;
            f.viewport.left = 50.0;
            f.viewport.right = 0.0;
            f.viewport.top = 0.0;
            f.viewport.bottom = 0.0;
            f.viewport.restrict_ui = true;
            f.scene = PhotoScene::Viewfinder;
            f.settings.viewfinder.display = Display::Full;
            f.settings.viewfinder.controls = OnScreen::Always;
            f.settings.dof.enabled = true;
            f.viewfinder.hinted = true;
            f.fov = crate::snapshot::fov_of_focal(50.0);
            f.frames(5);
            let area = |f: &ViewportPhotoFixture, name| {
                f.ctx
                    .memory(|m| m.area_rect(egui::Id::unique(name)))
                    .expect("actual viewfinder area")
            };
            let bounds = f.control_bounds();
            for name in ["vf_dock", "vf_ring", "vf_dials", "vf_af_choices"] {
                assert!(
                    bounds.expand(1.0).contains_rect(area(&f, name)),
                    "{size:?}/{native_scale}/{zoom}: {name} escapes {bounds:?}"
                );
            }
            let ring = area(&f, "vf_ring");
            assert!(!ring.intersects(area(&f, "vf_dock")));
            assert!(!area(&f, "vf_dials").intersects(area(&f, "vf_af_choices")));
            assert!((ring.center().x - bounds.center().x).abs() < 2.0);
            // Select the actual aperture value, adjust with the ring, then
            // select the actual focal value. The values share one ring but
            // must never lose its pointer ownership when the dial changes.
            let text_point = |f: &mut ViewportPhotoFixture, wanted: &str| {
                f.show(Vec::new())
                    .into_iter()
                    .find_map(|shape| match shape.shape {
                        egui::Shape::Text(text) if text.galley.text() == wanted => Some(text.pos + text.galley.size() * 0.5),
                        _ => None,
                    })
                    .expect("painted dial label")
            };
            let aperture = format!(
                "f/{}",
                aurora_i18n::number(
                    f64::from(f.settings.dof.f_number),
                    if f.settings.dof.f_number < 10.0 { 1 } else { 0 }
                )
            );
            let at = text_point(&mut f, &aperture);
            f.click(at);
            assert_eq!(f.viewfinder.dial, Dial::Aperture);
            f.frames(2);
            let ring = area(&f, "vf_ring");
            let before = f.settings.dof.f_number;
            f.click(Pos2::new(ring.right() - 16.0, ring.bottom() - 23.0));
            assert!(f.settings.dof.f_number > before);
            let at = text_point(&mut f, "50 mm");
            f.click(at);
            assert_eq!(f.viewfinder.dial, Dial::Focal);
            f.frames(2);
            let ring = area(&f, "vf_ring");
            let before = f.fov;
            f.click(Pos2::new(ring.right() - 16.0, ring.bottom() - 23.0));
            assert!(f.fov < before, "focal stopped after changing aperture at {size:?}");
            // Help must fit this same narrow scene and release its area when
            // closed, even after it has risen above the other foreground UI.
            f.viewfinder.help = true;
            f.frames(3);
            assert!(bounds.expand(1.0).contains_rect(area(&f, "vf_help")));
            f.viewfinder.help = false;
            f.frames(2);
            let ring = area(&f, "vf_ring");
            f.click(Pos2::new(ring.left() + 16.0, ring.bottom() - 23.0));
            assert!((f.fov - before).abs() < 1e-5);
        }
    }

    #[test]
    fn actual_viewfinder_short_canvas_stays_bounded_and_restores_lens_input() {
        use crate::snapshot::viewfinder::{Display, OnScreen};
        for (native_scale, zoom, left) in [(1.0, 1.0, 50.0), (1.25, 1.2, 0.0), (2.0, 1.5, 50.0)] {
            let mut f = ViewportPhotoFixture::new_at_scale(false, native_scale, zoom);
            f.native_size = (2048, 1080);
            f.viewport.left = left;
            f.viewport.right = 0.0;
            f.viewport.top = 50.0;
            f.viewport.bottom = 45.0;
            f.scene = PhotoScene::Viewfinder;
            f.settings.viewfinder.display = Display::Full;
            f.settings.viewfinder.controls = OnScreen::Always;
            f.viewfinder.hinted = true;
            f.fov = crate::snapshot::fov_of_focal(50.0);
            f.frames(5);
            let area = |f: &ViewportPhotoFixture, name| {
                f.ctx
                    .memory(|m| m.area_rect(egui::Id::unique(name)))
                    .expect("actual viewfinder area")
            };
            let bounds = f.control_bounds();
            assert!(bounds.height() <= 54.1, "exercise an allowed 5% high viewport");
            for name in ["vf_dock", "vf_ring", "vf_dials", "vf_af_choices"] {
                let rect = area(&f, name);
                assert!(rect.is_finite() && rect.is_positive(), "{name}: invalid short-canvas area {rect:?}");
                assert!(bounds.expand(1.0).contains_rect(rect), "{name}: {rect:?} escapes {bounds:?}");
            }
            // Area bounds alone miss the decorative bottom row: its fixed
            // baseline lies above this tiny canvas. Check the actual dot
            // and label painters, rather than only their computed positions.
            let mut dot = false;
            let mut label = false;
            for shape in f.show(Vec::new()) {
                let is_dot = matches!(&shape.shape, egui::Shape::Circle(circle) if circle.radius == 5.25);
                let is_label =
                    matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == aurora_i18n::tr!("photo-vf-sharp-from"));
                if is_dot || is_label {
                    assert!(
                        bounds.expand(1.0).contains_rect(shape.clip_rect),
                        "short-canvas decoration spills: {shape:?}"
                    );
                    dot |= is_dot;
                    label |= is_label;
                }
            }
            assert!(dot && label, "exercise the actual AF dot and sharp-range label");
            // Resizing out of the tiny canvas must restore the usable ring,
            // rather than retain a zero-sized scroll region or stuck input.
            f.viewport.top = 0.0;
            f.viewport.bottom = 0.0;
            f.frames(5);
            let ring = area(&f, "vf_ring");
            let before = f.fov;
            f.click(Pos2::new(ring.right() - 16.0, ring.bottom() - 23.0));
            assert!(f.fov < before, "lens input lost after restoring a short viewport");
        }
    }

    #[test]
    fn actual_viewfinder_header_leaves_the_guide_control_clickable() {
        for (width, height) in [(960, 1080), (2048, 1080), (1920, 1000)] {
            for restricted in [false, true] {
                let mut f = ViewportPhotoFixture::new_at_scale(false, 1.0, 1.0);
                f.native_size = (width, height);
                f.viewport.restrict_ui = restricted;
                f.viewport.left = 50.0;
                f.viewport.right = 0.0;
                f.viewport.top = 0.0;
                f.viewport.bottom = 0.0;
                f.scene = PhotoScene::Viewfinder;
                f.settings.viewfinder.display = crate::snapshot::viewfinder::Display::Full;
                f.viewfinder.hinted = true;
                f.frames(5);
                let shapes = f.show(Vec::new());
                let p = crate::theme::Theme::default().palette();
                let histogram = shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Rect(rect)
                            if rect.rect.size() == Vec2::new(150.0, 62.0) && rect.fill == p.navy.gamma_multiply(0.45) =>
                        {
                            Some(rect.rect)
                        }
                        _ => None,
                    })
                    .expect("painted viewfinder histogram");
                let area = |name| f.ctx.memory(|m| m.area_rect(egui::Id::unique(name))).expect("actual header area");
                let guide = area("vf_top_left");
                let header = area("vf_top_right");
                let dock = area("vf_dock");
                assert!(
                    histogram.right() + 12.0 <= dock.left(),
                    "dock covers histogram: {dock:?} over {histogram:?}"
                );
                assert!(f.control_bounds().contains_rect(histogram));
                let before = f.settings.guide;
                f.click(guide.center());
                assert_ne!(f.settings.guide, before, "header intercepts the actual guide button");
                assert!(
                    !guide.intersects(header),
                    "header's blank hit region covers the guide: {header:?} over {guide:?}"
                );
            }
        }
    }

    #[test]
    fn actual_viewfinder_overflow_ring_wheel_changes_focal_without_scrolling_its_container() {
        let mut f = ViewportPhotoFixture::new_at_scale(false, 1.0, 1.0);
        f.native_size = (2048, 1080);
        f.viewport.left = 50.0;
        f.viewport.right = 0.0;
        f.viewport.top = 50.0;
        f.viewport.bottom = 45.0;
        f.scene = PhotoScene::Viewfinder;
        f.settings.viewfinder.display = crate::snapshot::viewfinder::Display::Essential;
        f.settings.viewfinder.controls = crate::snapshot::viewfinder::OnScreen::Always;
        f.viewfinder.hinted = true;
        f.fov = crate::snapshot::fov_of_focal(50.0);
        f.frames(5);
        let ruler = |shapes: Vec<egui::epaint::ClippedShape>| {
            let p = crate::theme::Theme::default().palette();
            shapes
                .into_iter()
                .find_map(|shape| match shape.shape {
                    egui::Shape::Rect(rect) if rect.rect.height() == 46.0 && rect.fill == p.navy.gamma_multiply(0.35) => Some(rect.rect),
                    _ => None,
                })
                .expect("painted lens ruler")
        };
        let bounds = f.control_bounds();
        let point = Pos2::new(bounds.center().x, bounds.bottom() - 3.0);
        let before = ruler(f.show(vec![egui::Event::PointerMoved(point)]));
        assert!(
            before.contains(point),
            "pointer must be on the visible part of the overflowing ruler"
        );
        assert_eq!(
            f.ctx.layer_id_at(point),
            Some(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("vf_ring")))
        );
        let fov = f.fov;
        f.show(vec![egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            delta: Vec2::new(0.0, -1.0),
            modifiers: egui::Modifiers::NONE,
        }]);
        assert!(f.fov > fov, "the wheel must adjust the actual focal length");
        f.frames(8);
        let after = ruler(f.show(Vec::new()));
        assert!(
            (after.top() - before.top()).abs() < 0.5,
            "one wheel also scrolled the ring: {before:?} -> {after:?}"
        );
    }

    #[test]
    fn actual_snapshot_panel_and_classic_window_keep_full_frame_output_independent_of_tool_bounds() {
        for classic in [false, true] {
            for restricted in [false, true] {
                let mut f = ViewportPhotoFixture::new(classic);
                f.viewport.restrict_ui = restricted;
                f.ui.full_frame = true;
                let before = f.settings.clone();
                f.frames(12);
                if !restricted {
                    widgets::move_floater(&f.ctx, f.id(), f.native().min + Vec2::new(24.0, 80.0));
                    f.frames(6);
                }
                let rect = f.rect();
                assert!(f.native().expand(1.0).contains_rect(rect), "{classic}/{restricted}: {rect:?}");
                if restricted {
                    assert!(
                        f.bounds().expand(1.0).contains_rect(rect),
                        "{classic}: {rect:?} outside {:?}",
                        f.bounds()
                    );
                } else {
                    assert!(
                        rect.left() < f.bounds().left() - 5.0,
                        "disabled snapshot tools must use the full workspace"
                    );
                }
                assert!((f.ctx.content_rect().min - f.native().min).abs().max_elem() < 1.0);
                assert!((f.ctx.content_rect().max - f.native().max).abs().max_elem() < 1.0);
                assert_eq!(f.settings, before, "placement must not rewrite snapshot output settings");
                assert_eq!(f.settings.size((3200, 2000)), (3200, 2000));
                assert_eq!(f.fov, crate::camera::DEFAULT_FOV);
                assert!(f.ui.full_frame, "opening the tools must not change the requested full-frame mode");
            }
        }
    }

    #[test]
    fn actual_snapshot_tools_fit_a_small_viewport_and_keep_title_actions_and_close_reachable() {
        for classic in [false, true] {
            let mut f = ViewportPhotoFixture::new(classic);
            f.viewport.left = 50.0;
            f.viewport.right = 30.0;
            f.viewport.top = 30.0;
            f.viewport.bottom = 45.0;
            f.frames(12);
            assert!(f.bounds().width() < 640.0 && f.bounds().height() < 400.0);
            assert!(
                f.bounds().expand(1.0).contains_rect(f.rect()),
                "{classic}: {:?} outside {:?}",
                f.rect(),
                f.bounds()
            );
            let maximize = f
                .ctx
                .data(|data| data.get_temp::<Rect>(egui::Id::unique(("floater_action", f.id(), "maximize"))))
                .expect("actual title action");
            assert!(f.bounds().contains_rect(maximize));
            f.click(maximize.center());
            assert!(
                f.ui.full_frame,
                "the bounded title action must remain clickable above the scrolling body"
            );
            // The close button is two equal title-button cells to the right
            // of the measured maximize action (collapse is between them).
            let close = maximize.translate(Vec2::new(2.0 * (maximize.width() + 6.0), 0.0)).center();
            assert!(f.bounds().contains(close), "close must stay in the selected viewport");
            f.click(close);
            assert!(!f.open, "the close button must remain reachable on a short photo window");
        }
    }

    #[test]
    fn actual_snapshot_full_frame_strip_and_tags_stay_in_the_selected_viewport() {
        let mut f = ViewportPhotoFixture::new(false);
        f.scene = PhotoScene::FullFrame;
        f.ui.full_frame = true;
        let before = f.settings.clone();
        f.frames(12);
        for id in ["photo_full_strip", "photo_full_size", "photo_full_exit"] {
            let rect = f
                .ctx
                .memory(|memory| memory.area_rect(egui::Id::unique(id)))
                .expect("actual full-frame photo control");
            assert!(
                f.bounds().expand(1.0).contains_rect(rect),
                "{id}: {rect:?} outside {:?}",
                f.bounds()
            );
        }
        assert_eq!(f.settings, before);
        assert_eq!(f.settings.size((3200, 2000)), (3200, 2000));
        assert!((f.ctx.content_rect().size() - f.native().size()).abs().max_elem() < 1.0);
    }

    fn assert_rect_near(actual: Rect, expected: Rect) {
        assert!(
            (actual.min - expected.min).abs().max_elem() < 0.1 && (actual.max - expected.max).abs().max_elem() < 0.1,
            "{actual:?} differs from {expected:?}"
        );
    }

    #[test]
    fn actual_photo_guides_masks_and_comparison_follow_the_world_with_unrestricted_controls_and_dpi() {
        for scene in [PhotoScene::FullFrame, PhotoScene::Viewfinder] {
            for restricted in [false, true] {
                for (native_scale, zoom) in [(1.0, 1.0), (1.25, 1.2), (2.0, 1.5)] {
                    let mut f = ViewportPhotoFixture::new_at_scale(false, native_scale, zoom);
                    f.scene = scene;
                    f.viewport.restrict_ui = restricted;
                    f.settings.aspect = Aspect::R4x5;
                    f.settings.guide = Guide::Thirds;
                    f.settings.viewfinder.display = crate::snapshot::viewfinder::Display::Essential;
                    f.settings.looks.push(crate::snapshot::looks::Layer::new("Sepia"));
                    f.ui.looks.compare = Some(0.5);
                    f.viewfinder.hinted = true;
                    let before = f.settings.clone();
                    f.frames(4);
                    let shapes = f.show(Vec::new());
                    let p = crate::theme::Theme::default().palette();
                    let scale = native_scale * zoom;
                    let at = |x, y| f.native().min + Vec2::new(x, y) / scale;
                    // The physical scene is [800, 300, 2080, 1300]. A 4:5
                    // photo occupies its middle 1040 pixels, independently
                    // of the toolbar placement and native output size.
                    let expected = Rect::from_min_max(at(1320.0, 300.0), at(2360.0, 1600.0));
                    let guide_color = p.ink.gamma_multiply(if scene == PhotoScene::Viewfinder { 0.4 } else { 0.55 });
                    let guides: Vec<_> = shapes
                        .iter()
                        .filter_map(|shape| match &shape.shape {
                            egui::Shape::LineSegment { points, stroke } if stroke.color == guide_color && stroke.width == 1.0 => {
                                Some(*points)
                            }
                            _ => None,
                        })
                        .collect();
                    assert_eq!(guides.len(), 4, "{scene:?}/{restricted}/{scale}");
                    for points in [
                        [at(1320.0 + 1040.0 / 3.0, 300.0), at(1320.0 + 1040.0 / 3.0, 1600.0)],
                        [at(1320.0 + 2080.0 / 3.0, 300.0), at(1320.0 + 2080.0 / 3.0, 1600.0)],
                        [at(1320.0, 300.0 + 1300.0 / 3.0), at(2360.0, 300.0 + 1300.0 / 3.0)],
                        [at(1320.0, 300.0 + 2600.0 / 3.0), at(2360.0, 300.0 + 2600.0 / 3.0)],
                    ] {
                        assert!(
                            guides.iter().any(|actual| {
                                (actual[0] - points[0]).abs().max_elem() < 0.1 && (actual[1] - points[1]).abs().max_elem() < 0.1
                            }),
                            "missing guide for {scene:?}/{restricted}/{native_scale}/{zoom}: {points:?} in {guides:?}"
                        );
                    }
                    let mask = p.navy.gamma_multiply(if scene == PhotoScene::Viewfinder { 0.66 } else { 0.62 });
                    let masks: Vec<_> = shapes
                        .iter()
                        .filter_map(|shape| match &shape.shape {
                            egui::Shape::Rect(rect) if rect.fill == mask => Some(rect.rect),
                            _ => None,
                        })
                        .collect();
                    assert_eq!(masks.len(), 2, "{scene:?}/{restricted}/{scale}");
                    assert_rect_near(masks[0], Rect::from_min_max(at(800.0, 300.0), expected.left_bottom()));
                    assert_rect_near(masks[1], Rect::from_min_max(expected.right_top(), at(2880.0, 1600.0)));
                    for shape in &shapes {
                        let scene_shape = match &shape.shape {
                            egui::Shape::LineSegment { stroke, .. } => stroke.color == guide_color && stroke.width == 1.0,
                            egui::Shape::Rect(rect) => rect.fill == mask,
                            _ => false,
                        };
                        if scene_shape {
                            assert!(
                                f.bounds().expand(0.1).contains_rect(shape.clip_rect),
                                "scene painter spills into the workspace"
                            );
                        }
                    }
                    if scene == PhotoScene::Viewfinder {
                        let compare = f
                            .ctx
                            .memory(|memory| memory.area_rect(egui::Id::unique("vf_look_compare")))
                            .expect("actual photo comparison handle");
                        assert!((compare.center().x - expected.center().x).abs() < 1.0);
                        assert!((compare.top() - expected.top()).abs() < 1.0);
                        assert!((compare.bottom() - expected.bottom()).abs() < 1.0);
                    } else if !restricted {
                        let strip = f
                            .ctx
                            .memory(|memory| memory.area_rect(egui::Id::unique("photo_full_strip")))
                            .expect("actual unrestricted full-frame controls");
                        assert!(strip.center().y > f.bounds().bottom(), "controls retain the full workspace");
                    }
                    assert_rect_near(f.ctx.content_rect(), f.native());
                    assert_eq!(f.settings, before, "scene placement must preserve the photo settings");
                    assert_eq!(f.settings.size((3200, 2000)), (2560, 3200));
                }
            }
        }
    }

    #[test]
    fn actual_frozen_photo_uses_the_world_and_returns_to_the_native_canvas_when_cleared() {
        for restricted in [false, true] {
            let mut f = ViewportPhotoFixture::new(false);
            f.scene = PhotoScene::Frozen;
            f.viewport.restrict_ui = restricted;
            f.frames(2);
            for scoped in [true, false] {
                f.viewport.enabled = scoped;
                let shapes = f.show(Vec::new());
                let p = crate::theme::Theme::default().palette();
                let scale = f.native_scale * f.ctx.zoom_factor();
                let at = |x, y| f.native().min + Vec2::new(x, y) / scale;
                let expected = if scoped {
                    Rect::from_min_max(at(1320.0, 300.0), at(2360.0, 1600.0))
                } else {
                    Rect::from_min_max(at(800.0, 0.0), at(2400.0, 2000.0))
                };
                let picture = shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Mesh(mesh) if mesh.texture_id == f.frozen.id() => Some((mesh.calc_bounds(), shape.clip_rect)),
                        _ => None,
                    })
                    .expect("actual held photo image");
                let background = shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Rect(rect) if rect.fill == p.navy => Some((rect.rect, shape.clip_rect)),
                        _ => None,
                    })
                    .expect("actual held photo background");
                assert_rect_near(picture.0, expected);
                assert_rect_near(picture.1, f.bounds());
                assert_rect_near(background.0, f.bounds());
                assert_rect_near(background.1, f.bounds());
                assert_rect_near(f.ctx.content_rect(), f.native());
            }
        }
    }

    #[test]
    fn actual_viewfinder_edge_subjects_and_comparison_clip_to_the_world_with_unrestricted_controls() {
        for (native_scale, zoom) in [(1.25, 1.2), (2.0, 1.5)] {
            let mut f = ViewportPhotoFixture::new_at_scale(false, native_scale, zoom);
            f.scene = PhotoScene::Viewfinder;
            f.viewport.restrict_ui = false;
            f.viewport.left = 50.0;
            f.viewport.right = 5.0;
            f.viewport.top = 20.0;
            f.viewport.bottom = 10.0;
            f.settings.viewfinder.display = crate::snapshot::viewfinder::Display::Essential;
            f.viewfinder.hinted = true;
            f.viewfinder.af.track(5.0, 0.0);
            f.viewfinder.flash_at = Some(0.0);
            f.edge_subjects = true;
            f.frames(3);
            let shapes = f.show(Vec::new());
            let p = crate::theme::Theme::default().palette();
            let world = f.bounds();
            let zone = format!(
                "{}–{} m",
                crate::snapshot::viewfinder::distance_text(2.0),
                crate::snapshot::viewfinder::distance_text(8.0)
            );
            let mut bracket_marks = 0;
            let mut face_boxes = 0;
            let mut labels = std::collections::BTreeSet::new();
            let mut overflowing_marks = 0;
            let mut flash = false;
            for shape in &shapes {
                let mark = match &shape.shape {
                    // Each bracket contains both arms in one three-point
                    // path; it is not two independent LineSegment shapes.
                    egui::Shape::Path(path)
                        if path.points.len() == 3
                            && !path.closed
                            && path.stroke.width == 3.0
                            && matches!(&path.stroke.color, egui::epaint::ColorMode::Solid(color) if *color == p.success) =>
                    {
                        bracket_marks += 1;
                        true
                    }
                    egui::Shape::Rect(rect) if rect.stroke.color == p.success && rect.stroke.width == 2.5 => {
                        face_boxes += 1;
                        true
                    }
                    _ => false,
                };
                let label = matches!(&shape.shape, egui::Shape::Text(text) if ["Edge face", "Edge group", zone.as_str()].contains(&text.galley.text()));
                if mark || label {
                    assert_rect_near(shape.clip_rect, world);
                    if label && let egui::Shape::Text(text) = &shape.shape {
                        labels.insert(text.galley.text().to_owned());
                    }
                    overflowing_marks += usize::from(mark && !world.contains_rect(shape.shape.visual_bounding_rect()));
                }
                if let egui::Shape::Rect(rect) = &shape.shape
                    && rect.fill == p.navy
                    && rect.rect == world
                {
                    assert_rect_near(shape.clip_rect, world);
                    flash = true;
                }
            }
            assert!(
                bracket_marks == 4 && face_boxes == 2 && labels.len() == 3,
                "edge subjects must be painted: bracket paths={bracket_marks}, face boxes={face_boxes}, labels={labels:?}"
            );
            assert!(
                overflowing_marks > 0,
                "the fixture must exercise geometry crossing the world boundary"
            );
            assert!(flash, "the shutter blackout must be clipped to the world");
            f.settings.looks.push(crate::snapshot::looks::Layer::new("Sepia"));
            for compare in [0.0, 1.0] {
                f.ui.looks.compare = Some(compare);
                f.frames(3);
                let shapes = f.show(Vec::new());
                let mut comparison_shapes = 0;
                for shape in &shapes {
                    let comparison = match &shape.shape {
                        egui::Shape::Text(text) => {
                            [aurora_i18n::tr!("photo-look-before"), aurora_i18n::tr!("photo-look-after")].contains(&text.galley.text())
                        }
                        egui::Shape::Circle(circle) => circle.radius == 13.0,
                        _ => false,
                    };
                    if comparison {
                        assert!(
                            world.expand(0.1).contains_rect(shape.clip_rect),
                            "comparison overlay spills into the workspace"
                        );
                        comparison_shapes += 1;
                    }
                }
                assert!(comparison_shapes >= 4, "comparison grip and labels must be painted at either edge");
            }
            assert_rect_near(f.ctx.content_rect(), f.native());
        }
    }

    #[test]
    fn actual_photo_guides_keep_the_whole_scene_for_interface_capture_and_fit_a_held_photo() {
        for scene in [PhotoScene::FullFrame, PhotoScene::Viewfinder] {
            let mut f = ViewportPhotoFixture::new(false);
            f.scene = scene;
            f.viewport.restrict_ui = false;
            f.viewport.right = 20.0;
            f.settings.guide = Guide::Thirds;
            f.settings.viewfinder.display = crate::snapshot::viewfinder::Display::Essential;
            f.viewfinder.hinted = true;
            for (include_interface, frozen, aspect, physical_frame) in [
                (false, false, Aspect::Window, [800.0, 400.0, 2560.0, 1500.0]),
                (true, false, Aspect::Window, [800.0, 300.0, 2560.0, 1600.0]),
                (true, true, Aspect::Window, [800.0, 400.0, 2560.0, 1500.0]),
                (true, false, Aspect::R4x5, [1160.0, 300.0, 2200.0, 1600.0]),
            ] {
                f.settings.interface = include_interface;
                f.settings.aspect = aspect;
                f.ui.freeze = frozen;
                f.frames(3);
                let shapes = f.show(Vec::new());
                let p = crate::theme::Theme::default().palette();
                let guide_color = p.ink.gamma_multiply(if scene == PhotoScene::Viewfinder { 0.4 } else { 0.55 });
                let mut guide_bounds = Rect::NOTHING;
                let mut count = 0;
                for shape in shapes {
                    if let egui::Shape::LineSegment { points, stroke } = shape.shape
                        && stroke.color == guide_color
                        && stroke.width == 1.0
                    {
                        guide_bounds = guide_bounds.union(Rect::from_two_pos(points[0], points[1]));
                        count += 1;
                    }
                }
                assert_eq!(count, 4, "{scene:?}/{include_interface}/{frozen}/{aspect:?}");
                let scale = f.native_scale * f.ctx.zoom_factor();
                let at = |x, y| f.native().min + Vec2::new(x, y) / scale;
                let [left, top, right, bottom] = physical_frame;
                assert_rect_near(guide_bounds, Rect::from_min_max(at(left, top), at(right, bottom)));
            }
        }
    }

    #[test]
    fn a_sun_value_differs_past_its_shown_degree_and_azimuths_wrap() {
        assert!(!sun_differs(120.3, 120.0, true));
        assert!(sun_differs(121.0, 120.0, true));
        assert!(!sun_differs(359.8, 0.1, true), "across north");
        assert!(sun_differs(359.8, 0.1, false));
        assert!(sun_differs(-10.0, 10.0, false));
    }

    #[test]
    fn the_photo_fits_the_screen_and_the_rest_is_masked() {
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 900.0));
        let frame = fit(screen, 4.0 / 5.0);
        assert!((frame.height() - 900.0).abs() < 0.01);
        assert!((frame.width() - 720.0).abs() < 0.01);
        assert!((frame.center().x - 800.0).abs() < 0.01);
        let bands = outside(screen, frame);
        assert_eq!(bands.len(), 2);
        let masked: f32 = bands.iter().map(|r| r.area()).sum();
        assert!((masked + frame.area() - screen.area()).abs() < 1.0);
        // the screen's own proportions: nothing masked
        assert!(outside(screen, fit(screen, 16.0 / 9.0)).is_empty());
        // wider than the screen: bands above and below
        let wide = fit(screen, 3.0);
        assert_eq!(outside(screen, wide).len(), 2);
    }

    #[test]
    fn showing_the_focal_length_leaves_the_field_of_view_alone() {
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        let p = theme.palette();
        // 60° is 20.8 mm: the slider shows 21 mm, and must not write it back
        for start in [crate::camera::DEFAULT_FOV, 0.7, 1.31] {
            let mut fov = start;
            let mut changed = false;
            for _ in 0..2 {
                let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                    changed |= focal(ui, &p, &mut fov, crate::camera::DEFAULT_FOV)
                });
                output.textures_delta.clear();
            }
            assert!(!changed, "opening the window changed the focal length");
            assert_eq!(fov, start);
        }
        // the sky's exposure (1.234 shown 1.23) and its sun (shown in whole
        // degrees) are not taken over either
        let mut light = crate::snapshot::light::PhotoLight::default();
        let catalog = crate::world::env_select::SettingsCatalog::default();
        let env = EnvView {
            catalog: &catalog,
            own: crate::world::eep_env::LocalView::SHARED,
            sun: (123.456, 21.7),
            locked: false,
            loading: false,
        };
        for _ in 0..2 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                light_rows(ui, &p, &mut light, 1.234, true, &env, &mut Vec::new())
            });
            output.textures_delta.clear();
        }
        assert_eq!(light.exposure, None);
        assert!(light.env.is_empty(), "showing the Light section picked {:?}", light.env);
    }
}
