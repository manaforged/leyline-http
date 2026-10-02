use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, Instant};

use leyline::{Browser, Family, Platform, ProtocolPolicy, Session};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const PEET_URL: &str = "https://tls.peet.ws/api/all";
const H3_GET_HOST: &str = "cloudflare-quic.com";
const SMOKE_TIMEOUT: Duration = Duration::from_secs(25);
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
type SmokeFuture<'a> = Pin<Box<dyn Future<Output = Result<String>> + 'a>>;

#[tokio::test]
#[ignore]
async fn smoke_suite() {
    println!("=== Leyline Smoke Suite ===\n");

    let mut passed = 0u32;
    let mut failed = 0u32;

    run(
        "Default Chrome exact JA4 + H2",
        &mut passed,
        &mut failed,
        smoke(async {
            exact_fingerprint(
                Session::browser(Browser::default()),
                Browser::latest(Family::Chrome),
            )
            .await
        }),
    )
    .await;

    run(
        "Firefox 154 exact JA4 + H2",
        &mut passed,
        &mut failed,
        smoke(async { exact_fingerprint(firefox()?, Browser::Firefox154).await }),
    )
    .await;

    run(
        "Connection reuse (3 sequential requests)",
        &mut passed,
        &mut failed,
        smoke(async {
            let s = Session::browser(Browser::default());
            let t = Instant::now();
            let r1 = s.get(PEET_URL).await?;
            let t1 = t.elapsed();
            let r2 = s.get(PEET_URL).await?;
            let t2 = t.elapsed();
            let r3 = s.get(PEET_URL).await?;
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
            let s = Session::browser(Browser::default());
            let r = s.get(PEET_URL).await?;
            let body = r.text().await.unwrap();
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
            let s = Session::browser(Browser::default());
            let r = s.get("https://httpbin.org/get").await?;
            let status = r.status();
            ensure(status == 200, format!("status={status}"))?;
            let body = r.text().await?;
            ensure(body.contains("headers"), "httpbin body missing headers")?;
            Ok(format!("status={status} body={}B", body.len()))
        }),
    )
    .await;

    run(
        "POST JSON body round-trip",
        &mut passed,
        &mut failed,
        smoke(async {
            let s = Session::browser(Browser::default());
            let payload = serde_json::json!({"test": "leyline", "v": 2});
            let r = s.post("https://httpbin.org/post").json(&payload).await?;
            let v: Value = r.json().await?;
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
            let s = Session::browser(Browser::default());
            let r = s
                .post("https://httpbin.org/post")
                .form([("user", "alice"), ("pass", "s3cret!")])
                .await?;
            let v: Value = r.json().await?;
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
            let s = Session::browser(Browser::default());
            let r = s
                .request(http::Method::GET, "https://httpbin.org/get")
                .query([("foo", "bar"), ("n", "42")])
                .send()
                .await?;
            let v: Value = r.json().await?;
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
            let s = Session::browser(Browser::default());
            let r = s
                .request(http::Method::GET, "https://httpbin.org/get")
                .bearer_auth("test-token-123")
                .send()
                .await?;
            let v: Value = r.json().await?;
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
            let s = Session::browser(Browser::default());
            let r = s.get("https://httpbin.org/status/404").await?;
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
            let s = Session::browser(Browser::default());
            let r = s.get("https://httpbin.org/redirect/2").await?;
            ensure(r.status() == 200, format!("status={}", r.status()))?;
            ensure(!r.redirect_chain().is_empty(), "redirect chain empty")?;
            Ok(format!("{} redirects", r.redirect_chain().len()))
        }),
    )
    .await;

    run(
        "Chrome -> Firefox -> Safari fingerprints differ",
        &mut passed,
        &mut failed,
        smoke(async {
            let c = Session::browser(Browser::default()).get(PEET_URL).await?;
            let f = firefox()?.get(PEET_URL).await?;
            let s = safari()?.get(PEET_URL).await?;
            let ch: Value = c.json().await?;
            let fh: Value = f.json().await?;
            let sh: Value = s.json().await?;
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
        "Large response (50KB)",
        &mut passed,
        &mut failed,
        smoke(async {
            let s = Session::browser(Browser::default());
            let r = s.get("https://httpbin.org/bytes/50000").await?;
            ensure(r.status() == 200, format!("status={}", r.status()))?;
            let n = r.bytes().await?.len();
            ensure(n == 50_000, format!("got {n} bytes"))?;
            Ok(format!("{n}B"))
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
    let r = session.get(PEET_URL).await?;
    ensure(r.status() == 200, format!("status={}", r.status()))?;
    let v: Value = r.json().await?;

    let ja4 = json_str(&v["tls"]["ja4"], "tls.ja4")?;
    let h2 = normalize_akamai(json_str(
        &v["http2"]["akamai_fingerprint"],
        "http2.akamai_fingerprint",
    )?);

    let profile = browser.profile();
    let expected_ja4 = profile
        .expected_ja4()
        .ok_or_else(|| format!("{browser} profile missing expected JA4"))?;
    let expected_h2 = normalize_akamai(
        profile
            .expected_h2_fingerprint()
            .ok_or_else(|| format!("{browser} profile missing expected H2"))?,
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

    let session = Session::browser(Browser::default());
    let resp = session
        .request(http::Method::GET, format!("http://{addr}/wire?q=1"))
        .header("x-proof", "smoke")
        .send()
        .await?;
    ensure(resp.status() == 200, format!("status={}", resp.status()))?;

    let text = server
        .await
        .map_err(|e| format!("h1 server task failed: {e}"))??;

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
    let session = Session::builder()
        .browser(browser)
        .protocol(ProtocolPolicy::Http3)
        .build()?;
    let url = format!("https://{H3_GET_HOST}/");
    let resp = session
        .request(http::Method::GET, url)
        .header("accept", "text/html,application/xhtml+xml")
        .send()
        .await?;

    let status = resp.status();
    let body = resp.bytes().await?;
    ensure(status == 200, format!("status={status}"))?;
    ensure(!body.is_empty(), "empty H3 body")?;
    Ok(format!("status={status} body={}B", body.len()))
}

fn firefox() -> Result<Session> {
    Ok(Session::builder()
        .browser(Browser::latest(Family::Firefox))
        .platform(Platform::Windows)
        .build()?)
}

fn safari() -> Result<Session> {
    Ok(Session::builder()
        .browser(Browser::Safari26)
        .platform(Platform::MacOS)
        .build()?)
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
        .ok_or_else(|| format!("missing {name}").into())
}

fn ensure(condition: bool, message: impl Into<String>) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(message.into().into())
    }
}

fn normalize_akamai(fp: &str) -> String {
    fp.replace(";:1", ";8:1")
}
