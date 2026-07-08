//! Live smoke suite for Leyline's proof gates.

use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, Instant};

use bytes::Bytes;
use leyline::{Browser, Error, Result, Session};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const PEET_URL: &str = "https://tls.peet.ws/api/all";
const H3_GET_HOST: &str = "cloudflare-quic.com";
const H3_ECHO_HOST: &str = "httpbin.agrd.workers.dev";
const SMOKE_TIMEOUT: Duration = Duration::from_secs(25);
type SmokeFuture<'a> = Pin<Box<dyn Future<Output = Result<String>> + 'a>>;

// Live competitive smoke suite. Hits external services (tls.peet.ws, httpbin,
// cloudflare-quic). Opt-in: `cargo test -p leyline --test smoke -- --ignored
// --nocapture`.
#[tokio::test]
#[ignore]
async fn smoke_suite() {
    println!("=== Leyline Smoke Suite ===\n");

    let mut passed = 0u32;
    let mut failed = 0u32;

    run(
        "Chrome 150 exact JA4 + H2",
        &mut passed,
        &mut failed,
        smoke(async { exact_fingerprint(Session::chrome(), Browser::Chrome150).await }),
    )
    .await;

    run(
        "Firefox 150 exact JA4 + H2",
        &mut passed,
        &mut failed,
        smoke(async { exact_fingerprint(Session::firefox(), Browser::Firefox150).await }),
    )
    .await;

    run(
        "Connection reuse (3 sequential requests)",
        &mut passed,
        &mut failed,
        smoke(async {
            let s = Session::chrome();
            let t = Instant::now();
            let r1 = s.navigate(PEET_URL).await?;
            let t1 = t.elapsed();
            let r2 = s.navigate(PEET_URL).await?;
            let t2 = t.elapsed();
            let r3 = s.navigate(PEET_URL).await?;
            let t3 = t.elapsed();
            ensure(
                r1.status() == 200 && r2.status() == 200 && r3.status() == 200,
                {
                    format!(
                        "statuses were {}, {}, {}",
                        r1.status(),
                        r2.status(),
                        r3.status()
                    )
                },
            )?;
            Ok(format!("1st={t1:?} 2nd={:?} 3rd={:?}", t2 - t1, t3 - t2))
        }),
    )
    .await;

    run(
        "Decompression (brotli/gzip/zstd advertised)",
        &mut passed,
        &mut failed,
        smoke(async {
            let s = Session::chrome();
            let r = s.navigate(PEET_URL).await?;
            let body = r.text();
            let _: Value = serde_json::from_str(&body)?;
            ensure(
                body.contains("http2") && body.contains("tls"),
                "tls.peet response missing protocol sections",
            )?;
            Ok(format!("{}B valid JSON", body.len()))
        }),
    )
    .await;

    run(
        "HTTP/2 CDN GET (httpbin.org)",
        &mut passed,
        &mut failed,
        smoke(async {
            let s = Session::chrome();
            let r = s.navigate("https://httpbin.org/get").await?;
            ensure(r.status() == 200, format!("status={}", r.status()))?;
            ensure(r.text().contains("headers"), "httpbin body missing headers")?;
            Ok(format!("status={} body={}B", r.status(), r.bytes().len()))
        }),
    )
    .await;

    run(
        "POST JSON body round-trip",
        &mut passed,
        &mut failed,
        smoke(async {
            let s = Session::chrome();
            let payload = serde_json::json!({"test": "leyline", "v": 2});
            let r = s.post_json("https://httpbin.org/post", &payload).await?;
            let v: Value = r.json()?;
            let echoed = json_str(&v["json"]["test"], "json.test")?;
            ensure(echoed == "leyline", format!("echoed={echoed}"))?;
            Ok(format!("echoed={echoed}"))
        }),
    )
    .await;

    run(
        "POST form body round-trip",
        &mut passed,
        &mut failed,
        smoke(async {
            let s = Session::chrome();
            let r = s
                .post_form(
                    "https://httpbin.org/post",
                    &[("user", "alice"), ("pass", "s3cret!")],
                )
                .await?;
            let v: Value = r.json()?;
            let user = json_str(&v["form"]["user"], "form.user")?;
            let pass = json_str(&v["form"]["pass"], "form.pass")?;
            ensure(
                user == "alice" && pass == "s3cret!",
                format!("user={user} pass={pass}"),
            )?;
            Ok(format!("user={user} pass={pass}"))
        }),
    )
    .await;

    run(
        "GET with query params",
        &mut passed,
        &mut failed,
        smoke(async {
            let s = Session::chrome();
            let r = s
                .get("https://httpbin.org/get")
                .query([("foo", "bar"), ("n", "42")])
                .send()
                .await?;
            let v: Value = r.json()?;
            let arg_foo = json_str(&v["args"]["foo"], "args.foo")?;
            ensure(arg_foo == "bar", format!("foo={arg_foo}"))?;
            Ok(format!("foo={arg_foo}"))
        }),
    )
    .await;

    run(
        "Bearer auth header",
        &mut passed,
        &mut failed,
        smoke(async {
            let s = Session::chrome();
            let r = s
                .get("https://httpbin.org/get")
                .bearer_auth("test-token-123")
                .send()
                .await?;
            let v: Value = r.json()?;
            let auth = json_str(&v["headers"]["Authorization"], "headers.Authorization")?;
            ensure(auth.contains("test-token-123"), format!("auth={auth}"))?;
            Ok(format!("auth={auth}"))
        }),
    )
    .await;

    run(
        "error_for_status on 404",
        &mut passed,
        &mut failed,
        smoke(async {
            let s = Session::chrome();
            let r = s.navigate("https://httpbin.org/status/404").await?;
            ensure(r.status() == 404, format!("status={}", r.status()))?;
            ensure(r.error_for_status().is_err(), "404 should be error")?;
            Ok("404 -> Err".to_string())
        }),
    )
    .await;

    run(
        "Redirect following (302 x2)",
        &mut passed,
        &mut failed,
        smoke(async {
            let s = Session::chrome();
            let r = s.navigate("https://httpbin.org/redirect/2").await?;
            ensure(r.status() == 200, format!("status={}", r.status()))?;
            ensure(!r.redirect_chain().is_empty(), "redirect chain empty")?;
            Ok(format!("{} hops", r.redirect_chain().len()))
        }),
    )
    .await;

    run(
        "Chrome -> Firefox -> Safari fingerprints differ",
        &mut passed,
        &mut failed,
        smoke(async {
            let c = Session::chrome().navigate(PEET_URL).await?;
            let f = Session::firefox().navigate(PEET_URL).await?;
            let s = Session::safari().navigate(PEET_URL).await?;
            let ch: Value = c.json()?;
            let fh: Value = f.json()?;
            let sh: Value = s.json()?;
            let c_fp = normalize_akamai(json_str(
                &ch["http2"]["akamai_fingerprint"],
                "chrome http2.akamai_fingerprint",
            )?);
            let f_fp = normalize_akamai(json_str(
                &fh["http2"]["akamai_fingerprint"],
                "firefox http2.akamai_fingerprint",
            )?);
            let s_fp = normalize_akamai(json_str(
                &sh["http2"]["akamai_fingerprint"],
                "safari http2.akamai_fingerprint",
            )?);
            ensure(
                c_fp != f_fp && f_fp != s_fp && c_fp != s_fp,
                "fingerprints should be unique",
            )?;
            Ok("all 3 unique".to_string())
        }),
    )
    .await;

    run(
        "HTTP/1.1 browser wire shape",
        &mut passed,
        &mut failed,
        smoke(h1_wire_shape()),
    )
    .await;

    run(
        "HTTP/3 Chrome QUIC GET",
        &mut passed,
        &mut failed,
        smoke(async { h3_get(Browser::Chrome147).await }),
    )
    .await;

    run(
        "HTTP/3 Firefox QUIC GET",
        &mut passed,
        &mut failed,
        smoke(async { h3_get(Browser::Firefox150).await }),
    )
    .await;

    run(
        "HTTP/3 POST body round-trip",
        &mut passed,
        &mut failed,
        smoke(h3_post_body()),
    )
    .await;

    run(
        "Large response (50KB)",
        &mut passed,
        &mut failed,
        smoke(async {
            let s = Session::chrome();
            let r = s.navigate("https://httpbin.org/bytes/50000").await?;
            ensure(r.status() == 200, format!("status={}", r.status()))?;
            ensure(
                r.bytes().len() == 50_000,
                format!("got {} bytes", r.bytes().len()),
            )?;
            Ok(format!("{}B", r.bytes().len()))
        }),
    )
    .await;

    println!("\n  ────────────────────────────────────────");
    println!(
        "  {passed} passed, {failed} failed, {} total",
        passed + failed
    );
    assert!(failed == 0, "{failed} smoke subtest(s) failed");
}

async fn exact_fingerprint(session: Session, browser: Browser) -> Result<String> {
    let r = session.navigate(PEET_URL).await?;
    ensure(r.status() == 200, format!("status={}", r.status()))?;
    let v: Value = r.json()?;

    let ja4 = json_str(&v["tls"]["ja4"], "tls.ja4")?;
    let h2 = normalize_akamai(json_str(
        &v["http2"]["akamai_fingerprint"],
        "http2.akamai_fingerprint",
    )?);

    let profile = leyline::profile(browser);
    let expected_ja4 = profile
        .expected_ja4()
        .ok_or_else(|| Error::Http(format!("{browser} profile missing expected JA4")))?;
    let expected_h2 = normalize_akamai(
        profile
            .expected_h2_fingerprint()
            .ok_or_else(|| Error::Http(format!("{browser} profile missing expected H2")))?,
    );

    ensure(
        ja4 == expected_ja4,
        format!("{browser} JA4 mismatch: got {ja4}, expected {expected_ja4}"),
    )?;
    ensure(
        h2 == expected_h2,
        format!("{browser} H2 mismatch: got {h2}, expected {expected_h2}"),
    )?;

    Ok(format!("JA4={ja4} H2={h2}"))
}

async fn h1_wire_shape() -> Result<String> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;

    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.map_err(|e| e.to_string())?;
        let mut req = Vec::new();
        let mut tmp = [0u8; 1024];
        loop {
            let n = socket.read(&mut tmp).await.map_err(|e| e.to_string())?;
            if n == 0 {
                return Err("client closed before request headers".to_string());
            }
            req.extend_from_slice(&tmp[..n]);
            if req.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }

        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
            .await
            .map_err(|e| e.to_string())?;

        Ok::<_, String>(String::from_utf8_lossy(&req).into_owned())
    });

    let session = Session::chrome();
    let resp = session
        .get(&format!("http://{addr}/wire?q=1"))
        .append_header("x-proof", "smoke")
        .send()
        .await?;
    ensure(resp.status() == 200, format!("status={}", resp.status()))?;

    let text = server
        .await
        .map_err(|e| Error::Http(format!("h1 server task failed: {e}")))?
        .map_err(Error::Http)?;

    ensure(
        text.starts_with("GET /wire?q=1 HTTP/1.1\r\n"),
        format!("bad request line: {text}"),
    )?;
    for needle in [
        "\r\nHost: ",
        "\r\nUser-Agent: ",
        "\r\nAccept: ",
        "\r\nAccept-Encoding: ",
        "\r\nAccept-Language: ",
        "\r\nConnection: keep-alive\r\n",
        "\r\nx-proof: smoke\r\n",
    ] {
        ensure(text.contains(needle), format!("missing {needle:?}: {text}"))?;
    }

    Ok(format!("{} request bytes", text.len()))
}

async fn h3_get(browser: Browser) -> Result<String> {
    let session = Session::builder().browser(browser).http3().build()?;
    let url = format!("https://{H3_GET_HOST}/");
    let resp = session
        .get(&url)
        .header("accept", "text/html,application/xhtml+xml")
        .send()
        .await?;

    let status = resp.status();
    let body = resp.bytes();
    ensure(status == 200, format!("status={status}"))?;
    ensure(!body.is_empty(), "empty H3 body")?;
    Ok(format!("status={status} body={}B", body.len()))
}

async fn h3_post_body() -> Result<String> {
    let payload = br#"{"proof":"leyline-h3-body","n":42}"#;
    let session = Session::builder()
        .browser(Browser::Chrome147)
        .http3()
        .build()?;
    let url = format!("https://{H3_ECHO_HOST}/post");
    let resp = session
        .post(&url)
        .header("accept", "application/json")
        .header("content-type", "application/json")
        .body(Bytes::copy_from_slice(payload))
        .send()
        .await?;

    let status = resp.status();
    let body = String::from_utf8_lossy(resp.bytes()).into_owned();
    ensure(status == 200, format!("status={status}"))?;
    ensure(
        body.contains("leyline-h3-body"),
        format!("H3 POST echo missing payload, body={body}"),
    )?;
    Ok(format!("status={status} echoed={}B", payload.len()))
}

fn smoke<'a>(fut: impl Future<Output = Result<String>> + 'a) -> SmokeFuture<'a> {
    Box::pin(fut)
}

async fn run(name: &str, passed: &mut u32, failed: &mut u32, fut: SmokeFuture<'_>) {
    let t = Instant::now();
    match tokio::time::timeout(SMOKE_TIMEOUT, fut).await {
        Ok(Ok(detail)) => {
            println!("  ✓ {} ({:?}) — {}", name, t.elapsed(), detail);
            *passed += 1;
        }
        Ok(Err(e)) => {
            println!("  ✗ {} ({:?}) — {}", name, t.elapsed(), e);
            *failed += 1;
        }
        Err(_) => {
            println!("  ✗ {} (>{:?} timeout)", name, SMOKE_TIMEOUT);
            *failed += 1;
        }
    }
}

fn json_str<'a>(value: &'a Value, name: &str) -> Result<&'a str> {
    value
        .as_str()
        .ok_or_else(|| Error::Http(format!("missing {name}")))
}

fn ensure(condition: bool, message: impl Into<String>) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(Error::Http(message.into()))
    }
}

fn normalize_akamai(fp: &str) -> String {
    fp.replace(";:1", ";8:1")
}
