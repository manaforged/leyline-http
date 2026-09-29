# Proxies

Leyline tunnels through `http://`, `https://`, `socks5://`, and `socks5h://`
proxies. `ProxyUrl::parse` refuses any other scheme, and a URL that has no host.

An `http://` proxy receives the `CONNECT` request and any `Proxy-Authorization`
credentials in cleartext. An `https://` proxy receives them inside TLS.

A plain `http://` URL goes to an `http://` or `https://` proxy as an
absolute-form request. Through a `socks5://` or `socks5h://` proxy, it goes
over a SOCKS5 `CONNECT` tunnel to the origin.

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
feature, a `socks5://` or `socks5h://` URL passes validation, and the first
request fails. `ProxyUrl` does the same validation on its own, if you want to
check a URL before you store it.

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

A `wss://` WebSocket URL matches the same rules as an `https://` URL, so
`ProxyRule::https` covers it. Leyline opens WebSockets over `wss://` only: a
`ws://` URL fails with `Kind::Request` before any proxy is chosen. See
[WebSocket](websocket.md).

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
`ProxyConfig::no_proxy`, and when the session's proxy was discovered from the
environment. A per-request `proxy()` override replaces the session config, so
it uses its own `NoProxy`, not the session one.

## Environment discovery

A session with no explicit proxy reads the environment once, when it is built.
Leyline takes the first non-empty value in this order: `HTTPS_PROXY`,
`https_proxy`, `HTTP_PROXY`, `http_proxy`, `ALL_PROXY`, `all_proxy`. The value
applies to every URL scheme, whichever variable supplied it, so a value from
`HTTPS_PROXY` also serves `http://` URLs. `NO_PROXY` or `no_proxy` seeds the
bypass matcher.

A value that is not a valid proxy URL never falls back to a direct connection.
A valid value has one of the schemes `http`, `https`, `socks5`, or `socks5h`,
and a host, so `proxy.example:8080` is invalid. Leyline checks only the value
it picked: it does not try a lower variable. The error message names the
variable and never repeats the value, because the value can hold credentials.

- `SessionBuilder::build()` returns a `Kind::Config` error.
- `Session::new()` returns a session in which every request fails with
  `Kind::Proxy`. That includes requests to hosts that `NO_PROXY` lists.

Fix or unset the variable, give the builder a proxy with
`SessionBuilder::proxy`, or turn discovery off with `ProxyConfig::env(false)`.
A proxy set on a request, on a WebSocket, or through `Session::with_proxy`
replaces the session config, so it also replaces the failed state.

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
It works with the default feature set, with one limit: a session with
certificate pins or a client certificate cannot use it, because the origin TLS
identity must not go to the proxy. `build()` accepts that combination, and
every request that the proxy rules send through the `https://` proxy fails with
`Kind::Proxy`. Use an `http://` or `socks5://` proxy, or drop the pins and the
client certificate. See [TLS trust](tls-trust.md).

A `socks5://` or `socks5h://` proxy needs the `socks` feature, which is off by
default. Without it, `build()` accepts a SOCKS URL, and the first request fails
with an error whose message says that SOCKS proxy support requires the `socks`
feature. The exception is `ProtocolPolicy::Http3` with a SOCKS proxy that takes
every URL: without the feature no proxy can carry HTTP/3, so `build()` fails
with `Kind::Config`. Both schemes behave the same way: the proxy resolves the
hostname.

```toml
[dependencies]
leyline-http = { version = "0.1", features = ["socks"] }
```

## Proxy errors

A failure in the proxy dial, the proxy handshake, or `CONNECT` is `Kind::Proxy`.
`err.tls()` returns `TlsError::Proxy`, whose `status` holds the status that the
proxy answered to `CONNECT`, for example `407`.

`RetryTrigger::ConnectionError` retries an I/O failure on the way to the proxy
and a `CONNECT` answer of 502, 503, or 504. Leyline does not retry any other
answer, such as 407. See [Errors](errors.md) and
[Retries and timeouts](retries-and-timeouts.md).

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

The connect timeout covers the control connection, the `UDP ASSOCIATE`
exchange, and the QUIC handshake. When it elapses, the request fails with
`Kind::Connect` and `Error::is_timeout()` is true, as for a TCP connect. See
[Retries and timeouts](retries-and-timeouts.md).

HTTP/3 connections are pooled by host, port, and proxy, as HTTP/2 connections
are.

An `http://` or `https://` proxy cannot carry HTTP/3. `build()` rejects
`ProtocolPolicy::Http3` when such a proxy takes every URL. Without the `socks`
feature no proxy can carry HTTP/3, so `build()` also rejects it for a SOCKS
proxy that takes every URL. An `Http3` request fails with `Kind::Config` when
the proxy rules pick such a proxy for its URL. Under `ProtocolPolicy::Race`, a
request through such a proxy is not raced: it goes down the `Auto` path.
MASQUE (`CONNECT-UDP`, RFC 9298) is not supported. See [HTTP/3](http3.md).

## Next

Read [Cookies](cookies.md).
