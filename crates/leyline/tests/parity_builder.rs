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
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 443),
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
        .proxies(proxy)
        .dns(dns)
        .timeouts(timeouts)
        .pool_config(pool)
        .socket_config(socket)
        .redirect_policy(redirects)
        .compression(compression)
        .websocket_config(websocket)
        .https_only(true)
        .build()
        .expect("builder accepts parity knobs");

    assert_eq!(session.browser(), Some(Browser::Chrome147));
    assert_eq!(session.default_timeout(), Duration::from_secs(5));
    assert_eq!(session.pool_stats().max_connections, 8);
}

#[test]
fn config_structs_feed_the_session() {
    let session = Session::builder()
        .proxy("http://127.0.0.1:8080")
        .disable_env_proxies()
        .timeouts(
            leyline::TimeoutConfig::default()
                .total(Duration::from_secs(7))
                .connect(Duration::from_millis(500))
                .read(Duration::from_millis(750)),
        )
        .pool_config(
            leyline::PoolConfig::default()
                .idle_timeout(Duration::from_secs(12))
                .max_connections(3),
        )
        .max_redirects(0)
        .socket_config(
            leyline::SocketConfig::default()
                .tcp_nodelay(true)
                .tcp_keepalive(Duration::from_secs(9)),
        )
        .build()
        .expect("config structs build a session");

    assert_eq!(session.default_timeout(), Duration::from_secs(7));
    assert_eq!(session.pool_stats().max_connections, 3);
}

#[test]
fn default_session_timeout_is_five_minutes() {
    let session = Session::builder()
        .disable_env_proxies()
        .build()
        .expect("default session builds");

    assert_eq!(session.default_timeout(), Duration::from_secs(300));
}

#[test]
fn request_builder_is_send_and_movable_into_spawn() {
    fn assert_send<T: Send>(_: &T) {}

    let session = Session::builder()
        .disable_env_proxies()
        .build()
        .expect("session builds");

    let req = session
        .post("https://example.test/")
        .header("x-worker", "1")
        .body("payload");
    assert_send(&req);

    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    rt.block_on(async move {
        let handle = tokio::spawn(async move {
            let _ = req.timeout(Duration::from_millis(10)).send().await;
        });
        let _ = handle.await;
    });

    assert!(session.pool_stats().max_connections >= 1);
}
