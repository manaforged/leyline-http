use socket2::{Domain, SockRef, Socket, Type};
use tokio::net::{TcpListener, TcpStream};

use crate::{Browser, Platform, Session, SocketConfig};

async fn limits(session: Option<Session>) -> (u32, usize, bool) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("loopback socket");
    let address = listener.local_addr().expect("listener address");
    let stream = match session {
        Some(session) => session
            .inner
            .connector
            .dial_tcp("127.0.0.1", address.port())
            .await
            .expect("session TCP connection"),
        None => TcpStream::connect(address).await.expect("loopback socket"),
    };
    let socket = SockRef::from(&stream);
    (
        socket.tcp_mss().expect("TCP MSS"),
        socket.recv_buffer_size().expect("receive buffer"),
        socket.tcp_nodelay().expect("TCP_NODELAY"),
    )
}

#[tokio::test]
async fn bare_tcp_uses_os_limits() {
    let native = limits(None).await;
    let bare = limits(Some(Session::builder().build().unwrap())).await;
    assert_eq!(bare.0, native.0);
    assert_eq!(bare.1, native.1);
    assert!(bare.2);
}

#[tokio::test]
async fn tcp_profiles_keep_mss_and_nodelay() {
    let profile = Platform::Linux.tcp_profile();
    let browser = Session::builder()
        .browser(Browser::Chrome149)
        .platform(Platform::Linux)
        .build()
        .expect("Chrome session");
    let explicit = Session::builder()
        .tcp_profile(profile.clone())
        .build()
        .expect("TCP profile session");
    for session in [browser, explicit] {
        let (mss, _, nodelay) = limits(Some(session)).await;
        assert!(mss > 0 && mss <= profile.mss);
        assert!(nodelay);
    }
}

#[test]
fn profile_does_not_cap_receive_buffer() {
    let socket = Socket::new(Domain::IPV4, Type::STREAM, None).expect("TCP socket");
    let profile = Platform::Windows.tcp_profile();
    profile.apply(&socket, false);
    let buffer = socket.recv_buffer_size().expect("receive buffer");
    assert_ne!(buffer as u32, profile.window_size);
}

#[tokio::test]
async fn socket_config_keeps_explicit_buffer() {
    let socket = Socket::new(Domain::IPV4, Type::STREAM, None).expect("TCP socket");
    socket
        .set_recv_buffer_size(262_144)
        .expect("receive buffer override");
    let session = Session::builder()
        .socket(SocketConfig::default().recv_buffer_size(262_144))
        .build()
        .expect("socket configuration");
    let (_, buffer, _) = limits(Some(session)).await;
    assert_eq!(buffer, socket.recv_buffer_size().expect("receive buffer"));
}
