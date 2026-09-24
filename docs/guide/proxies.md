# Proxies

Leyline tunnels through `http://`, `https://`, and `socks5://` proxies. Any
other scheme is refused, because sending `CONNECT` to it would transmit the
request, and any `Proxy-Authorization` credentials, in cleartext.

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

The session URL is validated at `build()`. A request, WebSocket, or
`with_proxy` URL is validated when a request picks it, and an invalid URL
fails that `send()` with `Kind::Config`. One exception: without the `socks`
feature, a `socks5://` URL passes validation, and the first request fails. `ProxyUrl` does the same validation on its
own, if you want to check a URL before you store it.

```rust
use leyline::ProxyUrl;

assert!(ProxyUrl::parse("socks5://proxy.example:1080").is_ok());
assert!(ProxyUrl::parse("ftp://proxy.example").is_err());
```

`Debug` output for `ProxyUrl`, `ProxyRule`, and `Session` replaces the
password with `***`.

## Rules per scheme

`ProxyConfig` holds a list of `ProxyRule`s. A rule applies to all schemes,
to `http` only, or to `https` only. The first matching rule wins, in the order
you added them.

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

`ProxyRule::all(url)` applies to every scheme.

## Bypass with NO_PROXY

`NoProxy` matches hosts that must not go through a proxy. Build one
from a pattern iterator with `NoProxy::new`.

```rust
use leyline::{NoProxy, ProxyConfig, ProxyRule};

let bypass = NoProxy::new(["localhost", ".internal.example", "192.0.2.1"]);
let proxies = ProxyConfig::new()
    .rule(ProxyRule::all("http://proxy.example:8080"))
    .no_proxy(bypass);
# let _ = proxies;
```

A pattern matches the host itself or any subdomain of it. A leading dot is
optional. A trailing `:port` is stripped. `*` matches every host. Matching is
case-insensitive, and a trailing dot on the host is ignored.

The bypass list applies in two cases: when you set it yourself with
`ProxyConfig::no_proxy`, and when the session's
proxy was discovered from the environment. A per-request `proxy()` override
replaces the session config, so it uses its own `NoProxy`, not the session
one.

## Environment discovery

A session with no explicit proxy reads the environment at build time. Leyline
takes the first non-empty value in this order: `HTTPS_PROXY`, `https_proxy`,
`HTTP_PROXY`, `http_proxy`, `ALL_PROXY`, `all_proxy`. `NO_PROXY` or
`no_proxy` seeds the bypass matcher.

Uppercase `HTTP_PROXY` is skipped when the process looks like a CGI handler,
which is the httpoxy mitigation. Leyline detects that by the presence of any
of `GATEWAY_INTERFACE`, `REQUEST_METHOD`, `SERVER_SOFTWARE`, `SCRIPT_NAME`,
`SCRIPT_FILENAME`, `PATH_INFO`, `QUERY_STRING`, `SERVER_PROTOCOL`,
`SERVER_NAME`, or `SERVER_PORT`, and logs a warning on the
`leyline::env_proxy::cgi` target.

Turn discovery off with `ProxyConfig::env(false)`.

```rust,no_run
# fn run() -> leyline::Result<()> {
let session = leyline::Session::builder()
    .proxy(leyline::ProxyConfig::new().env(false))
    .build()?;
# let _ = session;
# Ok(())
# }
```

## SOCKS5 and HTTPS proxies

An `https://` proxy means TLS to the proxy first, then `CONNECT` through it.
It works with the default feature set.

A `socks5://` or `socks5h://` proxy needs the `socks` feature, which is off by
default. Without it, `build()` accepts a SOCKS URL, and the first request fails with an
error whose message says that SOCKS proxy support requires the
`socks` feature. Both schemes behave the same
way: the proxy resolves the hostname.

```toml
[dependencies]
leyline-http = { version = "0.1", features = ["socks"] }
```

## Per-request override

`RequestBuilder::proxy` replaces the session proxy config for one request. A
`ProxyConfig::new()` with no rule sends that request direct.
`Session::with_proxy` derives a whole session that differs only in its proxy
config, and keeps the cookie jar, the TLS context, the identity, and the pool.
Pool entries are keyed by proxy URL, so the derived session never reuses a
connection opened through another proxy. A rotation back to a proxy with warm
connections reuses them, and the rotation closes no connection that another
session uses. `with_proxy` with the current proxy URL also reuses the warm
connections.

To open new connections through the same proxy, call
`Session::fresh_pool`. It derives a session with a new, empty pool and TLS
session cache, so the next request opens a new socket.

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let resp = session
    .get("https://example.com/ip")
    .proxy("http://other-proxy.example:8080")
    .await?;
let rotated = session.with_proxy("http://gateway.example:8080").fresh_pool();
# let _ = (resp, rotated);
# Ok(())
# }
```

The pool is keyed by host, port, and proxy, so two proxies never share a
connection.

## HTTP/3 through a SOCKS5 proxy

HTTP/3 runs over a `socks5://` or `socks5h://` proxy with the `socks`
feature. Leyline opens a TCP control connection to the proxy, negotiates
authentication, and sends `UDP ASSOCIATE` (RFC 1928, section 7). Each QUIC
datagram goes to the relay address with the SOCKS5 UDP header, and the reply
header is removed before QUIC reads the datagram. The target host goes in the
UDP header as a domain name, so the proxy resolves it. When the control
connection closes, the QUIC connection fails and leaves the pool.

HTTP/3 connections are pooled by host, port, and proxy, as HTTP/2 connections
are.

An `http://` or `https://` proxy cannot carry HTTP/3. `build()` rejects
`ProtocolPolicy::Http3` when such a proxy takes every URL. An `Http3` request
fails with `Kind::Config` when `ProxyConfig::proxy_for` picks such a proxy.
Under `ProtocolPolicy::Race`, a request through such a proxy is not raced: it
goes down the `Auto` path. MASQUE (`CONNECT-UDP`, RFC 9298) is not supported.
See [HTTP/3](http3.md).

## Next

Read [Cookies](cookies.md).
