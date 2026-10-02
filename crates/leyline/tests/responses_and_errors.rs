#[path = "core_support/raw_server.rs"]
mod raw_server;

use std::time::Duration;

use leyline::{
    BlockKind, BlockRules, Browser, CompressionConfig, ErrorCategory, Family, Platform,
    ProtocolPolicy, RelayBody, Session, TimeoutConfig,
};
use raw_server::{RawResponse, RawServer};
use tokio::net::TcpListener;

fn plain() -> Session {
    Session::builder().build().unwrap()
}

#[tokio::test]
async fn one_category_classifies_each_failure() {
    let silent = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let silent_port = silent.local_addr().unwrap().port();
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = silent.accept().await {
            held.push(socket);
        }
    });
    let timeout = Session::builder()
        .timeout(TimeoutConfig::new().total(Duration::from_millis(200)))
        .build()
        .unwrap()
        .get(format!("http://127.0.0.1:{silent_port}/"))
        .await
        .unwrap_err();
    assert_eq!(timeout.category(), ErrorCategory::Timeout);
    assert_eq!(
        timeout.category().gateway_status().map(|s| s.as_u16()),
        Some(504)
    );

    let closed = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let closed_port = closed.local_addr().unwrap().port();
    drop(closed);
    let refused = plain()
        .get(format!("http://127.0.0.1:{closed_port}/"))
        .await
        .unwrap_err();
    assert_eq!(refused.category(), ErrorCategory::Connect);
    assert_eq!(
        refused.category().gateway_status().map(|s| s.as_u16()),
        Some(502)
    );

    let dns = plain()
        .get("http://no-such-host.invalid/")
        .await
        .unwrap_err();
    assert!(dns.is_dns(), "{dns:?}");
    assert_eq!(dns.category(), ErrorCategory::Dns);

    let url = plain().get("not a url").await.unwrap_err();
    assert_eq!(url.category(), ErrorCategory::Url);
    assert_eq!(
        url.category().gateway_status().map(|s| s.as_u16()),
        Some(500)
    );

    let server = RawServer::start(vec![
        RawResponse::status(404, "Not Found"),
        RawResponse::ok().body(vec![b'x'; 4096]),
    ])
    .await;
    let status = plain()
        .get(server.url("/missing"))
        .await
        .unwrap()
        .error_for_status()
        .unwrap_err();
    assert_eq!(status.category(), ErrorCategory::Status);
    assert_eq!(status.category().gateway_status(), None);
    let big = Session::builder()
        .compression(CompressionConfig::default().max_body_size(1024))
        .build()
        .unwrap()
        .get(server.url("/big"))
        .await
        .unwrap_err();
    assert_eq!(big.category(), ErrorCategory::BodyLimit);
}

#[tokio::test]
async fn link_headers_resolve_against_the_response_url() {
    let server = RawServer::start(vec![
        RawResponse::ok()
            .header(
                "link",
                "</items?page=2,3>; rel=\"next last\", <https://other.example/p>; rel=prev",
            )
            .header("link", "<../up>; rel=up"),
    ])
    .await;
    let resp = plain().get(server.url("/api/items")).await.unwrap();
    let next = server.url("/items?page=2,3");
    assert_eq!(resp.link("next").map(String::from), Some(next.clone()));
    assert_eq!(resp.link("LAST").map(String::from), Some(next));
    assert_eq!(
        resp.link("prev").map(String::from).as_deref(),
        Some("https://other.example/p")
    );
    assert_eq!(resp.link("up").map(String::from), Some(server.url("/up")));
    assert_eq!(resp.links().len(), 3);
}

#[tokio::test]
async fn relay_headers_drop_hop_by_hop_fields() {
    let server = RawServer::start(vec![
        RawResponse::ok()
            .header("connection", "x-hop")
            .header("keep-alive", "timeout=5")
            .header("x-hop", "1")
            .header("x-keep", "2"),
    ])
    .await;
    let headers = plain()
        .get(server.url("/"))
        .await
        .unwrap()
        .relay_headers(RelayBody::AsReceived);
    assert!(headers.contains_key("x-keep"));
    for hop in ["connection", "keep-alive", "x-hop"] {
        assert!(!headers.contains_key(hop), "{hop}");
    }
}

#[tokio::test]
async fn relay_headers_describe_the_body_you_forward() {
    use std::io::Write;
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(b"decoded body").unwrap();
    let server = RawServer::start(vec![
        RawResponse::ok()
            .header("content-encoding", "gzip")
            .body(encoder.finish().unwrap()),
    ])
    .await;
    let resp = plain().get(server.url("/")).stream().send().await.unwrap();
    let raw = resp.relay_headers(RelayBody::AsReceived);
    assert_eq!(raw.get("content-encoding").unwrap(), "gzip");
    assert!(raw.contains_key("content-length"));
    let decoded = resp.relay_headers(RelayBody::Decoded);
    assert!(!decoded.contains_key("content-encoding"));
    assert!(!decoded.contains_key("content-length"));
    assert_eq!(resp.text().await.unwrap(), "decoded body");
}

#[tokio::test]
async fn challenge_pages_are_detected_from_data() {
    let server = RawServer::start(vec![
        RawResponse::status(403, "Forbidden").header("cf-mitigated", "challenge"),
        RawResponse::status(429, "Too Many Requests").header("x-shield", "blocked"),
    ])
    .await;
    let challenge = plain().get(server.url("/a")).await.unwrap();
    let signal = challenge.block().unwrap();
    assert_eq!(signal.vendor, "cloudflare");
    assert_eq!(signal.kind, BlockKind::Challenge);

    let custom = BlockRules::from_toml(
        "[[rule]]\nvendor = \"in-house\"\nkind = \"block\"\nstatus = 429\nheader = \"x-shield\"\nvalue = \"blocked\"\n",
    )
    .unwrap();
    let blocked = plain().get(server.url("/b")).await.unwrap();
    assert_eq!(blocked.block(), None);
    let signal = custom.check(&blocked).unwrap();
    assert_eq!(
        (signal.vendor.as_str(), signal.kind),
        ("in-house", BlockKind::Block)
    );
}

#[tokio::test]
async fn a_plain_session_sends_no_language_and_browsers_format_theirs() {
    let mut server = RawServer::start(vec![RawResponse::ok(); 5]).await;
    plain().get(server.url("/plain")).await.unwrap();
    let langs = ["de-DE", "de", "en"];
    for (browser, platform) in [
        (Browser::latest(Family::Chrome), Platform::Windows),
        (Browser::latest(Family::Firefox), Platform::Windows),
        (Browser::latest(Family::Safari), Platform::MacOS),
    ] {
        Session::builder()
            .browser(browser)
            .platform(platform)
            .protocol(ProtocolPolicy::Http1)
            .languages(langs)
            .build()
            .unwrap()
            .get(server.url("/browser"))
            .await
            .unwrap();
    }
    Session::builder()
        .browser(Browser::latest(Family::Chrome))
        .protocol(ProtocolPolicy::Http1)
        .languages(["en-GB", "fr"])
        .build()
        .unwrap()
        .get(server.url("/expand"))
        .await
        .unwrap();

    assert_eq!(
        server.next_request().await.header_count("accept-language"),
        0
    );
    let expected = [
        "de-DE,de;q=0.9,en;q=0.8",
        "de-DE,de;q=0.9,en;q=0.8",
        "de-DE",
        "en-GB,en;q=0.9,fr;q=0.8",
    ];
    for want in expected {
        assert_eq!(
            server.next_request().await.header_values("accept-language"),
            [want]
        );
    }
}
