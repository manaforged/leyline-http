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
    .browser(Browser::Firefox154)
    .platform(Platform::MacOS)
    .user_agent("my-tool/1.0 (+https://my-tool.example)")
    .headers([("accept", "application/json")])
    .build()?;
# let _ = session;
# Ok(())
# }
```

`Session::new()` and `Session::default()` build a plain session and do not
fail. `Session::browser(b)` builds a session for browser `b` and does not fail
either. A trust-store problem then fails each request that needs it with
`Kind::Tls`, and an invalid proxy in the environment fails each request with
`Kind::Proxy`. To add settings to a browser session, start from
`Session::builder().browser(b)`, whose `build()` reports those problems.

`SessionBuilder::headers` takes any iterator of `(name, value)` pairs, or
references to pairs, where both sides are `AsRef<str>`. An entry replaces a
default or profile header of the same name, compared without case.
`user_agent(value)` replaces the default `user-agent`; it and a `user-agent`
pair in `headers` set the same header, and the later call wins. On a browser
session, a session `user-agent` also removes the profile's `sec-ch-ua` client
hint, so the user agent and the hint never name different browsers.

When a session has a browser and you do not call `protocol`, it uses
`ProtocolPolicy::Race` if the `http3` feature is on and the profile's `[h3]`
table sets `race = true`, as the bundled Chrome profiles do, and
`ProtocolPolicy::Auto` otherwise. See [HTTP/3](http3.md).

## Plain and browser sessions

| Session | Build it with | Imitates |
| --- | --- | --- |
| Plain | `Session::new()`, `Session::builder().build()`, or `leyline::get(url)` for one request | Nothing |
| Browser | `Session::browser(b)`, or a builder with `.browser()`, `.profile()`, or `.identity()` | A captured browser |

A plain HTTP/1.1 GET sends these headers, in this order:

| Header | Value |
| --- | --- |
| `Host` | From the URL |
| `user-agent` | `leyline/<crate version>` |
| `accept` | `*/*` |
| `accept-encoding` | The codings compiled in and on in `CompressionConfig`, in the order gzip, deflate, br, zstd. Left out when none is on |
| `Connection` | `keep-alive`. Left out on HTTP/2 and HTTP/3 |

A plain session sends no `accept-language` and no browser-only headers:
`sec-*` headers, client hints, `upgrade-insecure-requests`, and `priority`.
This is true for every preset. `languages` turns `accept-language` on. A
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
let repo = api.get("repos/leyline").await?;
let staging = api.with_base_url("https://staging.api.example/v1/")?;
let health = staging.get("/health").await?;
# drop((repo, health));
# Ok(())
# }
```

`with_base_url(url)` derives a session with another base URL; the session
token moves with it and goes to the new base URL's origin. It returns
`Kind::Url` for a URL that does not parse, and `Kind::Config` for a URL that
cannot be a base.

## Set the languages

`languages(tags)` sets `accept-language` in the format of the session's
browser family. The first tag has the highest weight. An invalid tag or an
empty list fails `build()` with `Kind::Config`. A plain session uses the
Chromium format.

| Family | Rule | Tags | `accept-language` |
| --- | --- | --- | --- |
| Chromium | Adds the base language after each region tag, unless the next tag has the same base. Removes duplicates. Weights go down by 0.1 from 1, to a minimum of 0.1 | `["de-DE", "de", "en-US"]` | `de-DE,de;q=0.9,en-US;q=0.8,en;q=0.7` |
| Firefox | No base language added. Canonical case. Weights go down in equal steps from 1, as Firefox computes them | `["fr", "en"]`, `["de-DE", "de", "en"]` | `fr,en;q=0.5`, `de-DE,de;q=0.7,en;q=0.3` |
| Safari | The first tag only | `["de-DE", "de"]` | `de-DE` |

Without `languages`, a browser session sends the value captured with its
profile. `SessionIdentity::languages()` returns the tags the session was built
with.

```rust,no_run
use leyline::{Browser, Session};

# fn run() -> leyline::Result<()> {
let session = Session::builder()
    .browser(Browser::default())
    .languages(["de-DE", "de", "en-US"])
    .build()?;
let langs = session.identity().languages().map(<[String]>::to_vec);
# let _ = langs;
# Ok(())
# }
```

## Choose a browser

The `Browser` enum has a variant for each bundled profile. The
[API reference](../reference/leyline-http/leyline.md#browser) lists them, and
[Browser profiles](profiles.md) gives the capture kind of each.

- `Browser::latest(Family)` is the newest profile of a product line that is
  not deprecated and whose capture kind the line accepts. Pin the line and
  take what the crate release carries.
- `Browser::default()` is `Browser::latest(Family::Chrome)`.
- `Browser::get(Family, version)` returns the profile for one major version,
  or `None`. `Browser::version()` returns the major version.
- `Browser::family()` returns the product line. The engine family is the
  `meta.family` field of `Browser::profile()`.

```rust
use leyline::{Browser, Family};

let latest = Browser::latest(Family::Firefox);
assert_eq!(latest.family(), Family::Firefox);
assert_eq!(Browser::default().family(), Family::Chrome);
```

## Choose a platform

`Platform` sets the operating system the session claims: `Windows`, `MacOS`,
`Linux`, `Android`, `IOS`, or `Host`. `Platform::Host` resolves to the OS you
compiled for, or `Windows` for an unrecognized target. The platform drives
`Sec-CH-UA-Platform`, `Sec-CH-UA-Mobile`, and the TCP fingerprint.

### Platform defaults and call order

- A browser with no platform claims the first platform, in the order Windows,
  macOS, Linux, Android, iOS, that its profile covers. Chrome claims Windows,
  `Browser::Safari26` macOS, and `Browser::SafariIOS27` iOS.
  `Session::browser(b)` uses the same rule.
- A `.profile()` with no `.platform()` claims Windows and logs one `info`
  event under the `leyline::session` target.
- `.browser()` and `.platform()` map the browser to its sibling for the
  platform, in either call order. `Browser::for_platform` does the same.
- A session with no browser claims the host platform.

## Apply a brand overlay

Edge and Opera are overlays on a Chrome profile: the HTTP/2 settings stay
Chrome's, and the `User-Agent` and `sec-ch-ua` brand list change. Edge also
leaves out the trust anchors extension of the ClientHello.

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

Both brands exist on desktop platforms only; a mobile platform fails
`build()`. Edge takes its version from Chromium and fits every desktop Chrome
profile. Opera supports Chromium 145 to 153, so `Browser::default()` with
`ChromiumBrand::Opera` can fail `build()`; pin an older Chrome such as
`Browser::Chrome153`. Brave has its own profiles, such as
`Browser::Brave146`.

`ChromiumBrand::all()` lists the brands. A brand prints as its lowercase name
(`chrome`, `edge`, `opera`), and `str::parse` reads it back without case. An
unknown name is a `Kind::Config` error.

`Browser::identity(platform, brand)` returns the `PlatformIdentity` a session
sends: `user_agent`, `sec_ch_ua`, and `accept_language`. Use it when a payload
that is not a header must carry the same values. It returns `None` when the
browser has no identity for the platform or the brand does not apply.

`Session::identity()` returns a `SessionIdentity` with the values `build()`
resolved: `identity()` (`None` for a plain or loaded-profile session),
`browser()`, `platform()`, `brand()`, and `user_agent()`. A `user-agent` from
`SessionBuilder::headers` is the value `user_agent()` returns.

```rust,no_run
use leyline::{Browser, ChromiumBrand, Platform};

let edge = Browser::default().identity(Platform::Windows, Some(ChromiumBrand::Edge));
assert!(edge.is_some_and(|id| id.user_agent.contains("Edg/")));

let session = leyline::Session::browser(Browser::default());
let sent = session.identity();
println!("{:?} on {} sends {}", sent.browser(), sent.platform(), sent.user_agent());
```

## Share a session

A clone is an `Arc` clone: it shares the cookie jar, the connection pool, the
TLS context, and every setting. `Session` is `Send + Sync`. Pass clones into
tasks instead of building a second session.

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

`host_limits(HostLimits)` and `proxy_pool(ProxyPool)` are shared by clones
and derived sessions too. See [Crawling](crawling.md#limit-each-host) and
[Proxies](proxies.md).

## Derive a session

A derived session changes one setting and shares the rest with its parent.
None of the calls below fail except `with_base_url` and `with_identity`.

| Derive | Changes | Cookie jar | Pool | TLS session cache |
| --- | --- | --- | --- | --- |
| `clone()` | Nothing | Shared | Shared | Shared |
| `with_proxy(config)` | The proxy | Shared | Shared | Shared |
| `with_cookie_jar(jar)` | The jar | The new jar | Shared | Shared |
| `with_redirect(policy)` | The redirect policy | Shared | Shared | Shared |
| `with_base_url(url)` | The base URL | Shared | Shared | Shared |
| `with_identity(identity)` | The browser identity | Shared | Shared, own partition | New |
| `fresh_pool()` | Nothing | Shared | New | New |

Every derived session keeps the host limits, the proxy pool, the audit
setting, and the shutdown state.

- `with_proxy` takes `impl Into<ProxyConfig>`. Pool entries are keyed by
  proxy, so connections never cross proxies. An invalid URL or an unsupported
  scheme fails the first `send()` that uses it, with `Kind::Config`.
- `with_identity` sends another `Identity`. Its connections sit in their own
  partition of the parent's pool, so they never cross identities, and they
  share the parent's pool limits and `pool_stats()`. It also shares the HSTS
  store and the `Alt-Svc` knowledge. It returns `Kind::Config` for a plain
  session. See
  [Mix and rotate identities](fingerprints.md#mix-and-rotate-identities).
- `fresh_pool` starts with an empty pool, TLS session cache, and HSTS store,
  and has its own pool limits. Call it when the next request must open new
  connections.
- `tab()` returns a `Tab` that keeps the current page. See
  [Requests](requests.md#keep-the-page-with-a-tab).

```rust,no_run
use leyline::cookie::Jar;

# fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let other_user = session.with_cookie_jar(Jar::new());
let via_proxy = session.with_proxy("http://user:pass@proxy.example:8080");
let new_exit = via_proxy.fresh_pool();
let no_follow = session.with_redirect(leyline::RedirectPolicy::none());
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
call in flight ends with that error. An established `WsConnection` stays open
until the caller closes it. Shutdown does not close idle pooled connections.

```rust,no_run
use leyline::Kind;

# async fn run() {
let session = leyline::Session::new();
let worker = session.clone();
session.shutdown();
assert!(worker.is_shut_down());
let err = worker.get("https://example.com/").await.unwrap_err();
assert_eq!(err.kind(), Kind::Request);
assert!(err.is_shut_down());
# }
```

## Save and restore an identity

`Identity` implements `Serialize` and `Deserialize`. `Browser`, `Platform`,
`Family`, and `ChromiumBrand` serialize as stable lowercase ids, such as
`chrome-154`, `windows`, and `edge`; `id()` returns the id and `str::parse`
reads it back. `Display` prints a label for people, not the id.

`session.identity().to_identity()` returns the `Identity` that rebuilds the
session, and `SessionBuilder::identity(id)` builds from it.

`profile_id()` returns 16 hex characters: a hash of the profile data the
session sends, with the platform and the brand. The same data gives the same
id in every release. It is `None` for a plain session. A profile loaded from
TOML gets an id from its TOML text. `SessionBuilder::expect_profile_id(id)`
makes `build()` fail with `Kind::Config` when the id differs or is `None`;
`Error::is_profile_changed()` is `true` for that error.

```rust,no_run
use leyline::{Browser, Identity, Platform, Session};

# fn run() -> Result<(), Box<dyn std::error::Error>> {
let session = Session::builder()
    .browser(Browser::Chrome154)
    .platform(Platform::MacOS)
    .build()?;
let sent = session.identity();
let saved = serde_json::to_string(&sent.to_identity())?;
let profile_id = sent.profile_id().map(str::to_owned).ok_or("plain session")?;

let identity: Option<Identity> = serde_json::from_str(&saved)?;
let restored = Session::builder()
    .identity(identity.ok_or("plain session")?)
    .expect_profile_id(&profile_id)
    .build()?;
# drop(restored);
# Ok(())
# }
```

To save the identity with the cookie jar and the proxy, use a `Device`; see
[Accounts](accounts.md). `Session::state()` returns the TLS session tickets,
the `Alt-Svc` entries, and the HSTS store as a `SessionState`; see
[Accounts](accounts.md#keep-the-connection-state).

## Keep connections warm

`preconnect(url)` opens the TCP, proxy, TLS, and HTTP/2 connection for an
`https` origin and stores it in the pool, so the first request skips the
handshake. It does nothing for an `http` URL or under
`ProtocolPolicy::Http1`. If the origin speaks only HTTP/1.1, the session
records that and returns `Ok`. To warm a connection through another proxy,
call `session.with_proxy(proxy).preconnect(url)`.

Before the pool reuses an HTTP/2 connection idle for 10 seconds, it sends a
PING, as Chrome does. Without an answer in 2 seconds it drops the connection
and opens a new one, and `pool_stats().h2_ping_failures` counts it.
[Network](network.md#connection-pool) lists the `PoolConfig` settings.

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::browser(leyline::Browser::default());
session.preconnect("https://example.com/").await?;
println!("{:?}", session.pool_stats());
# Ok(())
# }
```

## Trace the request lifecycle

`trace()` installs a listener for each phase of every request: name
resolution, connect, TLS handshake, request sent, response head, and
completion. Implement `leyline::trace::Trace` and override only the events you
want; every method has a no-op default. `TracingTrace` is the bundled
listener; see [Logging and tracing](logging.md).

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

- Every event carries an `id` unique per attempt. A retry is a new attempt
  with a new `id`.
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

`Response::timing()` needs no listener. See [Responses](responses.md#timing).
