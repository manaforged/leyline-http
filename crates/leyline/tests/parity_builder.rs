//! Non-network smoke coverage for the wreq-parity builder surface.
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
    let timeouts = TimeoutConfig {
        total: Duration::from_secs(5),
        connect: Some(Duration::from_secs(1)),
        read: Some(Duration::from_secs(2)),
        response_header: Some(Duration::from_secs(3)),
    };
    let pool = PoolConfig {
        idle_timeout: Duration::from_secs(30),
        max_connections: 8,
        max_h1_conns_per_host: 6,
        keepalive: true,
    };
    let socket = SocketConfig {
        local_address: Some(IpAddr::V4(Ipv4Addr::UNSPECIFIED)),
        tcp_nodelay: Some(true),
        tcp_keepalive: Some(Duration::from_secs(20)),
        ..SocketConfig::default()
    };
    let redirects = RedirectPolicy::custom(|attempt| {
        if attempt.status == 307 {
            RedirectAction::Stop
        } else {
            RedirectAction::Follow
        }
    });
    let compression = CompressionConfig {
        gzip: true,
        brotli: false,
        deflate: true,
        zstd: false,
    };
    let websocket = WebSocketConfig {
        prefer_http2: false,
        max_message_size: Some(1024 * 1024),
        ..WebSocketConfig::default()
    };

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
fn legacy_builder_methods_feed_new_configs() {
    let session = Session::builder()
        .proxy("http://127.0.0.1:8080")
        .disable_env_proxies()
        .timeout(Duration::from_secs(7))
        .connect_timeout(Duration::from_millis(500))
        .read_timeout(Duration::from_millis(750))
        .pool_idle_timeout(Duration::from_secs(11))
        .pool_limits(Duration::from_secs(12), 3)
        .max_redirects(0)
        .tcp_nodelay(true)
        .tcp_keepalive(Duration::from_secs(9))
        .build()
        .expect("legacy shorthands remain valid");

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

/// The request builder must be owned + `Send` so it can be built up front
/// and moved into a `tokio::spawn` / stored in a struct — the common
/// fan-out/worker pattern. Before the `Arc<SessionInner>` refactor the
/// builder borrowed `&Session` and this would not compile.
#[test]
fn request_builder_is_send_and_movable_into_spawn() {
    fn assert_send<T: Send>(_: &T) {}

    let session = Session::builder()
        .disable_env_proxies()
        .build()
        .expect("session builds");

    // Build the request, then move it across a thread/task boundary.
    let req = session
        .post("https://example.test/")
        .header("x-worker", "1")
        .body("payload");
    assert_send(&req);

    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    rt.block_on(async move {
        // `req` is owned + Send: a spawned task can take it. The await
        // fails fast (no network) but it must *compile and move*, which is
        // the property under test.
        let handle = tokio::spawn(async move {
            let _ = req.timeout(Duration::from_millis(10)).send().await;
        });
        let _ = handle.await;
    });

    // The session is independently usable afterwards — the builder owned a
    // cheap Arc clone, it did not borrow the session.
    assert!(session.pool_stats().max_connections >= 1);
}
