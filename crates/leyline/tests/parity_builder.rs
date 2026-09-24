use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use leyline::{
    Browser, CompressionConfig, DnsConfig, NoProxy, PoolConfig, ProxyConfig, ProxyRule,
    RedirectAction, RedirectPolicy, Session, SocketConfig, TimeoutConfig, WebSocketConfig,
};

#[test]
fn builder_accepts_wreq_parity_transport_knobs() {
    let dns = DnsConfig::new().resolve_host(
        "example.test",
        [SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 443)],
    );
    let proxy = ProxyConfig::new()
        .with_rule(ProxyRule::https("http://127.0.0.1:8080"))
        .no_proxy(NoProxy::new(["localhost", ".internal"]))
        .without_env();
    let timeouts = TimeoutConfig::default()
        .total(Duration::from_secs(5))
        .connect(Duration::from_secs(1))
        .read(Duration::from_secs(2))
        .response_header(Duration::from_secs(3));
    let pool = PoolConfig::default()
        .idle_timeout(Duration::from_secs(30))
        .max_connections(8)
        .max_h1_conns_per_host(6)
        .keepalive(true);
    let socket = SocketConfig::default()
        .local_address(IpAddr::V4(Ipv4Addr::UNSPECIFIED))
        .tcp_nodelay(true)
        .tcp_keepalive(Duration::from_secs(20));
    let redirects = RedirectPolicy::custom(|attempt| {
        if attempt.status == 307 {
            RedirectAction::Stop
        } else {
            RedirectAction::Follow
        }
    });
    let compression = CompressionConfig::default()
        .gzip(true)
        .brotli(false)
        .deflate(true)
        .zstd(false);
    let websocket = WebSocketConfig::default()
        .prefer_http2(false)
        .max_message_size(1024 * 1024);

    let session = Session::builder()
        .browser(Browser::Chrome147)
        .proxy(proxy)
        .dns(dns)
        .timeout(timeouts)
        .pool(pool)
        .socket(socket)
        .redirect(redirects)
        .compression(compression)
        .websocket_config(websocket)
        .https_only(true)
        .build()
        .expect("builder accepts parity knobs");

    assert!(format!("{session}").contains(&Browser::Chrome147.to_string()));
    assert_eq!(session.pool_stats().max_connections, 8);
}
