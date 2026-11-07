//! Five persistent profile slots. Labels are presentation, never account or
//! transport identity. Follows the stable Character1..5 profiles and separate
//! profile names in `tools/session-host/main.cpp` and `fssessionchatprefs.h`.
//!
//! The controller owns this registry exclusively. Before launching any worker
//! it publishes its fresh host identity. A slot worker takes its lifetime
//! profile lock and checks that stamp before any settings/cache writer starts.
//! Thus a delayed child of a dead controller either owns the profile already
//! (and blocks a new launch) or fails the new controller's stamp. No process is
//! adopted and no persisted slot identity grants session/chat authority.

use super::profile;
use aurora_session::{HostId, InstallationId, MAX_WORKERS};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

const VERSION: u16 = 1;
const MAX_BYTES: usize = 2048;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub(crate) struct SlotId(u8);

impl SlotId {
    pub fn all() -> impl Iterator<Item = Self> {
        (0..MAX_WORKERS as u8).map(Self)
    }

    pub fn number(self) -> u8 {
        self.0 + 1
    }

    fn index(self) -> usize {
        usize::from(self.0)
    }
}

impl TryFrom<u8> for SlotId {
    type Error = &'static str;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        (usize::from(value) < MAX_WORKERS)
            .then_some(Self(value))
            .ok_or("invalid character slot")
    }
}

impl From<SlotId> for u8 {
    fn from(value: SlotId) -> Self {
        value.0
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub(crate) struct SlotLabel(String);

impl SlotLabel {
    pub fn new(value: &str) -> Result<Self, &'static str> {
        if value.len() > 64 || value.chars().any(char::is_control) {
            return Err("profile names must fit in 64 UTF-8 bytes without control characters");
        }
        Ok(Self(value.trim().to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for SlotLabel {
    type Error = &'static str;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(&value)
    }
}

impl From<SlotLabel> for String {
    fn from(value: SlotLabel) -> Self {
        value.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SlotSummary {
    pub id: SlotId,
    pub label: SlotLabel,
    /// False includes detached viewers, pending launches and failed private
    /// path validation. The parent still rechecks the lock before each spawn.
    pub available: bool,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    version: u16,
    controller: HostId,
    labels: [SlotLabel; MAX_WORKERS],
}

pub(super) struct Registry {
    installation: InstallationId,
    path: PathBuf,
    stored: Stored,
    attention_preferences: super::attention_prefs::Store,
    _lock: File,
}

impl Registry {
    pub fn open(installation: InstallationId, controller: HostId) -> anyhow::Result<Self> {
        let directory = directory(installation)?;
        profile::private_dir(&directory)?;
        let lock = profile::try_private_lock(&directory.join("controller.lock"))?
            .ok_or_else(|| anyhow::anyhow!(aurora_i18n::tr!("host-profile-controller-busy")))?;
        let path = directory.join("slots.json");
        let mut stored = match std::fs::symlink_metadata(&path) {
            Ok(_) => read(&path)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                // A missing registry is not authority to adopt directories
                // left by an earlier registry or created through another path.
                match std::fs::symlink_metadata(profile::slots_root(&installation.to_string())?) {
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                    Ok(_) => anyhow::bail!(aurora_i18n::tr!("host-profile-registry-missing")),
                }
                Stored {
                    version: VERSION,
                    controller,
                    labels: std::array::from_fn(|_| SlotLabel::default()),
                }
            }
            Err(error) => return Err(error.into()),
        };
        stored.controller = controller;
        save(&path, &stored)?;
        // A corrupt optional preference file must not disable profile launches
        // or alter the rollback-compatible character registry schema.
        let attention_preferences = super::attention_prefs::Store::open(&directory);
        Ok(Self {
            installation,
            path,
            stored,
            attention_preferences,
            _lock: lock,
        })
    }

    pub fn label(&self, slot: SlotId) -> &SlotLabel {
        &self.stored.labels[slot.index()]
    }

    pub fn attention_preferences(&self) -> &super::attention_prefs::PreferenceState {
        self.attention_preferences.state()
    }

    pub fn save_attention_preferences(&mut self, ticket: u64, preferences: super::attention_prefs::Preferences) {
        self.attention_preferences.save(ticket, preferences);
    }

    pub fn rename(&mut self, slot: SlotId, label: SlotLabel) -> anyhow::Result<()> {
        // Persist before publishing UI success; an IO error leaves both the
        // durable registry and visible label at their previous value.
        let mut next = self.stored.clone();
        next.labels[slot.index()] = label;
        save(&self.path, &next)?;
        self.stored = next;
        Ok(())
    }

    pub fn available(&self, slot: SlotId) -> io::Result<bool> {
        profile::slot_available(&self.installation.to_string(), slot)
    }
}

fn directory(installation: InstallationId) -> io::Result<PathBuf> {
    Ok(profile::host_root()?.join("registries").join(installation.to_string()))
}

fn read(path: &Path) -> io::Result<Stored> {
    let stored: Stored = super::transport::read_json(path, MAX_BYTES)?;
    if stored.version != VERSION {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "unsupported character registry version"));
    }
    Ok(stored)
}

pub(super) fn validate_controller(installation: &str, controller: HostId) -> anyhow::Result<()> {
    let installation = InstallationId::parse_hex(installation)?;
    let directory = directory(installation)?;
    anyhow::ensure!(
        read(&directory.join("slots.json"))?.controller == controller
            && profile::try_private_lock(&directory.join("controller.lock"))?.is_none(),
        "character profile belongs to a previous controller launch"
    );
    Ok(())
}

fn save(path: &Path, stored: &Stored) -> io::Result<()> {
    write_json(path, stored, MAX_BYTES)
}

/// Bounded private metadata publication shared with the independent Attention
/// preference file. The caller owns the controller registry lock throughout.
pub(super) fn write_json(path: &Path, stored: &impl Serialize, limit: usize) -> io::Result<()> {
    write_json_with_rename(path, stored, limit, |from, to| std::fs::rename(from, to))
}

fn write_json_with_rename(
    path: &Path,
    stored: &impl Serialize,
    limit: usize,
    mut rename: impl FnMut(&Path, &Path) -> io::Result<()>,
) -> io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => profile::validate_private_file(path)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let bytes = serde_json::to_vec(stored).map_err(io::Error::other)?;
    if bytes.len() > limit {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "host metadata exceeds its bound"));
    }
    let parent = path.parent().ok_or_else(|| io::Error::other("character registry has no parent"))?;
    profile::private_dir(parent)?;
    let temporary = parent.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = profile::create_private_file(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        profile::validate_private_file(&temporary)?;
        for attempt in 0..=5 {
            match rename(&temporary, path) {
                Err(error) if error.kind() == io::ErrorKind::PermissionDenied && attempt < 5 => {
                    std::thread::sleep(std::time::Duration::from_millis(1 << attempt));
                }
                result => return result,
            }
        }
        unreachable!("bounded registry publication always returns")
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> (InstallationId, HostId) {
        let mut bytes = [0; 32];
        bytes[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
        bytes[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
        (
            InstallationId::from_bytes(bytes).unwrap(),
            HostId::from_bytes(*uuid::Uuid::new_v4().as_bytes()).unwrap(),
        )
    }

    #[test]
    fn slot_names_are_bounded_utf8_labels_and_never_profile_paths() {
        assert_eq!(SlotId::all().map(SlotId::number).collect::<Vec<_>>(), vec![1, 2, 3, 4, 5]);
        assert!(serde_json::from_str::<SlotId>("5").is_err());
        assert!(serde_json::from_str::<SlotId>("\"../other\"").is_err());
        assert!(SlotLabel::new(&"猫".repeat(22)).is_err());
        assert!(SlotLabel::new("line\nbreak").is_err());
        assert_eq!(SlotLabel::new("  ../A character  ").unwrap().as_str(), "../A character");
        assert_eq!(SlotLabel::new(&"é".repeat(32)).unwrap().as_str().len(), 64);
    }

    #[test]
    fn failed_attention_load_keeps_controller_registry_usable_without_rewriting_preferences() {
        let (install, host) = identity();
        let registry = Registry::open(install, host).unwrap();
        let directory = registry.path.parent().unwrap().to_path_buf();
        let path = directory.join("attention-preferences.json");
        assert!(!path.exists(), "opening slots does not eagerly save Attention defaults");
        drop(registry);
        let mut file = profile::create_private_file(&path).unwrap();
        file.write_all(b"malformed synthetic preferences").unwrap();
        drop(file);
        let mut registry = Registry::open(install, host).unwrap();
        assert!(!registry.attention_preferences().writable);
        assert!(registry.attention_preferences().error.is_some());
        assert!(SlotId::all().all(|slot| !registry.attention_preferences().preferences.includes(slot)));
        let slot = SlotId::try_from(2).unwrap();
        registry.rename(slot, SlotLabel::new("Synthetic renamed slot").unwrap()).unwrap();
        assert_eq!(registry.label(slot).as_str(), "Synthetic renamed slot");
        validate_controller(&install.to_string(), host).unwrap();
        registry.save_attention_preferences(81, super::super::attention_prefs::Preferences::default());
        assert!(registry.attention_preferences().operation.as_ref().unwrap().error.is_some());
        assert_eq!(std::fs::read(&path).unwrap(), b"malformed synthetic preferences");
        let stored: serde_json::Value = serde_json::from_slice(&std::fs::read(&registry.path).unwrap()).unwrap();
        assert_eq!(stored["version"], VERSION);
        assert_eq!(stored.as_object().unwrap().len(), 3, "slots schema remains rollback-compatible");
        drop(registry);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn atomic_metadata_publication_failure_preserves_previous_file_and_removes_temporary() {
        let (install, host) = identity();
        let registry = Registry::open(install, host).unwrap();
        let directory = registry.path.parent().unwrap().to_path_buf();
        let path = directory.join("attention-preferences.json");
        let before = serde_json::json!({"synthetic": "retained"});
        write_json(&path, &before, 512).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let next = serde_json::json!({"synthetic": "changed"});
        let mut temporary = None;
        let result = write_json_with_rename(&path, &next, 512, |from, to| {
            assert_eq!(to, path.as_path());
            assert_eq!(std::fs::read(from).unwrap(), serde_json::to_vec(&next).unwrap());
            assert_eq!(std::fs::read(to).unwrap(), bytes, "old file survives until atomic publication");
            temporary = Some(from.to_path_buf());
            Err(io::Error::other("synthetic atomic publication failure"))
        });
        assert!(result.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert!(!temporary.unwrap().exists());
        drop(registry);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn controller_lock_labels_and_stable_profile_data_survive_controller_restart() {
        let (install, host) = identity();
        let slot = SlotId::try_from(0).unwrap();
        let mut registry = Registry::open(install, host).unwrap();
        assert!(Registry::open(install, host).is_err(), "only one controller owns the registry");
        registry.rename(slot, SlotLabel::new("Shopping 猫").unwrap()).unwrap();
        let profile = profile::slot_root(&install.to_string(), slot).unwrap();
        profile::private_dir(&profile.join("config")).unwrap();
        std::fs::write(profile.join("config/preserved-settings"), b"synthetic settings").unwrap();
        let worker_lock = profile::profile_lock(&profile).unwrap();
        assert!(!registry.available(slot).unwrap());
        validate_controller(&install.to_string(), host).unwrap();
        drop(registry);
        assert!(
            validate_controller(&install.to_string(), host).is_err(),
            "a matching old stamp without a live controller lock cannot initialize a delayed worker"
        );
        let next_host = HostId::from_bytes(*uuid::Uuid::new_v4().as_bytes()).unwrap();
        let registry = Registry::open(install, next_host).unwrap();
        assert_eq!(registry.label(slot).as_str(), "Shopping 猫");
        assert!(!registry.available(slot).unwrap(), "a detached worker keeps its profile lease");
        assert!(
            validate_controller(&install.to_string(), host).is_err(),
            "late old bootstraps fail before writers"
        );
        validate_controller(&install.to_string(), next_host).unwrap();
        drop(worker_lock);
        assert!(
            registry.available(slot).unwrap(),
            "actual process/lock release permits manual reopening"
        );
        assert_eq!(
            std::fs::read(profile.join("config/preserved-settings")).unwrap(),
            b"synthetic settings"
        );
        // Failed process creation must not consume a slot or replace its
        // retained settings. The production broker similarly records a Child
        // only after spawn succeeds and removes failed coordinator entries.
        assert!(std::process::Command::new(profile.join("nonexistent-test-viewer")).spawn().is_err());
        assert!(registry.available(slot).unwrap());
    }

    #[test]
    fn malformed_newer_or_nonprivate_registries_are_not_rewritten() {
        let (install, host) = identity();
        let registry = Registry::open(install, host).unwrap();
        let path = registry.path.clone();
        drop(registry);
        let valid = std::fs::read(&path).unwrap();
        let mut newer: serde_json::Value = serde_json::from_slice(&valid).unwrap();
        newer["version"] = serde_json::json!(2);
        for bytes in [
            serde_json::to_vec(&newer).unwrap(),
            b"malformed".to_vec(),
            vec![b' '; MAX_BYTES + 1],
        ] {
            let mut file = profile::open_existing_private(&path).unwrap();
            file.set_len(0).unwrap();
            file.write_all(&bytes).unwrap();
            drop(file);
            assert!(Registry::open(install, host).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
            assert!(Registry::open(install, host).is_err());
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o644);
        }
    }

    #[test]
    fn missing_registry_never_adopts_unknown_existing_slot_profiles() {
        let (install, host) = identity();
        let slot = SlotId::try_from(3).unwrap();
        let registry = Registry::open(install, host).unwrap();
        assert!(registry.available(slot).unwrap());
        let path = registry.path.clone();
        drop(registry);
        let profile = profile::slot_root(&install.to_string(), slot).unwrap();
        let retained = profile.join("synthetic-retained-data");
        std::fs::write(&retained, b"never adopt or delete").unwrap();
        std::fs::remove_file(&path).unwrap();
        assert!(Registry::open(install, host).is_err());
        assert!(!path.exists(), "missing registry is not silently recreated");
        assert_eq!(std::fs::read(retained).unwrap(), b"never adopt or delete");
    }

    #[cfg(unix)]
    #[test]
    fn linked_registry_or_profile_paths_never_read_or_write_the_target() {
        use std::os::unix::fs::symlink;
        let (install, host) = identity();
        let registry = Registry::open(install, host).unwrap();
        let path = registry.path.clone();
        drop(registry);
        let outside = path.parent().unwrap().join("synthetic-outside");
        std::fs::write(&outside, b"outside data").unwrap();
        std::fs::remove_file(&path).unwrap();
        symlink(&outside, &path).unwrap();
        assert!(Registry::open(install, host).is_err());
        assert_eq!(std::fs::read(&outside).unwrap(), b"outside data");
        let (other_install, other_host) = identity();
        let registry = Registry::open(other_install, other_host).unwrap();
        let root = profile::slots_root(&other_install.to_string()).unwrap();
        profile::private_dir(root.parent().unwrap()).unwrap();
        symlink(outside.parent().unwrap(), &root).unwrap();
        assert!(registry.available(SlotId::try_from(0).unwrap()).is_err());
        assert!(!outside.parent().unwrap().join("1").exists());
    }
}
