# Proxies

Leyline tunnels through `http://`, `https://`, `socks5://`, and `socks5h://`
proxies. This page sets one proxy, rules per scheme, bypass lists, and
environment discovery, then rotates through a proxy list, and tells a proxy
failure from an origin failure.

An `http://` proxy receives the `CONNECT` request and any
`Proxy-Authorization` credentials in cleartext; an `https://` proxy receives
them inside TLS. A plain `http://` URL goes to an `http://` or `https://`
proxy as an absolute-form request, and through a SOCKS5 proxy over a
`CONNECT` tunnel.

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

`Session::proxy()` returns the session proxy with the password replaced by
`***`. `Debug` of `Proxy` and `Session` hides it the same way.

## SOCKS5 and HTTPS proxies

A `socks5://` or `socks5h://` proxy needs the `socks` feature, which is off by
default. With either scheme the proxy resolves the host name.

```toml
[dependencies]
leyline-http = { version = "0.2", features = ["socks"] }
```

Without the feature, `build()` refuses a SOCKS URL with `Kind::Config`,
whether it is the session proxy, an entry of a proxy list, or the
proxy from the environment. A SOCKS URL string given to a request fails the
first request that uses it. The message names the
feature.

An `https://` proxy means TLS to the proxy, then `CONNECT` through it. A
session with certificate pins or a client certificate cannot use one, because
the origin TLS identity must not go to the proxy: `build()` accepts the
combination, and every request routed through the `https://` proxy fails with
`Kind::Proxy`. See [TLS trust](tls-trust.md).

## Rules per scheme

`Proxy` holds a list of proxy rules. `all(url)` applies to every
scheme, `http(url)` to `http` only, and `https(url)` to `https` only. The
first matching rule wins, in the order you added them. A `wss://` WebSocket URL matches as
`https://`; a `ws://` URL fails with `Kind::Request` before any proxy is
chosen. See [WebSocket](websocket.md).

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

## Bypass with NO_PROXY

`Proxy::no_proxy(patterns)` lists the hosts that must not go through a
proxy.

```rust
use leyline::Proxy;

let proxies = Proxy::new()
    .all("http://proxy.example:8080")
    .no_proxy(["localhost", ".internal.example", "192.0.2.1"]);
# let _ = proxies;
```

A pattern matches the host and every subdomain of it. A leading dot is
optional, a trailing `:port` is stripped, and `*` matches every host.
Matching ignores case and a trailing dot on the host.

The bypass list applies when you set it with `Proxy::no_proxy`, and
when the session proxy came from the environment. A per-request `proxy()`
replaces the session config, bypass list included.

## Environment discovery

A session with no explicit proxy reads the environment once, when it is
built. It takes the first non-empty value of `HTTPS_PROXY`, `https_proxy`,
`HTTP_PROXY`, `http_proxy`, `ALL_PROXY`, `all_proxy`, and applies it to every
URL scheme. `NO_PROXY` or `no_proxy` seeds the bypass list.

A value that is not a valid proxy URL never falls back to a direct
connection. A valid value has a scheme of `http`, `https`, `socks5`, or
`socks5h` (the last two only with the `socks` feature) and a host, so
`proxy.example:8080` is invalid. Leyline checks only the value it picked and
does not try a lower variable. The error names the variable, never the value.

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

Uppercase `HTTP_PROXY` is skipped when the process looks like a CGI handler
(the httpoxy mitigation): when any of `GATEWAY_INTERFACE`, `REQUEST_METHOD`,
`SERVER_SOFTWARE`, `SCRIPT_NAME`, `SCRIPT_FILENAME`, `PATH_INFO`,
`QUERY_STRING`, `SERVER_PROTOCOL`, `SERVER_NAME`, or `SERVER_PORT` is set.
Leyline then logs a warning on the `leyline::env_proxy::cgi` target.

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
| Any other error, `ProxyTarget` included | No change |

After `ban_after` strikes, the proxy is banned for `ban_for`, and every origin
that stuck to it loses its sticky proxy. Each retry picks the next healthy
proxy and skips the one that just failed. A request with its own `.proxy(..)`
does not use the list, and a request or WebSocket `.proxy(..)` given a list
fails with `Kind::Config`. `ban_after(u32::MAX)` keeps every proxy in rotation. Blocks are described in
[Crawling](crawling.md#classify-a-block-response).

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

On first use of a proxy, the list derives a session for it from the request's
session with `to_builder().browser(..).platform(..)`, and keeps it. That session shares the
connection pool (in its own partition) and the host limits, and has its own
TLS session cache. Unlike a session you make with `to_builder()`, it gets its own
cookie jar, which starts empty. Each proxy has its own jar, also when two
proxies share a browser and platform, so cookies set through one proxy never go out
through another. When an origin moves to another proxy, its cookies stay in
the old proxy's jar.

The session must use a browser profile. A session with `Proxy::identified`
and no browser or profile fails at `build()` with `Kind::Config`.

## Proxy errors

A failure in the proxy dial, the proxy handshake, or `CONNECT` is
`Kind::Proxy`, and `Error::proxy_status()` holds the proxy's answer to
`CONNECT`, for example `407`.

When the proxy works but cannot reach the origin, the error is
`Kind::ProxyTarget`, and `Error::proxy_status()` holds the reply: `502` or
`504` for a `CONNECT` answer, or the SOCKS5 reply code from 3 to 6.

The default retry rule retries an I/O failure on the way to the proxy and a
`CONNECT` answer of 502, 503, or 504, and nothing else here. See
[Choose what to retry](retries-and-timeouts.md#choose-what-to-retry).

### Decide whether to ban a proxy

`Error::is_proxy()` is true when the proxy itself failed, before the tunnel
was up:

- The proxy dial was refused or timed out. A timeout is `Kind::Timeout`.
- The proxy answered `CONNECT` with an error such as 407, 403, or 503.
- The SOCKS5 handshake or authentication failed.
- The TLS handshake with an `https://` proxy failed.

It is false for `ProxyTargetUnreachable` and for every error after the tunnel
is up, so it takes a proxy out of rotation without blaming it for the origin.
A proxy list applies this rule for you.

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
MASQUE (`CONNECT-UDP`, RFC 9298) is not supported.

Many proxies do not support `UDP ASSOCIATE`. When an HTTP/3 connection
through a proxy fails, `Race` skips HTTP/3 for that origin and proxy for 5
minutes. To keep every request on TCP, set `Protocol::Auto`. See
[HTTP/3](http3.md).

## Next

Read [Cookies](cookies.md).
