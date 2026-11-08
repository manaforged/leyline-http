use std::net::SocketAddr;

use socket2::{Domain, Socket, Type};

use super::*;
use crate::profile::Platform;

const V4: SocketAddr = SocketAddr::V4(std::net::SocketAddrV4::new(Ipv4Addr::LOCALHOST, 1));
const V6: SocketAddr = SocketAddr::V6(std::net::SocketAddrV6::new(Ipv6Addr::LOCALHOST, 1, 0, 0));

fn profile_only() -> SocketOptions {
    SocketOptions {
        tcp_keepalive: None,
        tcp_keepalive_interval: None,
        tcp_keepalive_retries: None,
        ..SocketOptions::default()
    }
}

fn apply_profile(socket: &Socket, profile: &TcpProfile, peer: SocketAddr) {
    configure(socket, peer, profile, &profile_only()).expect("profile options never fail");
}

#[test]
fn profile_does_not_cap_receive_buffer() {
    let socket = Socket::new(Domain::IPV4, Type::STREAM, None).expect("TCP socket");
    let profile = Platform::Windows.tcp_profile();
    apply_profile(&socket, &profile, V4);
    let buffer = socket.recv_buffer_size().expect("receive buffer");
    assert_ne!(buffer as u32, profile.window_size);
}

#[test]
fn apply_does_not_panic() {
    let socket = Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::STREAM,
        Some(socket2::Protocol::TCP),
    )
    .unwrap();
    apply_profile(&socket, &Platform::Windows.tcp_profile(), V4);
    apply_profile(&socket, &Platform::Linux.tcp_profile(), V4);
    apply_profile(&socket, &Platform::MacOS.tcp_profile(), V4);
    apply_profile(&socket, &Platform::IOS.tcp_profile(), V4);

    let socket_v6 = Socket::new(
        socket2::Domain::IPV6,
        socket2::Type::STREAM,
        Some(socket2::Protocol::TCP),
    )
    .unwrap();
    apply_profile(&socket_v6, &Platform::Windows.tcp_profile(), V6);
    apply_profile(&socket_v6, &Platform::Linux.tcp_profile(), V6);
    apply_profile(&socket_v6, &Platform::MacOS.tcp_profile(), V6);
    apply_profile(&socket_v6, &Platform::IOS.tcp_profile(), V6);
}

#[cfg(target_os = "macos")]
#[test]
fn a_browser_profile_on_macos_sets_no_option_the_kernel_refuses() {
    let socket = Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::STREAM,
        Some(socket2::Protocol::TCP),
    )
    .unwrap();
    for platform in [Platform::Windows, Platform::MacOS, Platform::Linux] {
        apply_profile(&socket, &platform.tcp_profile(), V4);
    }
    let logged = crate::util::lock(&LOGGED);
    for option in ["IP_TTL", "TCP_NODELAY", "IP_DONTFRAG"] {
        assert!(!logged.contains(&option), "the kernel refused {option}");
    }
}

#[test]
fn default_keepalive_retries_never_fatal_under_strict() {
    let socket = Socket::new(socket2::Domain::IPV4, socket2::Type::STREAM, None).unwrap();
    let config = SocketOptions {
        strict: true,
        ..SocketOptions::default()
    };
    assert!(config.tcp_keepalive_retries.is_some());
    configure(&socket, V4, &TcpProfile::default(), &config).expect("expected Ok");
}

#[cfg(any(target_os = "linux", target_os = "android"))]
#[test]
fn tcp_user_timeout_reads_back() {
    let socket = Socket::new(socket2::Domain::IPV4, socket2::Type::STREAM, None).unwrap();
    let timeout = std::time::Duration::from_secs(7);
    let config = SocketOptions {
        tcp_user_timeout: Some(timeout),
        ..SocketOptions::default()
    };
    configure(&socket, V4, &TcpProfile::default(), &config).unwrap();
    assert_eq!(socket.tcp_user_timeout().unwrap(), Some(timeout));
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
#[test]
fn tcp_user_timeout_unsupported_errors_under_strict() {
    let socket = Socket::new(socket2::Domain::IPV4, socket2::Type::STREAM, None).unwrap();
    let config = SocketOptions {
        strict: true,
        tcp_user_timeout: Some(std::time::Duration::from_secs(7)),
        ..SocketOptions::default()
    };
    let err = configure(&socket, V4, &TcpProfile::default(), &config).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::Unsupported);
}
