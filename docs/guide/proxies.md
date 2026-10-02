# Proxies

Leyline tunnels through `http://`, `https://`, `socks5://`, and `socks5h://`
proxies. This page sets one proxy, rules per scheme, bypass lists, and
environment discovery, then rotates proxies with a retry list or a
`ProxyPool`, and tells a proxy failure from an origin failure.

An `http://` proxy receives the `CONNECT` request and any
`Proxy-Authorization` credentials in cleartext; an `https://` proxy receives
them inside TLS. A plain `http://` URL goes to an `http://` or `https://`
proxy as an absolute-form request, and through a SOCKS5 proxy over a
`CONNECT` tunnel.

## Set one proxy

Every proxy setter takes `impl Into<ProxyConfig>`: a URL string, a
`ProxyUrl`, or a `ProxyConfig`. That covers `SessionBuilder::proxy`,
`RequestBuilder::proxy`, `WebSocketBuilder::proxy`, and `Session::with_proxy`.
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

`build()` validates the session proxy. A URL string given to a request, a
WebSocket, or `with_proxy` is validated when a request picks it, and an
invalid one fails that `send()` with `Kind::Config`. To fail early, pass a
`ProxyUrl`: `ProxyUrl::parse` refuses any other scheme and a URL with no host.

```rust
use leyline::ProxyUrl;

assert!(ProxyUrl::parse("http://proxy.example:8080").is_ok());
assert!(ProxyUrl::parse("ftp://proxy.example").is_err());
```

`Debug` of `ProxyUrl`, `ProxyRule`, and `Session`, and `Display` of
`ProxyUrl`, replace the password with `***`. `ProxyUrl` implements
`Serialize` and `Deserialize`, and serde keeps the full URL, password
included, so protect a file that holds one.

## SOCKS5 and HTTPS proxies

A `socks5://` or `socks5h://` proxy needs the `socks` feature, which is off by
default. With either scheme the proxy resolves the host name.

```toml
[dependencies]
leyline-http = { version = "0.1", features = ["socks"] }
```

Without the feature, `ProxyUrl::parse` and `build()` refuse a SOCKS URL with
`Kind::Config`, whether it is the session proxy, a `ProxyPool` entry, or the
proxy from the environment. A SOCKS URL string given to a request or to
`with_proxy` fails the first request that uses it. The message names the
feature.

An `https://` proxy means TLS to the proxy, then `CONNECT` through it. A
session with certificate pins or a client certificate cannot use one, because
the origin TLS identity must not go to the proxy: `build()` accepts the
combination, and every request routed through the `https://` proxy fails with
`Kind::Proxy`. See [TLS trust](tls-trust.md).

## Rules per scheme

`ProxyConfig` holds a list of `ProxyRule`s. A rule applies to every scheme
(`ProxyRule::all`), to `http` only, or to `https` only. The first matching
rule wins, in the order you added them. A `wss://` WebSocket URL matches as
`https://`; a `ws://` URL fails with `Kind::Request` before any proxy is
chosen. See [WebSocket](websocket.md).

```rust,no_run
use leyline::{ProxyConfig, ProxyRule, Session};

# fn run() -> leyline::Result<()> {
let proxies = ProxyConfig::new()
    .rule(ProxyRule::https("http://secure-proxy.example:8080"))
    .rule(ProxyRule::http("http://plain-proxy.example:3128"));
let session = Session::builder().proxy(proxies).build()?;
# let _ = session;
# Ok(())
# }
```

## Bypass with NO_PROXY

`NoProxy::new(patterns)` matches hosts that must not go through a proxy.

```rust
use leyline::{NoProxy, ProxyConfig, ProxyRule};

let bypass = NoProxy::new(["localhost", ".internal.example", "192.0.2.1"]);
let proxies = ProxyConfig::new()
    .rule(ProxyRule::all("http://proxy.example:8080"))
    .no_proxy(bypass);
# let _ = proxies;
```

A pattern matches the host and every subdomain of it. A leading dot is
optional, a trailing `:port` is stripped, and `*` matches every host.
Matching ignores case and a trailing dot on the host.

The bypass list applies when you set it with `ProxyConfig::no_proxy`, and
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
- `Session::new()` and `Session::browser(..)` return a session in which every
  request fails with `Kind::Proxy`, hosts in `NO_PROXY` included.

Fix or unset the variable, set a proxy with `SessionBuilder::proxy`, or turn
discovery off. A proxy set on a request, a WebSocket, or with `with_proxy`
replaces the failed state too.

```rust,no_run
# fn run() -> leyline::Result<()> {
let session = leyline::Session::builder()
    .proxy(leyline::ProxyConfig::new().env(false))
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
`ProxyConfig::new()` with no rule sends that request direct.
`Session::with_proxy` derives a session that differs only in its proxy. It
shares the cookie jar and the connection pool, and keeps the TLS context and
the identity. See [Sessions](sessions.md#derive-a-session).

The pool is keyed by host, port, and proxy, so two proxies never share a
connection, and a return to a proxy reuses its warm connections.
`Session::fresh_pool` derives a session with a new, empty pool and TLS session
cache, so the next request opens a new socket through the same proxy.

```rust,no_run
# async fn run(proxy: Option<&str>) -> leyline::Result<()> {
let session = leyline::Session::new();
let mut request = session.get("https://example.com/ip");
if let Some(p) = proxy {
    request = request.proxy(p);
}
let resp = request.await?;
let rotated = session.with_proxy("http://gateway.example:8080").fresh_pool();
# let _ = (resp, rotated);
# Ok(())
# }
```

## Rotate proxies on retry

`RetryPolicy::rotate_proxies(list)` moves each retry to the next proxy in a
list. The first attempt uses the request's or the session's proxy. Retry `k`,
counted from 1, uses `list[(k - 1) % list.len()]`. On a session with a
`ProxyPool`, the pool picks the proxy and `rotate_proxies` is ignored.

```rust,no_run
use leyline::RetryPolicy;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let policy = RetryPolicy::transient()
    .on_status(403)
    .rotate_proxies(["http://proxy-a.example:8080", "http://proxy-b.example:8080"]);
let resp = session
    .get("https://shop.example/item/1")
    .retry(policy)
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

## Use a proxy pool

For bans and stickiness across many requests, give the session a `ProxyPool`
with `SessionBuilder::proxy_pool`. The pool picks a proxy for each request:

1. The sticky proxy of the origin, if it is not banned and was picked within
   `sticky_for`.
2. Else the next healthy proxy, round-robin.
3. If every proxy is banned, the proxy whose ban ends first.

```rust,no_run
use std::time::Duration;
use leyline::{BlockRules, ProxyPool, Session};

# fn run() -> leyline::Result<()> {
let pool = ProxyPool::new([
    "http://user:pass@proxy-a.example:8080",
    "http://user:pass@proxy-b.example:8080",
])
.sticky_for(Duration::from_secs(300))
.ban_after(3)
.ban_for(Duration::from_secs(60))
.rotate_on_block(BlockRules::statuses([403, 429]));
let session = Session::builder().proxy_pool(pool.clone()).build()?;
# let _ = session;
# Ok(())
# }
```

| Setter | Default | Effect |
| --- | --- | --- |
| `sticky_for(d)` | zero, no stickiness | Keep an origin on the same proxy for `d` |
| `ban_after(n)` | 3 | Strikes before a ban. The smallest value is 1 |
| `ban_for(d)` | 60 s | Length of a ban |
| `rotate_on_block(rules)` | none | A response the rules report as a block is a strike |

| Outcome of a request | Effect on its proxy |
| --- | --- |
| An error with `is_proxy()` | A strike. The origin loses its sticky proxy |
| A block under `rotate_on_block` | A strike. The origin loses its sticky proxy |
| Any other response | The strikes reset |
| Any other error, `ProxyTarget` included | No change |

After `ban_after` strikes, the proxy is banned for `ban_for`, and every origin
that stuck to it loses its sticky proxy. Each retry picks the next healthy
proxy and skips the one that just failed. A request with its own `.proxy(..)`
does not use the pool. Blocks and `BlockRules` are described in
[Crawling](crawling.md#detect-a-block-page).

`ProxyPool` is `Clone`, and clones share the health state. `stats()` returns
one `ProxyHealth` per proxy, with `proxy` (no password), `in_use`,
`failures`, and `banned_until`, which can be in the past when the ban has
ended.

```rust,no_run
use std::time::Instant;

# fn run(proxies: leyline::ProxyPool) {
let now = Instant::now();
for health in proxies.stats() {
    let banned = health.banned_until.is_some_and(|until| until > now);
    println!(
        "{} in_use={} failures={} banned={banned}",
        health.proxy, health.in_use, health.failures
    );
}
# }
```

### Pin one identity to each proxy

`ProxyPool::identified` takes `(proxy, Identity)` pairs. A request sent
through a proxy uses a session with that proxy's identity, so one proxy IP
never shows two browsers.

```rust,no_run
use leyline::{Browser, Family, Identity, Platform, ProxyPool, Session};

# fn run() -> leyline::Result<()> {
let proxies = (1..=20).map(|n| format!("http://user:pass@proxy-{n}.example:8080"));
let identities = [
    Identity::locked(Browser::latest(Family::Chrome), Platform::Windows),
    Identity::locked(Browser::latest(Family::Firefox), Platform::MacOS),
];
let pool = ProxyPool::identified(proxies.zip(identities.into_iter().cycle()));
let session = Session::builder()
    .browser(Browser::default())
    .proxy_pool(pool)
    .build()?;
# let _ = session;
# Ok(())
# }
```

On first use the pool derives the identity session from the request's
session with `Session::with_identity`, and keeps it. Entries with the same
identity share one derived session. That session shares the connection pool
(in its own partition) and the host limits, and has its own TLS session
cache. Unlike a plain `with_identity` session, it gets its own cookie jar,
which starts empty, so one identity's cookies never go out under another.
When an origin moves to a proxy with another identity, its cookies stay in
the old identity's jar.

The session must impersonate a browser: through `Session::new()`, the first
request sent to an identified proxy fails with `Kind::Config`.

## Proxy errors

A failure in the proxy dial, the proxy handshake, or `CONNECT` is
`Kind::Proxy`, and `err.tls()` returns `TlsError::Proxy`, whose `status`
holds the proxy's answer to `CONNECT`, for example `407`.

When the proxy works but cannot reach the origin, the error is also
`Kind::Proxy`, its category is `ProxyTarget`, and `err.tls()` returns
`TlsError::ProxyTargetUnreachable { reply, detail }`. `reply` is a
`ProxyReply`: `HttpStatus(502)` or `HttpStatus(504)` for a `CONNECT` answer,
or `Socks5(n)` for a SOCKS5 reply from 3 to 6. `detail` holds the text.

`RetryTrigger::ConnectionError` retries an I/O failure on the way to the proxy
and a `CONNECT` answer of 502, 503, or 504, and nothing else here. See
[Custom triggers](retries-and-timeouts.md#custom-triggers).

### Decide whether to ban a proxy

`Error::is_proxy()` is true when the proxy itself failed, before the tunnel
was up:

- The proxy dial was refused or timed out. A timeout is also `is_timeout()`.
- The proxy answered `CONNECT` with an error such as 407, 403, or 503.
- The SOCKS5 handshake or authentication failed.
- The TLS handshake with an `https://` proxy failed.

It is false for `ProxyTargetUnreachable` and for every error after the tunnel
is up, so it takes a proxy out of rotation without blaming it for the origin.
A `ProxyPool` applies this rule for you.

```rust,no_run
use leyline::TlsError;

# async fn run(session: leyline::Session, proxy: &str) -> leyline::Result<()> {
match session.get("https://example.com/").proxy(proxy).send().await {
    Ok(resp) => println!("{}", resp.status()),
    Err(err) if err.is_proxy() => println!("ban {proxy}: {err}"),
    Err(err) => match err.tls() {
        Some(TlsError::ProxyTargetUnreachable { reply, .. }) => {
            println!("origin unreachable through the proxy: {reply:?}")
        }
        _ => println!("origin error: {err}"),
    },
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
proxy without the `socks` feature. `build()` rejects `ProtocolPolicy::Http3`
when such a proxy takes every URL, and an `Http3` request fails with
`Kind::Config` when the rules pick such a proxy for its URL. Under
`ProtocolPolicy::Race`, a request through such a proxy takes the `Auto` path.
MASQUE (`CONNECT-UDP`, RFC 9298) is not supported.

Many proxies do not support `UDP ASSOCIATE`. When an HTTP/3 connection
through a proxy fails, `Race` skips HTTP/3 for that origin and proxy for 5
minutes. To keep every request on TCP, set `ProtocolPolicy::Auto`. See
[HTTP/3](http3.md).

## Next

Read [Cookies](cookies.md).
