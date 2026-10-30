use leyline::Browser;

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_h3_cloudflare() {
    let session = leyline::Session::builder()
        .browser(Browser::Chrome147)
        .protocol(leyline::Protocol::Http3)
        .build()
        .expect("h3 session builds");
    let resp = session
        .get("https://cloudflare-quic.com/")
        .send()
        .await
        .expect("H3 request failed");
    assert_eq!(resp.status(), 200, "H3 status: {}", resp.status());
    assert_eq!(resp.version(), http::Version::HTTP_3, "H3 ALPN");
    assert_eq!(
        resp.tls_version(),
        Some("TLSv1.3"),
        "QUIC is always TLS 1.3 (RFC 9001 §4.2)"
    );
    assert!(
        resp.tls_cipher()
            .is_some_and(|c| c.starts_with("TLS_") && c.contains("_SHA")),
        "expected a TLS 1.3 cipher suite, got {:?}",
        resp.tls_cipher()
    );
    assert!(
        resp.peer_certificate().is_some_and(|c| !c.is_empty()),
        "H3 peer certificate should be exposed"
    );
    let cipher = resp.tls_cipher().map(str::to_owned);
    let body_len = resp.bytes().await.expect("buffered H3 body").len();
    assert!(body_len > 0, "H3 body is empty");
    println!(
        "✓ HTTP/3 to cloudflare-quic.com: status 200, {} bytes, cipher {:?}",
        body_len, cipher
    );
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_h3_cloudflare_firefox_profile() {
    let session = leyline::Session::builder()
        .browser(Browser::Firefox150)
        .protocol(leyline::Protocol::Http3)
        .build()
        .expect("h3 session builds");
    let resp = session
        .get("https://cloudflare-quic.com/")
        .send()
        .await
        .expect("H3 request failed (firefox profile)");
    assert_eq!(
        resp.status(),
        200,
        "H3 status with Firefox profile: {}",
        resp.status()
    );
    println!("✓ HTTP/3 to cloudflare-quic.com (Firefox 150 profile): status 200");
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_h3_google() {
    let session = leyline::Session::builder()
        .browser(Browser::Chrome147)
        .protocol(leyline::Protocol::Http3)
        .build()
        .expect("h3 session builds");
    let resp = session
        .request(http::Method::GET, "https://www.google.com/")
        .header("accept", "text/html")
        .send()
        .await
        .expect("H3 request failed");
    assert_eq!(resp.status(), 200, "H3 status: {}", resp.status());
    println!("✓ HTTP/3 to www.google.com: status 200");
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_h3_pool_reuse() {
    let session = leyline::Session::builder()
        .browser(Browser::Chrome147)
        .protocol(leyline::Protocol::Http3)
        .build()
        .expect("h3 session builds");

    let r1 = session
        .get("https://cloudflare-quic.com/")
        .send()
        .await
        .expect("first H3 request failed");
    assert_eq!(r1.status(), 200);
    drop(r1.bytes().await);
    let after_first = session.stats();

    let r2 = session
        .get("https://cloudflare-quic.com/")
        .send()
        .await
        .expect("second H3 request failed");
    assert_eq!(r2.status(), 200);
    drop(r2.bytes().await);
    let after_second = session.stats();

    assert_eq!(
        after_first.h3_hits(),
        0,
        "first request should open a fresh connection, not hit the pool"
    );
    assert!(
        after_second.h3_hits() >= 1,
        "second request must reuse the pooled QUIC connection (h3_hits={}, entries={})",
        after_second.h3_hits(),
        after_second.entries()
    );
    assert_eq!(
        after_second.entries(),
        1,
        "exactly one pooled H3 connection expected, got {}",
        after_second.entries()
    );
    println!(
        "✓ HTTP/3 pool reuse: h3_hits={}, installs={}, entries={}",
        after_second.h3_hits(),
        after_second.installs(),
        after_second.entries()
    );
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_race() {
    let session = leyline::Session::builder()
        .browser(Browser::Chrome147)
        .protocol(leyline::Protocol::Race)
        .build()
        .expect("race session builds");

    let r1 = session
        .get("https://cloudflare-quic.com/")
        .send()
        .await
        .expect("first raced request failed");
    assert_eq!(r1.status(), 200, "race status: {}", r1.status());
    drop(r1.bytes().await);

    let after_first = session.stats();
    assert!(
        (1..=2).contains(&after_first.entries()),
        "race must warm the winner (and optionally the loser leg), got {} entries \
         (h2_misses={}, h3_misses={})",
        after_first.entries(),
        after_first.h2_misses(),
        after_first.h3_misses()
    );

    let r2 = session
        .get("https://cloudflare-quic.com/")
        .send()
        .await
        .expect("second raced request failed");
    assert_eq!(r2.status(), 200);
    drop(r2.bytes().await);

    let after_second = session.stats();
    assert!(
        after_second.h2_hits() + after_second.h3_hits() >= 1,
        "second raced request must reuse the winning connection \
         (h2_hits={}, h3_hits={})",
        after_second.h2_hits(),
        after_second.h3_hits()
    );
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_h3_concurrent_cold_requests_single_flight() {
    use futures_util::future::join_all;

    let session = leyline::Session::builder()
        .browser(Browser::Chrome147)
        .protocol(leyline::Protocol::Http3)
        .build()
        .expect("h3 session builds");

    let futs = (0..8).map(|_| session.get("https://cloudflare-quic.com/").send());
    for r in join_all(futs).await {
        assert_eq!(r.expect("concurrent H3 request failed").status(), 200);
    }

    let stats = session.stats();
    assert_eq!(
        stats.installs(),
        1,
        "8 concurrent cold H3 requests must share ONE handshake, got {} installs \
         (entries={}, h3_hits={}, h3_misses={})",
        stats.installs(),
        stats.entries(),
        stats.h3_hits(),
        stats.h3_misses()
    );
    assert_eq!(stats.entries(), 1, "exactly one pooled QUIC connection");
    println!(
        "✓ H3 single-flight: installs={}, entries={}, h3_hits={}",
        stats.installs(),
        stats.entries(),
        stats.h3_hits()
    );
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_h3_response_streaming_is_incremental() {
    use futures_util::StreamExt;

    let session = leyline::Session::builder()
        .browser(Browser::Chrome147)
        .protocol(leyline::Protocol::Http3)
        .build()
        .expect("h3 session builds");

    let buffered = session
        .get("https://cloudflare-quic.com/")
        .send()
        .await
        .expect("buffered H3 request failed");
    let expected = buffered.bytes().await.expect("buffered H3 body").to_vec();

    let resp = session
        .request(http::Method::GET, "https://cloudflare-quic.com/")
        .stream()
        .send()
        .await
        .expect("streamed H3 request failed");
    assert_eq!(resp.status(), 200);
    let encoding = resp.header("content-encoding").map(str::to_ascii_lowercase);

    let mut stream = resp.bytes_stream().expect("into_stream");
    let mut chunks = 0usize;
    let mut compressed: Vec<u8> = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.expect("h3 body chunk");
        chunks += 1;
        compressed.extend_from_slice(&chunk);
    }

    let decoded = decode_content_encoding(&compressed, encoding.as_deref());
    assert_eq!(
        decoded,
        expected,
        "streamed H3 body differs from buffered: {} decoded bytes (from {} compressed, encoding={encoding:?}) in {chunks} chunks vs {} buffered",
        decoded.len(),
        compressed.len(),
        expected.len(),
    );
    assert!(
        chunks > 1,
        "expected incremental H3 delivery, got {chunks} chunk(s) for {} bytes",
        compressed.len()
    );
    println!(
        "✓ H3 incremental streaming: {chunks} chunks, {} compressed → {} bytes ({encoding:?})",
        compressed.len(),
        decoded.len()
    );
}

fn decode_content_encoding(body: &[u8], encoding: Option<&str>) -> Vec<u8> {
    use std::io::Read;
    match encoding {
        Some("br") => {
            let mut out = Vec::new();
            brotli::Decompressor::new(body, 4096)
                .read_to_end(&mut out)
                .expect("brotli decode");
            out
        }
        Some("gzip") | Some("x-gzip") => {
            let mut out = Vec::new();
            flate2::read::GzDecoder::new(body)
                .read_to_end(&mut out)
                .expect("gzip decode");
            out
        }
        Some("zstd") => zstd::decode_all(body).expect("zstd decode"),
        Some("deflate") => {
            let mut out = Vec::new();
            flate2::read::ZlibDecoder::new(body)
                .read_to_end(&mut out)
                .expect("deflate decode");
            out
        }
        _ => body.to_vec(),
    }
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_websocket_echo() {
    let session = leyline::Session::from(Browser::default());
    let mut ws = session
        .get("wss://ws.postman-echo.com/raw")
        .websocket()
        .await
        .expect("ws connect failed");

    ws.send(leyline::Message::Text("leyline-ping".to_owned()))
        .await
        .expect("ws send failed");

    let reply = ws
        .recv()
        .await
        .expect("ws recv failed")
        .expect("ws closed before reply");

    let as_string = format!("{reply:?}");
    assert!(
        as_string.contains("leyline-ping"),
        "unexpected ws reply: {as_string}"
    );

    ws.close().await.expect("ws close failed");
    println!("✓ WebSocket echo: sent=recv=leyline-ping");
}

#[tokio::test]
#[ignore = "live: needs network"]
async fn live_firefox_and_safari_reach_http3_by_default() {
    for browser in [Browser::firefox(), Browser::safari(), Browser::safari_ios()] {
        let session = leyline::Session::from(browser);
        let mut versions = Vec::new();
        for _ in 0..10 {
            let resp = session
                .get("https://cloudflare-quic.com/")
                .send()
                .await
                .expect("request failed");
            versions.push(resp.version());
            drop(resp.bytes().await);
            if versions.last() == Some(&leyline::http::Version::HTTP_3) {
                break;
            }
        }
        assert_eq!(
            versions.last(),
            Some(&leyline::http::Version::HTTP_3),
            "{browser}: {versions:?}"
        );
        println!(
            "✓ {browser} reached HTTP/3 after {} requests",
            versions.len()
        );
    }
}
