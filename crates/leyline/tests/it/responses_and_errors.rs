use std::time::Duration;

use leyline::testing::{self, queue};
use leyline::{Block, Browser, Kind, Platform, Protocol, Session, Timeout};
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
        .timeout(Timeout::new().total(Duration::from_millis(200)))
        .build()
        .unwrap()
        .get(format!("http://127.0.0.1:{silent_port}/"))
        .send()
        .await
        .unwrap_err();
    assert_eq!(timeout.kind(), Kind::Timeout);
    assert_eq!(
        timeout.kind().gateway_status().map(|s| s.as_u16()),
        Some(504)
    );

    let closed = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let closed_port = closed.local_addr().unwrap().port();
    drop(closed);
    let refused = plain()
        .get(format!("http://127.0.0.1:{closed_port}/"))
        .send()
        .await
        .unwrap_err();
    assert_eq!(refused.kind(), Kind::Connect);
    assert_eq!(
        refused.kind().gateway_status().map(|s| s.as_u16()),
        Some(502)
    );

    let dns = plain()
        .get("http://no-such-host.invalid/")
        .send()
        .await
        .unwrap_err();
    assert!(dns.kind() == Kind::Dns, "{dns:?}");
    assert_eq!(dns.kind(), Kind::Dns);

    let url = plain().get("not a url").send().await.unwrap_err();
    assert_eq!(url.kind(), Kind::Url);
    assert_eq!(url.kind().gateway_status().map(|s| s.as_u16()), Some(500));

    let server = testing::Server::http(queue(vec![
        testing::Response::new(404).close(),
        testing::Response::new(200).close().body(vec![b'x'; 4096]),
    ]))
    .await
    .unwrap();
    let status = plain()
        .get(server.url("/missing"))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap_err();
    assert_eq!(status.kind(), Kind::Status);
    assert_eq!(status.kind().gateway_status(), None);
    let big = Session::builder()
        .max_body_size(1024)
        .build()
        .unwrap()
        .get(server.url("/big"))
        .send()
        .await
        .unwrap_err();
    assert_eq!(big.kind(), Kind::BodyLimit);
}

#[tokio::test]
async fn link_headers_resolve_against_the_response_url() {
    let server = testing::Server::http(queue(vec![
        testing::Response::new(200)
            .body("ok")
            .close()
            .header(
                "link",
                "</items?page=2,3>; rel=\"next last\", <https://other.example/p>; rel=prev",
            )
            .header("link", "<../up>; rel=up"),
    ]))
    .await
    .unwrap();
    let resp = plain().get(server.url("/api/items")).send().await.unwrap();
    let next = server.url("/items?page=2,3");
    let url = |rel: &str| {
        resp.links()
            .into_iter()
            .find(|link| link.has_rel(rel))
            .map(|link| String::from(link.url))
    };
    assert_eq!(url("next"), Some(next.clone()));
    assert_eq!(url("LAST"), Some(next));
    assert_eq!(url("prev").as_deref(), Some("https://other.example/p"));
    assert_eq!(url("up"), Some(server.url("/up")));
    assert_eq!(resp.links().len(), 3);
}

#[tokio::test]
async fn relay_headers_drop_hop_by_hop_fields() {
    let server = testing::Server::http(queue(vec![
        testing::Response::new(200)
            .body("ok")
            .close()
            .header("connection", "x-hop")
            .header("keep-alive", "timeout=5")
            .header("x-hop", "1")
            .header("x-keep", "2"),
    ]))
    .await
    .unwrap();
    let headers = plain()
        .get(server.url("/"))
        .send()
        .await
        .unwrap()
        .relay_headers();
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
    let gzipped = encoder.finish().unwrap();
    let reply = || {
        testing::Response::new(200)
            .close()
            .header("content-encoding", "gzip")
            .body(gzipped.clone())
    };
    let server = testing::Server::http(queue(vec![reply(), reply()]))
        .await
        .unwrap();
    let session = plain();
    let resp = session.get(server.url("/")).stream().send().await.unwrap();
    let decoded = resp.relay_headers();
    assert!(!decoded.contains_key("content-encoding"));
    assert!(!decoded.contains_key("content-length"));
    assert_eq!(resp.text().await.unwrap(), "decoded body");
    let resp = session
        .get(server.url("/"))
        .stream()
        .decode(false)
        .send()
        .await
        .unwrap();
    let raw = resp.relay_headers();
    assert_eq!(raw.get("content-encoding").unwrap(), "gzip");
    assert!(raw.contains_key("content-length"));
    assert_eq!(resp.bytes().await.unwrap(), gzipped);
}

#[tokio::test]
async fn decode_false_keeps_a_buffered_body_raw_and_the_offer_unchanged() {
    use std::io::Write;
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(b"decoded body").unwrap();
    let gzipped = encoder.finish().unwrap();
    let reply = || {
        testing::Response::new(200)
            .close()
            .header("content-encoding", "gzip")
            .body(gzipped.clone())
    };
    let server = testing::Server::http(queue(vec![reply(), reply()]))
        .await
        .unwrap();
    let session = plain();
    let decoded = session.get(server.url("/")).send().await.unwrap();
    let raw = session
        .get(server.url("/"))
        .decode(false)
        .send()
        .await
        .unwrap();
    assert_eq!(raw.header("content-encoding"), Some("gzip"));
    assert_eq!(decoded.text().await.unwrap(), "decoded body");
    assert_eq!(raw.bytes().await.unwrap(), gzipped);
    let offer = |request: leyline::testing::Request| {
        request
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("accept-encoding"))
            .map(|(_, value)| value.clone())
    };
    let first = offer(server.next_request().await.unwrap());
    let second = offer(server.next_request().await.unwrap());
    assert!(first.is_some());
    assert_eq!(first, second);
}

#[tokio::test]
async fn challenge_pages_are_detected_from_data() {
    let server = testing::Server::http(queue(vec![
        testing::Response::new(403)
            .close()
            .header("cf-mitigated", "challenge"),
        testing::Response::new(429)
            .close()
            .header("x-shield", "blocked"),
    ]))
    .await
    .unwrap();
    let challenge = plain().get(server.url("/a")).send().await.unwrap();
    assert_eq!(
        challenge.block(),
        Some(&Block::Challenge("cloudflare".into()))
    );

    let custom = Session::builder()
        .block_rules(
            "[[rule]]\nvendor = \"in-house\"\nkind = \"block\"\nstatus = 429\nheader = \"x-shield\"\nvalue = \"blocked\"\n",
        )
        .build()
        .unwrap();
    let blocked = custom.get(server.url("/b")).send().await.unwrap();
    assert_eq!(blocked.block(), Some(&Block::Denied("in-house".into())));
}

#[tokio::test]
async fn a_plain_session_sends_no_language_and_browsers_format_theirs() {
    let server = testing::Server::http(queue(vec![
        testing::Response::new(200).body("ok").close();
        5
    ]))
    .await
    .unwrap();
    plain().get(server.url("/plain")).send().await.unwrap();
    let langs = ["de-DE", "de", "en"];
    for (browser, platform) in [
        (Browser::chrome(), Platform::Windows),
        (Browser::firefox(), Platform::Windows),
        (Browser::safari(), Platform::MacOS),
    ] {
        Session::builder()
            .browser(browser)
            .platform(platform)
            .languages(langs)
            .protocol(Protocol::Http1)
            .build()
            .unwrap()
            .get(server.url("/browser"))
            .send()
            .await
            .unwrap();
    }
    Session::builder()
        .browser(Browser::chrome())
        .languages(["en-GB", "fr"])
        .protocol(Protocol::Http1)
        .build()
        .unwrap()
        .get(server.url("/expand"))
        .send()
        .await
        .unwrap();

    assert_eq!(
        server
            .next_request()
            .await
            .unwrap()
            .header_values("accept-language")
            .len(),
        0
    );
    let expected = [
        "de-DE,de;q=0.9,en;q=0.8",
        "de-DE,de;q=0.7,en;q=0.3",
        "de-DE",
        "en-GB,en;q=0.9,fr;q=0.8",
    ];
    for want in expected {
        assert_eq!(
            server
                .next_request()
                .await
                .unwrap()
                .header_values("accept-language"),
            [want]
        );
    }
}

#[tokio::test]
async fn a_status_error_leaves_the_response_readable() {
    let server = testing::Server::http(queue(vec![
        testing::Response::new(429).close().body("slow down"),
    ]))
    .await
    .unwrap();
    let resp = plain().get(server.url("/")).send().await.unwrap();
    let err = resp.error_for_status().unwrap_err();
    assert_eq!(err.status().map(|s| s.as_u16()), Some(429));
    assert_eq!(resp.text().await.unwrap(), "slow down");
}

#[test]
fn invalid_block_rules_fail_at_build() {
    let error = Session::builder()
        .block_rules("[[rule]]\nvendor = \"shop\"\nkind = \"teapot\"\nstatus = 403\n")
        .build()
        .unwrap_err();
    assert_eq!(error.kind(), Kind::Config);
}
