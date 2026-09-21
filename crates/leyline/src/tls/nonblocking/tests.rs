use super::{apply_socket_config, nonblocking_connect_started};
use crate::core::SocketConfig;

#[test]
fn default_keepalive_retries_never_fatal_under_strict() {
    let socket = socket2::Socket::new(socket2::Domain::IPV4, socket2::Type::STREAM, None).unwrap();
    let config = SocketConfig {
        strict: true,
        ..SocketConfig::default()
    };
    assert!(config.tcp_keepalive_retries.is_some());
    assert!(apply_socket_config(&socket, &config).is_ok());
}

#[cfg(any(target_os = "linux", target_os = "android"))]
#[test]
fn tcp_user_timeout_reads_back() {
    let socket = socket2::Socket::new(socket2::Domain::IPV4, socket2::Type::STREAM, None).unwrap();
    let timeout = std::time::Duration::from_secs(7);
    let config = SocketConfig {
        tcp_user_timeout: Some(timeout),
        ..SocketConfig::default()
    };
    apply_socket_config(&socket, &config).unwrap();
    assert_eq!(socket.tcp_user_timeout().unwrap(), Some(timeout));
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
#[test]
fn tcp_user_timeout_unsupported_errors_under_strict() {
    let socket = socket2::Socket::new(socket2::Domain::IPV4, socket2::Type::STREAM, None).unwrap();
    let config = SocketConfig {
        strict: true,
        tcp_user_timeout: Some(std::time::Duration::from_secs(7)),
        ..SocketConfig::default()
    };
    let err = apply_socket_config(&socket, &config).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::Unsupported);
}

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
