//! Platform creation only; all callers are dedicated IO workers. Connect starts
//! nonblocking, including pixel reattachment while metadata remains responsive.
use std::io;
use std::net::{Ipv4Addr, TcpStream};

pub(super) fn connect(port: u16) -> io::Result<TcpStream> {
    if port == 0 {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    let stream = platform_connect(port)?;
    stream.set_nodelay(true)?;
    Ok(stream)
}
pub(super) fn connected(stream: &TcpStream) -> io::Result<bool> {
    if let Some(error) = stream.take_error()? {
        return Err(error);
    }
    match stream.peer_addr() {
        Ok(address) if address.ip().is_loopback() => Ok(true),
        Ok(_) => Err(io::ErrorKind::PermissionDenied.into()),
        Err(error) if error.kind() == io::ErrorKind::NotConnected || error.kind() == io::ErrorKind::WouldBlock => Ok(false),
        Err(error) => Err(error),
    }
}
#[cfg(unix)]
fn platform_connect(port: u16) -> io::Result<TcpStream> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    // SAFETY: socket creates a fresh descriptor; OwnedFd takes sole ownership.
    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: fd was freshly returned by socket, and has no other owner.
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    // SAFETY: fcntl receives a live owned descriptor and scalar flag values.
    if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } < 0
        || unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) } < 0
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: all-zero sockaddr_in is initialized below before connect reads it.
    let mut address: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    address.sin_family = libc::AF_INET as _;
    address.sin_port = port.to_be();
    address.sin_addr.s_addr = u32::from_ne_bytes(Ipv4Addr::LOCALHOST.octets());
    #[cfg(target_os = "macos")]
    {
        address.sin_len = std::mem::size_of::<libc::sockaddr_in>() as u8;
    }
    // SAFETY: address points to a fully initialized sockaddr_in with exact size;
    // it outlives the call. Nonblocking connect never retains this pointer.
    if unsafe {
        libc::connect(
            fd.as_raw_fd(),
            (&address as *const libc::sockaddr_in).cast(),
            std::mem::size_of_val(&address) as _,
        )
    } < 0
    {
        let error = io::Error::last_os_error();
        if !matches!(error.raw_os_error(), Some(libc::EINPROGRESS | libc::EALREADY)) && error.kind() != io::ErrorKind::WouldBlock {
            return Err(error);
        }
    }
    Ok(TcpStream::from(fd))
}
#[cfg(windows)]
fn platform_connect(port: u16) -> io::Result<TcpStream> {
    use std::os::windows::io::{AsRawSocket, FromRawSocket, OwnedSocket};
    use windows::Win32::Networking::WinSock::*;
    // Like std's network runtime, keep one process-wide Winsock reference. It
    // must outlive sockets moved out of this function; per-call cleanup could
    // destroy an in-progress connection when no other Winsock reference exists.
    static START: std::sync::OnceLock<i32> = std::sync::OnceLock::new();
    let started = *START.get_or_init(|| {
        let mut data = WSADATA::default();
        // SAFETY: data is valid writable WSADATA storage for this synchronous call.
        unsafe { WSAStartup(0x0202, &mut data) }
    });
    if started != 0 {
        return Err(io::Error::from_raw_os_error(started));
    }
    // SAFETY: creates an independent IPv4 TCP socket under initialized Winsock.
    let raw = unsafe { socket(i32::from(AF_INET.0), SOCK_STREAM, IPPROTO_TCP.0) }.map_err(|_| {
        // SAFETY: reads this thread's last error from the preceding socket call.
        io::Error::from_raw_os_error(unsafe { WSAGetLastError().0 })
    })?;
    // SAFETY: freshly created socket transfers its sole ownership to OwnedSocket.
    let owned = unsafe { OwnedSocket::from_raw_socket(raw.0 as _) };
    let mut nonblocking = 1;
    // SAFETY: owned is live and nonblocking is a writable scalar for FIONBIO.
    if unsafe { ioctlsocket(SOCKET(owned.as_raw_socket() as _), FIONBIO, &mut nonblocking) } != 0 {
        // SAFETY: reads this thread's last Winsock error only.
        return Err(io::Error::from_raw_os_error(unsafe { WSAGetLastError().0 }));
    }
    let address = SOCKADDR_IN {
        sin_family: AF_INET,
        sin_port: port.to_be(),
        sin_addr: IN_ADDR {
            S_un: IN_ADDR_0 {
                S_addr: u32::from_ne_bytes(Ipv4Addr::LOCALHOST.octets()),
            },
        },
        ..Default::default()
    };
    // SAFETY: address is initialized and valid for exactly the supplied length;
    // nonblocking Winsock does not retain its pointer after returning.
    if unsafe {
        connect(
            SOCKET(owned.as_raw_socket() as _),
            (&address as *const SOCKADDR_IN).cast(),
            std::mem::size_of_val(&address) as i32,
        )
    } != 0
    {
        // SAFETY: reads only the current thread's immediately preceding error.
        let code = unsafe { WSAGetLastError() };
        if !matches!(code, WSAEWOULDBLOCK | WSAEINPROGRESS | WSAEALREADY) {
            return Err(io::Error::from_raw_os_error(code.0));
        }
    }
    Ok(TcpStream::from(owned))
}
