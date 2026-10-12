use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, Instant};

use http::StatusCode;
use leyline::{
    Browser, Error, ErrorCategory, Family, Kind, Platform, ProtocolPolicy, Response, Session,
    TimeoutConfig,
};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const PEET_URL: &str = "https://tls.peet.ws/api/all";
const H3_GET_HOST: &str = "cloudflare-quic.com";
const SMOKE_TIMEOUT: Duration = Duration::from_secs(25);
const HTTPBIN_ATTEMPTS: u32 = 3;
const HTTPBIN_BACKOFF: Duration = Duration::from_millis(200);
const HTTPBIN_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(8);
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
type SmokeFuture<'a> = Pin<Box<dyn Future<Output = Result<String>> + 'a>>;

#[tokio::test]
#[ignore = "reaches public internet endpoints"]
async fn smoke_suite() {
    println!("=== Leyline Smoke Suite ===\n");

    let mut passed = 0u32;
    let mut failed = 0u32;
    let mut skipped = 0u32;

    run(
        "Default Chrome exact JA4 + H2",
        &mut passed,
        &mut failed,
        &mut skipped,
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
        &mut skipped,
        smoke(async { exact_fingerprint(firefox()?, Browser::Firefox154).await }),
    )
    .await;

    run(
        "Connection reuse (3 sequential requests)",
        &mut passed,
        &mut failed,
        &mut skipped,
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
        &mut skipped,
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

    run_httpbin(
        "HTTP/2 CDN GET (httpbin.org)",
        &mut passed,
        &mut failed,
        &mut skipped,
        smoke(httpbin_cdn_get()),
    )
    .await;

    run_httpbin(
        "POST JSON body round-trip",
        &mut passed,
        &mut failed,
        &mut skipped,
        smoke(httpbin_post_json()),
    )
    .await;

    run_httpbin(
        "POST form body round-trip",
        &mut passed,
        &mut failed,
        &mut skipped,
        smoke(httpbin_post_form()),
    )
    .await;

    run_httpbin(
        "GET with query params",
        &mut passed,
        &mut failed,
        &mut skipped,
        smoke(httpbin_query()),
    )
    .await;

    run_httpbin(
        "Bearer auth header",
        &mut passed,
        &mut failed,
        &mut skipped,
        smoke(httpbin_bearer()),
    )
    .await;

    run_httpbin(
        "error_for_status on 404",
        &mut passed,
        &mut failed,
        &mut skipped,
        smoke(httpbin_status_404()),
    )
    .await;

    run_httpbin(
        "Redirect following (302 x2)",
        &mut passed,
        &mut failed,
        &mut skipped,
        smoke(httpbin_redirect()),
    )
    .await;

    run(
        "Chrome -> Firefox -> Safari fingerprints differ",
        &mut passed,
        &mut failed,
        &mut skipped,
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
        &mut skipped,
        smoke(h1_wire_shape()),
    )
    .await;

    run(
        "HTTP/3 Chrome QUIC GET",
        &mut passed,
        &mut failed,
        &mut skipped,
        smoke(async { h3_get(Browser::Chrome147).await }),
    )
    .await;

    run(
        "HTTP/3 Firefox QUIC GET",
        &mut passed,
        &mut failed,
        &mut skipped,
        smoke(async { h3_get(Browser::Firefox150).await }),
    )
    .await;

    run_httpbin(
        "Large response (50KB)",
        &mut passed,
        &mut failed,
        &mut skipped,
        smoke(httpbin_large()),
    )
    .await;

    println!("\n  ────────────────────────────────────────");
    println!(
        "  {passed} passed, {failed} failed, {skipped} skipped, {} total",
        passed + failed + skipped
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

async fn run(
    name: &str,
    passed: &mut u32,
    failed: &mut u32,
    skipped: &mut u32,
    fut: SmokeFuture<'_>,
) {
    finish(name, passed, failed, skipped, false, SMOKE_TIMEOUT, fut).await;
}

async fn run_httpbin(
    name: &str,
    passed: &mut u32,
    failed: &mut u32,
    skipped: &mut u32,
    fut: SmokeFuture<'_>,
) {
    finish(name, passed, failed, skipped, true, httpbin_budget(), fut).await;
}

async fn finish(
    name: &str,
    passed: &mut u32,
    failed: &mut u32,
    skipped: &mut u32,
    httpbin: bool,
    budget: Duration,
    fut: SmokeFuture<'_>,
) {
    let t = Instant::now();
    match tokio::time::timeout(budget, fut).await {
        Ok(Ok(detail)) => {
            println!("  ✓ {name} ({:?}) — {detail}", t.elapsed());
            *passed += 1;
        }
        Ok(Err(e)) if e.downcast_ref::<HttpbinUnavailable>().is_some() => {
            println!("  ! {name} ({:?}) — skipped: {e}", t.elapsed());
            *skipped += 1;
        }
        Ok(Err(e)) => {
            println!("  ✗ {name} ({:?}) — {e}", t.elapsed());
            *failed += 1;
        }
        Err(_) if httpbin => {
            println!("  ! {name} (>{budget:?} timeout) — skipped: httpbin unavailable");
            *skipped += 1;
        }
        Err(_) => {
            println!("  ✗ {name} (>{budget:?} timeout)");
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

#[derive(Debug)]
struct HttpbinUnavailable {
    reason: String,
}

impl fmt::Display for HttpbinUnavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.reason)
    }
}

impl std::error::Error for HttpbinUnavailable {}

#[derive(Debug)]
enum HttpbinAttempt {
    Unavailable(String),
    Fatal(String),
}

impl HttpbinAttempt {
    fn unavailable(reason: impl Into<String>) -> Self {
        Self::Unavailable(reason.into())
    }

    fn fatal(reason: impl Into<String>) -> Self {
        Self::Fatal(reason.into())
    }
}

impl From<Error> for HttpbinAttempt {
    fn from(err: Error) -> Self {
        if httpbin_transport_down(&err) {
            Self::unavailable(err.to_string())
        } else {
            Self::fatal(err.to_string())
        }
    }
}

fn httpbin_transport_down(err: &Error) -> bool {
    err.is_timeout() || matches!(err.category(), ErrorCategory::Connect | ErrorCategory::Dns)
}

fn httpbin_budget() -> Duration {
    let gaps = HTTPBIN_ATTEMPTS.saturating_sub(1);
    let pause = HTTPBIN_BACKOFF.saturating_mul(gaps * HTTPBIN_ATTEMPTS / 2);
    HTTPBIN_ATTEMPT_TIMEOUT
        .saturating_mul(HTTPBIN_ATTEMPTS)
        .saturating_add(pause)
        .saturating_add(Duration::from_secs(3))
}

fn httpbin_session() -> leyline::Result<Session> {
    session_with_timeout(HTTPBIN_ATTEMPT_TIMEOUT)
}

fn session_with_timeout(total: Duration) -> leyline::Result<Session> {
    Session::builder()
        .browser(Browser::default())
        .timeout(TimeoutConfig::new().total(total).connect(total))
        .build()
}

async fn with_httpbin<F, Fut>(op: F) -> Result<String>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = std::result::Result<String, HttpbinAttempt>>,
{
    attempt_httpbin(HTTPBIN_ATTEMPTS, HTTPBIN_BACKOFF, op).await
}

async fn attempt_httpbin<F, Fut>(attempts: u32, backoff: Duration, mut op: F) -> Result<String>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = std::result::Result<String, HttpbinAttempt>>,
{
    let attempts = attempts.max(1);
    let mut last = String::from("no response");
    for n in 1..=attempts {
        match op().await {
            Ok(detail) => return Ok(detail),
            Err(HttpbinAttempt::Fatal(reason)) => return Err(reason.into()),
            Err(HttpbinAttempt::Unavailable(reason)) => {
                last = reason;
                if n < attempts {
                    println!("    retry {n}/{attempts} after {last}");
                    tokio::time::sleep(backoff.saturating_mul(n)).await;
                }
            }
        }
    }
    Err(Box::new(HttpbinUnavailable {
        reason: format!("httpbin unavailable after {attempts} attempts: {last}"),
    }))
}

fn expect_httpbin_status(
    status: StatusCode,
    expected: StatusCode,
) -> std::result::Result<(), HttpbinAttempt> {
    if status == expected {
        Ok(())
    } else if status.is_server_error() {
        Err(HttpbinAttempt::unavailable(format!(
            "status={}",
            status.as_u16()
        )))
    } else {
        Err(HttpbinAttempt::fatal(format!("status={}", status.as_u16())))
    }
}

fn httpbin_eq(
    condition: bool,
    message: impl Into<String>,
) -> std::result::Result<(), HttpbinAttempt> {
    if condition {
        Ok(())
    } else {
        Err(HttpbinAttempt::fatal(message))
    }
}

fn httpbin_str<'a>(value: &'a Value, name: &str) -> std::result::Result<&'a str, HttpbinAttempt> {
    value
        .as_str()
        .ok_or_else(|| HttpbinAttempt::fatal(format!("missing {name}")))
}

async fn read_httpbin_text(
    response: Response,
    expected: StatusCode,
) -> std::result::Result<String, HttpbinAttempt> {
    expect_httpbin_status(response.status(), expected)?;
    Ok(response.text().await?)
}

async fn read_httpbin_json(response: Response) -> std::result::Result<Value, HttpbinAttempt> {
    expect_httpbin_status(response.status(), StatusCode::OK)?;
    match response.json().await {
        Ok(value) => Ok(value),
        Err(err) if err.kind() == Kind::Json => {
            Err(HttpbinAttempt::unavailable(format!("non-JSON body: {err}")))
        }
        Err(err) => Err(err.into()),
    }
}

async fn httpbin_cdn_get() -> Result<String> {
    with_httpbin(|| async {
        let session = httpbin_session()?;
        let response = session.get("https://httpbin.org/get").await?;
        let body = read_httpbin_text(response, StatusCode::OK).await?;
        httpbin_eq(body.contains("headers"), "httpbin body missing headers")?;
        Ok(format!("status=200 body={}B", body.len()))
    })
    .await
}

async fn httpbin_post_json() -> Result<String> {
    with_httpbin(|| async {
        let session = httpbin_session()?;
        let payload = serde_json::json!({"test": "leyline", "v": 2});
        let response = session
            .post("https://httpbin.org/post")
            .json(&payload)
            .await?;
        let value = read_httpbin_json(response).await?;
        let echoed = httpbin_str(&value["json"]["test"], "json.test")?;
        httpbin_eq(echoed == "leyline", format!("echoed={echoed}"))?;
        Ok(format!("echoed={echoed}"))
    })
    .await
}

async fn httpbin_post_form() -> Result<String> {
    with_httpbin(|| async {
        let session = httpbin_session()?;
        let response = session
            .post("https://httpbin.org/post")
            .form([("user", "alice"), ("pass", "s3cret!")])
            .await?;
        let value = read_httpbin_json(response).await?;
        let user = httpbin_str(&value["form"]["user"], "form.user")?;
        let pass = httpbin_str(&value["form"]["pass"], "form.pass")?;
        httpbin_eq(
            user == "alice" && pass == "s3cret!",
            format!("user={user} pass={pass}"),
        )?;
        Ok(format!("user={user} pass={pass}"))
    })
    .await
}

async fn httpbin_query() -> Result<String> {
    with_httpbin(|| async {
        let session = httpbin_session()?;
        let response = session
            .request(http::Method::GET, "https://httpbin.org/get")
            .query([("foo", "bar"), ("n", "42")])
            .send()
            .await?;
        let value = read_httpbin_json(response).await?;
        let arg_foo = httpbin_str(&value["args"]["foo"], "args.foo")?;
        httpbin_eq(arg_foo == "bar", format!("foo={arg_foo}"))?;
        Ok(format!("foo={arg_foo}"))
    })
    .await
}

async fn httpbin_bearer() -> Result<String> {
    with_httpbin(|| async {
        let session = httpbin_session()?;
        let response = session
            .request(http::Method::GET, "https://httpbin.org/get")
            .bearer_auth("test-token-123")
            .send()
            .await?;
        let value = read_httpbin_json(response).await?;
        let auth = httpbin_str(&value["headers"]["Authorization"], "headers.Authorization")?;
        httpbin_eq(auth.contains("test-token-123"), format!("auth={auth}"))?;
        Ok(format!("auth={auth}"))
    })
    .await
}

async fn httpbin_status_404() -> Result<String> {
    with_httpbin(|| async {
        let session = httpbin_session()?;
        let response = session.get("https://httpbin.org/status/404").await?;
        expect_httpbin_status(response.status(), StatusCode::NOT_FOUND)?;
        httpbin_eq(response.error_for_status().is_err(), "404 should be error")?;
        Ok("404 -> Err".to_string())
    })
    .await
}

async fn httpbin_redirect() -> Result<String> {
    with_httpbin(|| async {
        let session = httpbin_session()?;
        let response = session.get("https://httpbin.org/redirect/2").await?;
        expect_httpbin_status(response.status(), StatusCode::OK)?;
        let hops = response.redirect_chain().len();
        httpbin_eq(hops > 0, "redirect chain empty")?;
        Ok(format!("{hops} redirects"))
    })
    .await
}

async fn httpbin_large() -> Result<String> {
    with_httpbin(|| async {
        let session = httpbin_session()?;
        let response = session.get("https://httpbin.org/bytes/50000").await?;
        expect_httpbin_status(response.status(), StatusCode::OK)?;
        let n = response.bytes().await?.len();
        httpbin_eq(n == 50_000, format!("got {n} bytes"))?;
        Ok(format!("{n}B"))
    })
    .await
}

mod httpbin_policy {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use http::StatusCode;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    use super::{
        HTTPBIN_ATTEMPTS, HttpbinAttempt, HttpbinUnavailable, attempt_httpbin,
        expect_httpbin_status, finish, httpbin_eq, read_httpbin_json, read_httpbin_text,
        session_with_timeout, smoke, with_httpbin,
    };

    #[test]
    fn server_error_is_retried_and_other_status_fails() {
        match expect_httpbin_status(StatusCode::SERVICE_UNAVAILABLE, StatusCode::OK) {
            Err(HttpbinAttempt::Unavailable(reason)) => assert!(reason.contains("503"), "{reason}"),
            other => panic!("expected an unavailable status, got {other:?}"),
        }
        match expect_httpbin_status(StatusCode::NOT_FOUND, StatusCode::OK) {
            Err(HttpbinAttempt::Fatal(reason)) => assert!(reason.contains("404"), "{reason}"),
            other => panic!("expected a fatal status, got {other:?}"),
        }
        match expect_httpbin_status(StatusCode::BAD_REQUEST, StatusCode::NOT_FOUND) {
            Err(HttpbinAttempt::Fatal(reason)) => assert!(reason.contains("400"), "{reason}"),
            other => panic!("expected a fatal status, got {other:?}"),
        }
        expect_httpbin_status(StatusCode::NOT_FOUND, StatusCode::NOT_FOUND)
            .expect("the status the subtest asked for");
        expect_httpbin_status(StatusCode::OK, StatusCode::OK).expect("200 matches");
    }

    #[tokio::test]
    async fn httpbin_timeout_skips_and_other_timeout_fails() {
        let mut passed = 0;
        let mut failed = 0;
        let mut skipped = 0;
        finish(
            "hung httpbin",
            &mut passed,
            &mut failed,
            &mut skipped,
            true,
            std::time::Duration::from_millis(40),
            smoke(async {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                Ok("late".to_string())
            }),
        )
        .await;
        assert_eq!((passed, failed, skipped), (0, 0, 1));

        let mut passed = 0;
        let mut failed = 0;
        let mut skipped = 0;
        finish(
            "hung fingerprint",
            &mut passed,
            &mut failed,
            &mut skipped,
            false,
            std::time::Duration::from_millis(40),
            smoke(async {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                Ok("late".to_string())
            }),
        )
        .await;
        assert_eq!((passed, failed, skipped), (0, 1, 0));
    }

    #[tokio::test]
    async fn wrong_body_fails_without_retry_and_outage_skips() {
        let url = spawn_replies(vec![
            Reply {
                status: 200,
                headers: "",
                body: r#"{"json":{"test":"nope"}}"#,
            },
            Reply {
                status: 200,
                headers: "",
                body: r#"{"json":{"test":"leyline"}}"#,
            },
        ])
        .await;
        let calls = Arc::new(AtomicUsize::new(0));
        let err = attempt_httpbin(3, std::time::Duration::ZERO, {
            let url = url.clone();
            let calls = Arc::clone(&calls);
            move || {
                let url = url.clone();
                let calls = Arc::clone(&calls);
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    let session = session_with_timeout(std::time::Duration::from_secs(2))?;
                    let response = session.post(&url).await?;
                    let value = read_httpbin_json(response).await?;
                    let echoed = value["json"]["test"].as_str().unwrap_or("");
                    httpbin_eq(echoed == "leyline", format!("echoed={echoed}"))?;
                    Ok(format!("echoed={echoed}"))
                }
            }
        })
        .await
        .unwrap_err();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(err.downcast_ref::<HttpbinUnavailable>().is_none(), "{err}");
        assert!(err.to_string().contains("echoed=nope"), "{err}");

        let url = spawn_replies(vec![Reply {
            status: 503,
            headers: "",
            body: "<html>unavailable</html>",
        }])
        .await;
        let calls = Arc::new(AtomicUsize::new(0));
        let err = with_httpbin({
            let url = url.clone();
            let calls = Arc::clone(&calls);
            move || {
                let url = url.clone();
                let calls = Arc::clone(&calls);
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    let session = session_with_timeout(std::time::Duration::from_secs(2))?;
                    let response = session.get(&url).await?;
                    let body = read_httpbin_text(response, StatusCode::OK).await?;
                    httpbin_eq(body.contains("headers"), "httpbin body missing headers")?;
                    Ok(body)
                }
            }
        })
        .await
        .unwrap_err();
        assert_eq!(calls.load(Ordering::SeqCst), HTTPBIN_ATTEMPTS as usize);
        let skip = err
            .downcast_ref::<HttpbinUnavailable>()
            .unwrap_or_else(|| panic!("expected a skip, got {err}"));
        assert!(skip.to_string().contains("503"), "{skip}");
    }

    #[tokio::test]
    async fn non_json_retries_then_skips_and_bad_gzip_fails() {
        let url = spawn_replies(vec![
            Reply {
                status: 200,
                headers: "Content-Type: text/html\r\n",
                body: "<html>down</html>",
            },
            Reply {
                status: 200,
                headers: "Content-Type: application/json\r\n",
                body: r#"{"args":{"foo":"bar"}}"#,
            },
        ])
        .await;
        let calls = Arc::new(AtomicUsize::new(0));
        let detail = attempt_httpbin(3, std::time::Duration::ZERO, {
            let url = url.clone();
            let calls = Arc::clone(&calls);
            move || {
                let url = url.clone();
                let calls = Arc::clone(&calls);
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    let session = session_with_timeout(std::time::Duration::from_secs(2))?;
                    let response = session.get(&url).await?;
                    let value = read_httpbin_json(response).await?;
                    let foo = value["args"]["foo"].as_str().unwrap_or("");
                    httpbin_eq(foo == "bar", format!("foo={foo}"))?;
                    Ok(format!("foo={foo}"))
                }
            }
        })
        .await
        .expect("second body is JSON");
        assert_eq!(detail, "foo=bar");
        assert_eq!(calls.load(Ordering::SeqCst), 2);

        let url = spawn_replies(vec![Reply {
            status: 200,
            headers: "Content-Type: text/html\r\n",
            body: "not-json",
        }])
        .await;
        let calls = Arc::new(AtomicUsize::new(0));
        let err = attempt_httpbin(3, std::time::Duration::ZERO, {
            let calls = Arc::clone(&calls);
            move || {
                let url = url.clone();
                let calls = Arc::clone(&calls);
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    let session = session_with_timeout(std::time::Duration::from_secs(2))?;
                    let response = session.get(&url).await?;
                    let _ = read_httpbin_json(response).await?;
                    Ok("ok".to_string())
                }
            }
        })
        .await
        .unwrap_err();
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        let skip = err
            .downcast_ref::<HttpbinUnavailable>()
            .unwrap_or_else(|| panic!("expected a skip, got {err}"));
        assert!(skip.to_string().contains("non-JSON"), "{skip}");

        let url = spawn_replies(vec![Reply {
            status: 200,
            headers: "Content-Encoding: gzip\r\n",
            body: "this is not gzip",
        }])
        .await;
        let calls = Arc::new(AtomicUsize::new(0));
        let err = attempt_httpbin(3, std::time::Duration::ZERO, {
            let calls = Arc::clone(&calls);
            move || {
                let url = url.clone();
                let calls = Arc::clone(&calls);
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    let session = session_with_timeout(std::time::Duration::from_secs(2))?;
                    let response = session.get(&url).await?;
                    let _ = read_httpbin_text(response, StatusCode::OK).await?;
                    Ok("ok".to_string())
                }
            }
        })
        .await
        .unwrap_err();
        assert_eq!(calls.load(Ordering::SeqCst), 1, "{err}");
        assert!(err.downcast_ref::<HttpbinUnavailable>().is_none(), "{err}");
        assert!(err.to_string().contains("gzip"), "{err}");
    }

    #[tokio::test]
    async fn connect_and_read_timeouts_skip() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let calls = Arc::new(AtomicUsize::new(0));
        let err = attempt_httpbin(3, std::time::Duration::ZERO, {
            let calls = Arc::clone(&calls);
            move || {
                let calls = Arc::clone(&calls);
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    let session = session_with_timeout(std::time::Duration::from_secs(2))?;
                    let response = session.get(format!("http://{addr}/get")).await?;
                    let _ = read_httpbin_text(response, StatusCode::OK).await?;
                    Ok("ok".to_string())
                }
            }
        })
        .await
        .unwrap_err();
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        assert!(err.downcast_ref::<HttpbinUnavailable>().is_some(), "{err}");

        let url = spawn_hung().await;
        let calls = Arc::new(AtomicUsize::new(0));
        let err = attempt_httpbin(2, std::time::Duration::ZERO, {
            let calls = Arc::clone(&calls);
            move || {
                let url = url.clone();
                let calls = Arc::clone(&calls);
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    let session = session_with_timeout(std::time::Duration::from_millis(200))?;
                    let response = session.get(&url).await?;
                    let _ = read_httpbin_text(response, StatusCode::OK).await?;
                    Ok("ok".to_string())
                }
            }
        })
        .await
        .unwrap_err();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        let skip = err
            .downcast_ref::<HttpbinUnavailable>()
            .unwrap_or_else(|| panic!("expected a skip, got {err}"));
        assert!(skip.to_string().contains("timeout"), "{skip}");
    }

    #[tokio::test]
    async fn recovery_after_a_5xx_checks_the_body() {
        let url = spawn_replies(vec![
            Reply {
                status: 503,
                headers: "",
                body: "down",
            },
            Reply {
                status: 200,
                headers: "",
                body: r#"{"headers":{"Authorization":"Bearer test-token-123"}}"#,
            },
        ])
        .await;
        let calls = Arc::new(AtomicUsize::new(0));
        let detail = attempt_httpbin(3, std::time::Duration::ZERO, {
            let calls = Arc::clone(&calls);
            move || {
                let url = url.clone();
                let calls = Arc::clone(&calls);
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    let session = session_with_timeout(std::time::Duration::from_secs(2))?;
                    let response = session.get(&url).await?;
                    let value = read_httpbin_json(response).await?;
                    let auth = value["headers"]["Authorization"].as_str().unwrap_or("");
                    httpbin_eq(auth.contains("test-token-123"), format!("auth={auth}"))?;
                    Ok(format!("auth={auth}"))
                }
            }
        })
        .await
        .expect("the second response is valid");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert!(detail.contains("test-token-123"), "{detail}");
    }

    #[derive(Clone, Copy)]
    struct Reply {
        status: u16,
        headers: &'static str,
        body: &'static str,
    }

    async fn spawn_replies(replies: Vec<Reply>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let fallback = *replies.last().expect("at least one reply");
        let queue = Arc::new(Mutex::new(VecDeque::from(replies)));
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let queue = Arc::clone(&queue);
                tokio::spawn(async move {
                    let _ = read_headers(&mut socket).await;
                    let reply = {
                        let mut guard = queue.lock().unwrap_or_else(|err| err.into_inner());
                        guard.pop_front().unwrap_or(fallback)
                    };
                    let raw = format!(
                        "HTTP/1.1 {status} REPLY\r\nContent-Length: {len}\r\nConnection: close\r\n{headers}\r\n{body}",
                        status = reply.status,
                        len = reply.body.len(),
                        headers = reply.headers,
                        body = reply.body,
                    );
                    drop(socket.write_all(raw.as_bytes()).await);
                });
            }
        });
        format!("http://{addr}/get")
    }

    async fn spawn_hung() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    let _ = read_headers(&mut socket).await;
                    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                });
            }
        });
        format!("http://{addr}/get")
    }

    async fn read_headers(socket: &mut TcpStream) {
        let mut buf = Vec::new();
        let mut tmp = [0u8; 1024];
        while buf.len() < 16 * 1024 {
            let Ok(n) = socket.read(&mut tmp).await else {
                return;
            };
            if n == 0 {
                return;
            }
            buf.extend_from_slice(&tmp[..n]);
            if buf.windows(4).any(|window| window == b"\r\n\r\n") {
                return;
            }
        }
    }
}
