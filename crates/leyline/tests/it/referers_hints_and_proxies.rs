use std::time::Duration;

use leyline::testing::{TestResponse, TestServer, queue};
use leyline::{
    Browser, CompressionConfig, PoolConfig, Preset, ProtocolPolicy, Session, TimeoutConfig,
};
#[cfg(feature = "socks")]
use leyline::{ProxyReply, TlsError};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

fn chrome() -> Session {
    Session::builder()
        .browser(Browser::default())
        .protocol(ProtocolPolicy::Http1)
        .build()
        .unwrap()
}

fn plain() -> Session {
    Session::builder().build().unwrap()
}

#[tokio::test]
async fn https_page_to_http_target_is_cross_site_without_referer() {
    let server = TestServer::http(queue(vec![TestResponse::new(200).body("ok").close()]))
        .await
        .unwrap();
    chrome()
        .get(server.url("/api"))
        .preset(Preset::Xhr)
        .initiator("https://shop.example/product")
        .await
        .unwrap();
    let req = server.next_request().await.unwrap();
    assert!(req.header_values("referer").is_empty(), "{}", req.text());
    assert_eq!(req.header_values("origin"), ["https://shop.example"]);
    assert_eq!(req.header_values("sec-fetch-site"), ["cross-site"]);
}

#[tokio::test]
async fn link_navigation_differs_from_a_typed_url() {
    let server = TestServer::http(queue(vec![
        TestResponse::new(200).body("ok").close(),
        TestResponse::new(200).body("ok").close(),
    ]))
    .await
    .unwrap();
    let session = chrome();
    session
        .get(server.url("/next"))
        .initiator(server.url("/page"))
        .await
        .unwrap();
    session.get(server.url("/typed")).await.unwrap();

    let link = server.next_request().await.unwrap();
    assert_eq!(link.header_values("sec-fetch-site"), ["same-origin"]);
    assert_eq!(link.header_values("referer"), [server.url("/page")]);
    let names: Vec<&str> = link.headers.iter().map(|(n, _)| n.as_str()).collect();
    let at = |n: &str| {
        names
            .iter()
            .position(|h| h.eq_ignore_ascii_case(n))
            .unwrap()
    };
    assert_eq!(at("referer"), at("sec-fetch-dest") + 1, "{}", link.text());

    let typed = server.next_request().await.unwrap();
    assert_eq!(typed.header_values("sec-fetch-site"), ["none"]);
    assert!(typed.header_values("referer").is_empty());
}

#[tokio::test]
async fn plain_session_initiator_still_sends_a_referer() {
    let server = TestServer::http(queue(vec![TestResponse::new(200).body("ok").close()]))
        .await
        .unwrap();
    plain()
        .get(server.url("/x"))
        .initiator("http://shop.example/cart")
        .await
        .unwrap();
    assert_eq!(
        server
            .next_request()
            .await
            .unwrap()
            .header_values("referer"),
        ["http://shop.example/"]
    );
}

#[tokio::test]
async fn plain_session_advertises_only_enabled_codecs() {
    let server = TestServer::http(queue(vec![
        TestResponse::new(200).body("ok").close(),
        TestResponse::new(200).body("ok").close(),
        TestResponse::new(200).body("ok").close(),
    ]))
    .await
    .unwrap();
    for config in [
        CompressionConfig::default(),
        CompressionConfig::default().brotli(false),
        CompressionConfig::none(),
    ] {
        Session::builder()
            .compression(config)
            .build()
            .unwrap()
            .get(server.url("/"))
            .await
            .unwrap();
    }
    assert_eq!(
        server
            .next_request()
            .await
            .unwrap()
            .header_values("accept-encoding"),
        ["gzip, deflate, br, zstd"]
    );
    assert_eq!(
        server
            .next_request()
            .await
            .unwrap()
            .header_values("accept-encoding"),
        ["gzip, deflate, zstd"]
    );
    assert_eq!(
        server
            .next_request()
            .await
            .unwrap()
            .header_count("accept-encoding"),
        0
    );
}

#[cfg(feature = "socks")]
#[tokio::test]
async fn socks5_proxy_that_cannot_reach_the_origin_is_not_a_proxy_failure() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut greeting = [0u8; 3];
        socket.read_exact(&mut greeting).await.unwrap();
        socket.write_all(&[5, 0]).await.unwrap();
        let mut request = [0u8; 512];
        let _ = socket.read(&mut request).await.unwrap();
        socket
            .write_all(&[5, 5, 0, 1, 0, 0, 0, 0, 0, 0])
            .await
            .unwrap();
    });
    let err = Session::builder()
        .proxy(format!("socks5://127.0.0.1:{port}"))
        .build()
        .unwrap()
        .get("http://origin.test/")
        .await
        .unwrap_err();
    assert!(!err.is_proxy(), "{err:?}");
    assert!(
        matches!(
            err.tls(),
            Some(TlsError::ProxyTargetUnreachable {
                reply: ProxyReply::Socks5(5),
                ..
            })
        ),
        "{err:?}"
    );
}

#[tokio::test]
async fn a_proxy_that_never_answers_connect_is_a_proxy_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            held.push(socket);
        }
    });
    let err = Session::builder()
        .browser(Browser::default())
        .proxy(format!("http://127.0.0.1:{port}"))
        .timeout(
            TimeoutConfig::new()
                .connect(Duration::from_millis(300))
                .total(Duration::from_secs(5)),
        )
        .build()
        .unwrap()
        .get("https://origin.test/")
        .await
        .unwrap_err();
    assert!(err.is_proxy(), "{err:?}");
    assert!(err.is_timeout(), "{err:?}");
}

#[tokio::test]
async fn a_caller_limit_names_itself() {
    let server = TestServer::http(queue(vec![
        TestResponse::new(200).close().body(vec![b'x'; 1_000]),
    ]))
    .await
    .unwrap();
    let mut out = Vec::new();
    let err = plain()
        .get(server.url("/"))
        .stream()
        .await
        .unwrap()
        .copy_decoded_to(&mut out, Some(100))
        .await
        .unwrap_err();
    assert!(err.is_body_limit());
    assert!(err.to_string().contains("caller"), "{err}");
}

#[tokio::test]
async fn dropping_a_streamed_http1_response_frees_the_host_slot() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut stalled, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 2048];
        let _ = stalled.read(&mut buf).await.unwrap();
        stalled
            .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 100000\r\n\r\n")
            .await
            .unwrap();
        let (mut next, _) = listener.accept().await.unwrap();
        let _ = next.read(&mut buf).await.unwrap();
        next.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nok")
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_secs(30)).await;
        drop(stalled);
    });
    let session = Session::builder()
        .protocol(ProtocolPolicy::Http1)
        .pool(PoolConfig::new().max_h1_conns_per_host(1))
        .build()
        .unwrap();
    let first = session
        .get(format!("http://{addr}/stall"))
        .stream()
        .await
        .unwrap();
    drop(first);
    let second = tokio::time::timeout(
        Duration::from_secs(3),
        session.get(format!("http://{addr}/next")),
    )
    .await
    .expect("the dropped response must release the per-host slot")
    .unwrap();
    assert_eq!(second.text().await.unwrap(), "ok");
}

#[cfg(feature = "tower")]
#[tokio::test]
async fn tower_service_streams_with_proxy_and_metadata() {
    use tower_service::Service;
    let proxy = TestServer::http(queue(vec![TestResponse::new(200).body("ok").close()]))
        .await
        .unwrap();
    let mut service = leyline::LeylineService::new(plain());
    let mut request = leyline::http::Request::get("http://origin.test/thing")
        .body(leyline::Body::default())
        .unwrap();
    request
        .extensions_mut()
        .insert(leyline::ProxyConfig::from(format!(
            "http://{}",
            proxy.addr()
        )));
    let response = service.call(request).await.unwrap();
    assert_eq!(response.status().as_u16(), 200);
    assert_eq!(
        response
            .extensions()
            .get::<url::Url>()
            .map(url::Url::as_str),
        Some("http://origin.test/thing")
    );
    assert_eq!(
        response.extensions().get::<leyline::HttpVersion>(),
        Some(&leyline::HttpVersion::Http1_1)
    );
    assert!(
        response
            .extensions()
            .get::<leyline::ResponseTiming>()
            .is_some()
    );
    assert_eq!(
        proxy.next_request().await.unwrap().request_line,
        "GET http://origin.test/thing HTTP/1.1"
    );
}
