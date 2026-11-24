# Proxies

Leyline tunnels through `http://`, `https://`, `socks5://`, and `socks5h://`
proxies. This page sets one proxy, rules per scheme, bypass lists, and
environment discovery, then rotates through a proxy list, and tells a proxy
failure from an origin failure.

An `http://` proxy receives the `CONNECT` request and any
`Proxy-Authorization` credentials in cleartext; an `https://` proxy receives
them inside TLS. A plain `http://` URL goes to an `http://` or `https://`
proxy as a full-URL ("absolute-form") HTTP request, and through a SOCKS5 proxy
over a `CONNECT` tunnel.

## Choose a proxy mode

| Mode | Cookies | TLS session cache | Saved by `Session::save`? | Best for |
| --- | --- | --- | --- | --- |
| Single `Proxy` (`"http://..."`) | Shared session jar | Shared | Yes (without password) | Persistent logged-in accounts, corporate proxy |
| `Proxy::list(urls)` | Shared session jar (warns: cookies travel across exit IPs) | Shared | No (restores direct) | Stateless scraping or rotating IPs with `cookies(None)` |
| `Proxy::identified(triples)` | Isolated empty jar per proxy | Isolated per proxy | No (neither proxies nor per-proxy jars) | Multi-identity browser scraping pools without cross-IP cookie leaks |
| `Proxy::custom(f)` | Shared session jar | Shared | No | Dynamic per-URL / geo / gateway routing |

For persistent logged-in accounts, use one `Session` per account with one
`Proxy` and `Session::save`. For rotating scraping pools, use `Proxy::identified`
(or `Proxy::list` with `.cookies(None)`).

## Set one proxy

Every proxy setter takes `impl Into<Proxy>`: a URL string or a `Proxy`.
That covers `SessionBuilder::proxy` and
`RequestBuilder::proxy`, which a WebSocket opened with `RequestBuilder::websocket` also uses.
A URL applies to every scheme.

```rust,no_run
# fn run() -> leyline::Result<()> {
let session = leyline::Session::builder()
    .proxy("http://user:pass@proxy.example:8080")
    .build()?;
# let _ = session;
# Ok(())
# }
```

`build()` validates the session proxy, also on a session from
`to_builder()`, and refuses a scheme other than the four above or a URL with
no host with `Kind::Config`. A URL given to a request or a WebSocket is
validated when a request picks it, and an invalid one fails that `send()`
with `Kind::Config`.

`Session::proxy()` returns `Option<&Url>` with the password replaced by `***`
(`None` for `Proxy::list`, `Proxy::identified`, and `Proxy::custom`). `Debug`
of `Proxy` and `Session` masks credentials the same way.

## SOCKS5 and HTTPS proxies

A `socks5://` or `socks5h://` proxy needs the `socks` feature, which is off by
default. With either scheme the proxy resolves the host name.

```toml
[dependencies]
leyline-http = { version = "0.3", features = ["socks"] }
```

Without the feature, `build()` refuses a SOCKS URL with `Kind::Config` (session
proxy, proxy list entry, or environment proxy). A per-request `socks5://` URL
without the feature fails that request with `Kind::Proxy`. The message names
the feature.

An `https://` proxy means TLS to the proxy, then `CONNECT` through it. A
session with certificate pins or a client certificate cannot use one, because
the origin TLS identity must not go to the proxy: `build()` accepts the
combination, and every request routed through the `https://` proxy fails with
`Kind::Proxy`. See [TLS trust](tls-trust.md).

## Rules per scheme

`Proxy` holds per-scheme proxy rules: `http(url)` for `http://`, `https(url)`
for `https://`, and `all(url)` as the fallback for both. Scheme-specific rules
(`http`/`https`) take precedence over `all` regardless of call order, and
calling a setter again replaces that scheme's rule. A `wss://` WebSocket URL
matches as `https://`; a `ws://` URL fails with `Kind::Request` before any
proxy is chosen. See [WebSocket](websocket.md).

```rust,no_run
use leyline::{Proxy, Session};

# fn run() -> leyline::Result<()> {
let proxies = Proxy::new()
    .https("http://secure-proxy.example:8080")
    .http("http://plain-proxy.example:3128");
let session = Session::builder().proxy(proxies).build()?;
# let _ = session;
# Ok(())
# }
```

## Custom proxy headers

`Proxy::header` and `Proxy::headers` attach extra headers to plain-HTTP proxy
requests and `CONNECT` tunnel handshakes (HTTP/HTTPS proxies only, not SOCKS5),
for example sticky-session tags or a custom `Proxy-Authorization` header that
overrides URL userinfo:

- Header values are redacted in `Debug` output.
- Invalid or framing headers (`host`, `content-length`, `transfer-encoding`) fail `build()` (or `send()` for per-request proxies) with `Kind::Config`.
- Proxy headers are not written by `Session::save`, so credentials never land on disk; re-apply them on the loaded builder when needed.

```rust,no_run
use leyline::{Proxy, Session};

# fn run() -> leyline::Result<()> {
let proxy = Proxy::from("http://proxy.example:8080")
    .header("x-proxy-session", "sticky-1");
let session = Session::builder().proxy(proxy).build()?;
# let _ = session;
# Ok(())
# }
```

## Bypass with NO_PROXY

`Proxy::no_proxy(patterns)` lists hosts (and optional ports) that must go
direct.

```rust
use leyline::Proxy;

let proxies = Proxy::new()
    .all("http://proxy.example:8080")
    .no_proxy(["localhost", ".internal.example", "192.0.2.1", "[::1]:8080"]);
# let _ = proxies;
```

- A pattern matches the host and every subdomain of it; a leading dot is optional, `*` matches every host, and matching ignores case and a trailing dot.
- A trailing `:port` (including bracketed IPv6 such as `[::1]:8080`) restricts the bypass to that port (matched against the URL's explicit or default scheme port).
- The bypass list applies when set via `Proxy::no_proxy` or environment discovery. A per-request `proxy()` replaces the session proxy config, bypass list included.

## Environment discovery

A session with no explicit proxy reads environment variables once at build time:

| Variables (first non-empty wins) | Applies to |
| --- | --- |
| `HTTPS_PROXY`, `https_proxy` | `https://` and `wss://` URLs |
| `HTTP_PROXY`, `http_proxy` | `http://` URLs |
| `ALL_PROXY`, `all_proxy` | Fallback when `https` or `http` is unset (skipped if both are set) |
| `NO_PROXY`, `no_proxy` | Bypass list (`no_proxy`) |

Every selected variable must be a valid URL with a host and one of `http`,
`https`, `socks5`, or `socks5h` (the last two require the `socks` feature);
bare `proxy.example:8080` is rejected. Invalid values never fall back to direct
connections, and the error names the variable, never its value:

- `SessionBuilder::build()` returns `Kind::Config`.
- `Session::new()` and `Session::from(..)` return a session in which every
  request fails with `Kind::Proxy`, hosts in `NO_PROXY` included.

Fix or unset the variable, set a proxy with `SessionBuilder::proxy`, or turn
discovery off. A proxy set on a request, a WebSocket, or with
`to_builder().proxy` replaces the failed state too.

```rust,no_run
# fn run() -> leyline::Result<()> {
let session = leyline::Session::builder()
    .proxy(leyline::Proxy::new().env(false))
    .build()?;
# let _ = session;
# Ok(())
# }
```

Uppercase `HTTP_PROXY` is skipped when standard CGI variables (such as
`GATEWAY_INTERFACE`, `REQUEST_METHOD`, or `SCRIPT_NAME`) are set (the httpoxy
mitigation), logging a warning on `leyline::env_proxy::cgi`.

## Per-request override

`RequestBuilder::proxy` replaces the session proxy config for one request. A
`Proxy::new()` with no rule sends that request direct.
`session.to_builder().proxy(..)` makes a session that differs only in its
proxy. It shares the cookie jar and the connection pool, and keeps the TLS
context and the identity. See [Sessions](sessions.md#derive-a-session).

The pool is keyed by host, port, and proxy, so two proxies never share a
connection, and a return to a proxy reuses its warm connections.
`to_builder().share_connections(false)` makes a session with a new, empty pool
and TLS session cache, so the next request opens a new socket through the
same proxy.

```rust,no_run
# async fn run(proxy: Option<&str>) -> leyline::Result<()> {
let session = leyline::Session::new();
let mut request = session.get("https://example.com/ip");
if let Some(p) = proxy {
    request = request.proxy(p);
}
let resp = request.send().await?;
let rotated = session
    .to_builder()
    .proxy("http://gateway.example:8080")
    .share_connections(false)
    .build()?;
# let _ = (resp, rotated);
# Ok(())
# }
```

## Which proxy an attempt uses

Every attempt, retries included, takes the first of these that is set:

1. The request's own `.proxy(..)`.
2. The session's `Proxy`, from `SessionBuilder::proxy` or the
   environment. A proxy list picks one entry per attempt.

To rotate proxies across retries, give the session a list. To rotate for a
few requests only, derive a session that carries one:

```rust,no_run
use leyline::{Proxy, Retry};

# async fn run(session: leyline::Session) -> leyline::Result<()> {
let rotating = session
    .to_builder()
    .proxy(Proxy::list([
        "http://proxy-a.example:8080",
        "http://proxy-b.example:8080",
    ]))
    .build()?;
let resp = rotating
    .get("https://shop.example/item/1")
    .retry(Retry::new(3).when(|attempt| attempt.is_transient() || attempt.status().is_some_and(|s| s.as_u16() == 403)))
    .send()
    .await?;
# let _ = resp;
# Ok(())
# }
```

The usual retry rules apply: a trigger must match, the method must be
idempotent or allowed, and the body must be replayable. A `Retry-After` wait
still runs before the next proxy. When retries run out you get the last
response, for example the 403. See
[Retries and timeouts](retries-and-timeouts.md).

## Choose the proxy with a closure

`Proxy::custom(choose)` calls your closure with the URL of each request and
of each redirect hop. Return `Some(proxy)` to send that request through a
proxy, or `None` to send it direct.

```rust,no_run
use leyline::{Proxy, Session};

# fn run() -> leyline::Result<()> {
let session = Session::builder()
    .proxy(Proxy::custom(|url| {
        url.path()
            .starts_with("/geo/")
            .then(|| Proxy::from("http://user:pass@proxy.example:8080"))
    }))
    .build()?;
# let _ = session;
# Ok(())
# }
```

Limits:

- The returned `Proxy` must hold one proxy; returning `Proxy::list`, `Proxy::identified`, or another `Proxy::custom` fails the request with `Kind::Config`. Nesting `Proxy::custom` inside `Proxy::list` or `Proxy::identified` fails `build()` with `Kind::Config`.
- Environment proxies do not apply to a session with `Proxy::custom`.
- `no_proxy`, `header`, and `headers` on the `Proxy::custom` wrapper still apply (`no_proxy` bypasses the closure; `all`, `http`, and `https` on it have no effect).
- The closure runs once per request and per redirect hop, so it can keep a counter to rotate.
- `Session::save` does not save the closure.

## Use a proxy list

`Proxy::list(urls)` makes a proxy that picks one of its URLs for each
request, with stickiness and bans:

1. The sticky proxy of the origin, if it is not banned and was picked within
   `sticky_for`.
2. Else the next healthy proxy, round-robin.
3. If every proxy is banned, the proxy whose ban ends first.

```rust,no_run
use std::time::Duration;
use leyline::{Proxy, Session};

# fn run() -> leyline::Result<()> {
let proxies = Proxy::list([
    "http://user:pass@proxy-a.example:8080",
    "http://user:pass@proxy-b.example:8080",
])
.sticky_for(Duration::from_secs(300))
.ban_after(3)
.ban_for(Duration::from_secs(60));
let session = Session::builder()
    .proxy(proxies.clone())
    .block_statuses([403, 429])
    .build()?;
# let _ = session;
# Ok(())
# }
```

| Setter | Default | Effect |
| --- | --- | --- |
| `sticky_for(d)` | zero, no stickiness | Keep an origin on the same proxy for `d` |
| `ban_after(n)` | 3 | Strikes before a ban. The smallest value is 1 |
| `ban_for(d)` | 60 s | Length of a ban |

| Outcome of a request | Effect on its proxy |
| --- | --- |
| An error with `is_proxy()` | A strike. The origin loses its sticky proxy |
| A response where `block()` is `Some` | A strike. The origin loses its sticky proxy |
| Any other response | The strikes reset |
| Any other error (including origin-unreachable [`Kind::ProxyTarget`](#proxy-errors)) | No change |

- After `ban_after` strikes, the proxy is banned for `ban_for` and loses all sticky origins.
- Each retry moves to the next healthy proxy (skipping the one that just failed). Pair `Proxy::list` with `block_statuses([403, 429])` and `Retry::new(3).when(|a| a.is_transient() || a.is_block())` to rotate immediately on blocks (omit `HostLimits::pause_on([429])` if you want other proxies to keep going).
- `Proxy::list` shares one cookie jar across all proxies — pass `.cookies(None)` for stateless scraping or use [`Proxy::identified`](#pin-a-browser-and-platform-to-each-proxy) to isolate cookies per proxy.
- A per-request `.proxy(..)` bypasses the list, and passing a `Proxy::list` to a single request or WebSocket fails with `Kind::Config`. Set `ban_after(u32::MAX)` to keep every proxy in rotation. See [Classify a block response](crawling.md#classify-a-block-response).

Clones of a list share its health state. `Session::stats()` reports it:
`proxies()` lists each proxy without its password, and `proxy_in_use(p)`,
`proxy_failures(p)`, and `proxy_banned_until(p)` read one proxy. A ban time
can be in the past when the ban has ended.

```rust,no_run
use std::time::Instant;

# fn run(session: leyline::Session) {
let now = Instant::now();
let stats = session.stats();
for proxy in stats.proxies() {
    let banned = stats.proxy_banned_until(proxy).is_some_and(|until| until > now);
    println!(
        "{proxy} in_use={} failures={} banned={banned}",
        stats.proxy_in_use(proxy),
        stats.proxy_failures(proxy)
    );
}
# }
```

### Pin a browser and platform to each proxy

`Proxy::identified` takes `(proxy, Browser, Platform)` triples. A request
through a proxy sends that proxy's browser and platform.

```rust,no_run
use leyline::{Browser, Platform, Proxy, Session};

# fn run() -> leyline::Result<()> {
let proxies = (1..=20).map(|n| format!("http://user:pass@proxy-{n}.example:8080"));
let browsers = [
    (Browser::chrome(), Platform::Windows),
    (Browser::firefox(), Platform::MacOS),
];
let pool = Proxy::identified(
    proxies.zip(browsers.into_iter().cycle()).map(|(proxy, (b, p))| (proxy, b, p)),
);
let session = Session::builder()
    .browser(Browser::default())
    .proxy(pool)
    .build()?;
# let _ = session;
# Ok(())
# }
```

On first use of a proxy, the pool derives and caches a session for it via
`to_builder().browser(..).platform(..)`:

- **Shared with the parent session:** connection pool (in its own partition) and `host_limits`.
- **Isolated per proxy:** cookie jar (starts empty, even when two proxies share a browser and platform), TLS session cache, and browser/platform identity. Cookies never cross between proxies, and `session.cookies()` reads/writes only the parent jar (not per-proxy jars).
- **Parent browser & saving:** The parent session must use a browser profile (`Proxy::identified` on a plain session fails `build()` with `Kind::Config`; the parent profile handles `no_proxy` direct requests). `Session::save` does not persist `Proxy::identified` or its per-proxy cookie jars.

## Proxy errors

A failure in the proxy dial, proxy handshake, or `CONNECT` is `Kind::Proxy`,
with `Error::proxy_status()` carrying the proxy's status (such as `407`). When
the proxy works but cannot reach the origin, the error is `Kind::ProxyTarget`,
with `Error::proxy_status()` holding `502`/`504` (HTTP `CONNECT`) or SOCKS5
reply codes `3`–`6`. See [Predicates](errors.md#predicates) for which proxy
errors `Error::is_retryable()` and `Error::is_proxy()` match.

### Decide whether to ban a proxy

`Error::is_proxy()` is `true` only when the proxy itself failed before the
tunnel was up (including a proxy connect timeout, `CONNECT` `407`/`403`/`503`,
SOCKS5 auth/handshake errors, and `https://` proxy TLS errors), and `false` for
`Kind::ProxyTarget` and every post-tunnel origin error. A `Proxy::list` applies
this rule automatically; with manual per-request proxies, match on it directly:

```rust,no_run
use leyline::Kind;

# async fn run(session: leyline::Session, proxy: &str) -> leyline::Result<()> {
match session.get("https://example.com/").proxy(proxy).send().await {
    Ok(resp) => println!("{}", resp.status()),
    Err(err) if err.is_proxy() => println!("ban {proxy}: {err}"),
    Err(err) if err.kind() == Kind::ProxyTarget => {
        println!("origin unreachable through the proxy: {:?}", err.proxy_status())
    }
    Err(err) => println!("origin error: {err}"),
}
# Ok(())
# }
```

## HTTP/3 through a SOCKS5 proxy

With the `socks` feature, HTTP/3 runs over a SOCKS5 proxy. Leyline opens a
TCP control connection, authenticates, and sends `UDP ASSOCIATE` (RFC 1928,
section 7). Each QUIC datagram goes to the relay with the SOCKS5 UDP header,
which names the target host, so the proxy resolves it. When the control
connection closes, the QUIC connection fails and leaves the pool. The connect
timeout covers the control connection, `UDP ASSOCIATE`, and the QUIC
handshake.

An `http://` or `https://` proxy cannot carry HTTP/3, and neither can a SOCKS
proxy without the `socks` feature. `build()` rejects `Protocol::Http3`
when such a proxy takes every URL, and an `Http3` request fails with
`Kind::Config` when the rules pick such a proxy for its URL. Under
`Protocol::Race`, a request through such a proxy takes the `Auto` path.
HTTP/3 over HTTP proxies (`MASQUE` / `CONNECT-UDP`, RFC 9298) is not supported.

Many proxies do not support `UDP ASSOCIATE`. When an HTTP/3 connection
through a proxy fails, `Race` skips HTTP/3 for that origin and proxy for 5
minutes. To keep every request on TCP, set `Protocol::Auto`. See
[HTTP/3](http3.md).
