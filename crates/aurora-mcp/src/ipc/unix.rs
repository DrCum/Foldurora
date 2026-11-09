use super::*;
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt};
use std::os::unix::net::{UnixListener, UnixStream};

pub(super) fn plain_directory(path: &Path) -> io::Result<()> {
    let meta = std::fs::symlink_metadata(path)?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err(denied());
    }
    Ok(())
}
pub(super) fn private_directory(path: &Path) -> io::Result<()> {
    match std::fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    validate_directory(path)
}
pub(super) fn validate_directory(path: &Path) -> io::Result<()> {
    plain_directory(path)?;
    let meta = std::fs::symlink_metadata(path)?;
    // SAFETY: geteuid simply returns this process's effective user identity.
    if meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o777 != 0o700 {
        return Err(denied());
    }
    Ok(())
}
pub(super) fn validate_file(_: &Path, meta: &Metadata) -> io::Result<()> {
    // SAFETY: geteuid has no preconditions.
    if meta.file_type().is_symlink() || meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o077 != 0 || meta.nlink() != 1 {
        return Err(denied());
    }
    Ok(())
}
pub(super) fn same_object(a: &Metadata, b: &Metadata) -> bool {
    a.dev() == b.dev() && a.ino() == b.ino()
}
pub(super) type FileIdentity = (u64, u64);
pub(super) fn file_identity(file: &File) -> io::Result<FileIdentity> {
    let metadata = file.metadata()?;
    Ok((metadata.dev(), metadata.ino()))
}
pub(super) fn path_identity(path: &Path) -> io::Result<FileIdentity> {
    file_identity(&open_read(path)?)
}
pub(super) fn open_read(path: &Path) -> io::Result<File> {
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
}
pub(super) fn create_private_file(path: &Path) -> io::Result<File> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
}
fn directory(root: &Path) -> io::Result<File> {
    validate_directory(root)?;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(root)?;
    if !same_object(&file.metadata()?, &std::fs::symlink_metadata(root)?) {
        return Err(denied());
    }
    Ok(file)
}
fn endpoint(_root: &Path, directory: &File, instance: HostId) -> PathBuf {
    // Linux's fd-relative alias avoids sockaddr_un path truncation without
    // changing cwd; the socket itself remains under the verified namespace.
    #[cfg(target_os = "linux")]
    {
        PathBuf::from(format!("/proc/self/fd/{}/{}.sock", directory.as_raw_fd(), instance))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = directory;
        _root.join(format!("{instance}.sock"))
    }
}
fn peer(fd: RawFd) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        // SAFETY: zero is a valid initial value for this plain C output record.
        let mut creds: libc::ucred = unsafe { std::mem::zeroed() };
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        // SAFETY: initialized output and correct socklen describe the live socket.
        if unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&mut creds as *mut libc::ucred).cast(),
                &mut len,
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: geteuid has no preconditions.
        if len as usize != std::mem::size_of::<libc::ucred>() || creds.uid != unsafe { libc::geteuid() } {
            return Err(denied());
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let mut uid = 0;
        let mut gid = 0;
        // SAFETY: fd is a live connected Unix socket and both outputs are writable.
        if unsafe { libc::getpeereid(fd, &mut uid, &mut gid) } != 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: geteuid has no preconditions.
        if uid != unsafe { libc::geteuid() } {
            return Err(denied());
        }
    }
    Ok(())
}

pub(super) struct Stream(UnixStream);
impl Stream {
    pub(super) fn connect(root: &Path, instance: HostId) -> io::Result<Self> {
        let directory = directory(root)?;
        let actual = root.join(format!("{instance}.sock"));
        let before = std::fs::symlink_metadata(&actual)?;
        validate_file(&actual, &before)?;
        if !before.file_type().is_socket() {
            return Err(denied());
        }
        let stream = UnixStream::connect(endpoint(root, &directory, instance))?;
        peer(stream.as_raw_fd())?;
        if !same_object(&before, &std::fs::symlink_metadata(actual)?) {
            return Err(denied());
        }
        Ok(Self(stream))
    }
}
impl Read for Stream {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.0.read(out)
    }
}
impl Write for Stream {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.0.write(data)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}
impl TimedStream for Stream {
    fn timeout(&mut self, timeout: Duration) -> io::Result<()> {
        self.0.set_read_timeout(Some(timeout))?;
        self.0.set_write_timeout(Some(timeout))
    }
}
pub(super) struct Cancel(UnixStream);
impl Cancel {
    pub(super) fn cancel(&self) {
        let _ = self.0.shutdown(std::net::Shutdown::Both);
    }
}
pub(super) struct Listener {
    listener: UnixListener,
    root: PathBuf,
    socket: PathBuf,
    identity: Metadata,
    _directory: File,
}
impl Listener {
    pub(super) fn bind(root: &Path, instance: HostId) -> io::Result<Self> {
        let directory = directory(root)?;
        let socket = root.join(format!("{instance}.sock"));
        let listener = UnixListener::bind(endpoint(root, &directory, instance))?;
        // A Unix socket inherits umask, so tighten only the newly created object.
        // Existing paths are never repaired; bind fails if the name was occupied.
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
        let identity = std::fs::symlink_metadata(&socket)?;
        validate_file(&socket, &identity)?;
        listener.set_nonblocking(true)?;
        Ok(Self {
            listener,
            root: root.to_owned(),
            socket,
            identity,
            _directory: directory,
        })
    }
    pub(super) fn accept(&mut self) -> io::Result<(Stream, Cancel)> {
        validate_directory(&self.root)?;
        if !same_object(&self.identity, &std::fs::symlink_metadata(&self.socket)?) {
            return Err(denied());
        }
        let (stream, _) = self.listener.accept()?;
        peer(stream.as_raw_fd())?;
        stream.set_nonblocking(false)?;
        let cancel = Cancel(stream.try_clone()?);
        Ok((Stream(stream), cancel))
    }
}
impl Drop for Listener {
    fn drop(&mut self) {
        if std::fs::symlink_metadata(&self.socket).is_ok_and(|meta| same_object(&meta, &self.identity)) {
            let _ = std::fs::remove_file(&self.socket);
        }
    }
}
