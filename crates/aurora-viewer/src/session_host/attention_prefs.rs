//! Installation-local Attention filters, never live worker/chat authority.
//! Defaults follow HostExperience and ChatPreferences in the pinned Legacy
//! `fssessionexperience.h` / `fssessionchatprefs.h`: all five profiles, Direct
//! and initialized Groups on, Nearby off (`fssessioncategories.h`, legacy mask
//! 29). `oldest_first` removes offer-class priority; both orders
//! sort increasing event time (`fssessionattentionnav.h`).
//!
//! Only the host IO owner's exclusively locked slot registry opens this store.
//! It is a separate file so old binaries retain their character registry.

use super::slots::SlotId;
use aurora_session::MAX_WORKERS;
use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};

const VERSION: u16 = 3;
const MAX_BYTES: usize = 512;
const FILE_NAME: &str = "attention-preferences.json";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Preferences {
    pub include_slots: [bool; MAX_WORKERS],
    pub nearby: bool,
    pub direct: bool,
    pub groups: bool,
    pub conferences: bool,
    pub oldest_first: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            include_slots: [true; MAX_WORKERS],
            nearby: false,
            direct: true,
            groups: true,
            conferences: true,
            oldest_first: false,
        }
    }
}

impl Preferences {
    pub fn includes(self, slot: SlotId) -> bool {
        self.include_slots[usize::from(u8::from(slot))]
    }

    fn disabled() -> Self {
        Self {
            include_slots: [false; MAX_WORKERS],
            nearby: false,
            direct: false,
            groups: false,
            conferences: false,
            oldest_first: false,
        }
    }
}

/// Exact attempted values and ticket distinguish a failed save from the last
/// published preferences. Unrelated host operations never overwrite this.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PreferenceOperation {
    pub ticket: u64,
    pub preferences: Preferences,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PreferenceState {
    pub preferences: Preferences,
    pub writable: bool,
    pub error: Option<String>,
    pub operation: Option<PreferenceOperation>,
}

impl Default for PreferenceState {
    fn default() -> Self {
        // Until the IO thread validates the store, reminders remain disabled.
        Self {
            preferences: Preferences::disabled(),
            writable: false,
            error: None,
            operation: None,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    version: u16,
    preferences: Preferences,
}

// Decode both closed schemas without a JSON Value intermediary, retaining
// duplicate-field rejection. Opening never rewrites a user's saved filters.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyPreferences {
    include_slots: [bool; MAX_WORKERS],
    nearby: bool,
    direct: bool,
    oldest_first: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyStored {
    version: u16,
    preferences: LegacyPreferences,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GroupPreferencesV2 {
    include_slots: [bool; MAX_WORKERS],
    nearby: bool,
    direct: bool,
    groups: bool,
    oldest_first: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GroupStoredV2 {
    version: u16,
    preferences: GroupPreferencesV2,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum StoredInput {
    Current(Stored),
    Groups(GroupStoredV2),
    Legacy(LegacyStored),
}

pub(super) struct Store {
    path: PathBuf,
    state: PreferenceState,
}

impl Store {
    pub(super) fn open(directory: &Path) -> Self {
        let path = directory.join(FILE_NAME);
        let state = match load(&path) {
            Ok(preferences) => PreferenceState {
                preferences,
                writable: true,
                error: None,
                operation: None,
            },
            Err(error) => PreferenceState {
                error: Some(error.to_string()),
                ..Default::default()
            },
        };
        Self { path, state }
    }

    pub(super) fn state(&self) -> &PreferenceState {
        &self.state
    }

    pub(super) fn save(&mut self, ticket: u64, preferences: Preferences) {
        self.save_with(ticket, preferences, |path, stored| {
            super::slots::write_json(path, stored, MAX_BYTES)
        });
    }

    fn save_with(&mut self, ticket: u64, preferences: Preferences, publish: impl FnOnce(&Path, &Stored) -> io::Result<()>) {
        let result = if self.state.writable {
            publish(
                &self.path,
                &Stored {
                    version: VERSION,
                    preferences,
                },
            )
        } else {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Attention preferences are not writable",
            ))
        };
        if result.is_ok() {
            self.state.preferences = preferences;
        }
        let error = result.err().map(|error| error.to_string());
        // Preserve the initial validation error for a nonwritable store.
        if self.state.writable {
            self.state.error.clone_from(&error);
        }
        self.state.operation = Some(PreferenceOperation {
            ticket,
            preferences,
            error,
        });
    }
}

fn load(path: &Path) -> io::Result<Preferences> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => {
            match super::transport::read_json::<StoredInput>(path, MAX_BYTES)? {
                StoredInput::Current(stored) if stored.version == VERSION => Ok(stored.preferences),
                StoredInput::Groups(stored) if stored.version == 2 => Ok(Preferences {
                    include_slots: stored.preferences.include_slots,
                    nearby: stored.preferences.nearby,
                    direct: stored.preferences.direct,
                    groups: stored.preferences.groups,
                    // The new personal-session category inherits saved Direct
                    // silence; opening never writes or enables it implicitly.
                    conferences: stored.preferences.direct,
                    oldest_first: stored.preferences.oldest_first,
                }),
                StoredInput::Legacy(stored) if stored.version == 1 => Ok(Preferences {
                    include_slots: stored.preferences.include_slots,
                    nearby: stored.preferences.nearby,
                    direct: stored.preferences.direct,
                    // V1 had no Group filter. Preserve the existing choice to
                    // silence all supported non-Nearby reminders when off.
                    groups: stored.preferences.direct,
                    conferences: stored.preferences.direct,
                    oldest_first: stored.preferences.oldest_first,
                }),
                _ => Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unsupported Attention preferences version",
                )),
            }
        }
        // Absence uses pinned defaults without eagerly creating a file.
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Preferences::default()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::super::profile;
    use super::*;
    use std::io::Write;

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            // Synthetic files only; never open a saved profile or account.
            let root = profile::host_root()
                .unwrap()
                .join(format!("attention-test-{}", uuid::Uuid::new_v4()));
            profile::private_dir(&root).unwrap();
            Self(root)
        }

        fn path(&self) -> PathBuf {
            self.0.join(FILE_NAME)
        }

        fn write(&self, bytes: &[u8]) {
            let mut file = profile::create_private_file(&self.path()).unwrap();
            file.write_all(bytes).unwrap();
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn changed() -> Preferences {
        Preferences {
            include_slots: [false, true, false, true, false],
            nearby: true,
            direct: false,
            groups: true,
            conferences: false,
            oldest_first: true,
        }
    }

    fn encoded() -> Vec<u8> {
        serde_json::to_vec(&Stored {
            version: VERSION,
            preferences: changed(),
        })
        .unwrap()
    }

    #[test]
    fn missing_preferences_use_pinned_defaults_without_creating_a_file() {
        let fixture = Fixture::new();
        let pending = PreferenceState::default();
        assert!(!pending.writable);
        assert!(SlotId::all().all(|slot| !pending.preferences.includes(slot)));
        assert!(!pending.preferences.nearby && !pending.preferences.direct && !pending.preferences.groups);
        let store = Store::open(&fixture.0);
        assert_eq!(store.state.preferences, Preferences::default());
        assert_eq!(store.state.preferences.include_slots, [true; MAX_WORKERS]);
        assert!(!store.state.preferences.nearby);
        assert!(store.state.preferences.direct);
        assert!(store.state.preferences.groups);
        assert!(!store.state.preferences.oldest_first);
        assert!(store.state.writable);
        assert!(store.state.error.is_none());
        assert!(store.state.operation.is_none());
        assert!(!fixture.path().exists());
        assert!(!fixture.0.join("slots.json").exists());
    }

    #[test]
    fn preferences_roundtrip_all_five_stable_ids_independent_of_roster_order() {
        let fixture = Fixture::new();
        // Character-registry bytes are not part of this schema or write path.
        let registry = fixture.0.join("slots.json");
        let mut file = profile::create_private_file(&registry).unwrap();
        file.write_all(b"synthetic registry retained verbatim").unwrap();
        drop(file);
        let mut store = Store::open(&fixture.0);
        store.save(41, changed());
        assert_eq!(store.state.preferences, changed());
        assert_eq!(
            store.state.operation,
            Some(PreferenceOperation {
                ticket: 41,
                preferences: changed(),
                error: None
            })
        );
        let restored = Store::open(&fixture.0);
        assert_eq!(restored.state.preferences, changed());
        assert!(restored.state.writable);
        for raw in [4, 1, 3, 0, 2] {
            let slot = SlotId::try_from(raw).unwrap();
            assert_eq!(restored.state.preferences.includes(slot), raw == 1 || raw == 3);
        }
        assert_eq!(std::fs::read(registry).unwrap(), b"synthetic registry retained verbatim");
        let json: serde_json::Value = serde_json::from_slice(&std::fs::read(fixture.path()).unwrap()).unwrap();
        assert_eq!(json.as_object().unwrap().len(), 2);
        assert_eq!(json["preferences"].as_object().unwrap().len(), 6);
    }

    #[test]
    fn group_attention_preferences_migrate_v1_without_rewriting_or_enabling_previously_silenced_reminders() {
        for direct in [false, true] {
            let fixture = Fixture::new();
            let bytes = serde_json::to_vec(&serde_json::json!({
                "version": 1,
                "preferences": {"include_slots": [false, true, false, true, false],
                    "nearby": true, "direct": direct, "oldest_first": true}
            }))
            .unwrap();
            fixture.write(&bytes);
            let mut store = Store::open(&fixture.0);
            assert!(store.state.writable && store.state.error.is_none());
            let mut desired = changed();
            desired.direct = direct;
            desired.groups = direct;
            desired.conferences = direct;
            assert_eq!(store.state.preferences, desired);
            assert_eq!(std::fs::read(fixture.path()).unwrap(), bytes, "opening cannot rewrite a V1 file");
            desired.groups = !direct;
            store.save(5, desired);
            assert_eq!(Store::open(&fixture.0).state.preferences, desired);
            let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(fixture.path()).unwrap()).unwrap();
            assert_eq!(saved["version"], 3);
            assert_eq!(saved["preferences"]["direct"], direct);
            assert_eq!(saved["preferences"]["groups"], !direct);
        }
    }

    #[test]
    fn group_attention_preferences_reject_mismatched_closed_schemas_and_duplicate_fields_without_repair() {
        let v3 = String::from_utf8(encoded()).unwrap();
        let v2 = v3.replace("\"version\":3", "\"version\":2").replace(",\"conferences\":false", "");
        let v1 = v2.replace("\"version\":2", "\"version\":1").replace(",\"groups\":true", "");
        for bytes in [
            v2.replace("\"version\":2", "\"version\":1"),
            v1.replace("\"version\":1", "\"version\":2"),
            v2.replace("\"groups\":true", "\"groups\":true,\"groups\":false"),
            v1.replace("\"direct\":false", "\"direct\":false,\"direct\":true"),
            v2.replace("\"groups\":true", "\"groups\":null"),
            v1.replace("\"oldest_first\":true", "\"oldest_first\":true,\"future\":true"),
        ] {
            let fixture = Fixture::new();
            fixture.write(bytes.as_bytes());
            let mut store = Store::open(&fixture.0);
            assert!(!store.state.writable && store.state.error.is_some(), "{bytes}");
            assert_eq!(store.state.preferences, Preferences::disabled());
            store.save(8, Preferences::default());
            assert_eq!(std::fs::read(fixture.path()).unwrap(), bytes.as_bytes());
        }
    }

    #[test]
    fn conference_attention_preferences_migrate_v2_without_rewriting_and_preserve_direct_silence_independently_of_groups() {
        for direct in [false, true] {
            for groups in [false, true] {
                let fixture = Fixture::new();
                let bytes = serde_json::to_vec(&serde_json::json!({ "version": 2,
                    "preferences": { "include_slots": [true, true, true, true, true], "nearby": false,
                        "direct": direct, "groups": groups, "oldest_first": false }
                }))
                .unwrap();
                fixture.write(&bytes);
                let mut store = Store::open(&fixture.0);
                assert!(store.state.writable);
                assert_eq!(store.state.preferences.conferences, direct);
                assert_eq!(store.state.preferences.groups, groups);
                assert_eq!(std::fs::read(fixture.path()).unwrap(), bytes);
                let desired = Preferences {
                    conferences: !direct,
                    ..store.state.preferences
                };
                store.save(9, desired);
                assert_eq!(Store::open(&fixture.0).state.preferences, desired);
                let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(fixture.path()).unwrap()).unwrap();
                assert_eq!(saved["version"], 3);
            }
        }
    }

    #[test]
    fn conference_attention_preferences_keep_new_closed_schema_strict_and_failed_files_unchanged() {
        let current = String::from_utf8(encoded()).unwrap();
        for bytes in [
            current.replace("\"version\":3", "\"version\":2"),
            current.replace(",\"conferences\":false", ""),
            current.replace("\"conferences\":false", "\"conferences\":false,\"conferences\":true"),
            current.replace("\"conferences\":false", "\"conferences\":null"),
        ] {
            let fixture = Fixture::new();
            fixture.write(bytes.as_bytes());
            let mut store = Store::open(&fixture.0);
            assert!(!store.state.writable && store.state.error.is_some());
            store.save(10, Preferences::default());
            assert_eq!(std::fs::read(fixture.path()).unwrap(), bytes.as_bytes());
        }
    }

    #[test]
    fn failed_save_preserves_published_values_and_file_with_exact_failure_ticket() {
        let fixture = Fixture::new();
        let mut store = Store::open(&fixture.0);
        store.save(1, changed());
        let before = std::fs::read(fixture.path()).unwrap();
        store.save_with(2, Preferences::default(), |path, stored| {
            super::super::slots::write_json(path, stored, 0)
        });
        assert_eq!(store.state.preferences, changed());
        assert_eq!(std::fs::read(fixture.path()).unwrap(), before);
        let operation = store.state.operation.as_ref().unwrap();
        assert_eq!(operation.ticket, 2);
        assert_eq!(operation.preferences, Preferences::default());
        assert!(operation.error.is_some());
        assert!(store.state.error.is_some());
        store.save(3, Preferences::default());
        assert_eq!(store.state.preferences, Preferences::default());
        assert_eq!(store.state.operation.as_ref().unwrap().ticket, 3);
        assert!(store.state.error.is_none());
    }

    #[test]
    fn invalid_schema_versions_types_and_bounds_fail_closed_without_clobber() {
        let mut newer: serde_json::Value = serde_json::from_slice(&encoded()).unwrap();
        newer["version"] = serde_json::json!(VERSION + 1);
        let mut unknown = newer.clone();
        unknown["version"] = serde_json::json!(VERSION);
        unknown["source"] = serde_json::json!("not allowed");
        let mut nested: serde_json::Value = serde_json::from_slice(&encoded()).unwrap();
        nested["preferences"]["unknown"] = serde_json::json!(true);
        let mut cases = vec![
            b"malformed".to_vec(),
            vec![b' '; MAX_BYTES + 1],
            serde_json::to_vec(&newer).unwrap(),
            serde_json::to_vec(&unknown).unwrap(),
            serde_json::to_vec(&nested).unwrap(),
        ];
        for mask in [
            serde_json::json!([true, true, true, true]),
            serde_json::json!([true, true, true, true, true, true]),
            serde_json::json!([1, 1, 1, 1, 1]),
            serde_json::json!(31),
        ] {
            let mut invalid: serde_json::Value = serde_json::from_slice(&encoded()).unwrap();
            invalid["preferences"]["include_slots"] = mask;
            cases.push(serde_json::to_vec(&invalid).unwrap());
        }
        for bytes in cases {
            let fixture = Fixture::new();
            fixture.write(&bytes);
            let mut store = Store::open(&fixture.0);
            assert!(!store.state.writable);
            assert!(store.state.error.is_some());
            assert_eq!(store.state.preferences, Preferences::disabled());
            store.save(9, Preferences::default());
            assert_eq!(store.state.preferences, Preferences::disabled());
            assert!(store.state.operation.as_ref().unwrap().error.is_some());
            assert_eq!(std::fs::read(fixture.path()).unwrap(), bytes);
        }
    }

    #[test]
    fn preference_byte_bound_accepts_its_exact_limit_and_rejects_growth() {
        let fixture = Fixture::new();
        let mut bytes = encoded();
        bytes.resize(MAX_BYTES, b' ');
        fixture.write(&bytes);
        assert_eq!(Store::open(&fixture.0).state.preferences, changed());
        let mut file = profile::open_existing_private(&fixture.path()).unwrap();
        bytes.push(b' ');
        file.write_all(&bytes).unwrap();
        drop(file);
        let store = Store::open(&fixture.0);
        assert!(!store.state.writable);
        assert_eq!(store.state.preferences, Preferences::disabled());
        assert_eq!(std::fs::metadata(fixture.path()).unwrap().len(), (MAX_BYTES + 1) as u64);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_nonprivate_and_hardlinked_preferences_are_never_adopted_or_rewritten() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let fixture = Fixture::new();
        let target = fixture.0.join("synthetic-target");
        let mut file = profile::create_private_file(&target).unwrap();
        file.write_all(&encoded()).unwrap();
        drop(file);
        symlink(&target, fixture.path()).unwrap();
        let mut store = Store::open(&fixture.0);
        assert!(!store.state.writable);
        store.save(1, Preferences::default());
        assert!(std::fs::symlink_metadata(fixture.path()).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read(&target).unwrap(), encoded());
        std::fs::remove_file(fixture.path()).unwrap();
        std::fs::hard_link(&target, fixture.path()).unwrap();
        let mut store = Store::open(&fixture.0);
        assert!(!store.state.writable);
        store.save(2, Preferences::default());
        assert_eq!(std::fs::read(&target).unwrap(), encoded());
        std::fs::remove_file(fixture.path()).unwrap();
        fixture.write(&encoded());
        std::fs::set_permissions(fixture.path(), std::fs::Permissions::from_mode(0o644)).unwrap();
        let mut store = Store::open(&fixture.0);
        assert!(!store.state.writable);
        store.save(3, Preferences::default());
        assert_eq!(std::fs::read(fixture.path()).unwrap(), encoded());
        assert_eq!(std::fs::metadata(fixture.path()).unwrap().permissions().mode() & 0o777, 0o644);
    }
}
