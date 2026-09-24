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

The convenience constructors skip the builder and panic if construction
fails, for example when a built-in profile is missing or a required
compression feature is disabled: `Session::chrome()`, `Session::firefox()`,
`Session::safari()`, `Session::edge()`, `Session::brave()`, `Session::opera()`,
and `Session::vivaldi()`.

`Session::new()` and `Session::default()` build a bare session. A bare session
impersonates nothing: `session.browser()` returns `None`.

## Choose a browser

The `Browser` enum lists every bundled profile: Chrome 145 to 152, Brave 146,
Firefox 148 to 154, Safari 18 and 26, Safari on iOS 17 and iOS 18, OkHttp on
Android 10, and the CFNetwork stacks on iOS 18 and macOS 26.

Three helpers name a current version instead of a fixed one:

- `Browser::latest(Family)` is the highest bundled version of a product line,
  so you pin the line and take whatever the crate release carries. The
  families are `Chrome`, `Brave`, `Firefox`, `Safari`, `SafariIos`,
  `CfNetwork`, and `OkHttp`.
- `Browser::default_browser()` is what `Session::chrome()` selects.
- `Browser::default_firefox()` is what `Session::firefox()` selects.

```rust
use leyline::Browser;
use leyline::profile::Family;

let latest = Browser::latest(Family::Firefox);
assert_eq!(latest.family(), "firefox");
assert_eq!(Browser::default_browser().family(), "chrome");
```

`Browser::family` returns the product line as a string, for example
`"chrome"` or `"safari-ios"`. The engine family (`chromium`, `gecko`,
`webkit`) is the `meta.family` field of the profile that
`ProfileRegistry::global().get_browser(browser)` returns. `Browser::for_platform` maps a
profile to the sibling that exists on a platform. `Browser::hello_rep` names
the profile that owns the ClientHello, since several versions share one.

## Choose a platform

`Platform` sets the operating system the session claims: `Windows`, `MacOS`,
`Linux`, `Android`, `IOS`, or `Host`. `Platform::Host` resolves to the OS you
compiled for, and falls back to `Windows` for an unrecognized target.

The platform drives the `Sec-CH-UA-Platform` header, the `Sec-CH-UA-Mobile`
flag, and the TCP fingerprint.

```rust
use leyline::Platform;

assert_eq!(Platform::Windows.sec_ch_platform(), "Windows");
assert_eq!(Platform::Android.mobile_flag(), "?1");
```

The builder also has one method per platform: `.windows()`, `.macos()`,
`.linux()`, `.android()`, and `.ios()`.

### Platform defaults and call order

- If you select a browser and no platform, the session claims Windows. The
  builder logs one `info` event under the `leyline::session` target the first
  time this happens.
- `.safari()` and `.brave()` claim macOS when no platform is set.
- `Browser::Safari26` has a macOS identity only. `.browser(Browser::Safari26)`
  with no platform fails at `build()` with `Kind::Config`. Use `.safari()`, or
  add `.macos()`.
- `.platform()` maps a browser that is already set to its sibling for that
  platform. `.browser()` after `.platform()` does not map. Call `.platform()`
  after `.browser()`, or use `.profile(browser, platform)`.
- A session with no browser claims the host platform.

## Apply a brand overlay

Edge, Opera, and Vivaldi are Chromium browsers. Leyline treats them as an
identity overlay on a Chrome profile: the ClientHello and the HTTP/2 settings
stay Chrome's, while the `User-Agent`, the `sec-ch-ua` brand list, and a few
extra headers change.

```rust,no_run
use leyline::{ChromiumBrand, Session};

# fn run() -> leyline::Result<()> {
let session = Session::builder().chrome().brand(ChromiumBrand::Edge).build()?;
assert_eq!(session.brand(), Some(ChromiumBrand::Edge));
# Ok(())
# }
```

`ChromiumBrand::Chrome` is stock Chrome. `.edge()`, `.opera()`, and
`.vivaldi()` are shorthand for `.brand(...)` on the matching Chrome version: Edge and
Opera sit on `Browser::default_browser()`, and Vivaldi sits on Chrome 147, the
last major with a recorded Vivaldi build string.

Brave is not an overlay. `Session::brave()` selects `Browser::Brave146`, a
first-class profile.

If no capture exists for that brand, Chromium version, and platform, `build()`
returns a `BrandOverlayError` that names all three.

## What a session shares

One session holds:

- The cookie jar. Every request reads it and every response writes to it.
- The connection pool, keyed by host, port, and proxy.
- The TLS context built from the profile, plus the HTTP/2 and HTTP/3 settings.

A clone is cheap and shares the pool, the jar, and the TLS context with the
original. Pass clones into tasks instead of building a second session.

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::chrome();
let worker = session.clone();
tokio::spawn(async move { worker.get("https://example.com/").await })
    .await
    .expect("task panicked")?;
# Ok(())
# }
```

## Derive a session

Two methods derive a new session from an existing one and keep the expensive
parts:

- `with_cookie_jar(jar)` keeps the TLS connector, the protocol configuration,
  and the pool, and swaps in an independent jar.
- `with_proxy(url)` keeps the cookie jar, TLS, the identity, and the pool,
  and swaps only the proxy. Pool entries are keyed by proxy, so connections
  never cross proxies. A rebind to the current proxy URL takes a fresh pool,
  so the next request opens new connections. It returns `Result<Session>`: an invalid URL or an
  unsupported scheme fails here, the same way `build()` does.

```rust,no_run
use leyline::cookie::Jar;

# fn run() -> leyline::Result<()> {
let session = leyline::Session::chrome();
let other_user = session.with_cookie_jar(Jar::new());
let via_proxy = session.with_proxy("http://user:pass@proxy.example:8080")?;
# let _ = (other_user, via_proxy);
# Ok(())
# }
```

## Keep connections warm

`preconnect(url)` opens the TCP, proxy, TLS, and HTTP/2 connection for an
`https` origin and stores it in the pool. The first request to that origin
then reuses it and skips the handshake. For an `http` URL, or when the session
uses `ProtocolPolicy::Http1`, `preconnect` does nothing. If the origin only
speaks HTTP/1.1, the session records that and returns `Ok`.

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
    .chrome()
    .pool_config(PoolConfig::new().h2_ping_after_idle(Duration::from_secs(10)))
    .build()?;
session.preconnect("https://example.com/").await?;
# Ok(())
# }
```

## Read back what you built

`browser()`, `platform()`, `brand()`, `identity()`, `protocol_policy()`,
`default_timeout()`, `response_header_timeout()`, and `pool_stats()` report the session's
settled configuration. Use them in logs instead of restating your own builder
calls.

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

Two listeners ship with the crate. `leyline::trace::TracingTrace` writes each
event as a `tracing` debug event under the `leyline::trace` target.
`leyline::trace::Timing` records the same numbers as `ResponseTiming` and hands
them back through `snapshot()`.

The events fire inline on the request task. A listener that blocks, locks, or
sleeps slows the request that produced the event.

`dns` fires where the client resolves the name itself, which is every `https://`
connect. On a plaintext `http://` connect the operating system resolves inside
`connect`, so only `connect` fires. On HTTP/2 and HTTP/3, `sent` fires when the
request is handed to the connection driver, and its `elapsed` covers framing
only.

## Next

Read [Requests](requests.md) to shape a single request.
