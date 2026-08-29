//! Regression gates for H1 response-framing conflict
//! rejection. `validate_framing_headers` must refuse:
//!
//! - multiple `Content-Length` header lines
//! - a single `Content-Length` value with commas (`10, 10`)
//! - both `Content-Length` and `Transfer-Encoding` present
//! - `Transfer-Encoding` where `chunked` is not the final coding
//!
//! All four are RFC 9112 §6.1 request-smuggling vectors against a
//! keep-alive pool.
//!
//! Tests spin up a `tokio::net::TcpListener` mock serving the bad
//! response and assert the client surfaces a parse error rather than
//! silently reusing a desynced connection.
#![expect(
    clippy::unwrap_used,
    reason = "test/example harness: unwrap doubles as the assertion - a failed helper panics with the test location"
)]
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use leyline::pool::{H1Body, H1Response, H1ResponseBody, H1Target, Pool, send_request_h1_pooled};
use leyline::profile::{Browser, Platform, ProfileRegistry};
use leyline::tls::FingerprintConnector;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn connector() -> FingerprintConnector {
    let profiles = ProfileRegistry::builtin();
    let profile = profiles.get_browser(Browser::Chrome147).unwrap();
    let tcp_profile = Platform::default().tcp_profile();
    FingerprintConnector::new(profile, tcp_profile).unwrap()
}

async fn run_against(server_response: &'static [u8]) -> String {
    let (msg, stats) = run_against_with_stats(server_response).await;
    // Two invariants hold after a framing error:
    //
    // (a) `entries == 0` — the pool map does not hold a live slot.
    //     On its own this is weak: an empty pool is also the
    //     default before any request runs, and in the test we
    //     start from a fresh pool, so an error path that never
    //     installed anything trivially satisfies it.
    //
    // (b) `installs == 0` — the monotonic install counter proves
    //     no install EVER ran. This is the real invariant the
    //     request-smuggling defence relies on: a desynced socket
    //     must never be stashed in the pool, not even
    //     transiently. A hypothetical regression that installed
    //     the bad slot and immediately evicted it would pass (a)
    //     but fail (b) — which is what we want.
    assert_eq!(
        stats.entries, 0,
        "framing-conflict socket stayed in the pool (stats: {stats:?})"
    );
    assert_eq!(
        stats.installs, 0,
        "framing-conflict socket was briefly installed in the pool before eviction (stats: {stats:?})"
    );
    msg
}

/// Absolute-form request targets to a plaintext proxy are
/// scheme://authority + path + query; no userinfo, no fragment.
#[tokio::test]
async fn absolute_form_target_strips_userinfo_and_fragment() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server = tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut acc: Vec<u8> = Vec::with_capacity(2048);
        let mut buf = [0u8; 2048];
        loop {
            let n = sock.read(&mut buf).await.unwrap();
            if n == 0 {
                break;
            }
            acc.extend_from_slice(&buf[..n]);
            if acc.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
            .await
            .unwrap();
        sock.flush().await.unwrap();
        String::from_utf8_lossy(&acc).to_string()
    });

    let pool = Arc::new(Pool::new());
    let connector = connector();
    let url = url::Url::parse("http://user:pw@example.com/a?b#frag").unwrap();
    let proxy = format!("http://{addr}");
    let resp = send_request_h1_pooled(
        &pool,
        &connector,
        "http",
        "example.com",
        80,
        "GET",
        &url,
        vec![],
        H1Body::Empty,
        Some(&proxy),
        H1Target::AbsoluteForm,
        false,
    )
    .await
    .expect("proxied request succeeds");

    assert_eq!(resp.status, 200);
    let request_line = server.await.unwrap();
    let request_line = request_line.lines().next().unwrap().to_string();
    assert_eq!(
        request_line, "GET http://example.com/a?b HTTP/1.1",
        "userinfo and fragment must be stripped from the absolute-form target"
    );
}

async fn run_against_with_stats(
    server_response: &'static [u8],
) -> (String, leyline::pool::PoolStats) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        // Accumulate across reads — a single-read sniff for
        // `\r\n\r\n` was flaky whenever the kernel delivered the
        // request header in multiple segments (CI schedulers,
        // Nagle, etc.). Scan the running buffer for end-of-headers.
        let mut acc: Vec<u8> = Vec::with_capacity(2048);
        let mut buf = [0u8; 2048];
        loop {
            let n = sock.read(&mut buf).await.unwrap();
            if n == 0 {
                break;
            }
            acc.extend_from_slice(&buf[..n]);
            if acc.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        sock.write_all(server_response).await.unwrap();
        sock.flush().await.unwrap();
        // Keep the socket open briefly so the client has time to
        // read the response fully before we send FIN.
        tokio::time::sleep(Duration::from_millis(50)).await;
    });

    let pool = Arc::new(Pool::new());
    let connector = connector();
    let url = url::Url::parse(&format!("http://{addr}/")).unwrap();
    let err = send_request_h1_pooled(
        &pool,
        &connector,
        "http",
        "127.0.0.1",
        addr.port(),
        "GET",
        &url,
        vec![],
        H1Body::Empty,
        None,
        H1Target::OriginForm,
        false,
    )
    .await
    .err()
    .expect("must reject framing conflict");
    (format!("{err}"), pool.stats())
}

async fn informational_response(chunks: &'static [&'static [u8]], stream: bool) -> H1Response {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut req = Vec::new();
        let mut buf = [0u8; 2048];
        while !req.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = sock.read(&mut buf).await.unwrap();
            req.extend_from_slice(&buf[..n]);
        }
        for (index, chunk) in chunks.iter().enumerate() {
            sock.write_all(chunk).await.unwrap();
            sock.flush().await.unwrap();
            if index + 1 < chunks.len() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
    });

    let pool = Arc::new(Pool::new());
    let connector = connector();
    let url = url::Url::parse(&format!("http://{addr}/")).unwrap();
    send_request_h1_pooled(
        &pool,
        &connector,
        "http",
        "127.0.0.1",
        addr.port(),
        "GET",
        &url,
        vec![],
        H1Body::Empty,
        None,
        H1Target::OriginForm,
        stream,
    )
    .await
    .expect("informational response succeeds")
}

#[tokio::test]
async fn coalesced_informational_and_final_buffered_response_preserves_final_bytes() {
    let response = informational_response(
        &[b"HTTP/1.1 103 Early Hints\r\nLink: </app.css>; rel=preload\r\n\r\nHTTP/1.1 200 OK\r\nContent-Length: 5\r\nX-Final: yes\r\n\r\nhello"],
        false,
    )
    .await;

    assert_eq!(response.status, 200);
    assert!(response.headers.contains(&("X-Final".into(), "yes".into())));
    let H1ResponseBody::Buffered(body) = response.body else {
        panic!("buffered request returns a buffered body");
    };
    assert_eq!(body, b"hello");
}

#[tokio::test]
async fn split_informational_and_final_streaming_response_preserves_final_bytes() {
    let response = informational_response(
        &[
            b"HTTP/1.1 103 Early Hints\r\nLink: </app.css>; rel=preload\r\n\r\n",
            b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nX-Final: yes\r\n\r\nhello",
        ],
        true,
    )
    .await;

    assert_eq!(response.status, 200);
    assert!(response.headers.contains(&("X-Final".into(), "yes".into())));
    let H1ResponseBody::Streaming(mut body) = response.body else {
        panic!("streaming request returns a streaming body");
    };
    assert_eq!(body.next().await.unwrap().unwrap().as_ref(), b"hello");
}

#[tokio::test]
async fn multiple_content_length_headers_rejected() {
    let msg = run_against(
        b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\nContent-Length: 10\r\n\r\n1234567890",
    )
    .await;
    assert!(
        msg.contains("multiple Content-Length"),
        "unexpected error: {msg}"
    );
}

#[tokio::test]
async fn content_length_with_list_value_rejected() {
    let msg = run_against(b"HTTP/1.1 200 OK\r\nContent-Length: 10, 10\r\n\r\n1234567890").await;
    assert!(msg.contains("multiple values"), "unexpected error: {msg}");
}

#[tokio::test]
async fn content_length_and_transfer_encoding_rejected() {
    let msg = run_against(
        b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n",
    )
    .await;
    assert!(
        msg.contains("both Content-Length and Transfer-Encoding"),
        "unexpected error: {msg}"
    );
}

#[tokio::test]
async fn transfer_encoding_chunked_not_last_rejected() {
    let msg =
        run_against(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked, gzip\r\n\r\n0\r\n\r\n").await;
    assert!(
        msg.contains("`chunked` must be the final coding"),
        "unexpected error: {msg}"
    );
}

// A Content-Length value that passes the single-header and no-comma
// checks but fails `u64::parse` must surface a framing error before the
// body read — falling back to `read_to_close` lets a malicious origin
// desync the pool by emitting more bytes than the stated length.

#[tokio::test]
async fn content_length_with_plus_sign_rejected() {
    let msg = run_against(b"HTTP/1.1 200 OK\r\nContent-Length: +10\r\n\r\n1234567890").await;
    assert!(
        msg.contains("not a valid decimal integer"),
        "unexpected error: {msg}"
    );
}

#[tokio::test]
async fn content_length_with_trailing_garbage_rejected() {
    let msg = run_against(b"HTTP/1.1 200 OK\r\nContent-Length: 10 foo\r\n\r\n1234567890").await;
    assert!(
        msg.contains("not a valid decimal integer"),
        "unexpected error: {msg}"
    );
}

#[tokio::test]
async fn content_length_hex_rejected() {
    let msg = run_against(b"HTTP/1.1 200 OK\r\nContent-Length: 0x10\r\n\r\n1234567890").await;
    assert!(
        msg.contains("not a valid decimal integer"),
        "unexpected error: {msg}"
    );
}

#[tokio::test]
async fn content_length_u64_overflow_rejected() {
    let msg =
        run_against(b"HTTP/1.1 200 OK\r\nContent-Length: 99999999999999999999\r\n\r\n1234567890")
            .await;
    assert!(
        msg.contains("not a valid decimal integer"),
        "unexpected error: {msg}"
    );
}
