use super::*;

#[test]
fn profile_constants() {
    assert_eq!(TcpProfile::WINDOWS.ttl, 128);
    assert_eq!(TcpProfile::MACOS.ttl, 64);
    assert_eq!(TcpProfile::LINUX.window_scale, 7);
}

#[test]
fn apply_does_not_panic() {
    let socket = Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::STREAM,
        Some(socket2::Protocol::TCP),
    )
    .unwrap();
    TcpProfile::WINDOWS.apply(&socket, false);
    TcpProfile::LINUX.apply(&socket, false);
    TcpProfile::MACOS.apply(&socket, false);
    TcpProfile::IOS.apply(&socket, false);

    let socket_v6 = Socket::new(
        socket2::Domain::IPV6,
        socket2::Type::STREAM,
        Some(socket2::Protocol::TCP),
    )
    .unwrap();
    TcpProfile::WINDOWS.apply(&socket_v6, true);
    TcpProfile::LINUX.apply(&socket_v6, true);
    TcpProfile::MACOS.apply(&socket_v6, true);
    TcpProfile::IOS.apply(&socket_v6, true);
}
