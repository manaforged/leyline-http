use std::error::Error as StdError;
use std::fmt;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use bytes::Bytes;
use leyline::cookie::{Cookie, Jar, SameSite};
use leyline::http::StatusCode;
use leyline::profile::TcpProfile;
use leyline::testing::{self, Server};
use leyline::{Body, Browser, Error, Kind, Platform, Retry, Session, Tls};

type TestResult = Result<(), Box<dyn StdError>>;

#[derive(Debug, PartialEq, Eq)]
struct SigningError(&'static str);

impl fmt::Display for SigningError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl StdError for SigningError {}

fn scratch_dir(name: &str) -> io::Result<PathBuf> {
    let dir = std::env::temp_dir().join(format!("leyline-unlocks-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn der_to_pem(der: &[u8]) -> Vec<u8> {
    let mut pem = String::from("-----BEGIN CERTIFICATE-----\n");
    for chunk in STANDARD.encode(der).as_bytes().chunks(64) {
        for &b in chunk {
            pem.push(b as char);
        }
        pem.push('\n');
    }
    pem.push_str("-----END CERTIFICATE-----\n");
    pem.into_bytes()
}

#[tokio::test]
async fn before_send_and_refresh_token_propagate_request_errors_without_retrying() -> TestResult {
    let server = Server::http(|req| {
        if req.header("authorization") == Some("Bearer stale") {
            testing::Response::new(401).body("expired")
        } else {
            testing::Response::new(200).body("ok")
        }
    })
    .await?;

    let hook_runs = Arc::new(AtomicUsize::new(0));
    let runs_seen = Arc::clone(&hook_runs);
    let session = Session::builder()
        .retry(Retry::new(3).any_method(true))
        .before_send(move |_| {
            runs_seen.fetch_add(1, Ordering::SeqCst);
            Err(Error::request(SigningError("missing hmac key")))
        })
        .build()?;
    let Err(err) = session
        .post(server.url("/sign"))
        .body("payload")
        .send()
        .await
    else {
        return Err("hook rejection must fail request".into());
    };
    assert_eq!(hook_runs.load(Ordering::SeqCst), 1);
    assert!(server.requests().await.is_empty());
    assert_eq!(err.kind(), Kind::Request);
    assert_eq!(
        err.kind().gateway_status(),
        Some(StatusCode::INTERNAL_SERVER_ERROR)
    );
    assert_eq!(err.attempts(), 1);
    assert!(!err.is_retryable());
    assert!(!err.is_timeout());
    assert!(!err.is_connect());
    assert!(err.url().is_none());
    assert_eq!(err.to_string(), "request");
    let cause = err
        .source()
        .and_then(|src| src.downcast_ref::<SigningError>());
    assert_eq!(cause, Some(&SigningError("missing hmac key")));

    assert_io_request_stays_non_retryable();
    assert_refresh_failure_keeps_request_kind(&server).await
}

fn assert_io_request_stays_non_retryable() {
    for kind in [
        io::ErrorKind::TimedOut,
        io::ErrorKind::UnexpectedEof,
        io::ErrorKind::ConnectionRefused,
    ] {
        let err = Error::request(io::Error::new(kind, "local key agent failure"));
        assert_eq!(err.kind(), Kind::Request, "kind={kind:?}");
        assert!(!err.is_timeout(), "kind={kind:?}");
        assert!(!err.is_connect(), "kind={kind:?}");
        assert!(!err.is_retryable(), "kind={kind:?}");
    }
}

async fn assert_refresh_failure_keeps_request_kind(server: &Server) -> TestResult {
    let session = Session::builder()
        .base_url(server.url("/"))
        .bearer_auth("stale")
        .refresh_token(|| async { Err(Error::request("vault token expired")) })
        .build()?;
    let Err(err) = session.get("/items").send().await else {
        return Err("failed refresh must fail request".into());
    };
    assert_eq!(err.kind(), Kind::Request);
    assert!(!err.is_retryable());
    let refresh_source = err.source().and_then(StdError::source);
    assert_eq!(
        refresh_source.map(ToString::to_string).as_deref(),
        Some("vault token expired")
    );
    Ok(())
}

#[tokio::test]
async fn tcp_profile_toml_and_ja4t_layer_drive_session_fingerprints() -> TestResult {
    assert_eq!(TcpProfile::new(), TcpProfile::default());
    assert_eq!(TcpProfile::new().ja4t(), "0__0_0");

    let parsed: TcpProfile =
        "ttl = 64\nmss = 1380\nwindow_size = 64240\ndf = true\nwindow_scale = 10\nno_delay = true\noptions = [2, 4, 8, 1, 3]"
            .parse()?;
    assert_eq!(parsed.ttl(), 64);
    assert_eq!(parsed.mss(), 1380);
    assert_eq!(parsed.window_size(), 64240);
    assert!(parsed.df());
    assert_eq!(parsed.window_scale(), 10);
    assert!(parsed.no_delay());
    assert_eq!(parsed.options(), &[2, 4, 8, 1, 3]);
    assert_eq!(parsed.ja4t(), "64240_2-4-8-1-3_1380_10");

    let linux = Platform::Linux.tcp_profile();
    let json = serde_json::to_string(&linux)?;
    let decoded: TcpProfile = serde_json::from_str(&json)?;
    assert_eq!(decoded, linux);

    assert_invalid_tcp_inputs_fail_with_config()?;
    assert_ja4t_sessions_report_layered_fingerprints().await
}

fn assert_invalid_tcp_inputs_fail_with_config() -> TestResult {
    for bad_toml in ["bogus = 1", "ttl = 256", "mss = 70000", "window_scale = 15"] {
        let Err(err) = bad_toml.parse::<TcpProfile>() else {
            return Err(format!("invalid TCP TOML {bad_toml:?} succeeded").into());
        };
        assert_eq!(err.kind(), Kind::Config, "{bad_toml}");
    }
    for bad_ja4t in [
        "64240_2-4_1460",
        "64240_2-999_1460_8",
        "64240_2_70000_8",
        "64240_2_1460_15",
    ] {
        let Err(err) = TcpProfile::from_ja4t(bad_ja4t) else {
            return Err(format!("invalid JA4T {bad_ja4t:?} succeeded").into());
        };
        assert_eq!(err.kind(), Kind::Config, "{bad_ja4t}");
    }
    Ok(())
}

async fn assert_ja4t_sessions_report_layered_fingerprints() -> TestResult {
    let server = Server::https(|_| testing::Response::new(200).body("ok")).await?;
    let before_platform = Session::builder()
        .tls(server.tls())
        .tcp_profile(TcpProfile::from_ja4t("64240_2-4-8-1-3_1380_10")?)
        .platform(Platform::Linux)
        .audit(true)
        .build()?;
    let resp = before_platform.get(server.url("/a")).send().await?;
    assert_eq!(
        resp.fingerprint().ok_or("audit enabled")?.ja4t(),
        "64240_2-4-8-1-3_1380_10"
    );

    let child = before_platform
        .to_builder()
        .tcp_profile(TcpProfile::from_ja4t("32768__1200_7")?)
        .build()?;
    let child_resp = child.get(server.url("/b")).send().await?;
    assert_eq!(
        child_resp.fingerprint().ok_or("audit enabled")?.ja4t(),
        "32768__1200_7"
    );

    let replaced = Session::builder()
        .tls(server.tls())
        .browser(Browser::Chrome154)
        .tcp_profile(TcpProfile::from_ja4t("32768__1200_7")?)
        .tcp_profile(Platform::Windows.tcp_profile())
        .audit(true)
        .build()?;
    let replaced_resp = replaced.get(server.url("/c")).send().await?;
    assert_eq!(
        replaced_resp.fingerprint().ok_or("audit enabled")?.ja4t(),
        Platform::Windows.tcp_profile().ja4t()
    );
    Ok(())
}

#[test]
fn https_on_and_ca_pem_serve_and_trust_in_memory_roots() -> TestResult {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let bound = listener.local_addr()?;
    let server = Server::https_on(listener, |_| testing::Response::new(200).body("trusted"))?;
    assert_eq!(server.addr(), bound);

    let pem = der_to_pem(server.ca_der().ok_or("https server exposes ca_der")?);
    let trust = Tls::new().system_roots(false).env_roots(false).ca_pem(pem);
    let debug = format!("{trust:?}");
    assert!(debug.contains("Bytes(***)"), "{debug}");
    assert!(!debug.contains("BEGIN CERTIFICATE"), "{debug}");

    for invalid in [&b""[..], &b"not a certificate"[..]] {
        let Err(err) = Session::builder()
            .tls(
                Tls::new()
                    .system_roots(false)
                    .env_roots(false)
                    .ca_pem(invalid),
            )
            .build_blocking()
        else {
            return Err("garbage ca_pem must fail build".into());
        };
        assert_eq!(err.kind(), Kind::Config);
    }

    let untrusted = Session::builder()
        .tls(Tls::new().system_roots(false).env_roots(false))
        .build_blocking()?;
    let Err(tls_err) = untrusted.block_on(untrusted.get(server.url("/tls")).text()) else {
        return Err("untrusted root must fail handshake".into());
    };
    assert_eq!(tls_err.kind(), Kind::Tls);

    let first = Session::builder().tls(trust.clone()).build_blocking()?;
    let second = Session::builder().tls(trust).build_blocking()?;
    assert_eq!(
        first.block_on(first.get(server.url("/1")).text())?,
        "trusted"
    );
    assert_eq!(
        second.block_on(second.get(server.url("/2")).text())?,
        "trusted"
    );
    Ok(())
}

#[tokio::test]
async fn body_content_length_reports_buffered_file_and_stream_lengths_to_hooks() -> TestResult {
    assert_eq!(Body::default().content_length(), Some(0));
    assert_eq!(Body::from("hello").content_length(), Some(5));

    let dir = scratch_dir("body-len")?;
    let file_path = dir.join("payload.bin");
    std::fs::write(&file_path, b"ninebytes")?;
    let file_body = Body::file(&file_path)?;
    assert!(file_body.as_bytes().is_none());
    assert_eq!(file_body.content_length(), Some(9));

    let sized = Body::stream(
        futures_util::stream::iter([Ok(Bytes::from_static(b"abcd"))]),
        Some(4),
    );
    assert_eq!(sized.content_length(), Some(4));
    let unsized_stream = Body::stream(
        futures_util::stream::iter([Ok(Bytes::from_static(b"abcd"))]),
        None,
    );
    assert_eq!(unsized_stream.content_length(), None);

    let server = Server::http(|req| {
        let seen = req
            .header("x-payload-bytes")
            .unwrap_or("missing")
            .to_owned();
        testing::Response::new(200).body(seen)
    })
    .await?;
    let session = Session::builder()
        .before_send(|req| {
            let label = match req.body().content_length() {
                Some(len) => len.to_string(),
                None => String::from("chunked"),
            };
            req.headers_mut()
                .insert("x-payload-bytes", label.parse().map_err(Error::request)?);
            Ok(())
        })
        .build()?;
    let file_reply = session
        .put(server.url("/upload"))
        .body(Body::file(&file_path)?)
        .text()
        .await?;
    assert_eq!(file_reply, "9");
    let stream_reply = session
        .put(server.url("/stream"))
        .body(unsized_stream)
        .text()
        .await?;
    assert_eq!(stream_reply, "chunked");
    drop(std::fs::remove_dir_all(&dir));
    Ok(())
}

#[cfg(feature = "websocket")]
#[tokio::test]
async fn websocket_h1_exposes_version_multi_valued_headers_and_stores_cookies() -> TestResult {
    let (cert, key) = crate::tls_support::self_signed();
    let pem = der_to_pem(&cert.to_der()?);
    let port = crate::tls_support::tls_server_per_connection(
        cert,
        key,
        Arc::new(AtomicUsize::new(0)),
        |_| crate::tls_support::HTTP11,
        |_, stream| upgrade_h1_with_headers(stream),
    )
    .await;

    let session = Session::builder()
        .protocol(leyline::Protocol::Http1)
        .tls(Tls::new().env_roots(false).system_roots(false).ca_pem(pem))
        .build()?;
    let url = format!("wss://127.0.0.1:{port}/live");
    let mut ws = session.get(&url).websocket().await?;
    assert_eq!(ws.version(), leyline::http::Version::HTTP_11);
    assert_eq!(ws.header("x-edge"), Some("iad"));
    let edges = ws
        .headers()
        .get_all("x-edge")
        .iter()
        .map(|v| v.to_str())
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(edges, vec!["iad", "ord"]);
    assert_eq!(session.cookies().len(), 2);
    assert_eq!(
        session
            .cookies()
            .get(format!("https://127.0.0.1:{port}/live"), "ws_a"),
        Some("1".to_owned())
    );
    ws.close().await?;
    Ok(())
}

#[cfg(feature = "websocket")]
async fn upgrade_h1_with_headers(mut stream: leyline_bssl_tokio::SslStream<tokio::net::TcpStream>) {
    use leyline_bssl::hash::{MessageDigest, hash};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut head = Vec::new();
    let mut buf = [0u8; 1024];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
        let Ok(n) = stream.read(&mut buf).await else {
            return;
        };
        if n == 0 {
            return;
        }
        head.extend_from_slice(&buf[..n]);
    }
    let Ok(head_str) = std::str::from_utf8(&head) else {
        return;
    };
    let Some(sec_key) = head_str.lines().find_map(|line| {
        let (k, v) = line.split_once(':')?;
        k.eq_ignore_ascii_case("sec-websocket-key")
            .then(|| v.trim().to_owned())
    }) else {
        return;
    };
    let Ok(digest) = hash(
        MessageDigest::sha1(),
        format!("{sec_key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11").as_bytes(),
    ) else {
        return;
    };
    let reply = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
         Sec-WebSocket-Accept: {}\r\nX-Edge: iad\r\nX-Edge: ord\r\n\
         Set-Cookie: ws_a=1; Path=/\r\nSet-Cookie: ws_b=2; Path=/\r\n\r\n",
        STANDARD.encode(&*digest)
    );
    drop(stream.write_all(reply.as_bytes()).await);
    drop(stream.read(&mut buf).await);
}

#[cfg(all(feature = "bench-internals", feature = "websocket"))]
#[tokio::test]
async fn websocket_h2_exposes_version_multi_valued_headers_and_stores_cookies() -> TestResult {
    let (cert, key) = crate::tls_support::self_signed();
    let pem = der_to_pem(&cert.to_der()?);
    let port = crate::tls_support::tls_server(
        cert,
        key,
        Arc::new(AtomicUsize::new(0)),
        serve_h2_extended_connect,
    )
    .await;

    let session = Session::builder()
        .websocket_http2(true)
        .tls(Tls::new().env_roots(false).system_roots(false).ca_pem(pem))
        .build()?;
    let ws = session
        .get(format!("wss://127.0.0.1:{port}/h2ws"))
        .websocket()
        .await?;
    assert_eq!(ws.version(), leyline::http::Version::HTTP_2);
    assert_eq!(ws.header("x-edge"), Some("sfo"));
    let edges = ws
        .headers()
        .get_all("x-edge")
        .iter()
        .map(|v| v.to_str())
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(edges, vec!["sfo", "nrt"]);
    assert_eq!(
        session
            .cookies()
            .get(format!("https://127.0.0.1:{port}/h2ws"), "ws_sid"),
        Some("h2".to_owned())
    );
    Ok(())
}

#[cfg(all(feature = "bench-internals", feature = "websocket"))]
async fn serve_h2_extended_connect(
    mut stream: leyline_bssl_tokio::SslStream<tokio::net::TcpStream>,
) {
    use bytes::BytesMut;
    use leyline::internals::h2::frame::{FrameType, HeadersFrame};
    use leyline::internals::h2::hpack;
    use tokio::io::AsyncWriteExt;

    crate::h2_support::read_preface(&mut stream).await;
    let (h, _) = crate::h2_support::read_frame(&mut stream).await;
    assert_eq!(h.frame_type, FrameType::Settings as u8);
    crate::h2_support::write_server_settings_with(&mut stream, vec![(0x8u16, 1u32)]).await;
    crate::h2_support::write_settings_ack(&mut stream).await;
    loop {
        let (frame, _) = crate::h2_support::read_frame(&mut stream).await;
        if frame.frame_type == FrameType::Headers as u8 {
            let mut enc = hpack::Encoder::new();
            let block = enc.encode_header_block(&[
                (":status", "200"),
                ("x-edge", "sfo"),
                ("x-edge", "nrt"),
                ("set-cookie", "ws_sid=h2; Path=/"),
            ]);
            let reply = HeadersFrame {
                stream_id: frame.stream_id,
                end_stream: false,
                end_headers: true,
                priority: None,
                fragment: Bytes::from(block),
            };
            let mut buf = BytesMut::new();
            reply.encode(&mut buf);
            drop(stream.write_all(&buf).await);
            std::future::pending::<()>().await;
        }
    }
}

#[cfg(feature = "html")]
#[tokio::test]
async fn crawl_page_retains_non_sensitive_headers_across_checkpoints() -> TestResult {
    let server = Server::http(|req| match req.target() {
        "/" => testing::Response::new(200)
            .header("content-type", "text/html")
            .header("etag", "\"seed-v1\"")
            .header("link", "</a.html>; rel=\"preload\"")
            .header("link", "</b.html>; rel=\"alternate\"")
            .header("set-cookie", "sid=top-secret-token; Path=/")
            .body("<html><head><link rel=\"canonical\" href=\"/canon\"></head><body><a href=\"/canon\">next</a> seed words</body></html>"),
        "/canon" => testing::Response::new(200)
            .header("content-type", "text/html")
            .header("etag", "\"canon-v2\"")
            .header("last-modified", "Wed, 21 Oct 2026 07:28:00 GMT")
            .body("<html><body>canonical body content</body></html>"),
        _ => testing::Response::new(404),
    })
    .await?;

    let dir = scratch_dir("crawl-headers")?;
    let checkpoint = dir.join("crawl.json");
    let session = Session::new();
    let mut crawl = session
        .crawl(server.url("/"))
        .respect(None)
        .host_limits(leyline::HostLimits::new().max_in_flight(1));
    let first = crawl.next().await.ok_or("canonical page yielded")??;
    assert_eq!(first.header("etag"), Some("\"canon-v2\""));
    assert_eq!(
        first.header("last-modified"),
        Some("Wed, 21 Oct 2026 07:28:00 GMT")
    );
    assert!(first.headers().get("set-cookie").is_none());
    assert_eq!(
        session.cookies().get(server.url("/"), "sid"),
        Some("top-secret-token".to_owned())
    );

    let mut held_crawl = session
        .crawl(server.url("/"))
        .respect(None)
        .dedup(false)
        .limit(1);
    let seed_page = held_crawl.next().await.ok_or("seed page")??;
    assert_eq!(seed_page.header("etag"), Some("\"seed-v1\""));
    let links = seed_page
        .headers()
        .get_all("link")
        .iter()
        .map(|v| v.to_str())
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(
        links,
        vec!["</a.html>; rel=\"preload\"", "</b.html>; rel=\"alternate\""]
    );
    assert!(seed_page.headers().get("set-cookie").is_none());
    held_crawl.save(&checkpoint).await?;
    let raw_checkpoint = std::fs::read_to_string(&checkpoint)?;
    assert!(!raw_checkpoint.contains("top-secret-token"));

    let serialized = serde_json::to_string(&seed_page)?;
    let restored_page: leyline::crawl::Page = serde_json::from_str(&serialized)?;
    assert_eq!(restored_page.header("etag"), Some("\"seed-v1\""));
    let restored_links = restored_page
        .headers()
        .get_all("link")
        .iter()
        .map(|v| v.to_str())
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(
        restored_links,
        vec!["</a.html>; rel=\"preload\"", "</b.html>; rel=\"alternate\""]
    );

    let legacy_page: leyline::crawl::Page = serde_json::from_str(
        r#"{"url":"https://example.com/","depth":0,"status":200,"html":"<p>hi</p>","text":"hi"}"#,
    )?;
    assert!(legacy_page.headers().is_empty());

    let latin1_page: leyline::crawl::Page = serde_json::from_str(
        r#"{"url":"https://example.com/","depth":0,"status":200,"headers":[["x-raw",[195,40]]],"html":"<p>hi</p>","text":"hi"}"#,
    )?;
    assert_eq!(latin1_page.header("x-raw"), None);
    assert_eq!(
        latin1_page.headers().get("x-raw").map(|v| v.as_bytes()),
        Some(&[195u8, 40][..])
    );
    let roundtrip_latin1: leyline::crawl::Page =
        serde_json::from_str(&serde_json::to_string(&latin1_page)?)?;
    assert_eq!(
        roundtrip_latin1
            .headers()
            .get("x-raw")
            .map(|v| v.as_bytes()),
        Some(&[195u8, 40][..])
    );

    let mut resumed = leyline::crawl::Crawl::load(&session, &checkpoint)?
        .respect(None)
        .limit(None);
    let next = resumed.next().await.ok_or("resumed page")??;
    assert_eq!(next.header("etag"), Some("\"canon-v2\""));
    drop(std::fs::remove_dir_all(&dir));
    Ok(())
}

#[test]
fn cookie_jar_inspection_matches_rfc_send_order() -> TestResult {
    let parsed = Cookie::parse(
        "https://app.example.com/shop/cart",
        "sid=abc; Domain=example.com; Path=/shop; Secure; HttpOnly; SameSite=Strict",
    )
    .ok_or("valid Set-Cookie")?;
    assert_eq!(parsed.name(), "sid");
    assert_eq!(parsed.value(), "abc");
    assert_eq!(parsed.domain(), "example.com");
    assert_eq!(parsed.path(), "/shop");
    assert!(parsed.secure());
    assert!(parsed.http_only());
    assert_eq!(parsed.same_site(), SameSite::Strict);
    assert!(!parsed.host_only());
    assert!(Cookie::parse("https://app.example.com/", "sid=abc; Domain=other.example").is_none());
    assert!(Cookie::parse("://bad-url", "sid=abc").is_none());

    let jar = Jar::new();
    assert!(jar.is_empty());
    assert_eq!(jar.len(), 0);
    jar.insert(
        "https://app.example.com/",
        Cookie::builder("root", "1").path("/").build(),
    );
    jar.insert(
        "https://app.example.com/shop/checkout",
        Cookie::builder("deep", "3")
            .path("/shop/checkout")
            .secure(true)
            .build(),
    );
    jar.insert(
        "https://app.example.com/shop",
        Cookie::builder("mid", "2").path("/shop").build(),
    );
    jar.insert(
        "https://app.example.com/",
        Cookie::builder("gone", "0")
            .expires(SystemTime::UNIX_EPOCH + Duration::from_secs(1))
            .build(),
    );
    assert!(!jar.is_empty());
    assert_eq!(jar.len(), 3);

    let https_names: Vec<String> = jar
        .cookies_for("https://app.example.com/shop/checkout")
        .into_iter()
        .map(|c| c.name().to_owned())
        .collect();
    assert_eq!(https_names, vec!["deep", "mid", "root"]);

    let http_names: Vec<String> = jar
        .cookies_for("http://app.example.com/shop/checkout")
        .into_iter()
        .map(|c| c.name().to_owned())
        .collect();
    assert_eq!(http_names, vec!["mid", "root"]);
    assert!(jar.cookies_for("://bad-url").is_empty());
    Ok(())
}
