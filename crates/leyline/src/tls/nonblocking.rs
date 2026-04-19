//! Non-blocking TCP connect primitive shared by the Happy-Eyeballs
//! racer and any future direct-connect path.

use crate::tcp::TcpProfile;
use tokio::net::TcpStream;

/// Build a fingerprinted TCP connection to a single resolved address.
///
/// Extracted so Happy-Eyeballs can invoke it per candidate without
/// cloning the whole connector. The TCP profile is applied *before*
/// `connect` so SYN options (MSS, window scale, TFO, …) match the
/// browser fingerprint.
pub(crate) async fn connect_one(
    sock_addr: std::net::SocketAddr,
    tcp_profile: &TcpProfile,
) -> Result<TcpStream, std::io::Error> {
    let domain = match sock_addr {
        std::net::SocketAddr::V4(_) => socket2::Domain::IPV4,
        std::net::SocketAddr::V6(_) => socket2::Domain::IPV6,
    };
    let socket = socket2::Socket::new(domain, socket2::Type::STREAM, Some(socket2::Protocol::TCP))?;
    tcp_profile.apply(&socket);
    socket.set_nonblocking(true)?;

    match socket.connect(&sock_addr.into()) {
        Ok(()) => {}
        Err(e) if nonblocking_connect_started(&e) => {}
        Err(e) => return Err(e),
    }
    let std_stream: std::net::TcpStream = socket.into();
    let tcp_stream = TcpStream::from_std(std_stream)?;

    // Wait for the non-blocking connect to complete (or error out).
    tcp_stream.writable().await?;
    if let Some(e) = tcp_stream.take_error()? {
        return Err(e);
    }
    Ok(tcp_stream)
}

/// `Socket::connect` on a non-blocking socket returns an OS-specific
/// "would block / in progress" error that means "the async connect
/// has started; wait for writable". Unix: `EINPROGRESS` or the
/// generic `WouldBlock` kind. Windows WSA: `WSAEWOULDBLOCK` (10035)
/// or `WSAEINPROGRESS` (10036). Without this, non-blocking connects
/// on Windows fail instantly instead of completing asynchronously.
fn nonblocking_connect_started(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::WouldBlock
        || error.raw_os_error() == Some(libc::EINPROGRESS)
        || windows_nonblocking_connect_started(error)
}

#[cfg(windows)]
fn windows_nonblocking_connect_started(error: &std::io::Error) -> bool {
    const WSAEWOULDBLOCK: i32 = 10035;
    const WSAEINPROGRESS: i32 = 10036;
    matches!(
        error.raw_os_error(),
        Some(WSAEWOULDBLOCK) | Some(WSAEINPROGRESS)
    )
}

#[cfg(not(windows))]
fn windows_nonblocking_connect_started(_error: &std::io::Error) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::nonblocking_connect_started;

    #[test]
    fn accepts_would_block_kind() {
        let err = std::io::Error::from(std::io::ErrorKind::WouldBlock);
        assert!(nonblocking_connect_started(&err));
    }

    #[test]
    fn rejects_unrelated_connect_error() {
        let err = std::io::Error::from(std::io::ErrorKind::ConnectionRefused);
        assert!(!nonblocking_connect_started(&err));
    }

    #[cfg(unix)]
    #[test]
    fn accepts_unix_einprogress() {
        let err = std::io::Error::from_raw_os_error(libc::EINPROGRESS);
        assert!(nonblocking_connect_started(&err));
    }

    #[cfg(windows)]
    #[test]
    fn accepts_windows_wsaewouldblock() {
        let err = std::io::Error::from_raw_os_error(10035);
        assert!(nonblocking_connect_started(&err));
    }

    #[cfg(windows)]
    #[test]
    fn accepts_windows_wsaeinprogress() {
        let err = std::io::Error::from_raw_os_error(10036);
        assert!(nonblocking_connect_started(&err));
    }
}
