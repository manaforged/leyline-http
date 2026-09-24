# Sessions

A `Session` owns everything a request needs: the browser profile, the TLS
context, the connection pool, and the cookie jar. Build one and reuse it.

## Build a session

`Session::builder()` returns a `SessionBuilder`. Each concern has one method.
`build()` returns `Result<Session>`, because a bad proxy URL or an unknown
profile fails here rather than at the first request.

```rust,no_run
use leyline::{Browser, Platform, Session};

# fn run() -> leyline::Result<()> {
let session = Session::builder()
    .browser(Browser::Firefox154)
    .platform(Platform::MacOS)
    .build()?;
# let _ = session;
# Ok(())
# }
```

`Session::new()` skips the builder. It selects `Browser::default()`
with a Windows identity, and it uses `ProtocolPolicy::Race` when the `http3`
feature is on. `Session::default()` is the same session. Every other
configuration goes through `Session::builder()`.

`Session::builder().build()` with no browser builds a bare session. A bare
session impersonates nothing.

## Choose a browser

The `Browser` enum lists every bundled profile: Chrome 145 to 152, Brave 146,
Firefox 148 to 154, Safari 18 and 26, Safari on iOS 17 and iOS 18, OkHttp on
Android 10, and the CFNetwork stacks on iOS 18 and macOS 26.

These helpers select a profile without naming a variant:

- `Browser::latest(Family)` is the highest bundled version of a product line,
  so you pin the line and take whatever the crate release carries. The
  families are `Chrome`, `Brave`, `Firefox`, `Safari`, `SafariIos`,
  `CfNetwork`, and `OkHttp`.
- `Browser::default()` is what `Session::new()` selects: the latest Chrome.
- `Browser::get(Family, version)` returns the bundled profile for one version,
  or `None`. `Browser::version()` returns the major version.

```rust
use leyline::Browser;
use leyline::Family;

let latest = Browser::latest(Family::Firefox);
assert_eq!(latest.family(), Family::Firefox);
assert_eq!(Browser::default().family(), Family::Chrome);
```

`Browser::family` returns the product line as a `Family` value, for example
`Family::Chrome` or `Family::SafariIos`. The engine family (`chromium`, `gecko`,
`webkit`) is the `meta.family` field of the profile that
`Browser::profile()` returns. `Browser::for_platform` maps a profile to the
sibling that exists on a platform.

## Choose a platform

`Platform` sets the operating system the session claims: `Windows`, `MacOS`,
`Linux`, `Android`, `IOS`, or `Host`. `Platform::Host` resolves to the OS you
compiled for, and falls back to `Windows` for an unrecognized target.

The platform drives the `Sec-CH-UA-Platform` header, the `Sec-CH-UA-Mobile`
flag, and the TCP fingerprint.

Pass the platform to the builder with `.platform(Platform::MacOS)`.

### Platform defaults and call order

- If you select a browser and no platform, the session claims Windows. The
  builder logs one `info` event under the `leyline::session` target the first
  time this happens.
- `Browser::Safari26` has a macOS identity only. `.browser(Browser::Safari26)`
  with no platform fails at `build()` with `Kind::Config`. Add
  `.platform(Platform::MacOS)`.
- `.browser()` and `.platform()` map the browser to its sibling for the
  selected platform. The call order does not matter.
- A session with no browser claims the host platform.

## Apply a brand overlay

Edge, Opera, and Vivaldi are Chromium browsers. Leyline treats them as an
identity overlay on a Chrome profile: the ClientHello and the HTTP/2 settings
stay Chrome's, while the `User-Agent`, the `sec-ch-ua` brand list, and a few
extra headers change.

```rust,no_run
use leyline::{Browser, ChromiumBrand, Session};

# fn run() -> leyline::Result<()> {
let session = Session::builder()
    .browser(Browser::default())
    .brand(ChromiumBrand::Edge)
    .build()?;
# let _ = session;
# Ok(())
# }
```

`ChromiumBrand::Chrome` is stock Chrome. Put Edge and Opera on
`Browser::default()`. Put Vivaldi on `Browser::Chrome147`, the last
major with a recorded Vivaldi build string.

Brave is not an overlay. `.browser(Browser::Brave146)` selects a first-class
profile.

If no capture exists for that brand, Chromium version, and platform, `build()`
returns an error that names all three.

`Browser::identity(platform, brand)` returns the `PlatformIdentity` a session
sends for that browser, platform, and brand: `user_agent`, `sec_ch_ua`,
`accept_language`, and the extra headers. Use it when a payload that is not a
header must carry the same values. It returns `None` when no capture exists.

```rust,no_run
use leyline::{Browser, ChromiumBrand, Platform};

let edge = Browser::default().identity(Platform::Windows, Some(ChromiumBrand::Edge));
assert!(edge.is_some_and(|id| id.user_agent.contains("Edg/")));
```

## What a session shares

One session holds:

- The cookie jar. Every request reads it and every response writes to it.
- The connection pool, keyed by host, port, and proxy.
- The TLS context built from the profile, plus the HTTP/2 and HTTP/3 settings.

A clone is cheap and shares the pool, the jar, and the TLS context with the
original. Pass clones into tasks instead of building a second session.

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let worker = session.clone();
tokio::spawn(async move { worker.get("https://example.com/").await })
    .await
    .expect("task panicked")?;
# Ok(())
# }
```

## Derive a session

`with_proxy(config)` derives a new session from an existing one and keeps the
expensive parts. It takes `impl Into<ProxyConfig>`, keeps the cookie jar, TLS,
the identity, and the pool, and swaps only the proxy config. Pool entries are
keyed by proxy URL, so connections never cross proxies, and a session with the
same proxy reuses the warm connections. It does not fail: an invalid URL or an
unsupported scheme fails the first `send()` that picks it, with
`Kind::Config`.

`fresh_pool()` derives a session with a new, empty pool and TLS session cache.
Call it when the next request must open new connections.

`with_cookie_jar(jar)` derives a session the same way and swaps only the
cookie jar. The pool, TLS session cache, and every other setting stay shared.
Use it to run one jar per task on a warm pool.

```rust,no_run
use leyline::cookie::Jar;

# fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let other_user = session.with_cookie_jar(Jar::new());
let via_proxy = session.with_proxy("http://user:pass@proxy.example:8080");
let new_exit = via_proxy.fresh_pool();
# let _ = (other_user, new_exit);
# Ok(())
# }
```

## Keep connections warm

`preconnect(url)` opens the TCP, proxy, TLS, and HTTP/2 connection for an
`https` origin and stores it in the pool. The first request to that origin
then reuses it and skips the handshake. For an `http` URL, or when the session
uses `ProtocolPolicy::Http1`, `preconnect` does nothing. It uses the session
proxy config. To warm a connection through another proxy, call
`session.with_proxy(proxy).preconnect(url)`: the derived session shares the
pool. If the origin only speaks HTTP/1.1, the session records that and returns
`Ok`.

Before the pool reuses an HTTP/2 connection that has been idle for 10 seconds,
it sends a PING. Chrome does the same. If no acknowledgement arrives within 2
seconds, the pool drops the connection and opens a new one, so the request does
not wait on a dead socket. `pool_stats().h2_ping_failures` counts the dropped
connections. Change the thresholds with `PoolConfig::h2_ping_after_idle` and
`PoolConfig::h2_ping_timeout`. Pass `None` to `h2_ping_after_idle` to turn the
check off.

```rust,no_run
use std::time::Duration;

use leyline::PoolConfig;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::builder()
    .browser(leyline::Browser::default())
    .pool(PoolConfig::new().h2_ping_after_idle(Duration::from_secs(10)))
    .build()?;
session.preconnect("https://example.com/").await?;
# Ok(())
# }
```

## Read back what you built

`pool_stats()` reports the pool counters. The `Debug` output of a session
shows its browser, platform, and proxy. Use them in logs instead of restating
your own builder calls.

## Trace the request lifecycle

`trace()` installs a listener that reports each phase of every request: name
resolution, connect, TLS handshake, request send, response head, and
completion. Implement `leyline::trace::Trace` and override only the events you
want; every method has a no-op default.

```rust,no_run
use leyline::Session;
use leyline::trace::{Head, Trace};

struct Slow;

impl Trace for Slow {
    fn head(&self, ev: &Head<'_>) {
        if ev.elapsed.as_millis() > 500 {
            eprintln!("{} took {:?} to answer", ev.host, ev.elapsed);
        }
    }
}

# fn run() -> leyline::Result<()> {
let session = Session::builder().trace(Slow).build()?;
# let _ = session;
# Ok(())
# }
```

Every event carries an `id` that is unique per attempt, so a listener shared by
concurrent requests can group phases. A retried request is a new attempt with a
new `id`.

One listener ships with the crate. `leyline::trace::TracingTrace` writes each
event as a `tracing` debug event under the `leyline::trace` target.
`Response::timing()` returns the same numbers as a `ResponseTiming`.

The events fire inline on the request task. A listener that blocks, locks, or
sleeps slows the request that produced the event.

`dns` fires where the client resolves the name itself, which is every `https://`
connect. On a plaintext `http://` connect the operating system resolves inside
`connect`, so only `connect` fires. `sent` carries the request method and the
path with the query. On HTTP/2 and HTTP/3, `sent` fires when the
request is handed to the connection driver, and its `elapsed` covers framing
only.

## Next

Read [Requests](requests.md) to shape a single request.
