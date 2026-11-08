//! Closed settings-file adapter for the stopped-profile copy worker. Never
//! accepts a path from a request, repairs permissions, or backs up raw settings.
use super::*;
use crate::settings::profile_seed::MAX_FILE;
use std::io::{Read, Write};

pub(in crate::session_host) struct ProfileSettingsFile {
    path: PathBuf,
    slot: bool,
}

impl ProfileSettingsFile {
    pub(in crate::session_host) fn main_saved() -> io::Result<Self> {
        let directory = crate::settings::catalogue_dir();
        if !directory.is_absolute() {
            return Err(invalid("main settings directory is not absolute"));
        }
        Ok(Self {
            path: directory.join("settings.json"),
            slot: false,
        })
    }

    pub(in crate::session_host) fn slot(installation: &str, slot: super::super::slots::SlotId) -> io::Result<Self> {
        Ok(Self {
            path: slot_root(installation, slot)?.join("config/settings.json"),
            slot: true,
        })
    }

    fn checked_parent(&self) -> io::Result<&Path> {
        let parent = self.path.parent().ok_or_else(|| invalid("settings file has no parent"))?;
        // Inspect every ancestor without creating/repairing any existing one.
        let mut current = PathBuf::new();
        for component in parent.components() {
            if matches!(component, Component::CurDir | Component::ParentDir) {
                return Err(invalid("settings directory contains a non-normal component"));
            }
            current.push(component);
            if !matches!(component, Component::Prefix(_)) {
                plain_directory(&current)?;
            }
        }
        platform::validate_directory(parent)?;
        if self.slot {
            let root = host_root()?;
            relative_beneath(&root, &self.path)?;
            let mut current = parent;
            loop {
                platform::validate_directory(current)?;
                if current == root {
                    break;
                }
                current = current.parent().ok_or_else(|| invalid("settings file escapes host root"))?;
            }
        }
        Ok(parent)
    }

    pub(in crate::session_host) fn read(&self) -> io::Result<Option<Vec<u8>>> {
        self.checked_parent()?;
        match std::fs::symlink_metadata(&self.path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
            Ok(metadata) if !metadata.is_file() || metadata.file_type().is_symlink() || platform::is_reparse(&metadata) => {
                return Err(invalid("settings file is not plain"));
            }
            Ok(_) => {}
        }
        let mut file = read_only_private(&self.path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || platform::is_reparse(&metadata) || metadata.len() > MAX_FILE as u64 {
            return Err(invalid("settings file is not plain or exceeds its bound"));
        }
        #[cfg(windows)]
        validate_opened_single_link(&file)?;
        platform::validate_file(&self.path, &metadata)?;
        let mut bytes = vec![0; MAX_FILE + 1];
        let mut length = 0;
        while length < bytes.len() {
            match file.read(&mut bytes[length..]) {
                Ok(0) => break,
                Ok(count) => length += count,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        }
        if length > MAX_FILE {
            return Err(invalid("settings file exceeds its bound"));
        }
        bytes.truncate(length);
        Ok(Some(bytes))
    }

    /// Source/target byte CAS and cancellation are supplied by the one worker,
    /// after the private temporary file is durable, immediately before rename.
    pub(in crate::session_host) fn publish(&self, bytes: &[u8], final_check: impl FnOnce() -> io::Result<()>) -> io::Result<()> {
        self.publish_with(bytes, final_check, create_private_file, File::sync_all, |from, to| {
            std::fs::rename(from, to)
        })
    }

    fn publish_with(
        &self,
        bytes: &[u8],
        final_check: impl FnOnce() -> io::Result<()>,
        create: impl FnOnce(&Path) -> io::Result<File>,
        sync: impl FnOnce(&File) -> io::Result<()>,
        rename: impl FnOnce(&Path, &Path) -> io::Result<()>,
    ) -> io::Result<()> {
        if !self.slot || bytes.len() > MAX_FILE {
            return Err(invalid("settings publication is not a bounded slot write"));
        }
        let parent = self.checked_parent()?;
        let temporary = parent.join(format!(".profile-copy-{}.tmp", Uuid::new_v4()));
        let result = (|| {
            let mut file = create(&temporary)?;
            file.write_all(bytes)?;
            sync(&file)?;
            #[cfg(windows)]
            validate_opened_single_link(&file)?;
            validate_private_file(&temporary)?;
            self.checked_parent()?;
            final_check()?;
            // No retry delays after final_check. A refusal needs a new Inspect.
            rename(&temporary, &self.path)?;
            crate::atomic_file::sync_dir(parent);
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(temporary);
        }
        result
    }
}

/// Retain these handles throughout preparation, confirmation, publication and
/// cancellation. Stable ordering covers target and a stopped slot source.
pub(in crate::session_host) struct SlotLocks(Vec<(PathBuf, File)>);
impl SlotLocks {
    pub(in crate::session_host) fn check(&self) -> io::Result<()> {
        for (path, held) in &self.0 {
            validate_private_file(path)?;
            let current = read_only_private(path)?;
            if !same_file(held, &current)? {
                return Err(invalid("profile lifetime lock was replaced"));
            }
        }
        Ok(())
    }
}

pub(in crate::session_host) fn lock_slots(
    installation: &str,
    target: super::super::slots::SlotId,
    source: Option<super::super::slots::SlotId>,
) -> io::Result<SlotLocks> {
    let mut slots = vec![target];
    if let Some(source) = source {
        if source == target {
            return Err(invalid("settings source equals its target"));
        }
        slots.push(source);
    }
    slots.sort_by_key(|slot| u8::from(*slot));
    let mut locks = Vec::with_capacity(slots.len());
    for slot in slots {
        let root = slot_root(installation, slot)?;
        private_dir(&root)?;
        private_dir(&root.join("config"))?;
        let path = root.join("profile.lock");
        let file = lock_private(&path)?;
        locks.push((path, file));
    }
    let locks = SlotLocks(locks);
    locks.check()?;
    Ok(locks)
}

/// Strict OPEN_EXISTING/read-only counterpart to the lock creator. In
/// particular a missing main source never creates or opens a writer.
fn read_only_private(path: &Path) -> io::Result<File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || platform::is_reparse(&metadata) {
        return Err(invalid("settings or lifetime lock file is not plain"));
    }
    #[cfg(windows)]
    validate_opened_single_link(&file)?;
    platform::validate_file(path, &metadata)?;
    Ok(file)
}

#[cfg(unix)]
fn same_file(first: &File, second: &File) -> io::Result<bool> {
    use std::os::unix::fs::MetadataExt;
    let first = first.metadata()?;
    let second = second.metadata()?;
    Ok(first.dev() == second.dev() && first.ino() == second.ino())
}
#[cfg(windows)]
fn same_file(first: &File, second: &File) -> io::Result<bool> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle};
    fn identity(file: &File) -> io::Result<(u32, u32, u32)> {
        let mut info = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
        // SAFETY: the live File handle and correctly sized aligned output stay
        // valid during the query; only successful initialized output is read.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle().cast(), info.as_mut_ptr()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the successful query initialized all fields.
        let info = unsafe { info.assume_init() };
        Ok((info.dwVolumeSerialNumber, info.nFileIndexHigh, info.nFileIndexLow))
    }
    Ok(identity(first)? == identity(second)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn profile_copy_settings_file_temp_flush_final_check_and_rename_failures_preserve_original() {
        for fault in 0..4 {
            let installation = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
            let slot = super::super::super::slots::SlotId::try_from(0).unwrap();
            let root = slot_root(&installation, slot).unwrap();
            private_dir(&root).unwrap();
            private_dir(&root.join("config")).unwrap();
            let file = ProfileSettingsFile::slot(&installation, slot).unwrap();
            let original = br#"{"settings_version":32,"synthetic":"original"}"#;
            let mut original_file = create_private_file(&file.path).unwrap();
            original_file.write_all(original).unwrap();
            drop(original_file);
            let checked = AtomicBool::new(false);
            let fail = || io::Error::other("synthetic publication fault");
            let result = file.publish_with(
                b"synthetic replacement",
                || {
                    checked.store(true, Ordering::Release);
                    if fault == 2 { Err(fail()) } else { Ok(()) }
                },
                |path| if fault == 0 { Err(fail()) } else { create_private_file(path) },
                |file| if fault == 1 { Err(fail()) } else { file.sync_all() },
                |from, to| if fault == 3 { Err(fail()) } else { std::fs::rename(from, to) },
            );
            assert!(result.is_err());
            assert_eq!(file.read().unwrap().as_deref(), Some(original.as_slice()));
            assert_eq!(checked.load(Ordering::Acquire), fault >= 2);
            assert_eq!(
                std::fs::read_dir(root.join("config")).unwrap().count(),
                1,
                "private temp was removed"
            );
        }
    }

    #[test]
    fn profile_copy_settings_file_rename_receipt_is_success_even_if_cancel_latches_inside_rename() {
        let installation = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let slot = super::super::super::slots::SlotId::try_from(0).unwrap();
        let root = slot_root(&installation, slot).unwrap();
        private_dir(&root).unwrap();
        private_dir(&root.join("config")).unwrap();
        let file = ProfileSettingsFile::slot(&installation, slot).unwrap();
        let cancelled = AtomicBool::new(false);
        file.publish_with(
            b"synthetic replacement",
            || Ok(()),
            create_private_file,
            File::sync_all,
            |from, to| {
                std::fs::rename(from, to)?;
                cancelled.store(true, Ordering::Release);
                Ok(())
            },
        )
        .unwrap();
        assert!(cancelled.load(Ordering::Acquire));
        assert_eq!(file.read().unwrap().as_deref(), Some(b"synthetic replacement".as_slice()));
    }
}
