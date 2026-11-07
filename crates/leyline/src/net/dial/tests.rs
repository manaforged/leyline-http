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
