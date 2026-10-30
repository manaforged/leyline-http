use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use leyline::{Browser, Encoding, Proxy, Redirect, Session, Timeout};

#[test]
fn builder_accepts_wreq_parity_transport_knobs() {
    let proxy = Proxy::new()
        .https("http://127.0.0.1:8080")
        .no_proxy(["localhost", ".internal"])
        .env(false);
    let timeouts = Timeout::default()
        .total(Duration::from_secs(5))
        .connect(Duration::from_secs(1))
        .read(Duration::from_secs(2))
        .response_header(Duration::from_secs(3));
    let redirects = Redirect::custom(|attempt| {
        attempt.status() != Some(leyline::http::StatusCode::TEMPORARY_REDIRECT)
    });

    let session = Session::builder()
        .browser(Browser::Chrome147)
        .proxy(proxy)
        .resolve(
            "example.test",
            [SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 443)],
        )
        .timeout(timeouts)
        .pool_idle_timeout(Duration::from_secs(30))
        .max_connections(8)
        .max_connections_per_host(6)
        .keep_alive(true)
        .local_address(IpAddr::V4(Ipv4Addr::UNSPECIFIED))
        .tcp_nodelay(true)
        .tcp_keepalive(Duration::from_secs(20))
        .redirect(redirects)
        .accept_encoding([Encoding::Gzip, Encoding::Deflate])
        .websocket_http2(false)
        .websocket_max_message_size(1024 * 1024)
        .https_only(true)
        .build()
        .expect("builder accepts parity knobs");

    assert!(format!("{session}").contains(&Browser::Chrome147.to_string()));
    assert_eq!(session.stats().max_connections(), 8);
}
