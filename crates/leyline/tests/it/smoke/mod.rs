mod fake_server;
mod httpbin;
mod httpbin_policy;
mod live;

use std::future::Future;
use std::pin::Pin;
use std::time::{Duration, Instant};

use leyline::{Browser, Family, Session};
use serde_json::Value;

use httpbin::{
    HttpbinUnavailable, httpbin_bearer, httpbin_budget, httpbin_cdn_get, httpbin_large,
    httpbin_post_form, httpbin_post_json, httpbin_query, httpbin_redirect, httpbin_status_404,
};
use live::{exact_fingerprint, firefox, h1_wire_shape, h3_get, safari};

const PEET_URL: &str = "https://tls.peet.ws/api/all";
const SMOKE_TIMEOUT: Duration = Duration::from_secs(25);
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
