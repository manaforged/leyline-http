# Proxies

Leyline tunnels through `http://`, `https://`, and `socks5://` proxies. Any
other scheme is refused, because sending `CONNECT` to it would transmit the
request, and any `Proxy-Authorization` credentials, in cleartext.

## Set one proxy

`SessionBuilder::proxy` takes a URL string and applies it to every scheme.

```rust,no_run
# fn run() -> leyline::Result<()> {
let session = leyline::Session::builder()
    .proxy("http://user:pass@proxy.example:8080")
    .build()?;
# let _ = session;
# Ok(())
# }
```

The URL is validated at `build()`. `ProxyUrl` does the same validation on its
own, if you want to check a URL before you store it.

```rust
use leyline::ProxyUrl;

assert!(ProxyUrl::parse("socks5://proxy.example:1080").is_ok());
assert!(ProxyUrl::parse("ftp://proxy.example").is_err());
```

`ProxyUrl::http`, `::https`, `::socks5`, and `::socks5h` each also require that
scheme. `Debug` output for `ProxyUrl`, `ProxyRule`, and `Session` replaces the
password with `***`.

## Rules per scheme

`ProxyConfig` holds a list of `ProxyRule`s. A rule applies to all schemes,
to `http` only, or to `https` only. The first matching rule wins, in the order
you added them.

```rust,no_run
use leyline::{ProxyConfig, ProxyRule, Session};

# fn run() -> leyline::Result<()> {
let proxies = ProxyConfig::new()
    .with_rule(ProxyRule::https("http://secure-proxy.example:8080"))
    .with_rule(ProxyRule::http("http://plain-proxy.example:3128"));
let session = Session::builder().proxies(proxies).build()?;
# let _ = session;
# Ok(())
# }
```

`ProxyConfig::all(url)` is shorthand for adding a `ProxyRule::all`.

## Bypass with NO_PROXY

`NoProxy` matches hosts that must not go through a proxy. Build one from a
comma-separated list, from a pattern iterator, or from the environment.

```rust
use leyline::NoProxy;

let bypass = NoProxy::new(["localhost", ".internal.example", "192.0.2.1"]);
assert!(bypass.matches("api.internal.example"));
assert!(!bypass.matches("api.example.com"));
```

A pattern matches the host itself or any subdomain of it. A leading dot is
optional. A trailing `:port` is stripped. `*` matches every host. Matching is
case-insensitive, and a trailing dot on the host is ignored.

The bypass list applies in two cases: when you set it yourself with
`ProxyConfig::no_proxy` or `SessionBuilder::no_proxy`, and when the session's
proxy was discovered from the environment. An explicit `NoProxy` also
suppresses a per-request `proxy()` override for a matching host.

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

Turn discovery off with `SessionBuilder::disable_env_proxies`, or with
`ProxyConfig::without_env`. `ProxyConfig::uses_env` reports the current
setting.

```rust,no_run
# fn run() -> leyline::Result<()> {
let session = leyline::Session::builder().disable_env_proxies().build()?;
# let _ = session;
# Ok(())
# }
```

## SOCKS5 and HTTPS proxies

An `https://` proxy means TLS to the proxy first, then `CONNECT` through it.
It works with the default feature set.

A `socks5://` or `socks5h://` proxy needs the `socks` feature, which is off by
default. Without it, a SOCKS URL fails with a TLS profile error saying that
SOCKS proxy support requires the `socks` feature. Both schemes behave the same
way: the proxy resolves the hostname.

```toml
[dependencies]
leyline-http = { version = "0.1", features = ["socks"] }
```

## Per-request override

`RequestBuilder::proxy` replaces the session proxy for one request.
`Session::with_proxy` derives a whole session that differs only in its proxy,
and keeps the cookies, the TLS context, and the pool.

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::chrome();
let resp = session
    .get("https://example.com/ip")
    .proxy("http://other-proxy.example:8080")
    .await?;
# let _ = resp;
# Ok(())
# }
```

The pool is keyed by host, port, and proxy, so two proxies never share a
connection.

## HTTP/3 is not proxied

QUIC has no proxy path here. A request with `ProtocolPolicy::Http3` and any
proxy set fails with `Error::Config`, telling you to use `Auto` or `Http2`.
Under `ProtocolPolicy::Race`, a proxied request is not raced: it goes down the
`Auto` path instead. See [HTTP/3](http3.md).

## Next

Read [Cookies](cookies.md).
