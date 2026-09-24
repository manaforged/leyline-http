# Leyline API

**Locked.** This page is the public contract of `leyline-http` (import name
`leyline`). Code, guides, bindings, and reviews follow it. A public item
that is not on this page is private or deleted.

The modules `leyline::h2`, `leyline::pool`, and `leyline::fuzz` exist only
with the `bench-internals` feature, for leyline's own tests, benches, and
fuzz targets. They are not part of the contract.

## Frame (one)

```text
session     Session::new()                  latest bundled Chrome, Windows identity
            Session::builder() → SessionBuilder → build() → Session
            session.with_proxy(url)         clone that shares the pool
request     session.get / post / put / patch / delete / head / request(Method, url) → RequestBuilder
send        RequestBuilder.send() or .await → Response
            session.execute(http::Request<Body>) for prebuilt requests and tower
read        Response: status · headers · header · text · bytes · json · into_stream · copy_to · read_until
fail        Error · err.kind() → Kind
policy      TimeoutConfig · RetryPolicy · RedirectPolicy   (session default, request override)
identity    Browser · Platform · ChromiumBrand · Identity
observe     Trace hooks · Response::timing · Response::tls · Response::audit
tower       LeylineService (feature `tower`)
```

One function per job. A setter that takes a config type accepts
`impl Into<Config>` where a shorthand exists, so the common case stays one
call:

```rust,no_run
use std::time::Duration;

use leyline::{Browser, Platform, ProtocolPolicy, Session, TimeoutConfig};

# async fn run() -> leyline::Result<()> {
let session = Session::new();
let mut resp = session.get("https://example.com/").await?;
let body = resp.text().await?;

let session = Session::builder()
    .browser(Browser::default_firefox())
    .platform(Platform::MacOS)
    .protocol(ProtocolPolicy::Http2)
    .timeout(Duration::from_secs(15))
    .proxy("http://user:pass@host:8080")
    .build()?;

let resp = session
    .post("https://example.com/api")
    .header("x-trace", "1")
    .json(&serde_json::json!({ "q": 1 }))
    .timeout(TimeoutConfig::new().total(Duration::from_secs(5)))
    .await?;
# drop((body, resp));
# Ok(())
# }
```

## Owners

| Job | The one call | Internal owner |
|---|---|---|
| Default session | `Session::new()` | `SessionBuilder` |
| Pick a browser | `SessionBuilder::browser(Browser)` | `profile::ProfileRegistry` |
| Pick a platform | `SessionBuilder::platform(Platform)` | platform twins in the profile data |
| Brand overlay | `SessionBuilder::brand(ChromiumBrand)` | brand table in the profile data |
| Mix TLS and HTTP identities | `SessionBuilder::identity(Identity)` | `Identity` |
| Session default headers | `SessionBuilder::headers(pairs)` | session header merge |
| Build a request | `Session::request(Method, url)` and the verb shortcuts | `RequestBuilder` |
| Prebuilt request | `Session::execute(http::Request<Body>)` | `RequestBuilder::send` |
| Send | `RequestBuilder::send` or `.await` | `RequestBuilder::send` (the one retry loop) |
| Headers | `RequestBuilder::header` (append) / `headers` (append each) | `HeaderList` |
| Header order | `RequestBuilder::header_order`, profile order | `core::headers::reorder` |
| Body | `body` / `json` / `form` / `multipart` | `Body` |
| Query | `RequestBuilder::query` | `url::Url` |
| Auth | `basic_auth` / `bearer_auth` / `digest_auth` | `core::digest` |
| Timeout | `SessionBuilder::timeout(impl Into<TimeoutConfig>)`, `RequestBuilder::timeout(..)` | `RequestBuilder::send` deadline |
| Retry | `SessionBuilder::retry(RetryPolicy)`, `RequestBuilder::retry` | `RequestBuilder::send` loop |
| Redirect | `SessionBuilder::redirect(RedirectPolicy)` | redirect loop in `Session::execute_inner` |
| Cookies | `SessionBuilder::cookie_jar(Jar)`, `Session::cookies()` | `cookie::Jar`, one Set-Cookie parser |
| Proxy | `SessionBuilder::proxy(impl Into<ProxyConfig>)`, `RequestBuilder::proxy`, `Session::with_proxy` | `ProxyConfig::proxy_for` |
| DNS | `SessionBuilder::dns(impl Into<DnsConfig>)` | `DnsConfig` |
| TLS trust | `SessionBuilder::tls_trust(TlsTrustConfig)` | `tls::trust` |
| Protocol | `SessionBuilder::protocol(ProtocolPolicy)` | `transport_policy` |
| Socket and connect tuning | `SessionBuilder::socket(SocketConfig)` | `FingerprintConnector` |
| Pool | `SessionBuilder::pool(PoolConfig)`, `Session::pool_stats`, `Session::preconnect` | `pool` |
| Decompress | automatic; `SessionBuilder::compression(CompressionConfig)` | `session::decompress::Decoder` |
| Read body | `text` / `bytes` / `json` / `into_stream` / `copy_to` / `read_until` | `Response::drain` and `Decoder` |
| Response cookies | `Response::cookies()` (read-only, this response's Set-Cookie) | `cookie::parse` |
| TLS details | `Response::tls() -> Option<&TlsInfo>`, ALPN through `Response::version()` | `TlsInfo` |
| Errors | `Error::kind` | `core::error` |
| Trace | `SessionBuilder::trace(impl Trace)` | `trace` |
| Timing | `Response::timing` | `ResponseTiming` |
| Fingerprint audit | `SessionBuilder::audit(true)`, `Response::audit`, `audit::compute_*` | `audit` |
| WebSocket | `Session::websocket(url)` | `core::websocket` |
| tower | `LeylineService::new(session)` | `Session::execute` |

## Public surface

Counts are public functions. Trait impls (`Default`, `Clone`, `Debug`,
`Display`, `From`, `IntoFuture`, `Stream`, `tower_service::Service`) are not
counted.

### Root

| Type | Functions | Count |
|---|---|---:|
| `Session` | `builder`, `new`, `get`, `post`, `put`, `patch`, `delete`, `head`, `request`, `execute(http::Request<Body>)`, `websocket`, `with_proxy`, `cookies`, `pool_stats`, `preconnect(url, Option<&str>)` | 15 |
| `SessionBuilder` | `browser`, `platform`, `brand`, `identity`, `headers`, `proxy`, `timeout`, `retry`, `redirect`, `cookie_jar`, `dns`, `tls_trust`, `protocol`, `pool`, `socket`, `tcp_profile`, `compression`, `websocket_config`, `https_only`, `trace`, `audit`, `build` | 22 |
| `RequestBuilder` | `header`, `headers`, `header_order`, `anchored`, `query`, `body`, `json`, `form`, `multipart`, `basic_auth`, `bearer_auth`, `digest_auth`, `timeout`, `retry`, `proxy`, `preset`, `stream`, `compress`, `send` | 19 |
| `Response` | `status`, `version`, `url`, `headers`, `header`, `trailers`, `request_headers`, `redirect_chain`, `cookies`, `timing`, `tls`, `audit`, `content_length`, `error_for_status`, `text`, `text_with_charset`, `bytes`, `json`, `into_stream`, `copy_to`, `read_until` | 21 |
| `Body` | `stream(s, Option<u64>)`, `len_hint` | 2 |
| `BodyStream` | `Stream` impl only | 0 |
| `Error` | `kind`, `status`, `url`, `is_timeout`, `is_connect`, `is_status`, `tls`, `h2`, `io` | 9 |
| `Kind`, `HttpVersion` | `as_str` | 2 |
| `ResponseTiming`, `TlsInfo`, `PoolStats` | public fields, `#[non_exhaustive]` | 0 |
| `HeaderList` | `new`, `append`, `set`, `get`, `iter`, `remove_all` | 6 |

### Policy and config

| Type | Functions | Count |
|---|---|---:|
| `TimeoutConfig` | `new`, `total`, `connect`, `read`, `response_header`; `From<Duration>` | 5 |
| `RetryPolicy`, `RetryTrigger` | `none`, `transient`, `with_max_retries`, `with_backoff`, `on_status`, `with_max_retry_after`, `allow_non_idempotent` | 7 |
| `RedirectPolicy`, `RedirectAttempt`, `RedirectAction` | `limited`, `none`, `custom` | 3 |
| `ProxyConfig`, `ProxyRule`, `ProxyUrl`, `NoProxy` | `ProxyConfig::new`, `with_rule`, `no_proxy`, `without_env`; `ProxyUrl::parse`; `NoProxy::new`; `From<&str>`, `From<String>`, `From<ProxyUrl>` | 6 |
| `DnsConfig` | `new`, `resolver`, `resolve_host`; `From<Arc<dyn Resolver>>` | 3 |
| `TlsTrustConfig` | `new`, `add_ca_file`, `add_ca_der`, `add_pinned_leaf_sha256`, `without_env_roots`, `without_system_roots`, `client_identity`, `danger_accept_invalid_certs` | 8 |
| `ProtocolPolicy` | enum: `Auto`, `Http1`, `Http2`, `Http3`, `Race` | 0 |
| `PoolConfig` | `new`, `idle_timeout`, `max_connections`, `max_h1_conns_per_host`, `keepalive`, `h2_ping_after_idle`, `h2_ping_timeout` | 7 |
| `SocketConfig` | `new`, `local_address`, `tcp_nodelay`, `tcp_keepalive`, `tcp_keepalive_interval`, `tcp_keepalive_retries`, `tcp_user_timeout`, `send_buffer_size`, `recv_buffer_size`, `happy_eyeballs` | 10 |
| `CompressionConfig`, `ContentEncoding` | `new`, `none`, `gzip`, `deflate`, `brotli`, `zstd` | 6 |
| `TcpProfile`, `DigestAuth` | `DigestAuth::new` | 1 |

### Identity

| Type | Functions | Count |
|---|---|---:|
| `Browser` | `get(family, version)`, `latest(Family)`, `all`, `family`, `version`, `profile`, `for_platform` | 7 |
| `Family`, `Platform`, `ChromiumBrand`, `Preset` | enums; `Platform::detect_host` | 1 |
| `Identity` | `locked`, `rotate_tls`, `pass` | 3 |
| `BrowserProfile` | `from_toml`, `expected_ja4`, `expected_h2_fingerprint` | 3 |
| `profile::ProfileRegistry` | `global`, `load(dir)`, `get` | 3 |
| `profile::{TlsProfile, H2Profile, H3Profile, PlatformIdentity, HeaderAnchor}` | schema types, public fields | 0 |

### Modules

| Module | Surface | Count |
|---|---|---:|
| `cookie` | `Jar`: `new`, `get_cookie`, `set_cookie`, `all_cookies`, `remove_named`, `clear`, `export_cookies`, `load_cookies`, `cookie_header`; `Cookie`, `SameSite` | 9 |
| `multipart` | `Form`: `new`, `text`, `part`, `file`, `boundary`; `Part`: `text`, `bytes`, `stream`, `filename`, `mime`, `header` | 11 |
| WebSocket (feature `websocket`) | `WebSocketBuilder`: `header`, `headers`, `proxy`, `config`, `connect`; `WsConnection`: `send(WsMessage)`, `recv`, `close`, `split`, `protocol`, `header`; `WsSink`: `send`, `close`; `WsStream`: `recv`; `WsMessage`; `CloseFrame::new`; `WebSocketConfig`: 7 setters | 23 |
| `trace` | `Trace` (hook methods), events `Dns`, `Connect`, `Tls`, `Sent`, `Head`, `Done`, `TracingTrace` | 0 |
| `audit` | `AuditData`, `compute_ja3`, `compute_ja4`, `compute_ja4h`, `compute_ja4t`, input types | 4 |
| `tls` | `Resolver`, `SystemResolver`, `ClientIdentity`, `HappyEyeballsConfig`, `TlsMinVersion`, `TlsError` | 0 |
| tower (feature `tower`) | `LeylineService::new` | 1 |
| `http` | re-export of the `http` crate | 0 |
| `H2Error`, `ErrorCode` | sources reachable from `Error::h2` | 0 |

Total: 217 public functions.

## Semantics

- `Session::new()` and `Session::default()` impersonate the latest bundled
  Chrome with a Windows identity. With the `http3` feature they race
  HTTP/3 against HTTP/2. `Session::builder().build()` with no browser is a
  bare session with no impersonation.
- `SessionBuilder::browser` and `platform` commute: the browser maps to its
  platform twin whichever call comes first.
- Every config setter replaces the whole value. Start from `::new()`, which
  carries the defaults.
- `RequestBuilder::header` appends, with `http::HeaderMap::append`
  semantics. `json`, `form`, `multipart`, and the auth setters replace the
  header they own.
- `Session::execute` reads per-request policy from `http::Extensions`:
  `TimeoutConfig`, `RetryPolicy`, `Preset`. Other policy comes from the
  session.
- `Session::with_proxy` derives a clone that shares the pool. Pool entries
  are keyed by proxy. `with_proxy` with the current proxy URL takes a fresh
  pool, so the next request opens new connections.
- The session jar is the only cookie store. `Response::cookies()` parses
  the Set-Cookie headers of that response; it does not store anything.
- `text`, `bytes`, and `json` buffer on first call. `text` and
  `text_with_charset` decode the declared charset. `into_stream`,
  `copy_to`, and `read_until` consume the response. `into_stream` returns
  `Kind::Body` if the body was already taken.
- One decoder handles `Content-Encoding` for buffered and streamed reads:
  gzip, deflate (zlib or raw), brotli, zstd, at most 4 codings, 100 MiB
  decoded cap.
- `Response::read_until(limit, done)` calls `done(body, from)` after each
  decoded chunk. `body` is every decoded byte so far; `from` is where the
  newest chunk starts. It stops on `true`, at `limit` decoded bytes, or at
  end of stream.
- `Response::audit` is `None` unless the session was built with
  `audit(true)`.
- Public enums and config structs are `#[non_exhaustive]`.
- Builder input errors surface at `build()` or `send()`, never eagerly.

## Errors

One error type: `leyline::Error`. Read `err.kind()` for the `Kind`.
Downcast the source with `err.tls()`, `err.h2()`, or `err.io()`.
`Kind::as_str()` gives a stable lowercase label.

| Kind | Meaning |
|---|---|
| `Builder` | Configuration rejected at `build()` or `send()` |
| `Request` | HTTP-level failure (bad header, malformed request) |
| `Redirect` | Redirect policy stopped the chain |
| `Status` | `error_for_status` rejected the status |
| `Body` | Body read or write failed |
| `Decode` | Decompression or charset decode failed |
| `Timeout` | A timeout expired |
| `Connect` | TCP or TLS connect failed |
| `Tls` | TLS handshake or verification |
| `Http2` | HTTP/2 protocol failure |
| `Http3` | HTTP/3 protocol failure |
| `Proxy` | Proxy connect or tunnel failed |
| `Io` | Socket I/O |
| `Config` | Invalid configuration |
| `Url` | URL parse failure |
| `Json` | JSON serialize or deserialize failure |

## Features

Default: `cookies`, `charset`, `compression-gzip`, `compression-brotli`,
`compression-deflate`, `compression-zstd`, `multipart`, `stream`,
`websocket`, `http3`, `system-trust`. Opt-in: `socks`, `tower`,
`native-interface-bind`, `unstable-bssl`, `bench-internals` (outside
semver). `full` enables every opt-in except `bench-internals` and
`unstable-bssl`. The BoringSSL crates are outside the semver promise.

## Do not add

- A second constructor for a browser, brand, or platform
  (`Session::chrome()`, `SessionBuilder::macos()`). Use `browser`,
  `platform`, `brand`.
- A per-field copy of a config type on `SessionBuilder` or
  `RequestBuilder` (`connect_timeout`, `resolve_host`,
  `add_root_certificate_der`, `max_redirects`). Add the field to the config
  type.
- A second setter with other header semantics (`append_header`,
  `set_header`) or a typed shortcut for one header (`user_agent`,
  `referer`).
- A second request type. `Session::execute` takes `http::Request<Body>`.
- A second middleware model inside the send path. Wrap `LeylineService`
  with tower layers.
- A second body reader for a job on this page (`text_utf8`, `into_text`,
  `as_bytes`, `download_to`), or a second `Content-Encoding` decoder.
- A second cookie store, such as a per-response cookie map.
- A getter that copies session configuration back out.
- A second timing, retry, or redirect owner.
- A browser, version, or brand table in Rust. Profile data owns it.

## Profile schema

`BrowserProfile`, `profile::TlsProfile`, `profile::H2Profile`,
`profile::PlatformIdentity`, and the other `leyline::profile` types mirror
the bundled `profiles/<family>/<version>.toml` files; field names are the
TOML keys. Load custom profiles with `profile::ProfileRegistry::load(dir)`
or `BrowserProfile::from_toml`.
