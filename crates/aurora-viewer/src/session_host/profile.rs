//! Explicit production profiles for host workers. Selection happens before
//! settings, cache initialization or any other worker profile writer starts.
//! The shared account catalogue contains metadata; settings and account files
//! belong to this exact session generation and are never copied from another.

use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;
use uuid::Uuid;

struct Active {
    root: PathBuf,
    // Keep the OS lock until process exit, including after host detachment.
    _lock: File,
}

static ACTIVE: OnceLock<Active> = OnceLock::new();

pub(crate) fn root() -> Option<&'static Path> {
    ACTIVE.get().map(|active| active.root.as_path())
}

/// The only profile selector accepted by a worker. A config cannot supply an
/// arbitrary directory or substitute the updater's isolated test installation.
pub(super) fn activate(installation: &str, session: Uuid, generation: u64) -> anyhow::Result<()> {
    anyhow::ensure!(root().is_none(), "host worker profile was already selected");
    anyhow::ensure!(
        std::env::var_os("AURORA_UPDATE_TEST_ROOT").is_none() && aurora_updater::test_environment().is_none(),
        "host worker profiles cannot use an updater test installation"
    );
    let relative = profile_relative(installation, session, generation)?;
    let executable = std::fs::canonicalize(std::env::current_exe()?)?;
    let own_installation: String = Sha256::digest(executable.as_os_str().as_encoded_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    anyhow::ensure!(
        installation == own_installation,
        "host worker profile belongs to another installation"
    );
    let root = host_root()?.join(relative);
    private_dir(&root)?;
    private_dir(&root.join("config"))?;
    private_dir(&root.join("cache"))?;
    let lock = profile_lock(&root)?;
    ACTIVE
        .set(Active { root, _lock: lock })
        .map_err(|_| anyhow::anyhow!("host worker profile was already selected"))
}

fn profile_relative(installation: &str, session: Uuid, generation: u64) -> io::Result<PathBuf> {
    if !crate::characters::valid_id(installation) || session.is_nil() || generation == 0 {
        return Err(invalid("invalid host worker profile identity"));
    }
    Ok(Path::new("profiles").join(installation).join(format!("{session}-{generation}")))
}

/// A canonical per-user root for production host channels and worker profiles.
/// There is deliberately no current-directory fallback and no environment
/// variable accepting an arbitrary profile root.
pub(super) fn host_root() -> io::Result<PathBuf> {
    #[cfg(test)]
    let data = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .ok_or_else(|| invalid("unit-test worktree root is unavailable"))?
        .join("target/unit-test-user-data");
    #[cfg(not(test))]
    let data = directories::ProjectDirs::from("org", "Aurora", "AuroraViewer")
        .map(|dirs| dirs.data_local_dir().to_path_buf())
        .ok_or_else(|| invalid("per-user host directory is unavailable"))?;
    if !data.is_absolute() {
        return Err(invalid("per-user host directory must be absolute"));
    }
    // Do not follow an account-controlled junction while creating the base.
    ordinary_directories(&data)?;
    let data = std::fs::canonicalize(data)?;
    let root = data.join("session-host");
    platform::private_directory(&root)?;
    plain_directory(&root)?;
    Ok(root)
}

/// Create/validate only descendants of the derived production host root. The
/// transport uses this before publishing a private, versioned worker config.
pub(super) fn private_dir(path: &Path) -> io::Result<()> {
    let root = host_root()?;
    let relative = relative_beneath(&root, path)?;
    private_descendants(&root, relative)
}

/// Verify a transport/profile file's own access policy, even if it sits below
/// a private directory. Callers also open with no-follow and bound the read.
pub(super) fn validate_private_file(path: &Path) -> io::Result<()> {
    let root = host_root()?;
    let relative = relative_beneath(&root, path)?;
    if relative.as_os_str().is_empty() {
        return Err(invalid("host file must be below its private root"));
    }
    let mut parent = path.parent().ok_or_else(|| invalid("host file has no parent"))?;
    loop {
        plain_directory(parent)?;
        platform::validate_directory(parent)?;
        if parent == root {
            break;
        }
        parent = parent.parent().ok_or_else(|| invalid("host file escapes its private root"))?;
    }
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || platform::is_reparse(&metadata) {
        return Err(invalid("host file is a link or is not a regular file"));
    }
    platform::validate_file(path, &metadata)
}

fn relative_beneath<'a>(root: &Path, path: &'a Path) -> io::Result<&'a Path> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| invalid("host directory escapes its private root"))?;
    if relative.components().any(|component| !matches!(component, Component::Normal(_))) {
        return Err(invalid("host directory contains a non-normal component"));
    }
    Ok(relative)
}

fn private_descendants(root: &Path, relative: &Path) -> io::Result<()> {
    let mut path = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err(invalid("host directory contains a non-normal component"));
        };
        path.push(name);
        platform::private_directory(&path)?;
        plain_directory(&path)?;
    }
    Ok(())
}

fn ordinary_directories(path: &Path) -> io::Result<()> {
    let mut current = PathBuf::new();
    for component in path.components() {
        if matches!(component, Component::CurDir | Component::ParentDir) {
            return Err(invalid("per-user directory contains a non-normal component"));
        }
        current.push(component);
        // A Windows drive prefix alone ("C:") names that drive's working
        // directory; inspect it only after its following root separator.
        if matches!(component, Component::Prefix(_)) {
            continue;
        }
        match std::fs::symlink_metadata(&current) {
            Ok(_) => plain_directory(&current)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                match std::fs::create_dir(&current) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(error),
                }
                plain_directory(&current)?;
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn plain_directory(path: &Path) -> io::Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() || platform::is_reparse(&metadata) {
        return Err(invalid("host directory is a link or is not a directory"));
    }
    Ok(())
}

fn profile_lock(root: &Path) -> io::Result<File> {
    let path = root.join("profile.lock");
    if let Ok(metadata) = std::fs::symlink_metadata(&path)
        && (!metadata.is_file() || metadata.file_type().is_symlink() || platform::is_reparse(&metadata))
    {
        return Err(invalid("host profile lock is not a plain file"));
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || platform::is_reparse(&metadata) {
        return Err(invalid("host profile lock is not a plain file"));
    }
    platform::validate_file(&root.join("profile.lock"), &metadata)?;
    file.try_lock().map_err(io::Error::other)?;
    Ok(file)
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}

#[cfg(unix)]
mod platform {
    use super::*;
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};

    pub(super) fn private_directory(path: &Path) -> io::Result<()> {
        match std::fs::DirBuilder::new().mode(0o700).create(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        plain_directory(path)?;
        validate_directory(path)
    }

    pub(super) fn validate_directory(path: &Path) -> io::Result<()> {
        let metadata = std::fs::symlink_metadata(path)?;
        // SAFETY: geteuid has no preconditions and reads only this process's uid.
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o777 != 0o700 {
            return Err(invalid("host directory is not owned privately by this user"));
        }
        Ok(())
    }

    pub(super) fn validate_file(_: &Path, metadata: &std::fs::Metadata) -> io::Result<()> {
        // SAFETY: geteuid has no preconditions and reads only this process's uid.
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 || metadata.nlink() != 1 {
            return Err(invalid("host file is not private to this user"));
        }
        Ok(())
    }

    pub(super) fn is_reparse(_: &std::fs::Metadata) -> bool {
        false
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::ffi::c_void;
    use std::mem::size_of;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::MetadataExt;
    use std::ptr::null_mut;
    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, HANDLE};
    use windows_sys::Win32::Security::*;
    use windows_sys::Win32::Storage::FileSystem::{CreateDirectoryW, FILE_ALL_ACCESS, FILE_ATTRIBUTE_REPARSE_POINT};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    // SECURITY_DESCRIPTOR_REVISION / ACCESS_ALLOWED_ACE_TYPE from winnt.h.
    const DESCRIPTOR_REVISION: u32 = 1;
    const ALLOW_ACE_TYPE: u8 = 0;

    struct Token(HANDLE);

    impl Drop for Token {
        fn drop(&mut self) {
            // SAFETY: this handle was opened by OpenProcessToken and is owned here.
            unsafe { CloseHandle(self.0) };
        }
    }

    fn check(success: windows_sys::core::BOOL) -> io::Result<()> {
        if success == 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
    }

    /// Aligned storage for variable-sized Windows security data. The queried
    /// size is bounded before allocating, and every pointer stays in its owner.
    fn storage(bytes: u32) -> io::Result<Vec<usize>> {
        if !(1..=65_536).contains(&bytes) {
            return Err(invalid("invalid Windows security descriptor size"));
        }
        Ok(vec![0; (bytes as usize).div_ceil(size_of::<usize>())])
    }

    fn current_user() -> io::Result<Vec<usize>> {
        let mut handle = null_mut();
        // SAFETY: output is valid and TOKEN_QUERY only reads this process's identity.
        check(unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut handle) })?;
        let token = Token(handle);
        let mut bytes = 0;
        // SAFETY: a null buffer with zero size queries the required byte length.
        unsafe { GetTokenInformation(token.0, TokenUser, null_mut(), 0, &mut bytes) };
        let mut data = storage(bytes)?;
        // SAFETY: aligned data has at least the queried capacity, and bytes is writable.
        check(unsafe { GetTokenInformation(token.0, TokenUser, data.as_mut_ptr().cast(), bytes, &mut bytes) })?;
        Ok(data)
    }

    pub(super) fn private_directory(path: &Path) -> io::Result<()> {
        let name: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        if name[..name.len() - 1].contains(&0) {
            return Err(invalid("host directory contains a nul"));
        }
        let mut user = current_user()?;
        // SAFETY: successful TokenUser query initialized an aligned TOKEN_USER in user.
        let sid = unsafe { (*user.as_mut_ptr().cast::<TOKEN_USER>()).User.Sid };
        // SAFETY: sid points at the live, validated process-token user SID.
        let sid_size = unsafe { GetLengthSid(sid) };
        let acl_size = (size_of::<ACL>() + size_of::<ACCESS_ALLOWED_ACE>() - size_of::<u32>()) as u32 + sid_size;
        let mut acl_data = storage(acl_size)?;
        let acl = acl_data.as_mut_ptr().cast::<ACL>();
        let mut descriptor = SECURITY_DESCRIPTOR::default();
        let sd = (&mut descriptor as *mut SECURITY_DESCRIPTOR).cast::<c_void>();
        // SAFETY: all buffers are aligned, live, writable and sized for these APIs;
        // the only allow ACE names this process's user, inherited by files/children.
        unsafe {
            check(InitializeAcl(acl, acl_size, ACL_REVISION))?;
            check(AddAccessAllowedAceEx(
                acl,
                ACL_REVISION,
                OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE,
                FILE_ALL_ACCESS,
                sid,
            ))?;
            check(InitializeSecurityDescriptor(sd, DESCRIPTOR_REVISION))?;
            check(SetSecurityDescriptorOwner(sd, sid, 0))?;
            check(SetSecurityDescriptorDacl(sd, 1, acl, 0))?;
            check(SetSecurityDescriptorControl(sd, SE_DACL_PROTECTED, SE_DACL_PROTECTED))?;
        }
        let attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: sd,
            bInheritHandle: 0,
        };
        // SAFETY: name is nul-terminated and the descriptor/ACL/SID outlive the call.
        if unsafe { CreateDirectoryW(name.as_ptr(), &attributes) } == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(ERROR_ALREADY_EXISTS as i32) {
                return Err(error);
            }
        }
        plain_directory(path)?;
        verify_security(name.as_ptr(), sid, true)
    }

    pub(super) fn validate_directory(path: &Path) -> io::Result<()> {
        validate_path_security(path, true)
    }

    pub(super) fn validate_file(path: &Path, _: &std::fs::Metadata) -> io::Result<()> {
        validate_path_security(path, false)
    }

    fn validate_path_security(path: &Path, directory: bool) -> io::Result<()> {
        let name: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        if name[..name.len() - 1].contains(&0) {
            return Err(invalid("host path contains a nul"));
        }
        let mut user = current_user()?;
        // SAFETY: successful TokenUser query initialized an aligned TOKEN_USER in user.
        let sid = unsafe { (*user.as_mut_ptr().cast::<TOKEN_USER>()).User.Sid };
        verify_security(name.as_ptr(), sid, directory)
    }

    fn verify_security(name: *const u16, sid: PSID, directory: bool) -> io::Result<()> {
        let wanted = OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION;
        let mut bytes = 0;
        // SAFETY: name is a live nul-terminated path; null/zero queries required size.
        unsafe { GetFileSecurityW(name, wanted, null_mut(), 0, &mut bytes) };
        let mut descriptor = storage(bytes)?;
        let sd = descriptor.as_mut_ptr().cast::<c_void>();
        let mut owner = null_mut();
        let mut defaulted = 0;
        let mut present = 0;
        let mut acl = null_mut();
        let mut control = 0;
        let mut revision = 0;
        // SAFETY: successful descriptor query supplies the pointers read below;
        // their storage and the TokenUser SID remain alive throughout validation.
        unsafe {
            check(GetFileSecurityW(name, wanted, sd, bytes, &mut bytes))?;
            check(GetSecurityDescriptorOwner(sd, &mut owner, &mut defaulted))?;
            check(GetSecurityDescriptorDacl(sd, &mut present, &mut acl, &mut defaulted))?;
            check(GetSecurityDescriptorControl(sd, &mut control, &mut revision))?;
            if owner.is_null()
                || EqualSid(owner, sid) == 0
                || present == 0
                || acl.is_null()
                || (directory && control & SE_DACL_PROTECTED == 0)
            {
                return Err(invalid("host directory owner or protected DACL does not match"));
            }
            let mut information = ACL_SIZE_INFORMATION::default();
            check(GetAclInformation(
                acl,
                (&mut information as *mut ACL_SIZE_INFORMATION).cast(),
                size_of::<ACL_SIZE_INFORMATION>() as u32,
                AclSizeInformation,
            ))?;
            if information.AceCount != 1 {
                return Err(invalid("host directory must have exactly one private allow ACE"));
            }
            let mut ace = null_mut();
            check(GetAce(acl, 0, &mut ace))?;
            if ace.is_null() {
                return Err(invalid("host directory allow ACE is missing"));
            }
            let header = &*ace.cast::<ACE_HEADER>();
            if header.AceType != ALLOW_ACE_TYPE || usize::from(header.AceSize) < size_of::<ACCESS_ALLOWED_ACE>() {
                return Err(invalid("host directory allow ACE has an invalid type or size"));
            }
            let ace = &*ace.cast::<ACCESS_ALLOWED_ACE>();
            let flags = if directory {
                (OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE) as u8
            } else {
                INHERITED_ACE as u8
            };
            if (ace.Header.AceFlags != flags && (directory || ace.Header.AceFlags != 0))
                || ace.Mask != FILE_ALL_ACCESS
                || EqualSid((&ace.SidStart as *const u32).cast_mut().cast(), sid) == 0
            {
                return Err(invalid("host directory allow ACE is not private to this user"));
            }
        }
        Ok(())
    }

    pub(super) fn is_reparse(metadata: &std::fs::Metadata) -> bool {
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
}

#[cfg(not(any(unix, windows)))]
mod platform {
    use super::*;

    pub(super) fn private_directory(_: &Path) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "private host directories are unsupported",
        ))
    }

    pub(super) fn validate_directory(_: &Path) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "private host directories are unsupported",
        ))
    }

    pub(super) fn validate_file(_: &Path, _: &std::fs::Metadata) -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "private host files are unsupported"))
    }

    pub(super) fn is_reparse(_: &std::fs::Metadata) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_path_binds_installation_session_and_generation() {
        let install = "ab".repeat(32);
        let session = Uuid::from_u128(1);
        let path = profile_relative(&install, session, 7).expect("valid identity");
        assert_eq!(path, Path::new("profiles").join(&install).join(format!("{session}-7")));
        assert_ne!(path, profile_relative(&install, session, 8).expect("next generation"));
        assert_ne!(path, profile_relative(&install, Uuid::from_u128(2), 7).expect("other session"));
        assert_ne!(path, profile_relative(&"cd".repeat(32), session, 7).expect("other installation"));
        for install in ["", "..", "../profiles", &"AB".repeat(32), &"a".repeat(63)] {
            assert!(profile_relative(install, session, 7).is_err());
        }
        assert!(profile_relative(&install, Uuid::nil(), 7).is_err());
        assert!(profile_relative(&install, session, 0).is_err());
    }

    #[test]
    fn arbitrary_or_traversing_private_paths_are_rejected() {
        let root = Path::new("host");
        assert_eq!(
            relative_beneath(root, Path::new("host/channels/install")).expect("child"),
            Path::new("channels/install")
        );
        for path in [
            "global-profile",
            "hostile",
            "host/../global-profile",
            "host/channels/../../global-profile",
        ] {
            assert!(relative_beneath(root, Path::new(path)).is_err());
        }
    }

    fn temporary_root() -> PathBuf {
        let root = std::env::temp_dir().join(format!("aurora-host-profile-{}", Uuid::new_v4()));
        platform::private_directory(&root).expect("private test root");
        root
    }

    #[test]
    fn a_profile_can_only_be_opened_by_one_worker_at_a_time() {
        let root = temporary_root();
        let first = profile_lock(&root).expect("first worker");
        assert!(profile_lock(&root).is_err());
        drop(first);
        drop(profile_lock(&root).expect("lock released on worker exit"));
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[cfg(windows)]
    #[test]
    fn created_windows_directories_and_inherited_files_are_private() {
        let root = temporary_root();
        private_descendants(&root, Path::new("channel")).expect("protected private child");
        let path = root.join("channel/command.json");
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .expect("inherited private file");
        platform::validate_file(&path, &file.metadata().expect("file metadata")).expect("owner-only inherited file");
        drop(file);
        // A directory created by some other path must not be silently adopted
        // or have its access policy rewritten by host startup.
        let inherited = root.join("inherited");
        std::fs::create_dir(&inherited).expect("unprotected inherited directory");
        assert!(platform::private_directory(&inherited).is_err());
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[cfg(unix)]
    #[test]
    fn links_and_nonprivate_directories_are_rejected_without_writing_through_them() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let root = temporary_root();
        let outside = temporary_root();
        symlink(&outside, root.join("link")).expect("test link");
        assert!(private_descendants(&root, Path::new("link/child")).is_err());
        assert!(!outside.join("child").exists());
        let shared = root.join("shared");
        std::fs::create_dir(&shared).expect("nonprivate directory");
        std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o755)).expect("test permissions");
        assert!(private_descendants(&root, Path::new("shared/child")).is_err());
        assert!(!shared.join("child").exists());
        assert_eq!(
            std::fs::metadata(&shared).expect("unchanged directory").permissions().mode() & 0o777,
            0o755
        );
        std::fs::remove_dir_all(root).expect("cleanup root");
        std::fs::remove_dir_all(outside).expect("cleanup outside");
    }

    #[cfg(unix)]
    #[test]
    fn public_permissions_and_hard_links_do_not_become_private_files() {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let root = temporary_root();
        let path = root.join("command.json");
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&path)
            .expect("private file");
        assert!(platform::validate_file(&path, &file.metadata().expect("private metadata")).is_ok());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("public permissions");
        assert!(platform::validate_file(&path, &file.metadata().expect("public metadata")).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("restore permissions");
        std::fs::hard_link(&path, root.join("alias")).expect("hard link");
        assert!(platform::validate_file(&path, &file.metadata().expect("linked metadata")).is_err());
        drop(file);
        std::fs::remove_dir_all(root).expect("cleanup");
    }
}
