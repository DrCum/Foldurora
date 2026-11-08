//! Ordinary-settings compatibility, deliberately separate from strict host
//! lock/temp policy. Retained native directories prove confinement; file modes
//! or inherited ACL spelling alone do not establish it. No permission repair.
use super::{File, Path, PathBuf, SettingsRole, invalid, io, same_file};

const MAX_ANCESTORS: usize = 128;
const MAX_PATH_BYTES: usize = 32 * 1024;

// Pure policy consumes evidence from opened native handles, never a DTO claim.
#[cfg(any(windows, test))]
fn windows_settings_allowed(user_owner: bool, admin_owner: bool, closed: bool, confined: bool) -> bool {
    closed && (user_owner || confined && admin_owner)
}
#[cfg(any(windows, test))]
fn windows_source_allowed(user_owner: bool, admin_owner: bool, closed: bool, confined: bool) -> bool {
    // The opened ACL parser already refused untrusted mutation. Ordinary
    // current-user SOURCE read grants need no confidentiality proof. Preserve
    // the narrower administrator-owner exception unchanged.
    user_owner || windows_settings_allowed(user_owner, admin_owner, closed, confined)
}
#[cfg(any(windows, test))]
#[derive(Clone, Copy)]
enum WindowsGrantRole {
    RetainedPrefix,
    SettingsObject,
}
// Exact Windows servicing identity measured by Root on the native runner.
// This is not a service-SID namespace or an ACL grant trust rule.
#[cfg(any(windows, test))]
const TRUSTED_INSTALLER_SUBAUTHORITIES: [u32; 6] = [80, 956_008_885, 3_418_522_649, 1_831_038_044, 1_853_292_631, 2_271_478_464];
#[cfg(any(windows, test))]
fn windows_trusted_installer_sid(authority: [u8; 6], subauthorities: &[u32]) -> bool {
    authority == [0, 0, 0, 0, 0, 5] && subauthorities == TRUSTED_INSTALLER_SUBAUTHORITIES.as_slice()
}
#[cfg(any(windows, test))]
fn windows_owner_allowed(existing_trusted: bool, installer: bool, role: WindowsGrantRole) -> bool {
    existing_trusted || installer && matches!(role, WindowsGrantRole::RetainedPrefix)
}
#[cfg(any(windows, test))]
fn windows_allow_grant(trusted: bool, mask: u32, role: WindowsGrantRole) -> Result<bool, ()> {
    // FILE_READ_DATA/EA/ATTRIBUTES/EXECUTE, READ_CONTROL, SYNCHRONIZE,
    // GENERIC_READ/EXECUTE. Every other untrusted grant is refused.
    const READ_ONLY: u32 = 0x0000_0001 | 0x0000_0008 | 0x0000_0080 | 0x0000_0020 | 0x0002_0000 | 0x0010_0000 | 0x8000_0000 | 0x2000_0000;
    // These aliases mean adding a different child on a directory, but writing
    // data/appending on a file. Only a prefix with an independently retained
    // existing next-directory child gets the directory interpretation.
    let additions = match role {
        WindowsGrantRole::RetainedPrefix => 0x2 | 0x4,
        WindowsGrantRole::SettingsObject => 0,
    };
    if trusted {
        Ok(true)
    } else if mask & !(READ_ONLY | additions) == 0 {
        Ok(false)
    } else {
        Err(())
    }
}
#[derive(Clone, Copy)]
enum DirectoryRole {
    RetainedPrefix,
    SettingsParent,
}
struct Directory {
    path: PathBuf,
    handle: File,
    role: DirectoryRole,
}
pub(super) struct Confinement {
    directories: Vec<Directory>,
    private_ancestor: Option<usize>,
    role: SettingsRole,
}
impl Confinement {
    pub(super) fn capture(path: &Path, role: SettingsRole) -> io::Result<Self> {
        if !path.is_absolute() || path.as_os_str().as_encoded_bytes().len() > MAX_PATH_BYTES {
            return Err(invalid("settings path is not a bounded absolute native path"));
        }
        let parent = path.parent().ok_or_else(|| invalid("settings file has no parent"))?;
        let mut directories = Vec::new();
        let mut current = PathBuf::new();
        let mut private_ancestor = None;
        for component in parent.components() {
            if matches!(component, std::path::Component::CurDir | std::path::Component::ParentDir) {
                return Err(invalid("settings directory has a non-normal component"));
            }
            current.push(component);
            if matches!(component, std::path::Component::Prefix(_)) {
                continue;
            }
            if directories.len() >= MAX_ANCESTORS {
                return Err(invalid("settings directory ancestry exceeds its bound"));
            }
            let handle = native::open(&current, true)?;
            let role = if current == parent {
                DirectoryRole::SettingsParent
            } else {
                DirectoryRole::RetainedPrefix
            };
            if native::directory(&handle, role)? && private_ancestor.is_none() {
                private_ancestor = Some(directories.len());
            }
            directories.push(Directory {
                path: current.clone(),
                handle,
                role,
            });
        }
        let proof = Self {
            directories,
            private_ancestor,
            role,
        };
        proof.check()?;
        Ok(proof)
    }

    pub(super) fn check(&self) -> io::Result<()> {
        for (index, directory) in self.directories.iter().enumerate() {
            let originally_confined = self.private_ancestor == Some(index);
            if originally_confined && !native::directory(&directory.handle, directory.role)? {
                return Err(invalid("settings private ancestor lost confinement"));
            }
            // Validate every retained handle and its current native-path
            // counterpart, including safe ancestry above the private ancestor.
            native::directory(&directory.handle, directory.role)?;
            let current = native::open(&directory.path, true)?;
            let private = native::directory(&current, directory.role)?;
            if !same_file(&directory.handle, &current)? || originally_confined && !private {
                return Err(invalid("settings ancestor was replaced or lost confinement"));
            }
        }
        Ok(())
    }

    pub(super) fn validate_settings(&self, file: &File) -> io::Result<()> {
        native::settings(file, self.private_ancestor.is_some(), self.role)
    }

    pub(super) fn require_private_target(&self) -> io::Result<()> {
        if self.role != SettingsRole::PrivateTarget || self.private_ancestor.is_none() {
            return Err(invalid("settings target has no private host confinement capability"));
        }
        Ok(())
    }
}
pub(super) fn open_settings(path: &Path) -> io::Result<File> {
    native::open(path, false)
}

#[cfg(unix)]
mod native {
    use super::*;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    pub(super) fn open(path: &Path, directory: bool) -> io::Result<File> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | if directory { libc::O_DIRECTORY } else { 0 })
            .open(path)?;
        let metadata = file.metadata()?;
        if directory && !metadata.is_dir() || !directory && (!metadata.is_file() || metadata.nlink() != 1) {
            return Err(invalid("settings native object has the wrong type or link count"));
        }
        Ok(file)
    }
    pub(super) fn directory(file: &File, _: DirectoryRole) -> io::Result<bool> {
        let metadata = file.metadata()?;
        // SAFETY: geteuid has no preconditions; it only reads this process uid.
        let user = unsafe { libc::geteuid() };
        if !metadata.is_dir() || metadata.uid() != user && metadata.uid() != 0 || metadata.mode() & 0o022 != 0 {
            return Err(invalid("settings ancestry is not owned safely"));
        }
        Ok(metadata.uid() == user && metadata.mode() & 0o077 == 0 && metadata.mode() & 0o700 == 0o700)
    }
    pub(super) fn settings(file: &File, confined: bool, role: SettingsRole) -> io::Result<()> {
        let metadata = file.metadata()?;
        // SAFETY: geteuid has no preconditions; it only reads this process uid.
        let user = unsafe { libc::geteuid() };
        if !metadata.is_file()
            || metadata.nlink() != 1
            || metadata.uid() != user
            || metadata.mode() & 0o022 != 0
            || role == SettingsRole::PrivateTarget && !confined && metadata.mode() & 0o077 != 0
        {
            return Err(invalid("ordinary settings file is not privately confined"));
        }
        Ok(())
    }
}

#[cfg(windows)]
mod native {
    use super::super::super::platform;
    use super::*;
    use std::mem::size_of;
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use std::ptr::null_mut;
    use windows_sys::Win32::Security::*;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, READ_CONTROL,
    };

    const ALLOW: u8 = 0;
    const DENY: u8 = 1;

    pub(super) fn open(path: &Path, directory: bool) -> io::Result<File> {
        let mut options = std::fs::OpenOptions::new();
        options
            .read(true)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | if directory { FILE_FLAG_BACKUP_SEMANTICS } else { 0 });
        if directory {
            options.access_mode(READ_CONTROL | FILE_READ_ATTRIBUTES);
        }
        let file = options.open(path)?;
        let metadata = file.metadata()?;
        if platform::is_reparse(&metadata) || directory && !metadata.is_dir() || !directory && !metadata.is_file() {
            return Err(invalid("settings native object is a reparse point or has the wrong type"));
        }
        if !directory {
            super::super::super::validate_opened_single_link(&file)?;
        }
        Ok(file)
    }

    struct Access {
        user_owner: bool,
        admin_owner: bool,
        closed: bool,
    }
    /// The caller supplies the owner of a live, successfully queried native
    /// security descriptor. Validate its SID before reading exact components.
    unsafe fn trusted_installer_owner(owner: PSID) -> bool {
        // SAFETY: owner remains within the caller's live descriptor; the SID
        // APIs below precede component reads and validate its complete extent.
        unsafe {
            if owner.is_null() || IsValidSid(owner) == 0 || GetLengthSid(owner) != 32 {
                return false;
            }
            let authority = GetSidIdentifierAuthority(owner);
            let count = GetSidSubAuthorityCount(owner);
            if authority.is_null() || count.is_null() || *count != 6 {
                return false;
            }
            let mut subauthorities = [0; 6];
            for (index, value) in subauthorities.iter_mut().enumerate() {
                let component = GetSidSubAuthority(owner, index as u32);
                if component.is_null() {
                    return false;
                }
                *value = component.read_unaligned();
            }
            windows_trusted_installer_sid((*authority).Value, &subauthorities)
        }
    }
    fn access(file: &File, role: WindowsGrantRole) -> io::Result<Access> {
        let wanted = OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION;
        let mut bytes = 0;
        // SAFETY: this live File handle is queried with a null/zero size buffer.
        unsafe { GetKernelObjectSecurity(file.as_raw_handle().cast(), wanted, null_mut(), 0, &mut bytes) };
        let mut storage = platform::storage(bytes)?;
        let descriptor = storage.as_mut_ptr().cast();
        let mut user = platform::current_user()?;
        // SAFETY: the successful TokenUser query contains an aligned TOKEN_USER.
        let sid = unsafe { (*user.as_mut_ptr().cast::<TOKEN_USER>()).User.Sid };
        let mut system = [0usize; 9];
        let mut admins = [0usize; 9];
        let mut system_bytes = std::mem::size_of_val(&system) as u32;
        let mut admin_bytes = std::mem::size_of_val(&admins) as u32;
        let mut owner = null_mut();
        let mut acl = null_mut();
        let mut present = 0;
        let mut defaulted = 0;
        // SAFETY: SID buffers are aligned and at least SECURITY_MAX_SID_SIZE;
        // descriptor storage is bounded/aligned and populated before parsing.
        unsafe {
            platform::check(CreateWellKnownSid(
                WinLocalSystemSid,
                null_mut(),
                system.as_mut_ptr().cast(),
                &mut system_bytes,
            ))?;
            platform::check(CreateWellKnownSid(
                WinBuiltinAdministratorsSid,
                null_mut(),
                admins.as_mut_ptr().cast(),
                &mut admin_bytes,
            ))?;
            platform::check(GetKernelObjectSecurity(
                file.as_raw_handle().cast(),
                wanted,
                descriptor,
                bytes,
                &mut bytes,
            ))?;
            platform::check(GetSecurityDescriptorOwner(descriptor, &mut owner, &mut defaulted))?;
            platform::check(GetSecurityDescriptorDacl(descriptor, &mut present, &mut acl, &mut defaulted))?;
            if owner.is_null() || present == 0 || acl.is_null() {
                return Err(invalid("ordinary settings security descriptor has no owner or DACL"));
            }
            let user_owner = EqualSid(owner, sid) != 0;
            let admin_owner = EqualSid(owner, admins.as_mut_ptr().cast()) != 0;
            let system_owner = EqualSid(owner, system.as_mut_ptr().cast()) != 0;
            // Owner-only exception for an exact measured servicing SID on a
            // retained prefix. It never changes user_owner/admin_owner, ACL
            // principal trust, settings ownership or private-ancestor credit.
            let installer_owner = matches!(role, WindowsGrantRole::RetainedPrefix) && trusted_installer_owner(owner);
            if !windows_owner_allowed(user_owner || admin_owner || system_owner, installer_owner, role) {
                return Err(invalid("ordinary settings native object has an untrusted owner"));
            }
            let mut information = ACL_SIZE_INFORMATION::default();
            platform::check(GetAclInformation(
                acl,
                (&mut information as *mut ACL_SIZE_INFORMATION).cast(),
                size_of::<ACL_SIZE_INFORMATION>() as u32,
                AclSizeInformation,
            ))?;
            if information.AceCount > 256 {
                return Err(invalid("ordinary settings ACL exceeds its entry bound"));
            }
            let mut closed = true;
            for index in 0..information.AceCount {
                let mut raw = null_mut();
                platform::check(GetAce(acl, index, &mut raw))?;
                if raw.is_null() {
                    return Err(invalid("ordinary settings ACL entry is missing"));
                }
                let header = &*raw.cast::<ACE_HEADER>();
                // This grants nothing on the opened object. Descendants are
                // independently checked, so inheritance is neither private
                // evidence nor a substitute for any file's current ACL.
                if header.AceFlags & INHERIT_ONLY_ACE as u8 != 0 {
                    continue;
                }
                if header.AceType != ALLOW && header.AceType != DENY || usize::from(header.AceSize) < size_of::<ACCESS_ALLOWED_ACE>() {
                    return Err(invalid("ordinary settings ACL entry type is unsupported"));
                }
                // Deny cannot justify accepting a broad applicable allow.
                if header.AceType == DENY {
                    continue;
                }
                let ace = &*raw.cast::<ACCESS_ALLOWED_ACE>();
                let principal: PSID = (&ace.SidStart as *const u32).cast_mut().cast();
                let available = usize::from(header.AceSize) - (size_of::<ACCESS_ALLOWED_ACE>() - size_of::<u32>());
                // Validate the SID header/count's complete extent before any
                // SID API can inspect its variable SubAuthority array.
                if available < 8
                    || *principal.cast::<u8>().add(1) > 15
                    || 8 + 4 * usize::from(*principal.cast::<u8>().add(1)) > available
                    || IsValidSid(principal) == 0
                    || GetLengthSid(principal) as usize > available
                {
                    return Err(invalid("ordinary settings ACL SID is invalid"));
                }
                let trusted = EqualSid(principal, sid) != 0
                    || EqualSid(principal, system.as_mut_ptr().cast()) != 0
                    || EqualSid(principal, admins.as_mut_ptr().cast()) != 0;
                closed &= windows_allow_grant(trusted, ace.Mask, role)
                    .map_err(|_| invalid("ordinary settings ancestor grants untrusted mutation"))?;
            }
            Ok(Access {
                user_owner,
                admin_owner,
                closed,
            })
        }
    }
    pub(super) fn directory(file: &File, role: DirectoryRole) -> io::Result<bool> {
        if !file.metadata()?.is_dir() {
            return Err(invalid("settings ancestor is not a directory"));
        }
        let grant_role = match role {
            DirectoryRole::RetainedPrefix => WindowsGrantRole::RetainedPrefix,
            DirectoryRole::SettingsParent => WindowsGrantRole::SettingsObject,
        };
        let access = access(file, grant_role)?;
        Ok(access.user_owner && access.closed)
    }
    pub(super) fn settings(file: &File, confined: bool, role: SettingsRole) -> io::Result<()> {
        super::super::super::validate_opened_single_link(file)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || platform::is_reparse(&metadata) {
            return Err(invalid("settings file is not plain"));
        }
        let access = access(file, WindowsGrantRole::SettingsObject)?;
        let allowed = match role {
            SettingsRole::ReadOnlySource => windows_source_allowed(access.user_owner, access.admin_owner, access.closed, confined),
            SettingsRole::PrivateTarget => windows_settings_allowed(access.user_owner, access.admin_owner, access.closed, confined),
        };
        if !allowed {
            return Err(invalid("ordinary settings file is not privately confined"));
        }
        Ok(())
    }
}

#[cfg(not(any(unix, windows)))]
mod native {
    use super::*;
    pub(super) fn open(_: &Path, _: bool) -> io::Result<File> {
        Err(invalid("ordinary settings native policy is unsupported"))
    }
    pub(super) fn directory(_: &File, _: DirectoryRole) -> io::Result<bool> {
        Err(invalid("ordinary settings native policy is unsupported"))
    }
    pub(super) fn settings(_: &File, _: bool, _: SettingsRole) -> io::Result<()> {
        Err(invalid("ordinary settings native policy is unsupported"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_copy_windows_trusted_installer_sid_requires_exact_authority_count_and_every_service_component() {
        // Root's native runner matched this complete fixed servicing SID,
        // never an arbitrary NT SERVICE principal or a translated SID name.
        let authority = [0, 0, 0, 0, 0, 5];
        let exact = [80, 956_008_885, 3_418_522_649, 1_831_038_044, 1_853_292_631, 2_271_478_464];
        assert!(windows_trusted_installer_sid(authority, &exact));
        for index in 0..exact.len() {
            let mut neighbor = exact;
            neighbor[index] = neighbor[index].wrapping_add(1);
            assert!(!windows_trusted_installer_sid(authority, &neighbor));
        }
        for index in 0..authority.len() {
            let mut other_authority = authority;
            other_authority[index] ^= 1;
            assert!(!windows_trusted_installer_sid(other_authority, &exact));
        }
        for count in 0..exact.len() {
            assert!(!windows_trusted_installer_sid(authority, &exact[..count]));
        }
        let mut longer = exact.to_vec();
        longer.push(0);
        assert!(!windows_trusted_installer_sid(authority, &longer));
    }

    #[test]
    fn profile_copy_windows_servicing_owner_is_prefix_only_without_acl_trust_or_private_settings_credit() {
        assert!(windows_owner_allowed(false, true, WindowsGrantRole::RetainedPrefix));
        assert!(!windows_owner_allowed(false, true, WindowsGrantRole::SettingsObject));
        for role in [WindowsGrantRole::RetainedPrefix, WindowsGrantRole::SettingsObject] {
            assert!(
                windows_owner_allowed(true, false, role),
                "existing User/System/Administrators owner trust remains"
            );
            assert!(!windows_owner_allowed(false, false, role), "unestablished owner remains refused");
            for mask in [0x40, 0x10000, 0x40000, 0x80000, 0x001f_01ff, 0x1000_0000, 0x4000_0000, u32::MAX] {
                assert_eq!(
                    windows_allow_grant(false, mask, role),
                    Err(()),
                    "owner recognition never trusts this ACL principal"
                );
            }
        }
        assert!(!windows_settings_allowed(false, false, true, true));
        assert!(!windows_source_allowed(false, false, true, true));
        assert_eq!(windows_allow_grant(false, 4, WindowsGrantRole::RetainedPrefix), Ok(false));
        assert_eq!(windows_allow_grant(false, 4, WindowsGrantRole::SettingsObject), Err(()));
    }

    fn native_directory() -> PathBuf {
        let installation = format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple());
        let slot = crate::session_host::slots::SlotId::try_from(0).unwrap();
        let root = crate::session_host::profile::slot_root(&installation, slot).unwrap();
        crate::session_host::profile::private_dir(&root).unwrap();
        crate::session_host::profile::private_dir(&root.join("config")).unwrap();
        root.join("config")
    }
    fn ordinary_save(path: &Path) {
        // Same ordinary writer as Settings::write_saved. No host-private
        // creator, chmod, ACL repair or alternate native root is used here.
        crate::atomic_file::write(path, br#"{"settings_version":32,"ui_scale":1.0}"#).unwrap();
    }

    #[test]
    fn profile_copy_ordinary_native_saver_output_is_readable_under_retained_private_slot_ancestry() {
        let directory = native_directory();
        let path = directory.join("settings.json");
        ordinary_save(&path);
        let file = super::super::ProfileSettingsFile::selected(path.clone(), true).unwrap();
        assert_eq!(file.read().unwrap(), Some(std::fs::read(&path).unwrap()));
        // Publication and a subsequent ordinary replacement must both remain
        // compatible. The same retained ancestor handles are still required.
        file.publish(br#"{"settings_version":32,"ui_scale":1.25}"#, || Ok(())).unwrap();
        ordinary_save(&path);
        assert_eq!(file.read().unwrap(), Some(std::fs::read(path).unwrap()));
    }

    #[test]
    fn profile_copy_ordinary_main_saved_shape_can_use_confidential_enclosing_ancestor() {
        let directory = native_directory();
        let nested = directory.join("ordinary-main-config");
        std::fs::create_dir(&nested).unwrap();
        let path = nested.join("settings.json");
        ordinary_save(&path);
        let file = super::super::ProfileSettingsFile::selected(path.clone(), false).unwrap();
        assert_eq!(file.read().unwrap(), Some(std::fs::read(path).unwrap()));
        assert!(file.publish(b"main saved must never be written", || Ok(())).is_err());
    }

    #[test]
    fn profile_copy_retained_confinement_rejects_ancestor_replacement_before_read_or_publish() {
        let directory = native_directory();
        let path = directory.join("settings.json");
        ordinary_save(&path);
        let file = super::super::ProfileSettingsFile::selected(path.clone(), true).unwrap();
        let retired = directory.with_extension("original-confinement");
        std::fs::rename(&directory, &retired).unwrap();
        crate::session_host::profile::private_dir(&directory).unwrap();
        ordinary_save(&path);
        assert!(file.read().is_err());
        assert!(file.publish(b"must never appear", || Ok(())).is_err());
        assert_eq!(std::fs::read(path).unwrap(), std::fs::read(retired.join("settings.json")).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn profile_copy_unix_confined_0644_is_compatible_but_unconfined_or_writable_is_refused_without_repair() {
        use std::os::unix::fs::PermissionsExt;
        let directory = native_directory();
        let path = directory.join("settings.json");
        ordinary_save(&path);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let proof = Confinement::capture(&path, SettingsRole::PrivateTarget).unwrap();
        let file = open_settings(&path).unwrap();
        assert!(proof.private_ancestor.is_some());
        proof.validate_settings(&file).unwrap();
        // No genuine private ancestor evidence is supplied to this negative
        // policy case. A path/DTO alone cannot synthesize that authority.
        let unconfined = Confinement {
            directories: Vec::new(),
            private_ancestor: None,
            role: SettingsRole::PrivateTarget,
        };
        assert!(unconfined.validate_settings(&file).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        unconfined.validate_settings(&file).unwrap();
        for mode in [0o620, 0o602, 0o666] {
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
            assert!(proof.validate_settings(&file).is_err());
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, mode);
        }
    }

    #[cfg(unix)]
    #[test]
    fn profile_copy_unix_confined_settings_still_refuse_symlink_hardlink_and_ancestor_writability() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        for fault in 0..3 {
            let directory = native_directory();
            let path = directory.join("settings.json");
            ordinary_save(&path);
            let file = super::super::ProfileSettingsFile::selected(path.clone(), true).unwrap();
            match fault {
                0 => {
                    std::fs::rename(&path, path.with_extension("original")).unwrap();
                    symlink(path.with_extension("original"), &path).unwrap();
                }
                1 => std::fs::hard_link(&path, path.with_extension("hardlink")).unwrap(),
                2 => std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o722)).unwrap(),
                _ => unreachable!(),
            }
            assert!(file.read().is_err());
        }
    }

    #[test]
    fn profile_copy_windows_elevated_normal_save_owner_requires_closed_acl_and_same_user_confinement() {
        assert!(windows_settings_allowed(true, false, true, false));
        assert!(windows_settings_allowed(false, true, true, true));
        assert!(
            !windows_settings_allowed(false, true, true, false),
            "unconfined/escaped admin-owned settings"
        );
        assert!(!windows_settings_allowed(false, false, true, true), "system/other settings owner");
        assert!(
            !windows_settings_allowed(false, true, false, true),
            "broad ACL cannot borrow confinement"
        );
    }

    #[test]
    fn profile_copy_windows_closed_allow_principals_never_accept_untrusted_mutation_or_broad_settings_read() {
        // TokenUser/LocalSystem/BuiltinAdministrators classification is done
        // by EqualSid on the opened-handle descriptor, never group membership.
        assert_eq!(windows_allow_grant(true, u32::MAX, WindowsGrantRole::SettingsObject), Ok(true));
        assert_eq!(windows_allow_grant(false, 0x8000_0000, WindowsGrantRole::SettingsObject), Ok(false));
        for unsafe_mask in [0x2, 0x4, 0x40, 0x10000, 0x40000, 0x80000, 0x1000_0000, 0x4000_0000, 0x0200_0000] {
            assert_eq!(windows_allow_grant(false, unsafe_mask, WindowsGrantRole::SettingsObject), Err(()));
        }
        assert!(!windows_settings_allowed(true, false, false, true));
    }

    #[test]
    fn profile_copy_windows_normal_drive_prefix_can_add_other_children_without_proving_private_confinement() {
        // Root's normal native ACL shape: applicable Users/AuthenticatedUsers
        // read/traverse + FILE_ADD_FILE/FILE_ADD_SUBDIRECTORY on the drive prefix.
        // Existing next child directories still have their own retained handles,
        // owner/ACL checks and current-path identity checks before/after reads.
        for mask in [0x2, 0x4, 0x2 | 0x4, 0x8000_0000 | 0x2 | 0x4] {
            assert_eq!(windows_allow_grant(false, mask, WindowsGrantRole::RetainedPrefix), Ok(false));
            // Accepted creation is explicitly NOT a closed/private ACL, even
            // when the prefix happens to have the current user's owner SID.
            assert!(!windows_settings_allowed(true, false, false, true));
            assert_eq!(windows_allow_grant(false, mask, WindowsGrantRole::SettingsObject), Err(()));
        }
    }

    #[test]
    fn profile_copy_windows_prefix_creation_never_accepts_parent_delete_child_or_existing_child_delete() {
        // Parent DELETE_CHILD or DELETE/WRITE_DAC/WRITE_OWNER on an existing
        // child permits replacement; generic/unknown grants are not expanded
        // into presumed harmless directory creation.
        for dangerous in [0x40, 0x10000, 0x40000, 0x80000, 0x1000_0000, 0x4000_0000, 0x0200_0000] {
            for mask in [dangerous, dangerous | 0x2, dangerous | 0x4, dangerous | 0x2 | 0x4] {
                assert_eq!(windows_allow_grant(false, mask, WindowsGrantRole::RetainedPrefix), Err(()));
                assert_eq!(windows_allow_grant(false, mask, WindowsGrantRole::SettingsObject), Err(()));
            }
        }
    }

    #[cfg(windows)]
    #[test]
    fn profile_copy_windows_ordinary_temp_ancestry_read_uses_real_retained_prefix_roles_and_normal_saver() {
        // Root alone executes this native Windows fixture. No claim about the
        // machine's actual ACL is made by the author. Unsupported prefix owner
        // principals (including an unestablished TrustedInstaller case) still
        // refuse; this test is a real native acceptance gate, never a skip.
        let directory = std::env::temp_dir().join(format!("aurora-profile-copy-prefix-{}", uuid::Uuid::new_v4()));
        crate::session_host::profile::platform::private_directory(&directory).unwrap();
        let path = directory.join("settings.json");
        ordinary_save(&path);
        let file = super::super::ProfileSettingsFile::selected(path.clone(), false).unwrap();
        assert!(
            file.confinement
                .directories
                .iter()
                .any(|node| matches!(node.role, DirectoryRole::RetainedPrefix))
        );
        assert!(matches!(
            file.confinement.directories.last().unwrap().role,
            DirectoryRole::SettingsParent
        ));
        assert_eq!(file.read().unwrap(), Some(std::fs::read(&path).unwrap()));
        file.confinement.check().unwrap();
        drop(file);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn profile_copy_readonly_source_ordinary_0644_needs_no_private_evidence_and_cannot_be_target_even_if_0600() {
        use std::os::unix::fs::PermissionsExt;
        let directory = native_directory();
        let path = directory.join("settings.json");
        ordinary_save(&path);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let mut source = super::super::SourceSettingsFile(
            super::super::ProfileSettingsFile::capture(path.clone(), true, SettingsRole::ReadOnlySource).unwrap(),
        );
        // CI/test workspaces may themselves be private. Discard any such
        // optional confidentiality evidence; retain all real opened ancestors
        // and identity/integrity checks. No private authority is fabricated.
        source.0.confinement.private_ancestor = None;
        assert_eq!(source.read().unwrap(), Some(std::fs::read(&path).unwrap()));
        assert!(source.0.confinement.require_private_target().is_err());
        assert!(source.0.publish(b"source cannot publish", || Ok(())).is_err());
        let mut target = super::super::TargetSettingsFile(
            super::super::ProfileSettingsFile::capture(path.clone(), true, SettingsRole::PrivateTarget).unwrap(),
        );
        target.0.confinement.private_ancestor = None;
        assert!(target.read().is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(
            target.read().is_err(),
            "a private file does not replace actual private host target confinement"
        );
        assert!(target.publish(b"target has no private authority", || Ok(())).is_err());
        assert_eq!(std::fs::read(path).unwrap(), br#"{"settings_version":32,"ui_scale":1.0}"#);
    }

    #[cfg(unix)]
    #[test]
    fn profile_copy_readonly_source_still_refuses_writable_link_ancestor_turnover_and_root_escape() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        for fault in 0..5 {
            let directory = native_directory();
            let path = directory.join("settings.json");
            ordinary_save(&path);
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
            let source = super::super::SourceSettingsFile(
                super::super::ProfileSettingsFile::capture(path.clone(), true, SettingsRole::ReadOnlySource).unwrap(),
            );
            match fault {
                0 => std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap(),
                1 => std::fs::hard_link(&path, path.with_extension("hardlink")).unwrap(),
                2 => {
                    std::fs::rename(&path, path.with_extension("original")).unwrap();
                    symlink(path.with_extension("original"), &path).unwrap();
                }
                3 => {
                    std::fs::rename(&directory, directory.with_extension("retired")).unwrap();
                    crate::session_host::profile::private_dir(&directory).unwrap();
                    ordinary_save(&path);
                }
                4 => std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o722)).unwrap(),
                _ => unreachable!(),
            }
            assert!(source.read().is_err());
        }
        let directory = native_directory();
        let escaped = directory.join("../settings.json");
        assert!(super::super::ProfileSettingsFile::capture(escaped, true, SettingsRole::ReadOnlySource).is_err());
        assert!(
            super::super::SourceSettingsFile::stopped_slot("../foreign", crate::session_host::slots::SlotId::try_from(0).unwrap()).is_err()
        );
    }

    #[test]
    fn profile_copy_windows_readonly_source_can_read_broad_read_acl_without_lending_target_or_admin_authority() {
        assert!(windows_source_allowed(true, false, false, false));
        assert!(!windows_settings_allowed(true, false, false, false));
        assert!(windows_source_allowed(false, true, true, true));
        assert!(!windows_source_allowed(false, true, true, false));
        assert!(!windows_source_allowed(false, true, false, true));
        assert!(!windows_source_allowed(false, false, true, true));
        assert_eq!(windows_allow_grant(false, 0x8000_0000, WindowsGrantRole::SettingsObject), Ok(false));
        for mutation in [0x2, 0x4, 0x40, 0x10000, 0x40000, 0x80000, 0x4000_0000] {
            assert_eq!(windows_allow_grant(false, mutation, WindowsGrantRole::SettingsObject), Err(()));
        }
    }
}
