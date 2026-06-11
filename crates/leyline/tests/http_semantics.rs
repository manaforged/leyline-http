//! General HTTP-semantics tests: decompression, redirects, cookies, body
//! round-trips. These verify leyline behaviour that has nothing to do with
//! wire fingerprinting, so they live here rather than in `tls_peet.rs`.
//!
//! They used to hit `httpbin.org` as `#[ignore]` "live" tests, which made
//! them (a) flaky — httpbin.org regularly 503s — and (b) part of the
//! fingerprint pre-commit gate's live matrix, where an httpbin outage would
//! block an unrelated commit. They now run against an in-process
//! `httpbin_lite` server (below): no network, no third party, deterministic,
//! and they exercise the real gzip/deflate/brotli decode path because the
//! mock actually compresses its responses.

use leyline::Session;
use serde_json::Value;

/// A minimal, in-process httpbin-compatible server. Implements exactly the
/// endpoints these tests need (`/gzip`, `/brotli`, `/deflate`, `/get`,
/// `/headers`, `/cookies[/set]`, `/redirect/N`, `/redirect-to`, `/post`),
/// matching httpbin's JSON shape and header capitalisation. One request per
/// connection (`Connection: close`); the accept loop handles many
/// connections so redirect chains and multi-request flows work.
mod httpbin_lite {
    use std::io::Write as _;

    use serde_json::{Map, Value};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    /// Spawn the server on an ephemeral localhost port and return its base
    /// URL (e.g. `http://127.0.0.1:54123`). The server task runs until the
    /// test's runtime is dropped.
    pub async fn spawn() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((sock, _)) = listener.accept().await {
                tokio::spawn(handle(sock));
            }
        });
        format!("http://{addr}")
    }

    struct Request {
        method: String,
        path: String,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    }

    async fn read_request(sock: &mut TcpStream) -> Option<Request> {
        let mut buf = Vec::new();
        let mut tmp = [0u8; 4096];
        let header_end = loop {
            let n = sock.read(&mut tmp).await.ok()?;
            if n == 0 {
                return None; // client closed (e.g. a pool probe) — nothing to do
            }
            buf.extend_from_slice(&tmp[..n]);
            if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break i + 4;
            }
        };
        let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
        let mut lines = head.split("\r\n");
        let req_line = lines.next().unwrap_or("");
        let mut parts = req_line.split_whitespace();
        let method = parts.next().unwrap_or("").to_string();
        let path = parts.next().unwrap_or("/").to_string();

        let mut headers = Vec::new();
        let mut content_length = 0usize;
        for line in lines {
            if line.is_empty() {
                continue;
            }
            if let Some((k, v)) = line.split_once(':') {
                let k = k.trim().to_string();
                let v = v.trim().to_string();
                if k.eq_ignore_ascii_case("content-length") {
                    content_length = v.parse().unwrap_or(0);
                }
                headers.push((k, v));
            }
        }

        let mut body = buf[header_end..].to_vec();
        while body.len() < content_length {
            let n = sock.read(&mut tmp).await.ok()?;
            if n == 0 {
                break;
            }
            body.extend_from_slice(&tmp[..n]);
        }
        Some(Request {
            method,
            path,
            headers,
            body,
        })
    }

    async fn handle(mut sock: TcpStream) {
        let Some(req) = read_request(&mut sock).await else {
            return;
        };
        let (route, query) = match req.path.split_once('?') {
            Some((p, q)) => (p, q),
            None => (req.path.as_str(), ""),
        };
        let header_val = Value::Object(title_cased_headers(&req.headers));

        let resp = match (req.method.as_str(), route) {
            ("GET", "/gzip") => json_compressed(
                "gzip",
                serde_json::json!({"gzipped": true, "headers": header_val}),
            ),
            ("GET", "/brotli") => json_compressed(
                "br",
                serde_json::json!({"brotli": true, "headers": header_val}),
            ),
            ("GET", "/deflate") => json_compressed(
                "deflate",
                serde_json::json!({"deflated": true, "headers": header_val}),
            ),
            ("GET", "/get") => json_200(serde_json::json!({"url": route, "headers": header_val})),
            ("GET", "/headers") => json_200(serde_json::json!({"headers": header_val})),
            ("GET", "/cookies") => {
                let cookies = req
                    .headers
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("cookie"))
                    .map(|(_, v)| parse_cookies(v))
                    .unwrap_or_default();
                json_200(serde_json::json!({"cookies": Value::Object(cookies)}))
            }
            ("GET", "/cookies/set") => {
                // query is `name=value`; httpbin sets it then 302s to /cookies.
                redirect_302("/cookies", vec![("Set-Cookie", format!("{query}; Path=/"))])
            }
            ("GET", p) if p.starts_with("/redirect/") => {
                let n: usize = p.trim_start_matches("/redirect/").parse().unwrap_or(0);
                let loc = if n <= 1 {
                    "/get".to_string()
                } else {
                    format!("/redirect/{}", n - 1)
                };
                redirect_302(&loc, vec![])
            }
            ("GET", "/redirect-to") => {
                let target = query.strip_prefix("url=").unwrap_or("/get");
                redirect_302(target, vec![])
            }
            ("POST", "/post") => {
                let ct = req
                    .headers
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
                    .map(|(_, v)| v.clone())
                    .unwrap_or_default();
                let body_str = String::from_utf8_lossy(&req.body).to_string();
                let json_field = if ct.contains("application/json") {
                    serde_json::from_slice::<Value>(&req.body).unwrap_or(Value::Null)
                } else {
                    Value::Null
                };
                let form_field = if ct.contains("x-www-form-urlencoded") {
                    parse_form(&body_str)
                } else {
                    Map::new()
                };
                json_200(serde_json::json!({
                    "json": json_field,
                    "form": Value::Object(form_field),
                    "data": body_str,
                    "headers": header_val,
                }))
            }
            _ => json_status(404, "Not Found", serde_json::json!({"error": "not found"})),
        };

        let _ = sock.write_all(&resp).await;
        let _ = sock.flush().await;
    }

    fn build(status: u16, reason: &str, extra: Vec<(&str, String)>, body: Vec<u8>) -> Vec<u8> {
        let mut head = format!("HTTP/1.1 {status} {reason}\r\n");
        head.push_str(&format!("Content-Length: {}\r\n", body.len()));
        head.push_str("Connection: close\r\n");
        for (k, v) in extra {
            head.push_str(&format!("{k}: {v}\r\n"));
        }
        head.push_str("\r\n");
        let mut out = head.into_bytes();
        out.extend_from_slice(&body);
        out
    }

    fn json_200(v: Value) -> Vec<u8> {
        json_status(200, "OK", v)
    }

    fn json_status(status: u16, reason: &str, v: Value) -> Vec<u8> {
        build(
            status,
            reason,
            vec![("Content-Type", "application/json".to_string())],
            serde_json::to_vec(&v).unwrap(),
        )
    }

    fn json_compressed(encoding: &str, v: Value) -> Vec<u8> {
        let raw = serde_json::to_vec(&v).unwrap();
        let body = match encoding {
            "gzip" => {
                let mut e =
                    flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
                e.write_all(&raw).unwrap();
                e.finish().unwrap()
            }
            "deflate" => {
                // httpbin's `deflate` is zlib-wrapped (RFC 1950), which is
                // what real browsers accept under `Content-Encoding: deflate`.
                let mut e =
                    flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
                e.write_all(&raw).unwrap();
                e.finish().unwrap()
            }
            "br" => {
                let mut out = Vec::new();
                {
                    let mut w = brotli::CompressorWriter::new(&mut out, 4096, 5, 22);
                    w.write_all(&raw).unwrap();
                }
                out
            }
            other => panic!("unsupported encoding {other}"),
        };
        build(
            200,
            "OK",
            vec![
                ("Content-Type", "application/json".to_string()),
                ("Content-Encoding", encoding.to_string()),
            ],
            body,
        )
    }

    fn redirect_302(location: &str, mut extra: Vec<(&str, String)>) -> Vec<u8> {
        extra.insert(0, ("Location", location.to_string()));
        build(302, "Found", extra, Vec::new())
    }

    /// Capitalise header names the way httpbin echoes them (`Content-Type`,
    /// `Authorization`, …) so assertions on `json["headers"]["Authorization"]`
    /// match regardless of the case leyline put on the wire.
    fn title_cased_headers(headers: &[(String, String)]) -> Map<String, Value> {
        let mut m = Map::new();
        for (k, v) in headers {
            m.insert(title_case(k), Value::String(v.clone()));
        }
        m
    }

    fn title_case(name: &str) -> String {
        name.split('-')
            .map(|seg| {
                let mut c = seg.chars();
                match c.next() {
                    Some(first) => {
                        first.to_ascii_uppercase().to_string() + &c.as_str().to_ascii_lowercase()
                    }
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join("-")
    }

    fn parse_cookies(header: &str) -> Map<String, Value> {
        let mut m = Map::new();
        for pair in header.split(';') {
            if let Some((k, v)) = pair.trim().split_once('=') {
                m.insert(k.trim().to_string(), Value::String(v.trim().to_string()));
            }
        }
        m
    }

    fn parse_form(body: &str) -> Map<String, Value> {
        let mut m = Map::new();
        for pair in body.split('&') {
            if pair.is_empty() {
                continue;
            }
            match pair.split_once('=') {
                Some((k, v)) => {
                    m.insert(k.to_string(), Value::String(v.to_string()));
                }
                None => {
                    m.insert(pair.to_string(), Value::String(String::new()));
                }
            }
        }
        m
    }
}

// ─── Decompression ────────────────────────────────────────────────────────

#[tokio::test]
async fn decompression_gzip() {
    let base = httpbin_lite::spawn().await;
    let session = Session::chrome();
    let resp = session.navigate(&format!("{base}/gzip")).await.unwrap();
    assert_eq!(resp.status(), 200);
    let json: Value = serde_json::from_str(&resp.text()).expect("gzip-decoded body not JSON");
    assert_eq!(json["gzipped"], true);
}

#[tokio::test]
async fn decompression_brotli() {
    let base = httpbin_lite::spawn().await;
    let session = Session::chrome();
    let resp = session.navigate(&format!("{base}/brotli")).await.unwrap();
    assert_eq!(resp.status(), 200);
    let json: Value = serde_json::from_str(&resp.text()).expect("brotli-decoded body not JSON");
    assert_eq!(json["brotli"], true);
}

#[tokio::test]
async fn decompression_deflate() {
    let base = httpbin_lite::spawn().await;
    let session = Session::chrome();
    let resp = session.navigate(&format!("{base}/deflate")).await.unwrap();
    assert_eq!(resp.status(), 200);
    let json: Value = serde_json::from_str(&resp.text()).expect("deflate-decoded body not JSON");
    assert_eq!(json["deflated"], true);
}

// ─── Cookies ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn cookies_set_then_sent() {
    let base = httpbin_lite::spawn().await;
    let session = Session::chrome();
    // Set a cookie via a Set-Cookie + redirect, then confirm it is sent back.
    let resp1 = session
        .navigate(&format!("{base}/cookies/set?token=abc123"))
        .await
        .unwrap();
    assert_eq!(resp1.status(), 200);

    let resp2 = session.navigate(&format!("{base}/cookies")).await.unwrap();
    assert_eq!(resp2.status(), 200);
    let json: Value = serde_json::from_str(&resp2.text()).unwrap();
    assert_eq!(
        json["cookies"]["token"].as_str(),
        Some("abc123"),
        "cookie not sent on second request"
    );
}

// ─── Redirects ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn redirect_follows_and_rewrites_url() {
    let base = httpbin_lite::spawn().await;
    let session = Session::chrome();
    let resp = session
        .navigate(&format!("{base}/redirect/3"))
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.redirect_chain().len(),
        3,
        "expected 3 redirects in chain"
    );
    assert!(
        resp.url().ends_with("/get"),
        "final URL wrong: {}",
        resp.url()
    );
}

#[tokio::test]
async fn redirect_preserves_auth_same_host() {
    let base = httpbin_lite::spawn().await;
    let session = Session::chrome();
    let resp = session
        .get(&format!("{base}/redirect-to?url={base}/headers"))
        .bearer_auth("secret-token-xyz")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let json: Value = serde_json::from_str(&resp.text()).unwrap();
    // Same-host redirect preserves Authorization.
    assert_eq!(
        json["headers"]["Authorization"].as_str(),
        Some("Bearer secret-token-xyz"),
        "same-host redirect should preserve Authorization"
    );
}

// ─── Body round-trips ─────────────────────────────────────────────────────

#[tokio::test]
async fn post_json_body_roundtrip() {
    let base = httpbin_lite::spawn().await;
    let session = Session::chrome();
    let body = serde_json::json!({"test": "leyline", "n": 42});
    let resp = session
        .post_json(&format!("{base}/post"), &body)
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let json: Value = serde_json::from_str(&resp.text()).unwrap();
    assert_eq!(json["json"]["test"], "leyline");
    assert_eq!(json["json"]["n"], 42);
}

#[tokio::test]
async fn post_form_body_roundtrip() {
    let base = httpbin_lite::spawn().await;
    let session = Session::chrome();
    let resp = session
        .post_form(&format!("{base}/post"), &[("u", "alice"), ("p", "s3cret")])
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let json: Value = serde_json::from_str(&resp.text()).unwrap();
    assert_eq!(json["form"]["u"], "alice");
    assert_eq!(json["form"]["p"], "s3cret");
}
