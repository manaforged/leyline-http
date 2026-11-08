#[cfg(feature = "http3")]
use std::net::UdpSocket;
#[cfg(feature = "http3")]
use std::time::Duration;

use crate::Session;
#[cfg(feature = "http3")]
use crate::core::Protocol;
#[cfg(feature = "http3")]
use crate::profile::{Browser, ChromiumBrand};
#[cfg(feature = "http3")]
use crate::testing::{Request, Response, Server};
#[cfg(feature = "http3")]
use crate::{Proxy, SessionBuilder};

#[test]
fn rotate() {
    let session = Session::builder()
        .proxy("http://first:1")
        .build()
        .expect("bare session builds");
    let rotated = session
        .to_builder()
        .proxy("http://second:2")
        .build()
        .unwrap();

    let url = url::Url::parse("https://example.test/").unwrap();
    assert_eq!(
        rotated
            .proxy_for(&url, None)
            .expect("valid proxy url")
            .map(crate::net::ProxyUrl::as_str),
        Some("http://second:2"),
        "with_proxy must win over the build-time proxy"
    );
}

#[cfg(feature = "http3")]
const QUIC_LONG_HEADER: u8 = 0xc0;

#[cfg(feature = "http3")]
fn datagram(probe: &UdpSocket) -> Option<u8> {
    let mut buffer = [0u8; 1500];
    match probe.recv(&mut buffer) {
        Ok(_) => Some(buffer[0]),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => None,
        Err(error) => panic!("probe socket: {error}"),
    }
}

#[cfg(feature = "http3")]
async fn advertising_origin() -> (Server, UdpSocket) {
    let server = Server::https(|request: &Request| {
        let host = request.header("host").expect("host header");
        let (_, port) = host.rsplit_once(':').expect("host port");
        Response::new(200).header("alt-svc", format!("h3=\":{port}\"; ma=3600"))
    })
    .await
    .expect("server starts");
    let probe = UdpSocket::bind(server.addr()).expect("UDP port of the origin");
    probe.set_nonblocking(true).expect("nonblocking probe");
    (server, probe)
}

#[cfg(feature = "http3")]
async fn first_response(builder: SessionBuilder, server: &Server, probe: &UdpSocket) -> Session {
    let session = builder
        .tls(server.tls())
        .proxy(Proxy::new().env(false))
        .build()
        .expect("session");
    let first = session.get(server.url("/")).send().await.expect("response");
    assert_eq!(first.version(), http::Version::HTTP_11);
    assert_eq!(datagram(probe), None, "no HTTP/3 before Alt-Svc");
    session
}

#[cfg(feature = "http3")]
async fn assert_dials_quic_after_alt_svc(builder: SessionBuilder) {
    let (server, probe) = advertising_origin().await;
    let session = first_response(builder, &server, &probe).await;
    let url = server.url("/");
    let second = tokio::spawn(async move { drop(session.get(url).send().await) });
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let first_byte = loop {
        if let Some(byte) = datagram(&probe) {
            break byte;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "no QUIC packet after Alt-Svc"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    };
    second.abort();
    assert_eq!(first_byte & QUIC_LONG_HEADER, QUIC_LONG_HEADER);
}

#[cfg(feature = "http3")]
async fn assert_stays_on_tcp_after_alt_svc(builder: SessionBuilder) {
    let (server, probe) = advertising_origin().await;
    let session = first_response(builder, &server, &probe).await;
    let second = session.get(server.url("/")).send().await.expect("response");
    assert_eq!(second.version(), http::Version::HTTP_11);
    assert_eq!(datagram(&probe), None, "the policy must not dial HTTP/3");
}

#[cfg(feature = "http3")]
#[tokio::test]
async fn race() {
    assert_dials_quic_after_alt_svc(
        Session::builder()
            .identity(Browser::default())
            .protocol(Protocol::Race),
    )
    .await;
}

#[cfg(feature = "http3")]
#[tokio::test]
async fn chrome_race() {
    assert_dials_quic_after_alt_svc(Session::builder().identity(Browser::default())).await;
}

#[cfg(feature = "http3")]
#[tokio::test]
async fn http1_then_edge_keeps_http1() {
    assert_stays_on_tcp_after_alt_svc(
        Session::builder().protocol(Protocol::Http1).identity(
            crate::core::Identity::from(Browser::default()).with_brand(ChromiumBrand::Edge),
        ),
    )
    .await;
}

#[cfg(feature = "http3")]
#[tokio::test]
async fn http1_then_chrome_keeps_http1() {
    assert_stays_on_tcp_after_alt_svc(
        Session::builder()
            .protocol(Protocol::Http1)
            .identity(Browser::default()),
    )
    .await;
}

#[cfg(feature = "http3")]
#[tokio::test]
async fn bare() {
    assert_stays_on_tcp_after_alt_svc(Session::builder()).await;
}

#[cfg(feature = "http3")]
#[tokio::test]
async fn brave_firefox_and_safari_race_http3() {
    for browser in [
        Browser::brave(),
        Browser::firefox(),
        Browser::safari(),
        Browser::safari_ios(),
    ] {
        assert_dials_quic_after_alt_svc(Session::builder().identity(browser)).await;
    }
}
