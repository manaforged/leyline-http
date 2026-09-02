# leyline

HTTP client that mimics browsers on the wire. One obvious call per task, errors that say what to change, and rustdoc that stands on its own.

Bundled: Chrome 152, Firefox 154, Safari 26 (WKWebView TLS/H2; HTTP identity is synthesized when the dump omits `Safari/`). Edge is a brand overlay on Chrome (same ClientHello on Windows and macOS; UA platform tokens differ). `Session::chrome()` races HTTP/3 against HTTP/2 only for origins that advertised `h3` in `Alt-Svc` or already completed a QUIC handshake; other origins get one TCP handshake, as in Chrome. HTTP/3 QPACK advertises a 0-capacity table; the decoder has no dynamic table. MIT. `leyline-bssl` is Apache-2.0. `leyline-bssl-tokio` is MIT OR Apache-2.0. BoringSSL and quiche licenses live in [NOTICE](NOTICE).

```toml
[dependencies]
leyline-http = "0.1"
tokio = { version = "1", features = ["full"] }
```

```rust,no_run
use leyline::Session;

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let session = Session::chrome();
    let resp = session
        .post("https://example.com/api")
        .header("x-request-id", "1")
        .header("content-type", "application/json")
        .body(r#"{"ok":true}"#)
        .await?;
    println!("{}", resp.status());
    Ok(())
}
```

## One call per task

Every task below has one path. Await the builder for the default send, or chain `.send()`.

```rust,no_run
use leyline::{RetryPolicy, Session};
use std::time::Duration;

# async fn run() -> leyline::Result<()> {
let session = Session::chrome();

// GET, then read the body.
let text = session.get("https://example.com/").await?.text().await?;

// POST JSON. The preset is inferred from content-type; no header dance.
let created: serde_json::Value = session
    .post("https://example.com/items")
    .json(&serde_json::json!({ "name": "leyline" }))
    .await?
    .json()
    .await?;

// Stream a large response instead of buffering it.
let resp = session.get("https://example.com/big").stream().await?;
let mut body = resp.into_stream()?;
while let Some(chunk) = futures_util::StreamExt::next(&mut body).await {
    let _bytes = chunk?;
}

// Retry 429, 502, 503, 504, connect errors, and timeouts on one request.
let resp = session
    .get("https://example.com/flaky")
    .retry(RetryPolicy::transient().with_max_retries(4))
    .await?;

// Turn a 4xx or 5xx into an Error that carries the status and a body prefix.
let ok = resp.error_for_status()?;

// One request through a different proxy.
let via = session
    .get("https://example.com/ip")
    .proxy("socks5://user:pass@proxy.example:1080")
    .timeout(Duration::from_secs(10))
    .await?;

// WebSocket over the same session and cookies.
let mut ws = session.websocket("wss://example.com/live").connect().await?;
ws.send("hello").await?;
if let Some(msg) = ws.recv().await? {
    println!("{msg:?}");
}
# let _ = (text, created, ok, via);
# Ok(())
# }
```

Session-wide settings live on the builder, one method per concern:

```rust,no_run
use leyline::{Browser, Platform, ProxyConfig, ProxyRule, RetryPolicy, Session, TimeoutConfig};
use std::time::Duration;

# fn build() -> leyline::Result<Session> {
let session = Session::builder()
    .browser(Browser::Firefox154)
    .platform(Platform::MacOS)
    .proxies(ProxyConfig::new().with_rule(ProxyRule::all("http://proxy.example:8080")))
    .timeouts(
        TimeoutConfig::default()
            .total(Duration::from_secs(60))
            .connect(Duration::from_secs(5)),
    )
    .retry(RetryPolicy::transient())
    .trace(leyline::trace::TracingTrace)
    .audit(true)
    .build()?;
# Ok(session)
# }
```

`.layer(...)`, behind the `tower` feature, wraps each request attempt in a Tower stack for logging or header edits; see [Requests](docs/guide/requests.md).

`.trace(...)` reports each request's DNS, connect, TLS, send, response-head, and completion events to a listener you write; see [Sessions](docs/guide/sessions.md).

`resp.audit()` returns the JA3, JA4, JA4T, and Akamai HTTP/2 fingerprints the session sends, so you can compare them with what a server logged.

## `http` types

The public surface speaks the `http` crate, the way reqwest and hyper do.
`resp.status()` is an `http::StatusCode`, `resp.headers()` yields
`(&HeaderName, &HeaderValue)` in wire order with duplicates intact, and
`resp.header_map()` copies them into an `http::HeaderMap` when you want lookup
instead of order. `Session::request` takes an `http::Method` and anything that
parses as an `http::Uri`; header setters take anything that converts to a
`HeaderName` or `HeaderValue`, so `&str` keeps working and an invalid name or
value surfaces from `send`. The crate re-exports `http`, so you share one
version of these types.

```rust,no_run
use leyline::Session;
use leyline::http::Method;

# async fn run() -> leyline::Result<()> {
let session = Session::chrome();
let resp = session.request(Method::DELETE, "https://example.com/items/1").await?;
if resp.status().is_success() {
    for (name, value) in resp.headers() {
        println!("{name}: {}", value.to_str().unwrap_or_default());
    }
}
# Ok(())
# }
```

## Errors

`leyline::Error` is one struct. `err.kind()` names the layer that failed: `Builder`, `Request`, `Redirect`, `Status`, `Body`, `Decode`, `Timeout`, `Connect`, `Tls`, `Http2`, `Http3`, `Proxy`, `Io`, `Config`, `Url`, `Json`.

`is_timeout()`, `is_connect()`, and `is_connection_closed()` answer the questions a retry loop asks; `is_status()`, `is_redirect()`, `is_body()`, and `is_decode()` cover the rest. `status()` returns the `StatusCode` a failed `error_for_status()` carries, `body_prefix()` returns the first 16 KiB of that response body, and `url()` returns the request URL. Call `without_url()` to drop the URL before you log the error; `Debug` already replaces any userinfo with `***`.

`std::error::Error::source()` returns the wrapped `TlsError`, `H2Error`, `std::io::Error`, `url::ParseError`, or `serde_json::Error`, so `?` keeps the original cause. Messages say what to change, for example a streaming body that cannot be replayed tells you to buffer it or set `max_redirects(0)`.

## Examples

`cargo run -p leyline-http --example <name>`: `oneliner`, `post_json`, `headers`, `cookies`, `streaming`, `retry`, `concurrent`, `presets`, `http3`, `audit`, `trace`, `websocket`.

Longer walkthroughs live in the [user guide](docs/guide/README.md).

## Targets

Prebuilt BoringSSL ships for `aarch64-apple-darwin`, `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, and `x86_64-pc-windows-msvc`. Other targets need `BORING_BSSL_PATH` pointing at a BoringSSL build; the build script prints the steps.
