# Sessions

A `Session` owns everything a request needs: the browser profile, the TLS
context, the connection pool, and the cookie jar. Build one, reuse it, and
clone it into tasks. This chapter covers building, deriving, stopping, and
saving a session.

## Build a session

`Session::builder()` returns a `SessionBuilder`. `build()` returns
`Result<Session>`, so a bad proxy URL or an unknown profile fails there
rather than at the first request.

```rust,no_run
use leyline::{Browser, Platform, Session};

# fn run() -> leyline::Result<()> {
let session = Session::builder()
    .browser(Browser::Firefox154).platform(Platform::MacOS)
    .headers([("user-agent", "my-tool/1.0 (+https://my-tool.example)")])
    .headers([("accept", "application/json")])
    .build()?;
# let _ = session;
# Ok(())
# }
```

`Session::new()` and `Session::default()` build a plain session and do not
fail. `Session::from(b)` builds a session for browser `b` and does not fail
either. A trust-store problem then fails each request that needs it with
`Kind::Tls`, and an invalid proxy in the environment fails each request with
`Kind::Proxy`. To add settings to a browser session, start from
`Session::builder().browser(b)`, whose `build()` reports those problems.

`SessionBuilder::headers` takes any iterator of `(name, value)` pairs, or
references to pairs, where both sides are `AsRef<str>`. An entry replaces a
default or profile header of the same name, compared without case.
A `user-agent` pair replaces the default `user-agent`, and a later pair wins.
On a browser session, a session `user-agent` also removes the profile's `sec-ch-ua` client
hint, so the user agent and the hint never name different browsers.

When a session has a browser and you do not call `protocol`, it uses
`Protocol::Race` if the `http3` feature is on and the profile's `[h3]`
table sets `race = true`, as the bundled Chrome profiles do, and
`Protocol::Auto` otherwise. See [HTTP/3](http3.md).

## Plain and browser sessions

| Session | Build it with | Imitates |
| --- | --- | --- |
| Plain | `Session::new()` or `Session::builder().build()` | Nothing |
| Browser | `Session::from(b)`, or a builder with `.browser(b)` | A captured browser |

A plain HTTP/1.1 GET sends these headers, in this order:

| Header | Value |
| --- | --- |
| `Host` | From the URL |
| `user-agent` | `leyline/<crate version>` |
| `accept` | `*/*` |
| `accept-encoding` | The codings compiled in and offered by `accept_encoding`, in the order gzip, deflate, br, zstd. Left out when none is offered |
| `Connection` | `keep-alive`. Left out on HTTP/2 and HTTP/3 |

A plain session sends no `accept-language` and no browser-only headers:
`sec-*` headers, client hints, `upgrade-insecure-requests`, and `priority`.
This is true for every request kind. `SessionBuilder::languages` turns `accept-language` on. A
request adds the headers it needs, such as `content-type`, `origin`,
`referer`, and `cookie`.

## Set a token and a base URL

`bearer_auth(token)` sends `authorization: Bearer <token>` and replaces an
`authorization` header set earlier on the builder. With a `base_url`, the
token goes only to requests whose origin (scheme, host, and port) matches the
base URL. Without one, it goes to every host. A redirect to another origin
drops it. `RequestBuilder::bearer_auth` overrides it for one request, which
is how to reach a second host. An `authorization` header set with `headers`
is not scoped. `Debug` output masks the token.

`base_url(url)` resolves a relative `&str` or `String` request URL by the
rules of `Url::join` (RFC 3986). A `Url` value goes out unchanged.

| Base URL | Request URL | Result |
| --- | --- | --- |
| `https://api.example/v1/` | `repos/x` | `https://api.example/v1/repos/x` |
| `https://api.example/v1/` | `/health` | `https://api.example/health` |
| `https://api.example/v1` | `repos/x` | `https://api.example/repos/x` |
| any | `https://example.com/a` | `https://example.com/a` |

End the base URL with `/` to keep its last path segment. An invalid base URL
fails `build()` with `Kind::Config`. A relative URL in a session with no base
URL fails at `send()` with `Kind::Url`.

```rust,no_run
# async fn run() -> leyline::Result<()> {
let api = leyline::Session::builder()
    .base_url("https://api.example/v1/")
    .bearer_auth("my-token")
    .build()?;
let repo = api.get("repos/leyline").send().await?;
let staging = api.to_builder().base_url("https://staging.api.example/v1/").build()?;
let health = staging.get("/health").send().await?;
# drop((repo, health));
# Ok(())
# }
```

`to_builder().base_url(url)` makes a session with another base URL; the
session token moves with it and goes to the new base URL's origin. `build()`
returns `Kind::Config` for a base URL that does not parse or cannot be a
base.

## Set the languages

`SessionBuilder::languages(tags)` sets `accept-language` in the format of the session's
browser family. The first tag has the highest weight. An invalid tag or an
empty list fails `build()` with `Kind::Config`. A plain session uses the
Chromium format.

| Browser | Rule | Tags | `accept-language` |
| --- | --- | --- | --- |
| Chromium | Adds the base language after each region tag, unless the next tag has the same base. Removes duplicates. Weights go down by 0.1 from 1, to a minimum of 0.1 | `["de-DE", "de", "en-US"]` | `de-DE,de;q=0.9,en-US;q=0.8,en;q=0.7` |
| Firefox | No base language added. Canonical case. Weights go down in equal steps from 1, as Firefox computes them | `["fr", "en"]`, `["de-DE", "de", "en"]` | `fr,en;q=0.5`, `de-DE,de;q=0.7,en;q=0.3` |
| Safari | The first tag only | `["de-DE", "de"]` | `de-DE` |

Without languages, a browser session sends the value captured with its
profile. `Session::languages()` returns the tags the session was built
with.

```rust,no_run
use leyline::{Browser, Session};

# fn run() -> leyline::Result<()> {
let session = Session::builder()
    .browser(Browser::default()).languages(["de-DE", "de", "en-US"])
    .build()?;
let langs = session.languages().map(<[String]>::to_vec);
# let _ = langs;
# Ok(())
# }
```

## Choose a browser

The `Browser` enum has a variant for each bundled profile. The
[API reference](https://docs.rs/leyline-http/latest/leyline/enum.Browser.html) lists them, and
[Browser profiles](profiles.md) gives the capture kind of each.

- One function per product returns its newest build that is not deprecated
  and whose capture kind the product accepts: `Browser::chrome()`,
  `edge()`, `opera()`, `brave()`, `firefox()`, `safari()`, `safari_ios()`,
  `cfnetwork()`, and `okhttp()`. A release can move them to newer captures;
  pin a variant such as `Browser::Chrome154` to keep one fingerprint.
- `Browser::default()` is `Browser::chrome()`.
- `Browser::version()` returns the product's major version, and
  `Browser::id()` a stable id such as `chrome-154` or `edge-154`. The engine
  family is the `meta.family` field of `Browser::profile()`.

```rust
use leyline::Browser;

assert_eq!(Browser::default(), Browser::chrome());
assert_eq!(Browser::Edge154.id(), "edge-154");
assert_eq!(Browser::Opera139.version(), 139);
```

## Choose a platform

`Platform` sets the operating system the session claims: `Windows`, `MacOS`,
`Linux`, `Android`, `IOS`, or `Host`. `Platform::Host` resolves to the OS you
compiled for, or `Windows` for an unrecognized target. The platform drives
`Sec-CH-UA-Platform`, `Sec-CH-UA-Mobile`, and the TCP fingerprint.

### Platform defaults and call order

- `.browser(browser)` claims the first platform, in the order Windows,
  macOS, Linux, Android, iOS, that its profile covers. Chrome claims Windows,
  `Browser::Safari26` macOS, and `Browser::SafariIOS27` iOS.
  `Session::from(b)` uses the same rule.
- `.platform(platform)` always wins, called before or after `.browser(..)`.
  It maps the browser to its sibling for the platform.
  `Browser::for_platform` does the same.
- `.profile(profile).platform(platform)` and
  `Session::builder().platform(platform)` take the platform as given.
- A session with no browser, profile, or platform claims the host platform.
- A second `.browser(..)` call replaces the first. Languages stay until
  `.languages(..)` sets new ones, so `to_builder().browser(..)` keeps the
  parent's.

## Pick Edge or Opera

Edge and Opera are `Browser` variants built on the Chrome captures:
`Browser::Edge145` to `Browser::Edge156`, and `Browser::Opera129` to
`Browser::Opera139`. The TLS and HTTP/2 fingerprint stays Chrome's, and the
`User-Agent` and `sec-ch-ua` brand list change. Edge also leaves out the
trust anchors extension of the ClientHello.

```rust,no_run
use leyline::{Browser, Platform, Session};

# fn run() -> leyline::Result<()> {
let edge = Session::from(Browser::edge());
let opera = Session::builder()
    .browser(Browser::Opera139).platform(Platform::MacOS)
    .build()?;
# let _ = (edge, opera);
# Ok(())
# }
```

An Edge version equals its Chrome version. Each Opera version runs on the
Chrome version that `profiles/brands.toml` maps it to: `Browser::Opera139`
sends Chrome 155's TLS. Both exist on Windows, macOS, and
Linux only; `Platform::Android` or `Platform::IOS` fails `build()` with
`Kind::Config`. Brave has its own profiles, such as `Browser::Brave146`.

`Browser::headers(platform)` returns the identity headers a session
sends as `(name, value)` pairs: `user-agent` first, then `sec-ch-ua` when the
browser sends it, then `accept-language` when the profile has one. Use it when a payload
that is not a header must carry the same values. It returns `None` when the
browser has no identity for the platform.

`Session::browser()`, `hello()`, `platform()`, and `languages()` return
the values `build()` resolved. `browser()` and `hello()` are `None` for a
plain or loaded-profile session. `Session::user_agent()`
returns the `User-Agent` the session sends, including one from
`SessionBuilder::headers`.

```rust,no_run
use leyline::{Browser, Platform};

let edge = Browser::Edge154.headers(Platform::Windows);
assert!(edge.is_some_and(|headers| headers[0].1.contains("Edg/")));

let session = leyline::Session::from(Browser::default());
println!("{:?} on {} sends {}", session.browser(), session.platform(), session.user_agent());
```

## Share a session

A clone is an `Arc` clone: it shares the cookie jar, the connection pool, the
TLS context, and every setting. `Session` is `Send + Sync`. Pass clones into
tasks instead of building a second session.

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let worker = session.clone();
tokio::spawn(async move { worker.get("https://example.com/").send().await })
    .await
    .expect("task panicked")?;
# Ok(())
# }
```

`host_limits(HostLimits)` and a `Proxy::list` are shared by clones
and sessions made with `to_builder()` too. See [Crawling](crawling.md#limit-each-host) and
[Proxies](proxies.md).

## Derive a session

`session.to_builder()` returns a `SessionBuilder` that holds the session's
settings and its cookie jar. Change any setting, then call `build()`. The
new session shares the rest with its parent, and `build()` checks the
settings as it does for a new session.

| Change | Cookie jar | Pool | TLS session cache |
| --- | --- | --- | --- |
| None (`clone()`) | Shared | Shared | Shared |
| `proxy`, `redirect`, `base_url`, `headers`, `retry`, `timeout` with the same connect limit, and other request settings | Shared | Shared | Shared |
| `cookies(jar)` | The new jar | Shared | Shared |
| `identity` | Shared | Shared, own partition | New |
| A connection setting: `max_connections`, `max_connections_per_host`, `pool_idle_timeout`, `keep_alive`, `http2_ping_after_idle`, `http2_ping_timeout`, `dns_resolver`, `resolve`, a socket setter such as `local_address`, `tcp_nodelay`, or `happy_eyeballs_delay`, `tls`, `tcp_profile`, `accept_encoding`, `max_body_size`, `protocol`, or a `timeout` with another connect limit | Shared | New | New |

Every derived session keeps the shutdown state. It keeps the host limits
unless you set `host_limits`.

- Pool entries are keyed by proxy, so connections never cross proxies.
- A new identity's connections sit in their own partition of the parent's
  pool, so they never cross identities, and they share the parent's pool
  limits and `stats()`. They also share the HSTS store and the
  `Alt-Svc` knowledge. See
  [Mix the hello and the headers](fingerprints.md#mix-the-hello-and-the-headers).
- `share_connections(false)` starts with an empty pool, TLS session cache,
  and HSTS store, with its own pool limits. Set it when the next request must
  open new connections.
- `tab()` returns a `Tab` that keeps the current page. See
  [Requests](requests.md#keep-the-page-with-a-tab).

```rust,no_run
use leyline::cookie::Jar;

# fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let other_user = session.to_builder().cookies(Jar::new()).build()?;
let new_exit = session
    .to_builder()
    .proxy("http://user:pass@proxy.example:8080")
    .share_connections(false)
    .build()?;
let no_follow = session
    .to_builder()
    .redirect(leyline::Redirect::none())
    .build()?;
# let _ = (other_user, new_exit, no_follow);
# Ok(())
# }
```

## Stop a session

`Session::shutdown()` stops a session, its clones, and every session derived
from it. Requests in flight and new requests fail with `Kind::Request` and the
message "session shut down", and a streamed body fails on its next read.
`Error::is_shut_down()` is `true` for this error, and `Session::is_shut_down()`
is `true` after the call.

WebSocket handshakes and `Session::preconnect` follow the same rule: a call
after shutdown fails with the shut-down error before it sends any bytes, and a
call in flight ends with that error. An established `WebSocket` stays open
until the caller closes it. Shutdown does not close idle pooled connections.

```rust,no_run
use leyline::Kind;

# async fn run() {
let session = leyline::Session::new();
let worker = session.clone();
session.shutdown().await;
assert!(worker.is_shut_down());
let err = worker.get("https://example.com/").send().await.unwrap_err();
assert_eq!(err.kind(), Kind::Request);
assert!(err.is_shut_down());
# }
```

## Save and restore a session

`Session` implements `Serialize`, and `SessionBuilder` implements
`Deserialize`. `Browser` and `Platform`
serialize as stable lowercase ids, such as `chrome-154`, `edge-154`, and
`windows`; `id()` returns the id and `str::parse`
reads it back. `Display` prints a label for people, not the id.

`serde_json::to_string(&session)` saves the session, and
`serde_json::from_str::<SessionBuilder>(&json)?.build()?` rebuilds it.
`Session::save(path)` and `Session::load(path)` do the same with a file.

`profile_id()` returns 16 hex characters: a hash of the profile data the
session sends, with the browser and the platform. The same data gives the same
id in every release. It is `None` for a plain session. A profile loaded from
TOML gets an id from its TOML text. Compare it with a saved id after
`build()` to catch a profile change. `build()` on the builder from `Session::load` makes the same check for
a saved session, and `Error::is_profile_changed()` is `true` for its error.

```rust,no_run
use leyline::{Browser, Platform, Session, SessionBuilder};

# fn run() -> Result<(), Box<dyn std::error::Error>> {
let session = Session::builder()
    .browser(Browser::Chrome154).platform(Platform::MacOS)
    .build()?;
let saved = serde_json::to_string(&session)?;
let profile_id = session.profile_id().ok_or("plain session")?;

let restored = serde_json::from_str::<SessionBuilder>(&saved)?.build()?;
if restored.profile_id() != Some(profile_id) {
    return Err("the profile changed".into());
}
# Ok(())
# }
```

To save the identity with the cookie jar, the proxy, the TLS session tickets, the `Alt-Svc` entries, and the HSTS store, use `Session::save` and `Session::load`; see [Accounts](accounts.md).

## Keep connections warm

`preconnect(url)` opens the TCP, proxy, TLS, and HTTP/2 connection for an
`https` origin and stores it in the pool, so the first request skips the
handshake. It does nothing for an `http` URL or under
`Protocol::Http1`. If the origin speaks only HTTP/1.1, the session
records that and returns `Ok`. To warm a connection through another proxy,
call `session.to_builder().proxy(proxy).build()?.preconnect(url)`.

Before the pool reuses an HTTP/2 connection idle for 10 seconds, it sends a
PING, as Chrome does. Without an answer in 2 seconds it drops the connection
and opens a new one, and `stats().h2_ping_failures()` counts it.
[Network](network.md#connection-pool) lists the pool setters.

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::from(leyline::Browser::default());
session.preconnect("https://example.com/").await?;
println!("{:?}", session.stats());
# Ok(())
# }
```

## Trace the request lifecycle

`trace()` installs a listener for each phase of every request: name
resolution, connect, TLS handshake, request sent, response head, and
completion. Implement `leyline::trace::Trace` and override only the events you
want; every method has a no-op default. `Log` is the bundled
listener; see [Logging and tracing](logging.md).

```rust,no_run
use leyline::Session;
use leyline::trace::{Event, Trace};

struct Slow;

impl Trace for Slow {
    fn event(&self, event: &Event<'_>) {
        if let Event::Head { host, elapsed, .. } = event
            && elapsed.as_millis() > 500
        {
            eprintln!("{host} took {elapsed:?} to answer");
        }
    }
}

# fn run() -> leyline::Result<()> {
let session = Session::builder().trace(Slow).build()?;
# let _ = session;
# Ok(())
# }
```

- Every phase event (`dns` through `done`) carries an `id` unique per
  attempt. A retry is a new attempt with a new `id`. `summary` and `body`
  carry the request id, one per `send`, which no phase event shares.
- An event fires on the task that produced it. Leyline opens an HTTP/2 or
  HTTP/3 connection on a task it spawns, so those events fire there, with the
  `id` of the request that started the open. A listener that blocks slows
  every request waiting for that connection.
- `dns` fires each time Leyline resolves a name for a TCP connection. Through
  a proxy it reports the proxy host. A direct HTTP/3 connection fires none.
- `sent` carries the method and the path with the query. On HTTP/2 and
  HTTP/3 it fires before the connection driver takes the request, and its
  `elapsed` is zero.
- `head` carries the response headers as received, before decompression and
  redirect handling.

`Response::reused`, `connect_time`, `send_time`, and `elapsed` need no listener. See [Responses](responses.md#timing).
