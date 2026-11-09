//! Current-user protected named pipes. Native runtime acceptance remains separate.
use super::*;
use std::ffi::c_void;
use std::mem::size_of;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::MetadataExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::ptr::null_mut;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Security::*;
use windows_sys::Win32::Storage::FileSystem::*;
use windows_sys::Win32::System::IO::*;
use windows_sys::Win32::System::Pipes::*;
use windows_sys::Win32::System::Threading::*;

#[path = "windows_path.rs"]
mod windows_path;

pub(super) fn check(ok: windows_sys::core::BOOL) -> io::Result<()> {
    if ok == 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
}
struct Handle(HANDLE);
// SAFETY: handles are owned kernel objects. All concurrent use is limited to
// documented CancelIoEx and independent overlapped operations; closure is owned.
unsafe impl Send for Handle {}
// SAFETY: shared access never closes the handle, and cancellation is thread-safe.
unsafe impl Sync for Handle {}
impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: Handle uniquely owns this valid kernel handle.
        unsafe {
            CloseHandle(self.0);
        }
    }
}

fn storage(bytes: u32) -> io::Result<Vec<usize>> {
    if !(1..=65_536).contains(&bytes) {
        return Err(denied());
    }
    Ok(vec![0; (bytes as usize).div_ceil(size_of::<usize>())])
}
fn user_for(process: HANDLE) -> io::Result<Vec<usize>> {
    let mut token = null_mut();
    // SAFETY: process is live, output valid, and query access is read-only.
    check(unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) })?;
    let token = Handle(token);
    let mut bytes = 0;
    // SAFETY: null/zero queries the required aligned token information length.
    unsafe {
        GetTokenInformation(token.0, TokenUser, null_mut(), 0, &mut bytes);
    }
    let mut out = storage(bytes)?;
    // SAFETY: out is aligned and has the queried length.
    check(unsafe { GetTokenInformation(token.0, TokenUser, out.as_mut_ptr().cast(), bytes, &mut bytes) })?;
    Ok(out)
}
fn current_user() -> io::Result<Vec<usize>> {
    // SAFETY: GetCurrentProcess returns a borrowed pseudo-handle.
    user_for(unsafe { GetCurrentProcess() })
}
fn sid(user: &mut [usize]) -> PSID {
    // SAFETY: callers pass successfully initialized aligned TokenUser storage.
    unsafe { (*user.as_mut_ptr().cast::<TOKEN_USER>()).User.Sid }
}
fn private_security<T>(directory: bool, create: impl FnOnce(&SECURITY_ATTRIBUTES) -> io::Result<T>) -> io::Result<T> {
    let mut user = current_user()?;
    let sid = sid(&mut user);
    // SAFETY: SID points into the live queried TokenUser storage.
    let sid_bytes = unsafe { GetLengthSid(sid) };
    let bytes = (size_of::<ACL>() + size_of::<ACCESS_ALLOWED_ACE>() - size_of::<u32>()) as u32 + sid_bytes;
    let mut acl = storage(bytes)?;
    let mut descriptor = SECURITY_DESCRIPTOR::default();
    let sd = (&mut descriptor as *mut SECURITY_DESCRIPTOR).cast::<c_void>();
    // SAFETY: every pointer is aligned, valid and sized for the invoked API;
    // only the current TokenUser SID receives access; the DACL is protected.
    unsafe {
        check(InitializeAcl(acl.as_mut_ptr().cast(), bytes, ACL_REVISION))?;
        check(AddAccessAllowedAceEx(
            acl.as_mut_ptr().cast(),
            ACL_REVISION,
            if directory { OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE } else { 0 },
            FILE_ALL_ACCESS,
            sid,
        ))?;
        check(InitializeSecurityDescriptor(sd, 1))?;
        check(SetSecurityDescriptorOwner(sd, sid, 0))?;
        check(SetSecurityDescriptorDacl(sd, 1, acl.as_mut_ptr().cast(), 0))?;
        check(SetSecurityDescriptorControl(sd, SE_DACL_PROTECTED, SE_DACL_PROTECTED))?;
    }
    create(&SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd,
        bInheritHandle: 0,
    })
}
fn native(path: &Path) -> io::Result<Vec<u16>> {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir | Component::CurDir)) {
        return Err(denied());
    }
    let absolute = std::path::absolute(path)?;
    let units: Vec<u16> = absolute.as_os_str().encode_wide().collect();
    windows_path::encode_absolute(&units).ok_or_else(denied)
}
pub(super) fn plain_directory(path: &Path) -> io::Result<()> {
    let meta = std::fs::symlink_metadata(path)?;
    if !meta.is_dir() || meta.file_type().is_symlink() || meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(denied());
    }
    Ok(())
}
pub(super) fn private_directory(path: &Path) -> io::Result<()> {
    let name = native(path)?;
    private_security(true, |attributes| {
        // SAFETY: name/attributes remain live through CreateDirectoryW.
        if unsafe { CreateDirectoryW(name.as_ptr(), attributes) } == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(ERROR_ALREADY_EXISTS as i32) {
                return Err(error);
            }
        }
        validate_directory(path)
    })
}
fn validate_security(path: &Path, directory: bool) -> io::Result<()> {
    let name = native(path)?;
    let mut user = current_user()?;
    let user_sid = sid(&mut user);
    let wanted = OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION;
    let mut bytes = 0;
    // SAFETY: null/zero queries size; name remains nul-terminated and live.
    unsafe {
        GetFileSecurityW(name.as_ptr(), wanted, null_mut(), 0, &mut bytes);
    }
    let mut descriptor = storage(bytes)?;
    let sd = descriptor.as_mut_ptr().cast::<c_void>();
    let mut owner = null_mut();
    let mut defaulted = 0;
    let mut present = 0;
    let mut acl = null_mut();
    let mut control = 0;
    let mut revision = 0;
    // SAFETY: APIs fill initialized pointers within descriptor's live allocation;
    // subsequent ACE/SID access remains inside that validated descriptor.
    unsafe {
        check(GetFileSecurityW(name.as_ptr(), wanted, sd, bytes, &mut bytes))?;
        check(GetSecurityDescriptorOwner(sd, &mut owner, &mut defaulted))?;
        check(GetSecurityDescriptorDacl(sd, &mut present, &mut acl, &mut defaulted))?;
        check(GetSecurityDescriptorControl(sd, &mut control, &mut revision))?;
        if owner.is_null()
            || EqualSid(owner, user_sid) == 0
            || present == 0
            || acl.is_null()
            || (directory && control & SE_DACL_PROTECTED == 0)
        {
            return Err(denied());
        }
        let mut information = ACL_SIZE_INFORMATION::default();
        check(GetAclInformation(
            acl,
            (&mut information as *mut ACL_SIZE_INFORMATION).cast(),
            size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        ))?;
        if information.AceCount != 1 {
            return Err(denied());
        }
        let mut ace = null_mut();
        check(GetAce(acl, 0, &mut ace))?;
        if ace.is_null() {
            return Err(denied());
        }
        let header = &*ace.cast::<ACE_HEADER>();
        if header.AceType != 0 || usize::from(header.AceSize) < size_of::<ACCESS_ALLOWED_ACE>() {
            return Err(denied());
        }
        let ace = &*ace.cast::<ACCESS_ALLOWED_ACE>();
        let flags = if directory {
            (OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE) as u8
        } else {
            INHERITED_ACE as u8
        };
        if (ace.Header.AceFlags != flags && (directory || ace.Header.AceFlags != 0))
            || ace.Mask != FILE_ALL_ACCESS
            || EqualSid((&ace.SidStart as *const u32).cast_mut().cast(), user_sid) == 0
        {
            return Err(denied());
        }
    }
    Ok(())
}
pub(super) fn validate_directory(path: &Path) -> io::Result<()> {
    plain_directory(path)?;
    validate_security(path, true)
}
pub(super) fn validate_file(path: &Path, meta: &Metadata) -> io::Result<()> {
    if !meta.is_file() || meta.file_type().is_symlink() || meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(denied());
    }
    validate_security(path, false)
}
pub(super) fn same_object(a: &Metadata, b: &Metadata) -> bool {
    a.creation_time() == b.creation_time()
        && a.last_write_time() == b.last_write_time()
        && a.file_size() == b.file_size()
        && a.file_attributes() == b.file_attributes()
}
pub(super) type FileIdentity = (u32, u64);
pub(super) fn file_identity(file: &File) -> io::Result<FileIdentity> {
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: the live file owns the handle and info is writable.
    check(unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) })?;
    if info.nNumberOfLinks != 1 || info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(denied());
    }
    Ok((
        info.dwVolumeSerialNumber,
        (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
    ))
}
pub(super) fn path_identity(path: &Path) -> io::Result<FileIdentity> {
    file_identity(&open_read(path)?)
}
fn file(path: &Path, create: bool) -> io::Result<File> {
    let name = native(path)?;
    private_security(false, |attributes| {
        // SAFETY: no following reparse points; create_new never replaces existing data.
        let handle = unsafe {
            CreateFileW(
                name.as_ptr(),
                if create { GENERIC_WRITE } else { GENERIC_READ },
                FILE_SHARE_READ | FILE_SHARE_DELETE,
                attributes,
                if create { CREATE_NEW } else { OPEN_EXISTING },
                FILE_FLAG_OPEN_REPARSE_POINT,
                null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: newly owned CreateFileW handle transfers into File exactly once.
        let file = unsafe { File::from_raw_handle(handle) };
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        // SAFETY: file owns a valid handle and info is a writable output.
        check(unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) })?;
        if info.nNumberOfLinks != 1 {
            return Err(denied());
        }
        Ok(file)
    })
}
pub(super) fn open_read(path: &Path) -> io::Result<File> {
    file(path, false)
}
pub(super) fn create_private_file(path: &Path) -> io::Result<File> {
    file(path, true)
}

fn pipe_name(instance: HostId) -> Vec<u16> {
    format!("\\\\.\\pipe\\FolduroraHostMcp-{instance}")
        .encode_utf16()
        .chain(Some(0))
        .collect()
}
fn verify_peer(pipe: HANDLE, server: bool) -> io::Result<()> {
    let mut pid = 0;
    // SAFETY: pipe is connected and pid points to writable process-id storage.
    check(unsafe {
        if server {
            GetNamedPipeServerProcessId(pipe, &mut pid)
        } else {
            GetNamedPipeClientProcessId(pipe, &mut pid)
        }
    })?;
    // SAFETY: process query obtains no mutation capability.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return Err(io::Error::last_os_error());
    }
    let process = Handle(process);
    let mut remote = user_for(process.0)?;
    let mut local = current_user()?;
    // SAFETY: both SIDs remain within successfully queried live token buffers.
    if unsafe { EqualSid(sid(&mut remote), sid(&mut local)) } == 0 {
        return Err(denied());
    }
    Ok(())
}
pub(super) struct Stream {
    handle: Arc<Handle>,
    timeout: Duration,
}
impl Stream {
    pub(super) fn connect(_: &Path, instance: HostId) -> io::Result<Self> {
        let name = pipe_name(instance);
        let deadline = Instant::now() + DEADLINE;
        let handle = loop {
            // SAFETY: name is live and nul-terminated; handle is non-inheritable.
            let handle = unsafe {
                CreateFileW(
                    name.as_ptr(),
                    GENERIC_READ | GENERIC_WRITE,
                    0,
                    null_mut(),
                    OPEN_EXISTING,
                    FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                    null_mut(),
                )
            };
            if handle != INVALID_HANDLE_VALUE {
                break handle;
            }
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_FILE_NOT_FOUND as i32) {
                // A connected instance may just have closed before the next
                // listening instance is prepared. No bytes have been sent.
                thread::sleep(remaining(deadline)?.min(Duration::from_millis(20)));
                continue;
            }
            if error.raw_os_error() != Some(ERROR_PIPE_BUSY as i32) {
                return Err(error);
            }
            let wait = remaining(deadline)?.as_millis().max(1).min(u32::MAX as u128 - 1) as u32;
            // SAFETY: waiting for a matching instance occurs before any secret or
            // request bytes are sent; server SID is still checked after opening.
            if unsafe { WaitNamedPipeW(name.as_ptr(), wait) } == 0 {
                let error = io::Error::last_os_error();
                if error.raw_os_error() != Some(ERROR_FILE_NOT_FOUND as i32) {
                    return Err(error);
                }
                remaining(deadline)?;
            }
        };
        let handle = Arc::new(Handle(handle));
        verify_peer(handle.0, true)?;
        Ok(Self { handle, timeout: DEADLINE })
    }
    fn transfer(&self, pointer: *mut u8, bytes: usize, write: bool) -> io::Result<usize> {
        // SAFETY: CreateEventW receives valid null name/security inputs.
        let event = unsafe { CreateEventW(null_mut(), 1, 0, null_mut()) };
        if event.is_null() {
            return Err(io::Error::last_os_error());
        }
        let event = Handle(event);
        let mut overlap = OVERLAPPED {
            hEvent: event.0,
            ..Default::default()
        };
        let mut transferred = 0;
        // SAFETY: caller's slice and stack OVERLAPPED outlive completion/cancellation;
        // this stream admits only one sequential operation at a time.
        let ok = unsafe {
            if write {
                WriteFile(
                    self.handle.0,
                    pointer,
                    bytes.min(u32::MAX as usize) as u32,
                    &mut transferred,
                    &mut overlap,
                )
            } else {
                ReadFile(
                    self.handle.0,
                    pointer,
                    bytes.min(u32::MAX as usize) as u32,
                    &mut transferred,
                    &mut overlap,
                )
            }
        };
        if ok == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(ERROR_IO_PENDING as i32) {
                return Err(error);
            }
            // SAFETY: event is live; deadline is finite and clamped below INFINITE.
            let wait = unsafe { WaitForSingleObject(event.0, self.timeout.as_millis().max(1).min(u32::MAX as u128 - 1) as u32) };
            if wait != WAIT_OBJECT_0 {
                // SAFETY: drain cancellation before dropping OVERLAPPED or its buffer.
                unsafe {
                    CancelIoEx(self.handle.0, &overlap);
                    GetOverlappedResult(self.handle.0, &overlap, &mut transferred, 1);
                }
                return Err(io::Error::new(io::ErrorKind::TimedOut, "private host deadline"));
            }
            // SAFETY: event signals completion for this live OVERLAPPED operation.
            check(unsafe { GetOverlappedResult(self.handle.0, &overlap, &mut transferred, 0) })?;
        }
        Ok(transferred as usize)
    }
}
impl Read for Stream {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.transfer(out.as_mut_ptr(), out.len(), false)
    }
}
impl Write for Stream {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.transfer(data.as_ptr().cast_mut(), data.len(), true)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl TimedStream for Stream {
    fn timeout(&mut self, timeout: Duration) -> io::Result<()> {
        self.timeout = timeout;
        Ok(())
    }
}
pub(super) struct Cancel(Arc<Handle>);
impl Cancel {
    pub(super) fn cancel(&self) {
        // SAFETY: Arc keeps the pipe live while cancelling its owned IO and connection.
        unsafe {
            CancelIoEx(self.0.0, null_mut());
            DisconnectNamedPipe(self.0.0);
        }
    }
}

pub(super) struct Listener {
    name: Vec<u16>,
    handle: Option<Arc<Handle>>,
    overlap: Box<OVERLAPPED>,
    event: Handle,
    pending: bool,
    first: bool,
}
// SAFETY: listener moves with its heap-pinned OVERLAPPED and event; no other
// thread mutates those fields, and Drop drains its outstanding operation.
unsafe impl Send for Listener {}
impl Listener {
    pub(super) fn bind(_: &Path, instance: HostId) -> io::Result<Self> {
        // SAFETY: null attributes/name specify an unnamed non-inherited event.
        let event = unsafe { CreateEventW(null_mut(), 1, 0, null_mut()) };
        if event.is_null() {
            return Err(io::Error::last_os_error());
        }
        let mut value = Self {
            name: pipe_name(instance),
            handle: None,
            overlap: Box::default(),
            event: Handle(event),
            pending: false,
            first: true,
        };
        value.prepare()?;
        Ok(value)
    }
    fn prepare(&mut self) -> io::Result<()> {
        let handle = private_security(false, |attributes| {
            // SAFETY: name/security are live; current-user DACL, local-only,
            // non-inherited pipe with bounded buffers and four active peers.
            let pipe = unsafe {
                CreateNamedPipeW(
                    self.name.as_ptr(),
                    PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED | if self.first { FILE_FLAG_FIRST_PIPE_INSTANCE } else { 0 },
                    PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                    CLIENT_LIMIT as u32 + 1,
                    RESPONSE_LIMIT as u32,
                    REQUEST_LIMIT as u32,
                    0,
                    attributes,
                )
            };
            if pipe == INVALID_HANDLE_VALUE {
                return Err(io::Error::last_os_error());
            }
            Ok(Arc::new(Handle(pipe)))
        })?;
        self.first = false;
        *self.overlap = OVERLAPPED {
            hEvent: self.event.0,
            ..Default::default()
        };
        // SAFETY: event and heap-pinned OVERLAPPED live until completion or Drop drain.
        unsafe {
            ResetEvent(self.event.0);
        }
        let ok = unsafe { ConnectNamedPipe(handle.0, &mut *self.overlap) };
        self.pending = ok == 0;
        if ok == 0 {
            let error = io::Error::last_os_error();
            match error.raw_os_error() {
                Some(code) if code == ERROR_PIPE_CONNECTED as i32 => self.pending = false,
                Some(code) if code == ERROR_IO_PENDING as i32 => {}
                _ => return Err(error),
            }
        }
        self.handle = Some(handle);
        Ok(())
    }
    pub(super) fn accept(&mut self) -> io::Result<(Stream, Cancel)> {
        if self.handle.is_none() {
            self.prepare()?;
        }
        let handle = self.handle.as_ref().ok_or_else(denied)?;
        if self.pending {
            let mut transferred = 0;
            // SAFETY: handle/OVERLAPPED remain live throughout the pending connect.
            let ok = unsafe { GetOverlappedResult(handle.0, &*self.overlap, &mut transferred, 0) };
            if ok == 0 {
                let error = io::Error::last_os_error();
                if error.raw_os_error() == Some(ERROR_IO_INCOMPLETE as i32) {
                    return Err(io::ErrorKind::WouldBlock.into());
                }
                return Err(error);
            }
            self.pending = false;
        }
        let handle = self.handle.take().ok_or_else(denied)?;
        verify_peer(handle.0, false)?;
        Ok((
            Stream {
                handle: handle.clone(),
                timeout: DEADLINE,
            },
            Cancel(handle),
        ))
    }
}
impl Drop for Listener {
    fn drop(&mut self) {
        if self.pending
            && let Some(handle) = &self.handle
        {
            let mut transferred = 0;
            // SAFETY: cancellation is drained before OVERLAPPED/event/handle drop.
            unsafe {
                CancelIoEx(handle.0, &*self.overlap);
                GetOverlappedResult(handle.0, &*self.overlap, &mut transferred, 1);
            }
        }
    }
}
