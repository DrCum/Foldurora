//! Immutable assistant upload preparation. Shared native codecs run once on a
//! worker; the later approved submission never reads a path or mutable Entry.
use super::*;
pub(crate) struct Encoded {
    pub kind: Kind,
    pub dimensions: (u32, u32),
    pub bytes: Arc<[u8]>,
}
pub(crate) fn encode(bytes: &[u8], extension: &str, options: import_texture::Options, sound_limit: u32) -> Result<Encoded, &'static str> {
    if bytes.is_empty()
        || bytes.len() > 64 * 1024 * 1024
        || !matches!(extension, "png" | "jpg" | "jpeg" | "tga" | "bmp" | "wav" | "anim" | "bvh")
    {
        return Err("upload_input_limit_or_type");
    }
    let source = files::decode_simple(bytes, extension).map_err(|_| "native_asset_decode_failed")?;
    if let Source::Sound(sound) = &source
        && !sound::Options::default().importable(sound, sound_limit)
    {
        return Err("native_sound_duration_limit");
    }
    let dimensions = source.dimensions(options);
    let kind = source.kind();
    let bytes = files::encode(&source, options, sound::Options::default()).map_err(|_| "native_asset_encode_failed")?;
    if bytes.is_empty() || bytes.len() > 64 * 1024 * 1024 {
        return Err("encoded_upload_limit");
    }
    Ok(Encoded {
        kind,
        dimensions,
        bytes: bytes.into(),
    })
}

pub(crate) fn mesh(
    path: &std::path::Path,
    bytes: Vec<u8>,
    options: import_texture::Options,
    prefs: &Preferences,
) -> Result<Vec<Entry>, &'static str> {
    files::load_data(path, Choice::Mesh, options, 30, prefs, false, bytes).map_err(|_| "native_mesh_review_decode_failed")
}

#[derive(Clone)]
pub(crate) struct Destination {
    folder: Uuid,
    settings: (bool, Uuid, Option<folders::Layout>, [Uuid; 5]),
}
impl Destination {
    pub(super) fn current(&self, prefs: &Preferences, inventory: &crate::ui::inventory::InventoryPreferences) -> bool {
        self.settings
            == (
                prefs.use_default_folders,
                prefs.batch_folder,
                prefs.folder_layout,
                inventory.upload_folders,
            )
    }
    pub(super) fn folder(&self) -> Uuid {
        self.folder
    }
}
impl Imports {
    pub(crate) fn assistant_mesh_idle(&self) -> bool {
        !self.any_window_open() && !self.busy() && !self.confirm && self.pending_files.is_none() && self.entries.is_empty()
    }
    pub(crate) fn assistant_mesh_begin(&mut self, id: Uuid, ctx: &egui::Context) -> Result<(), &'static str> {
        if !self.assistant_mesh_idle() {
            return Err("native_mesh_importer_occupied");
        }
        self.pending_files = Some(id);
        self.pending_file_choice = Some(Choice::Mesh);
        self.picker_files = false;
        self.open_type_window(ctx, Choice::Mesh);
        Ok(())
    }
    pub(crate) fn assistant_mesh_cancel(&mut self, id: Uuid) {
        if self.pending_files != Some(id) {
            return;
        }
        self.pending_files = None;
        self.pending_file_choice = None;
        if self.entries.is_empty()
            && !self.busy()
            && !self.confirm
            && let Some(index) = Choice::Mesh.window_index()
        {
            self.windows[index].open = false;
        }
    }
    pub(crate) fn assistant_mesh_finish(
        &mut self,
        id: Uuid,
        folder: Uuid,
        mut entries: Vec<Entry>,
        prefs: &Preferences,
        inventory: &crate::ui::inventory::InventoryPreferences,
    ) -> Result<(), &'static str> {
        if self.pending_files != Some(id) || !self.window_open(Choice::Mesh) || !self.entries.is_empty() || self.busy() || self.confirm {
            return Err("native_mesh_review_replaced");
        }
        if entries.is_empty() || entries.len() > 32 || entries.iter().any(|e| !matches!(e.source, Some(Source::Mesh(_)))) {
            return Err("native_mesh_review_decode_failed");
        }
        let fixed = Destination {
            folder,
            settings: (
                prefs.use_default_folders,
                prefs.batch_folder,
                prefs.folder_layout,
                inventory.upload_folders,
            ),
        };
        for entry in &mut entries {
            entry.folder = folder;
            entry.upload_path = None;
            entry.assistant_destination = Some(fixed.clone());
        }
        self.pending_files = None;
        self.pending_file_choice = None;
        self.entries.extend(entries);
        self.reconcile_window(Choice::Mesh);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mesh_review_preserves_requested_destination_until_resident_changes_native_preferences() {
        let ctx = egui::Context::default();
        let mut imports = Imports::default();
        let id = Uuid::new_v4();
        let requested = Uuid::from_u128(6601);
        let prefs = Preferences::default();
        let inventory = crate::ui::inventory::InventoryPreferences::default();
        imports.assistant_mesh_begin(id, &ctx).unwrap();
        let entry = Entry::new(
            "Synthetic mesh".into(),
            Ok(Source::Mesh(Box::new(super::super::tests::mesh_fixture()))),
            Default::default(),
        );
        imports
            .assistant_mesh_finish(id, requested, vec![entry], &prefs, &inventory)
            .unwrap();
        let fixed = imports.entries[0].assistant_destination.as_ref().unwrap();
        assert_eq!(fixed.folder(), requested);
        assert!(fixed.current(&prefs, &inventory));
        let mut changed = prefs.clone();
        changed.batch_folder = Uuid::new_v4();
        assert!(!fixed.current(&changed, &inventory));
        assert!(imports.pending_files.is_none());
        assert!(imports.current.is_none());
        assert!(!imports.confirm);
        assert!(imports.entries.iter().all(|e| e.folder == requested));
    }
    #[test]
    fn mesh_review_refuses_occupied_importer_and_only_cancels_its_own_empty_window() {
        let ctx = egui::Context::default();
        let mut imports = Imports::default();
        let id = Uuid::new_v4();
        imports.assistant_mesh_begin(id, &ctx).unwrap();
        assert!(imports.assistant_mesh_begin(Uuid::new_v4(), &ctx).is_err());
        imports.assistant_mesh_cancel(Uuid::new_v4());
        assert_eq!(imports.pending_files, Some(id));
        imports.assistant_mesh_cancel(id);
        assert!(imports.pending_files.is_none());
        assert!(imports.assistant_mesh_idle());
        imports.entries.push(Entry::new(
            "Resident draft".into(),
            Err("Synthetic error".into()),
            Default::default(),
        ));
        assert!(imports.assistant_mesh_begin(Uuid::new_v4(), &ctx).is_err());
        assert_eq!(imports.entries.len(), 1);
    }
}
