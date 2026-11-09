//! Bounded local upload inputs. Input ancestry is opened without following a
//! symlink/reparse point; only immutable bytes and safe names leave this module.
use std::{
    fs::File,
    io::{Error, ErrorKind, Read, Result},
    path::{Component, Path, PathBuf},
};
pub(super) const FILE_LIMIT: usize = 64 * 1024 * 1024;
pub(super) const BATCH_LIMIT: usize = 128 * 1024 * 1024;
pub(super) struct Input {
    pub name: String,
    pub extension: String,
    pub folders: Vec<String>,
    pub bytes: Vec<u8>,
}
fn bad() -> Error {
    Error::new(ErrorKind::InvalidInput, "invalid local upload input")
}
fn name(value: &std::ffi::OsStr, max: usize) -> Result<String> {
    let text = value
        .to_str()
        .filter(|s| !s.is_empty() && s.len() <= max && !s.chars().any(char::is_control) && !s.contains(['/', '\\', ':', '|']))
        .ok_or_else(bad)?;
    Ok(text.to_owned())
}
fn supported(path: &Path) -> bool {
    path.extension().and_then(|s| s.to_str()).is_some_and(|s| {
        matches!(
            s.to_ascii_lowercase().as_str(),
            "png" | "jpg" | "jpeg" | "tga" | "bmp" | "wav" | "anim" | "bvh"
        )
    })
}
fn path(value: &str) -> Result<PathBuf> {
    if value.is_empty() || value.len() > 4096 || value.contains('\0') || value.starts_with("//") || value.starts_with("\\\\") {
        return Err(bad());
    }
    let path = PathBuf::from(value);
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir | Component::CurDir)) {
        return Err(bad());
    }
    Ok(path)
}
struct Open {
    file: File,
    #[cfg(windows)]
    _parents: Vec<File>,
}
#[cfg(unix)]
fn open(path: &Path) -> Result<Open> {
    use std::{
        ffi::CString,
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::ffi::OsStrExt,
        },
    };
    let mut parts = path.components();
    if !matches!(parts.next(), Some(Component::RootDir)) {
        return Err(bad());
    }
    let mut parent = File::open("/")?;
    let parts = parts.collect::<Vec<_>>();
    if parts.is_empty() {
        return Ok(Open { file: parent });
    }
    for (index, part) in parts.iter().enumerate() {
        let Component::Normal(part) = part else { return Err(bad()) };
        let component = CString::new(part.as_bytes()).map_err(|_| bad())?;
        let directory = index + 1 < parts.len();
        let flags = libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK | if directory { libc::O_DIRECTORY } else { 0 };
        // SAFETY: parent and component are live, NUL-terminated and independent;
        // openat transfers exactly one successful descriptor to the new File.
        let fd = unsafe { libc::openat(parent.as_raw_fd(), component.as_ptr(), flags) };
        if fd < 0 {
            return Err(Error::last_os_error());
        }
        // SAFETY: fd is newly owned and not aliased by another File.
        parent = unsafe { File::from_raw_fd(fd) };
        if directory && !parent.metadata()?.is_dir() {
            return Err(bad());
        }
    }
    Ok(Open { file: parent })
}
#[cfg(windows)]
fn open(path: &Path) -> Result<Open> {
    use std::{
        fs::OpenOptions,
        os::windows::{
            ffi::OsStrExt,
            fs::{MetadataExt, OpenOptionsExt},
        },
    };
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ,
    };
    let mut parts = path.components();
    let Some(Component::Prefix(prefix)) = parts.next() else {
        return Err(bad());
    };
    if !matches!(prefix.kind(), std::path::Prefix::Disk(_)) || !matches!(parts.next(), Some(Component::RootDir)) {
        return Err(bad());
    }
    let mut current = PathBuf::from(prefix.as_os_str());
    current.push("\\");
    let mut chain = Vec::new();
    let mut paths = vec![current.clone()];
    for part in parts {
        let Component::Normal(part) = part else { return Err(bad()) };
        if part.encode_wide().any(|c| c == 0 || c == u16::from(b':')) {
            return Err(bad());
        }
        current.push(part);
        paths.push(current.clone());
    }
    for p in paths {
        // No WRITE or DELETE sharing: held ancestors cannot be replaced with a
        // reparse point while later components or the directory are read.
        let file = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
            .open(p)?;
        if file.metadata()?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(bad());
        }
        chain.push(file);
    }
    let file = chain.pop().ok_or_else(bad)?;
    Ok(Open { file, _parents: chain })
}
#[cfg(not(any(unix, windows)))]
fn open(_: &Path) -> Result<Open> {
    Err(Error::new(ErrorKind::Unsupported, "local upload platform"))
}
#[cfg(unix)]
fn open_child(parent: &Open, value: &std::ffi::OsStr, _path: &Path) -> Result<Open> {
    use std::{
        ffi::CString,
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::ffi::OsStrExt,
        },
    };
    let component = CString::new(value.as_bytes()).map_err(|_| bad())?;
    // SAFETY: the held parent descriptor anchors this single NUL-terminated
    // component. NOFOLLOW rejects links; NONBLOCK prevents a raced FIFO hang.
    let fd = unsafe {
        libc::openat(
            parent.file.as_raw_fd(),
            component.as_ptr(),
            libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
        )
    };
    if fd < 0 {
        return Err(Error::last_os_error());
    }
    // SAFETY: the successful descriptor is newly owned exactly once.
    Ok(Open {
        file: unsafe { File::from_raw_fd(fd) },
    })
}
#[cfg(not(unix))]
fn open_child(_parent: &Open, _value: &std::ffi::OsStr, path: &Path) -> Result<Open> {
    open(path)
}
fn read(path: &Path, opened: &Open, folders: Vec<String>) -> Result<Input> {
    if !supported(path) {
        return Err(bad());
    }
    let bytes = read_bytes(opened)?;
    let stem = name(path.file_stem().ok_or_else(bad)?, 63)?;
    Ok(Input {
        name: stem,
        extension: path.extension().and_then(|e| e.to_str()).ok_or_else(bad)?.to_ascii_lowercase(),
        folders,
        bytes,
    })
}
fn read_bytes(opened: &Open) -> Result<Vec<u8>> {
    let before = opened.file.metadata()?;
    let size = usize::try_from(before.len()).map_err(|_| bad())?;
    if !before.is_file() || size == 0 || size > FILE_LIMIT {
        return Err(bad());
    }
    let mut bytes = Vec::new();
    (&opened.file).take(FILE_LIMIT as u64 + 1).read_to_end(&mut bytes)?;
    let after = opened.file.metadata()?;
    if bytes.len() != size || after.len() != before.len() || after.modified()? != before.modified()? {
        return Err(bad());
    }
    Ok(bytes)
}
pub(super) fn mesh(value: &str) -> Result<(PathBuf, Vec<u8>)> {
    let path = path(value)?;
    if !path
        .extension()
        .and_then(|v| v.to_str())
        .is_some_and(|v| v.eq_ignore_ascii_case("dae"))
    {
        return Err(bad());
    }
    let opened = open(&path)?;
    let bytes = read_bytes(&opened)?;
    Ok((path, bytes))
}
fn directory(
    path: &Path,
    opened: &Open,
    folders: &[String],
    inputs: &mut Vec<Input>,
    total: &mut usize,
    visited: &mut usize,
) -> Result<()> {
    if folders.len() > 8 {
        return Err(bad());
    }
    if !opened.file.metadata()?.is_dir() {
        return Err(bad());
    }
    // These are trusted kernel descriptor aliases, not caller path ancestry.
    // Directory traversal stays anchored to the already opened directory even
    // if its original name is renamed while enumeration runs.
    #[cfg(target_os = "linux")]
    let listing = {
        use std::os::fd::AsRawFd;
        PathBuf::from(format!("/proc/self/fd/{}", opened.file.as_raw_fd()))
    };
    #[cfg(all(unix, not(target_os = "linux")))]
    let listing = {
        use std::os::fd::AsRawFd;
        PathBuf::from(format!("/dev/fd/{}", opened.file.as_raw_fd()))
    };
    #[cfg(not(unix))]
    let listing = path.to_owned();
    let mut entries = std::fs::read_dir(&listing)?.take(1025).collect::<Result<Vec<_>>>()?;
    if entries.len() > 1024 {
        return Err(bad());
    }
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        *visited = visited.checked_add(1).ok_or_else(bad)?;
        if *visited > 4096 {
            return Err(bad());
        }
        let child_name = name(&entry.file_name(), 255)?;
        let child = path.join(&child_name);
        let child_open = open_child(opened, &entry.file_name(), &child)?;
        let meta = child_open.file.metadata()?;
        if meta.is_dir() {
            let mut nested = folders.to_vec();
            nested.push(name(&entry.file_name(), 63)?);
            directory(&child, &child_open, &nested, inputs, total, visited)?;
        } else if meta.is_file() {
            if inputs.len() >= 32 {
                return Err(bad());
            }
            let input = read(Path::new(&entry.file_name()), &child_open, folders.to_vec())?;
            *total = total.checked_add(input.bytes.len()).ok_or_else(bad)?;
            if *total > BATCH_LIMIT {
                return Err(bad());
            }
            inputs.push(input);
        } else {
            return Err(bad());
        }
    }
    Ok(())
}
pub(super) fn collect(files: &[String], folder: Option<&str>) -> Result<Vec<Input>> {
    if files.is_empty() == folder.is_none() || files.len() > 32 {
        return Err(bad());
    }
    let mut inputs = Vec::new();
    let mut total = 0usize;
    if let Some(folder) = folder {
        let path = path(folder)?;
        let opened = open(&path)?;
        directory(&path, &opened, &[], &mut inputs, &mut total, &mut 0)?;
    } else {
        let mut seen = std::collections::HashSet::new();
        for file in files {
            let path = path(file)?;
            if !seen.insert(path.clone()) {
                return Err(bad());
            }
            let opened = open(&path)?;
            let input = read(&path, &opened, Vec::new())?;
            total = total.checked_add(input.bytes.len()).ok_or_else(bad)?;
            if total > BATCH_LIMIT {
                return Err(bad());
            }
            inputs.push(input);
        }
    }
    if inputs.is_empty() {
        return Err(bad());
    }
    Ok(inputs)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("foldurora-mcp-upload-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn upload_inputs_reject_symlink_leaf_and_ancestry_without_following_them() {
        let f = Fixture::new();
        let real = f.0.join("real");
        std::fs::create_dir(&real).unwrap();
        std::fs::write(real.join("asset.png"), b"synthetic").unwrap();
        std::os::unix::fs::symlink(&real, f.0.join("linked")).unwrap();
        std::os::unix::fs::symlink(real.join("asset.png"), f.0.join("leaf.png")).unwrap();
        assert!(collect(&[f.0.join("linked/asset.png").to_str().unwrap().into()], None).is_err());
        assert!(collect(&[f.0.join("leaf.png").to_str().unwrap().into()], None).is_err());
    }
    #[test]
    fn directory_children_remain_bound_to_open_handle_after_path_replacement() {
        let f = Fixture::new();
        let selected = f.0.join("selected");
        std::fs::create_dir(&selected).unwrap();
        std::fs::write(selected.join("asset.png"), b"approved-source").unwrap();
        let opened = open(&selected).unwrap();
        std::fs::rename(&selected, f.0.join("original")).unwrap();
        std::fs::create_dir(&selected).unwrap();
        std::fs::write(selected.join("asset.png"), b"replacement-source").unwrap();
        let mut inputs = Vec::new();
        directory(&selected, &opened, &[], &mut inputs, &mut 0, &mut 0).unwrap();
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].bytes, b"approved-source");
    }
    #[test]
    fn upload_inputs_stop_before_accepting_a_thirty_third_file() {
        let f = Fixture::new();
        for i in 0..33 {
            std::fs::write(f.0.join(format!("{i}.png")), b"synthetic").unwrap();
        }
        assert!(collect(&[], Some(f.0.to_str().unwrap())).is_err());
    }
}
