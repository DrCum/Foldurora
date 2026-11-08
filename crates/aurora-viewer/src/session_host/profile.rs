//! Explicit production profiles for host workers. Selection happens before
//! settings, cache initialization or any other worker profile writer starts.
//! The shared account catalogue contains metadata; settings and account files
//! belong to an explicit persistent slot or a legacy session generation and
//! are never copied from another profile.

use sha2::{Digest, Sha256};
use std::fs::File;
#[cfg(any(not(windows), test))]
use std::fs::OpenOptions;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;
use uuid::Uuid;

pub(super) mod settings_file;

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
pub(super) fn activate(
    installation: &str,
    session: Uuid,
    generation: u64,
    slot: Option<super::slots::SlotId>,
    controller: aurora_session::HostId,
) -> anyhow::Result<()> {
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
    let root = match slot {
        Some(slot) => slot_root(installation, slot)?,
        None => host_root()?.join(relative),
    };
    private_dir(&root)?;
    private_dir(&root.join("config"))?;
    private_dir(&root.join("cache"))?;
    let lock = profile_lock(&root)?;
    if slot.is_some() {
        // The new controller publishes its stamp before probing any slot.
        // Check after taking the lifetime profile lock, before ACTIVE exposes
        // paths to settings/cache initialization. A stale child cannot race
        // a replacement controller into two writers.
        super::slots::validate_controller(installation, controller)?;
    }
    ACTIVE
        .set(Active { root, _lock: lock })
        .map_err(|_| anyhow::anyhow!("host worker profile was already selected"))
}

pub(super) fn slot_root(installation: &str, slot: super::slots::SlotId) -> io::Result<PathBuf> {
    Ok(slots_root(installation)?.join(slot.number().to_string()))
}

pub(super) fn slots_root(installation: &str) -> io::Result<PathBuf> {
    if !crate::characters::valid_id(installation) {
        return Err(invalid("invalid persistent profile installation"));
    }
    Ok(host_root()?.join("profiles").join(installation).join("slots"))
}

pub(super) fn slot_available(installation: &str, slot: super::slots::SlotId) -> io::Result<bool> {
    let root = slot_root(installation, slot)?;
    private_dir(&root)?;
    Ok(try_private_lock(&root.join("profile.lock"))?.is_some())
}

pub(super) fn try_private_lock(path: &Path) -> io::Result<Option<File>> {
    let file = open_private_file(path, false)?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(error)) => Err(error),
    }
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

pub(super) fn profile_lock(root: &Path) -> io::Result<File> {
    let path = root.join("profile.lock");
    if let Ok(metadata) = std::fs::symlink_metadata(&path)
        && (!metadata.is_file() || metadata.file_type().is_symlink() || platform::is_reparse(&metadata))
    {
        return Err(invalid("host profile lock is not a plain file"));
    }
    lock_private(&path)
}

pub(super) fn lock_private(path: &Path) -> io::Result<File> {
    try_private_lock(path)?.ok_or_else(|| io::Error::new(io::ErrorKind::WouldBlock, "private host file is locked"))
}

#[cfg(test)]
pub(super) fn open_existing_private(path: &Path) -> io::Result<File> {
    open_private_file(path, false)
}

/// The parent must already be private. Ownership is assigned at creation;
/// Windows' default TokenOwner can be a group even with a user-only inherited ACL.
pub(super) fn create_private_file(path: &Path) -> io::Result<File> {
    open_private_file(path, true)
}

fn open_private_file(path: &Path, create_new: bool) -> io::Result<File> {
    #[cfg(windows)]
    let file = platform::open_private_file(path, create_new)?;
    #[cfg(not(windows))]
    let file = {
        let mut options = OpenOptions::new();
        options
            .read(!create_new)
            .write(true)
            .create(!create_new)
            .create_new(create_new)
            .truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        options.open(path)?
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() || platform::is_reparse(&metadata) {
        return Err(invalid("host file is not a plain file"));
    }
    #[cfg(windows)]
    validate_opened_single_link(&file)?;
    platform::validate_file(path, &metadata)?;
    Ok(file)
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}

/// The ACL belongs to the file, so another hardlink can have the same private
/// owner/DACL while bypassing the selected profile's namespace. Query the
/// already opened handle, not a second path lookup.
#[cfg(windows)]
fn validate_opened_single_link(file: &File) -> io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle};
    let mut info = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
    // SAFETY: File owns a live handle; the writable, aligned output has the
    // exact Win32 ABI size and is read only after a successful query.
    if unsafe { GetFileInformationByHandle(file.as_raw_handle().cast(), info.as_mut_ptr()) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the successful API call initialized this output structure.
    if unsafe { info.assume_init() }.nNumberOfLinks != 1 {
        return Err(invalid("private file has multiple hard links"));
    }
    Ok(())
}

/// A closed native-store catalogue. Kinds are selected by native code, never by
/// a file/DTO; paths and hard byte limits cannot be supplied by a consumer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NativeStoreKind {
    Workspaces,
    Boards,
    #[cfg(test)]
    InventoryArchive,
}
impl NativeStoreKind {
    fn directory(self) -> &'static str {
        match self {
            Self::Workspaces => "named-workspaces",
            Self::Boards => "gesture-boards",
            #[cfg(test)]
            Self::InventoryArchive => "inventory-archive",
        }
    }
    fn filename(self) -> &'static str {
        match self {
            Self::Workspaces => "named-v1.json",
            Self::Boards => "boards-v1.json",
            #[cfg(test)]
            Self::InventoryArchive => "archive-v1.json",
        }
    }
    pub(crate) const fn max_bytes(self) -> usize {
        match self {
            Self::Workspaces => 256 * 1024,
            Self::Boards => 4 * 1024 * 1024,
            #[cfg(test)]
            Self::InventoryArchive => 128 * 1024,
        }
    }
}
/// Logical cancellation metadata only, never permission or a path selector.
/// A rename racing after its final check can still finish at the original path;
/// callers must retain their exact source/operation reply fences.
#[derive(Clone, Default)]
pub(crate) struct NativeStoreCancellation(std::sync::Arc<std::sync::atomic::AtomicBool>);
impl NativeStoreCancellation {
    pub(crate) fn new() -> Self {
        Self::default()
    }
    pub(crate) fn cancel(&self) {
        self.0.store(true, std::sync::atomic::Ordering::Release);
    }
    pub(crate) fn is_cancelled(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Acquire)
    }
    fn check(&self) -> io::Result<()> {
        if self.is_cancelled() {
            Err(io::Error::new(io::ErrorKind::Interrupted, "native store operation cancelled"))
        } else {
            Ok(())
        }
    }
}

/// Uses the already selected character profile, including standalone viewers.
/// No DTO supplies a directory and no catalogue is copied.
pub(crate) struct NativeDirectory {
    kind: NativeStoreKind,
    path: PathBuf,
    account: Uuid,
    grid: String,
}
impl NativeDirectory {
    pub(crate) fn select(kind: NativeStoreKind, account: Uuid, grid: &str) -> io::Result<Self> {
        if account.is_nil() || !crate::characters::valid_id(grid) {
            return Err(invalid("invalid native store identity"));
        }
        let config = crate::settings::config_dir();
        if !config.is_absolute() {
            return Err(invalid("native store profile must be absolute"));
        }
        let root = config.join(kind.directory());
        let relative = PathBuf::from(account.to_string()).join(grid);
        Ok(Self {
            kind,
            path: root.join(relative),
            account,
            grid: grid.to_owned(),
        })
    }
    pub(crate) fn matches(&self, kind: NativeStoreKind, account: Uuid, grid: &str) -> bool {
        self.kind == kind && self.account == account && self.grid == grid
    }
    fn checked(&self) -> io::Result<()> {
        let grid = self.path.parent().ok_or_else(|| invalid("invalid native store parent"))?;
        let root = grid.parent().ok_or_else(|| invalid("invalid native store root"))?;
        ordinary_directories(root.parent().ok_or_else(|| invalid("invalid config root"))?)?;
        for p in [root, grid, self.path.as_path()] {
            plain_directory(p)?;
            platform::validate_directory(p)?;
        }
        Ok(())
    }
    pub(crate) fn lock(&self) -> io::Result<File> {
        let account = self.path.parent().ok_or_else(|| invalid("invalid native store parent"))?;
        let root = account.parent().ok_or_else(|| invalid("invalid native store root"))?;
        ordinary_directories(root.parent().ok_or_else(|| invalid("invalid config root"))?)?;
        platform::private_directory(root)?;
        platform::private_directory(account)?;
        platform::private_directory(&self.path)?;
        self.checked()?;
        lock_private(&self.path.join("store.lock"))
    }
    pub(crate) fn read(&self) -> io::Result<Option<Vec<u8>>> {
        use std::io::Read;
        let limit = self.kind.max_bytes();
        self.checked()?;
        let path = self.path.join(self.kind.filename());
        match std::fs::symlink_metadata(&path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
            Ok(m) if !m.is_file() || m.file_type().is_symlink() || platform::is_reparse(&m) => {
                return Err(invalid("native store file is not plain"));
            }
            Ok(_) => {}
        }
        let mut o = std::fs::OpenOptions::new();
        o.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            o.custom_flags(libc::O_NOFOLLOW);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            o.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
        }
        let f = o.open(&path)?;
        let m = f.metadata()?;
        if !m.is_file() || platform::is_reparse(&m) {
            return Err(invalid("native store file is not plain"));
        }
        #[cfg(windows)]
        validate_opened_single_link(&f)?;
        platform::validate_file(&path, &m)?;
        let mut b = Vec::new();
        f.take((limit + 1) as u64).read_to_end(&mut b)?;
        if b.len() > limit {
            return Err(invalid("native store file too large"));
        }
        Ok(Some(b))
    }
    /// Caller holds store.lock and has checked the original file revision.
    pub(crate) fn publish(&self, bytes: &[u8], expected: Option<&[u8]>) -> io::Result<()> {
        self.publish_staged(bytes, expected, None, || {})
    }
    pub(crate) fn publish_cancellable(
        &self,
        bytes: &[u8],
        expected: Option<&[u8]>,
        cancellation: &NativeStoreCancellation,
    ) -> io::Result<()> {
        self.publish_staged(bytes, expected, Some(cancellation), || {})
    }
    fn publish_staged(
        &self,
        bytes: &[u8],
        expected: Option<&[u8]>,
        cancellation: Option<&NativeStoreCancellation>,
        before_publish: impl FnOnce(),
    ) -> io::Result<()> {
        use std::io::Write;
        if let Some(c) = cancellation {
            c.check()?;
        }
        if bytes.len() > self.kind.max_bytes() || expected.is_some_and(|old| old.len() > self.kind.max_bytes()) {
            return Err(invalid("native store file too large"));
        }
        self.checked()?;
        let temp = self.path.join(format!(".native-store-{}.tmp", Uuid::new_v4()));
        let result = (|| {
            let mut f = create_private_file(&temp)?;
            f.write_all(bytes)?;
            f.sync_all()?;
            #[cfg(windows)]
            validate_opened_single_link(&f)?;
            drop(f);
            before_publish();
            if let Some(c) = cancellation {
                c.check()?;
            }
            // The caller's cooperative store lock covers read/encode/rename;
            // also detect a non-cooperating replacement during the disk flush.
            if self.read()?.as_deref() != expected {
                return Err(invalid("native store file changed before publication"));
            }
            if let Some(c) = cancellation {
                c.check()?;
            }
            crate::atomic_file::rename_retrying(&temp, &self.path.join(self.kind.filename()))?;
            crate::atomic_file::sync_dir(&self.path);
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(temp);
        }
        result
    }
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
    use std::os::windows::io::FromRawHandle;
    use std::ptr::null_mut;
    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Security::*;
    use windows_sys::Win32::Storage::FileSystem::{
        CREATE_NEW, CreateDirectoryW, CreateFileW, FILE_ALL_ACCESS, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_ALWAYS,
    };
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

    pub(super) fn check(success: windows_sys::core::BOOL) -> io::Result<()> {
        if success == 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
    }

    /// Aligned storage for variable-sized Windows security data. The queried
    /// size is bounded before allocating, and every pointer stays in its owner.
    pub(super) fn storage(bytes: u32) -> io::Result<Vec<usize>> {
        if !(1..=65_536).contains(&bytes) {
            return Err(invalid("invalid Windows security descriptor size"));
        }
        Ok(vec![0; (bytes as usize).div_ceil(size_of::<usize>())])
    }

    fn token_information(kind: TOKEN_INFORMATION_CLASS) -> io::Result<Vec<usize>> {
        let mut handle = null_mut();
        // SAFETY: output is valid and TOKEN_QUERY only reads this process's identity.
        check(unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut handle) })?;
        let token = Token(handle);
        let mut bytes = 0;
        // SAFETY: a null buffer with zero size queries the required byte length.
        unsafe { GetTokenInformation(token.0, kind, null_mut(), 0, &mut bytes) };
        let mut data = storage(bytes)?;
        // SAFETY: aligned data has at least the queried capacity, and bytes is writable.
        check(unsafe { GetTokenInformation(token.0, kind, data.as_mut_ptr().cast(), bytes, &mut bytes) })?;
        Ok(data)
    }

    pub(super) fn current_user() -> io::Result<Vec<usize>> {
        token_information(TokenUser)
    }

    fn with_private_security<T>(
        path: &Path,
        directory: bool,
        create: impl FnOnce(*const u16, &SECURITY_ATTRIBUTES, PSID) -> io::Result<T>,
    ) -> io::Result<T> {
        let name: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        if name[..name.len() - 1].contains(&0) {
            return Err(invalid("host path contains a nul"));
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
        // the only allow ACE names this process's user; directories pass it on.
        unsafe {
            check(InitializeAcl(acl, acl_size, ACL_REVISION))?;
            check(AddAccessAllowedAceEx(
                acl,
                ACL_REVISION,
                if directory { OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE } else { 0 },
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
        create(name.as_ptr(), &attributes, sid)
    }

    pub(super) fn private_directory(path: &Path) -> io::Result<()> {
        with_private_security(path, true, |name, attributes, sid| {
            // SAFETY: the nul-terminated name and descriptor/ACL/SID outlive the call.
            if unsafe { CreateDirectoryW(name, attributes) } == 0 {
                let error = io::Error::last_os_error();
                if error.raw_os_error() != Some(ERROR_ALREADY_EXISTS as i32) {
                    return Err(error);
                }
            }
            plain_directory(path)?;
            verify_security(name, sid, true)
        })
    }

    pub(super) fn open_private_file(path: &Path, create_new: bool) -> io::Result<File> {
        with_private_security(path, false, |name, attributes, _| {
            // SECURITY_ATTRIBUTES gives NEW files the TokenUser owner, independent
            // of TokenOwner. OPEN_ALWAYS ignores it for an existing lock; the
            // caller must still reject that file's existing owner/ACL/reparse state.
            // SAFETY: all inputs outlive the call. Sharing/access match OpenOptions;
            // no truncation or following an existing reparse point is allowed.
            let handle = unsafe {
                CreateFileW(
                    name,
                    if create_new { GENERIC_WRITE } else { GENERIC_READ | GENERIC_WRITE },
                    FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                    attributes,
                    if create_new { CREATE_NEW } else { OPEN_ALWAYS },
                    FILE_FLAG_OPEN_REPARSE_POINT,
                    null_mut(),
                )
            };
            if handle == INVALID_HANDLE_VALUE {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: CreateFileW returned a valid, uniquely owned, non-inheritable
            // file handle, transferred once into File so it closes on every exit.
            Ok(unsafe { File::from_raw_handle(handle) })
        })
    }

    #[cfg(test)]
    pub(super) fn default_owner_is_current_user() -> io::Result<bool> {
        let user = current_user()?;
        let owner = token_information(TokenOwner)?;
        // SAFETY: successful token queries initialized both aligned structures;
        // the SIDs remain within their respective live backing allocations.
        Ok(unsafe {
            EqualSid(
                (*user.as_ptr().cast::<TOKEN_USER>()).User.Sid,
                (*owner.as_ptr().cast::<TOKEN_OWNER>()).Owner,
            ) != 0
        })
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
            if owner.is_null() || EqualSid(owner, sid) == 0 {
                return Err(invalid("host path owner does not match this user"));
            }
            if present == 0 || acl.is_null() {
                return Err(invalid("host path private DACL is missing"));
            }
            if directory && control & SE_DACL_PROTECTED == 0 {
                return Err(invalid("host directory DACL is not protected"));
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
    fn native_store_kinds_have_fixed_disjoint_profile_paths_and_captured_identity() {
        let account = Uuid::new_v4();
        let grid = "b".repeat(64);
        let workspaces = NativeDirectory::select(NativeStoreKind::Workspaces, account, &grid).unwrap();
        let boards = NativeDirectory::select(NativeStoreKind::Boards, account, &grid).unwrap();
        assert!(workspaces.matches(NativeStoreKind::Workspaces, account, &grid));
        assert!(!workspaces.matches(NativeStoreKind::Boards, account, &grid));
        assert!(boards.matches(NativeStoreKind::Boards, account, &grid));
        assert_ne!(workspaces.path, boards.path);
        assert!(
            workspaces
                .path
                .ends_with(Path::new("named-workspaces").join(account.to_string()).join(&grid))
        );
        assert!(
            boards
                .path
                .ends_with(Path::new("gesture-boards").join(account.to_string()).join(&grid))
        );
        let _a = workspaces.lock().unwrap();
        let _b = boards.lock().unwrap();
        workspaces.publish(b"workspaces", None).unwrap();
        boards.publish(b"boards", None).unwrap();
        assert_eq!(workspaces.read().unwrap().unwrap(), b"workspaces");
        assert_eq!(boards.read().unwrap().unwrap(), b"boards");
        assert!(workspaces.path.join("named-v1.json").is_file());
        assert!(boards.path.join("boards-v1.json").is_file());
    }
    #[test]
    fn native_store_cancellation_before_io_or_after_sync_never_replaces_original_bytes() {
        let d = NativeDirectory::select(NativeStoreKind::Boards, Uuid::new_v4(), &"c".repeat(64)).unwrap();
        let _lock = d.lock().unwrap();
        d.publish(b"original", None).unwrap();
        let cancelled = NativeStoreCancellation::new();
        cancelled.cancel();
        assert_eq!(
            d.publish_cancellable(b"replacement", Some(b"original"), &cancelled)
                .unwrap_err()
                .kind(),
            io::ErrorKind::Interrupted
        );
        let cancellation = NativeStoreCancellation::new();
        let during_flush = cancellation.clone();
        assert_eq!(
            d.publish_staged(b"replacement", Some(b"original"), Some(&cancellation), move || during_flush
                .cancel())
                .unwrap_err()
                .kind(),
            io::ErrorKind::Interrupted
        );
        assert_eq!(d.read().unwrap().unwrap(), b"original");
        assert!(
            std::fs::read_dir(&d.path)
                .unwrap()
                .all(|e| !e.unwrap().file_name().to_string_lossy().starts_with(".native-store-"))
        );
        let current = NativeStoreCancellation::new();
        d.publish_cancellable(b"new", Some(b"original"), &current).unwrap();
        assert_eq!(d.read().unwrap().unwrap(), b"new");
    }
    #[test]
    fn native_store_kind_limits_reject_oversized_reads_and_publication_without_writeback() {
        let account = Uuid::new_v4();
        let grid = "d".repeat(64);
        let small = NativeDirectory::select(NativeStoreKind::Workspaces, account, &grid).unwrap();
        let _lock = small.lock().unwrap();
        small.publish(b"original", None).unwrap();
        assert!(
            small
                .publish(&vec![0; NativeStoreKind::Workspaces.max_bytes() + 1], Some(b"original"))
                .is_err()
        );
        assert_eq!(small.read().unwrap().unwrap(), b"original");
        let large = NativeDirectory::select(NativeStoreKind::Boards, account, &grid).unwrap();
        let _lock = large.lock().unwrap();
        let path = large.path.join("boards-v1.json");
        let mut file = create_private_file(&path).unwrap();
        file.write_all(&vec![0; NativeStoreKind::Boards.max_bytes() + 1]).unwrap();
        file.sync_all().unwrap();
        drop(file);
        assert!(large.read().is_err());
        assert!(large.publish(b"replacement", None).is_err());
        assert_eq!(
            std::fs::metadata(path).unwrap().len(),
            (NativeStoreKind::Boards.max_bytes() + 1) as u64
        );
    }
    #[cfg(windows)]
    #[test]
    fn native_workspace_windows_hardlinks_to_private_data_lock_and_temporary_handles_are_rejected() {
        let d = NativeDirectory::select(NativeStoreKind::Workspaces, Uuid::new_v4(), &"c".repeat(64)).unwrap();
        let lock = d.lock().unwrap();
        d.publish(b"original", None).unwrap();
        drop(lock);
        let path = d.path.join("named-v1.json");
        let alias = d.path.join("data-alias");
        std::fs::hard_link(&path, &alias).unwrap();
        assert!(d.read().is_err(), "matching private ACL does not make a hardlink safe");
        assert!(d.publish(b"replacement", Some(b"original")).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
        assert_eq!(std::fs::read(&alias).unwrap(), b"original");
        std::fs::remove_file(alias).unwrap();
        assert_eq!(d.read().unwrap().unwrap(), b"original");
        let lock_alias = d.path.join("lock-alias");
        std::fs::hard_link(d.path.join("store.lock"), &lock_alias).unwrap();
        assert!(d.lock().is_err());
        std::fs::remove_file(lock_alias).unwrap();
        let temporary = d.path.join("temporary");
        let f = create_private_file(&temporary).unwrap();
        let temp_alias = d.path.join("temporary-alias");
        std::fs::hard_link(&temporary, &temp_alias).unwrap();
        assert!(
            validate_opened_single_link(&f).is_err(),
            "publication rechecks the same opened temp handle after sync"
        );
        drop(f);
        std::fs::remove_file(temp_alias).unwrap();
        std::fs::remove_file(temporary).unwrap();
        drop(d.lock().unwrap());
    }
    #[test]
    fn native_workspace_private_store_is_exclusive_and_never_repairs_a_link_or_public_file() {
        let d = NativeDirectory::select(NativeStoreKind::Workspaces, Uuid::new_v4(), &"a".repeat(64)).unwrap();
        let lock = d.lock().unwrap();
        assert!(d.lock().is_err());
        d.publish(b"original", None).unwrap();
        assert_eq!(d.read().unwrap().unwrap(), b"original");
        drop(lock);
        #[cfg(unix)]
        {
            use std::os::unix::fs::{PermissionsExt, symlink};
            let path = d.path.join("named-v1.json");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
            assert!(d.read().is_err());
            assert_eq!(std::fs::read(&path).unwrap(), b"original");
            std::fs::remove_file(&path).unwrap();
            symlink(d.path.join("store.lock"), &path).unwrap();
            assert!(d.read().is_err());
            std::fs::remove_file(path).unwrap();
        }
    }
    use std::io::Write;

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

    #[test]
    fn private_file_creation_is_exclusive_and_existing_files_are_not_truncated() {
        let root = temporary_root();
        let path = root.join("command.json");
        let mut file = create_private_file(&path).expect("private file creation");
        file.write_all(b"retained contents").expect("write private file");
        drop(file);
        assert_eq!(
            create_private_file(&path).expect_err("existing file is never replaced").kind(),
            io::ErrorKind::AlreadyExists
        );
        drop(open_private_file(&path, false).expect("open without changing owner or permissions"));
        assert_eq!(std::fs::read(&path).expect("unchanged contents"), b"retained contents");
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[cfg(windows)]
    #[test]
    fn created_windows_directories_and_explicitly_owned_files_are_private() {
        let root = temporary_root();
        private_descendants(&root, Path::new("channel")).expect("protected private child");
        let path = root.join("channel/command.json");
        let file = create_private_file(&path).expect("explicitly owned private file");
        platform::validate_file(&path, &file.metadata().expect("file metadata")).expect("owner-only file");
        drop(file);
        // A directory created by some other path must not be silently adopted
        // or have its access policy rewritten by host startup.
        let inherited = root.join("inherited");
        std::fs::create_dir(&inherited).expect("unprotected inherited directory");
        assert!(platform::private_directory(&inherited).is_err());
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[cfg(windows)]
    #[test]
    fn inherited_windows_file_ownership_is_checked_without_repairing_existing_files() {
        let root = temporary_root();
        let path = root.join("default-owner.json");
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .expect("default-owner file");
        file.write_all(b"retained contents").expect("test file contents");
        let metadata = file.metadata().expect("file metadata");
        drop(file);
        // Elevated tokens can default new objects to a group owner. Inheriting
        // the user-only parent DACL must never bypass the independent owner check.
        let same_owner = platform::default_owner_is_current_user().expect("default owner identity");
        let validation = platform::validate_file(&path, &metadata);
        let reopened = open_private_file(&path, false);
        if same_owner {
            validation.expect("same-user inherited file remains valid");
            drop(reopened.expect("existing private file"));
        } else {
            for error in [
                validation.expect_err("group owner is rejected"),
                reopened.expect_err("existing owner is not repaired"),
            ] {
                assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
                assert_eq!(error.to_string(), "host path owner does not match this user");
            }
            assert!(
                platform::validate_file(&path, &metadata).is_err(),
                "existing owner remains unchanged"
            );
        }
        assert_eq!(std::fs::read(&path).expect("unchanged file contents"), b"retained contents");
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
