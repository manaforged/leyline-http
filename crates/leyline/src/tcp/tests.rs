use super::*;
use crate::profile::Platform;

#[test]
fn profile_constants() {
    assert_eq!(Platform::Windows.tcp_profile().ttl, 128);
    assert_eq!(Platform::MacOS.tcp_profile().ttl, 64);
    assert_eq!(Platform::Linux.tcp_profile().window_scale, 10);
}

#[test]
fn apply_does_not_panic() {
    let socket = Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::STREAM,
        Some(socket2::Protocol::TCP),
    )
    .unwrap();
    Platform::Windows.tcp_profile().apply(&socket, false);
    Platform::Linux.tcp_profile().apply(&socket, false);
    Platform::MacOS.tcp_profile().apply(&socket, false);
    Platform::IOS.tcp_profile().apply(&socket, false);

    let socket_v6 = Socket::new(
        socket2::Domain::IPV6,
        socket2::Type::STREAM,
        Some(socket2::Protocol::TCP),
    )
    .unwrap();
    Platform::Windows.tcp_profile().apply(&socket_v6, true);
    Platform::Linux.tcp_profile().apply(&socket_v6, true);
    Platform::MacOS.tcp_profile().apply(&socket_v6, true);
    Platform::IOS.tcp_profile().apply(&socket_v6, true);
}
