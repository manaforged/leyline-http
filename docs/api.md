# Leyline API

**Locked.** This page is the public contract of `leyline-http` (import name
`leyline`). Code, guides, bindings, and reviews follow it. A public item
that is not on this page is private or deleted.

The modules `leyline::h2`, `leyline::pool`, and `leyline::fuzz` exist only
with the `bench-internals` feature, for leyline's own tests, benches, and
fuzz targets. They are not part of the contract. The same feature also makes
`tls::FingerprintConnector`, `H3Config`, `TlsContext`, and these internal
methods public for those tests: `ProfileRegistry::builtin` and `get_browser`;
`BrowserProfile::load_warnings`, `identity_for`, `resolve_for_platform`, and
`bare`; `Browser::chromium_major`; `Platform::identity_key`; and
`Pool::bench_populate_h2` and `bench_probe`. None of them is part of the
contract.

## Frame (one)

```text
session     Session::new()                  newest browser-captured Chrome, Windows
            Session::builder() → SessionBuilder → build() → Session
            session.with_proxy(config)      clone that shares the pool, other proxy
            session.fresh_pool()            clone with a new pool and TLS session cache
            session.with_cookie_jar(jar)    clone that shares the pool, other jar
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

use leyline::{Browser, Family, Platform, ProtocolPolicy, Session, TimeoutConfig};

# async fn run() -> leyline::Result<()> {
let session = Session::new();
let body = session.get("https://example.com/").await?.text().await?;

let session = Session::builder()
    .browser(Browser::latest(Family::Firefox))
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
| Send a loaded profile | `SessionBuilder::profile(BrowserProfile)` | `profile::resolve_identity` |
| Pick a platform | `SessionBuilder::platform(Platform)` | platform twins in the profile data |
| Brand overlay | `SessionBuilder::brand(ChromiumBrand)` | brand table in the profile data |
| Mix TLS and HTTP identities | `SessionBuilder::identity(Identity)` | `Identity` |
| Session default headers | `SessionBuilder::headers(pairs)` | session header merge |
| URL input | `impl IntoUrl`: `&str`, `String`, `&String`, `url::Url`, `&url::Url` | `IntoUrl::into_url` |
| Build a request | `Session::request(Method, url)` and the verb shortcuts | `RequestBuilder` |
| Prebuilt request | `Session::execute(http::Request<Body>)` | `RequestBuilder::send` |
| Send | `RequestBuilder::send` or `.await` | `RequestBuilder::send` (the one retry loop) |
| Headers | `RequestBuilder::header` (append) / `headers` (append each) | `HeaderList` |
| Header order | `RequestBuilder::header_order`, profile order | `core::headers::reorder` |
| Body | `body` / `json` / `form` / `multipart` | `Body` |
| Query | `RequestBuilder::query` | `url::Url` |
| Auth | `basic_auth` / `bearer_auth` / `digest_auth` | `core::digest` |
| Timeout | `SessionBuilder::timeout(impl Into<TimeoutConfig>)`, `RequestBuilder::timeout(..)` | `core::deadline::Deadline` |
| Retry | `SessionBuilder::retry(RetryPolicy)`, `RequestBuilder::retry`, `RetryPolicy::retry_on(impl IntoIterator<Item = RetryTrigger>)` | `RequestBuilder::send` loop |
| Redirect | `SessionBuilder::redirect(RedirectPolicy)`, `RequestBuilder::redirect(RedirectPolicy)`, `Session::with_redirect(RedirectPolicy)` | redirect loop in `Session::execute_inner` |
| Cookies | `SessionBuilder::cookie_jar(Jar)`, `Session::cookies()`, `Session::with_cookie_jar(Jar)` | `cookie::Jar`, one Set-Cookie parser |
| Seed a cookie with attributes | `Jar::store_set_cookie(&str, &Url)` | `cookie::parse`, the same parser responses use |
| Copy and merge a jar | `Jar::snapshot() -> Jar`, `Jar::extend_from(&Jar)` | `cookie::Jar` |
| Remove cookies by name | `Jar::remove(&Url, &str) -> usize` (one host, every path), `Jar::remove_named(&str) -> usize` (every host) | `cookie::Jar` |
| Identity values | `Browser::identity(Platform, Option<ChromiumBrand>) -> Option<PlatformIdentity>`; `Session::identity() -> SessionIdentity` (what a built session sends) | `profile::resolve_identity`, the one resolver the session builder also calls |
| TCP fingerprint | `Platform::tcp_profile() -> TcpProfile`, `SessionBuilder::tcp_profile` | `profiles/platforms.toml` |
| Proxy | `impl Into<ProxyConfig>` on `SessionBuilder::proxy`, `RequestBuilder::proxy`, `WebSocketBuilder::proxy`, `Session::with_proxy` | `ProxyConfig::proxy_for` |
| DNS | `SessionBuilder::dns(impl Into<DnsConfig>)`, `DnsConfig::resolve_host(host, impl IntoIterator<Item = SocketAddr>)` | `DnsConfig` |
| TLS trust | `SessionBuilder::tls_trust(TlsTrustConfig)` | `tls::trust` |
| Protocol | `SessionBuilder::protocol(ProtocolPolicy)` | `transport_policy` |
| Socket and connect tuning | `SessionBuilder::socket(SocketConfig)` | `FingerprintConnector` |
| Pool | `SessionBuilder::pool(PoolConfig)`, `Session::pool_stats`, `Session::preconnect`, `Session::fresh_pool` | `pool` |
| Decompress | automatic; `SessionBuilder::compression(CompressionConfig)` | `session::decompress::Decoder` |
| Response body cap | `CompressionConfig::max_body_size` | the one limit that HTTP/1.1, HTTP/2, HTTP/3, and `Decoder` read |
| Response headers | `Response::headers() -> &http::HeaderMap`, `Response::header(name)` | `Response` |
| Read body | `text` / `bytes` / `json` / `into_stream` / `copy_to` / `read_until` | `Response::bytes` and `Decoder` |
| Status check | `Response::error_for_status` (consume) / `error_for_status_ref` (borrow) | `Response::status_error` |
| Response cookies | `Response::cookies()` (read-only, this response's Set-Cookie) | `cookie::parse` |
| TLS details | `Response::tls() -> Option<&TlsInfo>`, ALPN through `Response::version()` | `TlsInfo` |
| Errors | `Error::kind` | `core::error` |
| Trace | `SessionBuilder::trace(impl Trace)`; `Sent` carries `method` and `path`; `Head` carries `headers: &http::HeaderMap` | `trace` |
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
| `Session` | `builder`, `new`, `get`, `post`, `put`, `patch`, `delete`, `head`, `request`, `execute(http::Request<Body>)`, `websocket`, `with_proxy(impl Into<ProxyConfig>)`, `fresh_pool`, `with_cookie_jar(Jar)`, `with_redirect(RedirectPolicy)`, `identity() -> SessionIdentity`, `cookies`, `pool_stats`, `preconnect(url)` | 19 |
| `SessionBuilder` | `browser`, `profile(BrowserProfile)`, `platform`, `brand`, `identity`, `headers`, `proxy`, `timeout`, `retry`, `redirect`, `cookie_jar`, `dns`, `tls_trust`, `protocol`, `pool`, `socket`, `tcp_profile`, `compression`, `websocket_config`, `https_only`, `trace`, `audit`, `build` | 23 |
| `RequestBuilder` | `header`, `headers`, `header_order`, `anchored`, `query`, `body`, `json`, `form`, `multipart`, `basic_auth`, `bearer_auth`, `digest_auth`, `timeout`, `retry`, `redirect(RedirectPolicy)`, `proxy`, `preset`, `stream`, `compress`, `send` | 20 |
| `Response` | `status`, `version`, `url`, `headers`, `header`, `trailers`, `request_headers`, `redirect_chain`, `cookies`, `timing`, `tls`, `audit`, `content_length`, `error_for_status`, `error_for_status_ref`, `text`, `text_with_charset`, `bytes`, `json`, `into_stream`, `copy_to`, `read_until` | 22 |
| `Body` | `stream(s, Option<u64>)`, `len_hint` | 2 |
| `BodyStream` | `Stream` impl only | 0 |
| `Error` | `kind`, `status`, `url`, `is_timeout`, `is_connect`, `is_status`, `is_retryable`, `tls`, `h2`, `io` | 10 |
| `Kind`, `HttpVersion` | `as_str` | 2 |
| `ResponseTiming`, `TlsInfo`, `PoolStats` | public fields, `#[non_exhaustive]` | 0 |
| `HeaderList` | `new`, `append`, `set`, `get`, `iter`, `remove_all` | 6 |

### Policy and config

| Type | Functions | Count |
|---|---|---:|
| `TimeoutConfig` | `new`, `total`, `connect`, `read`, `response_header`; `From<Duration>` | 5 |
| `RetryPolicy`, `RetryTrigger` | `none`, `transient`, `max_retries`, `initial_backoff`, `max_backoff`, `backoff_factor`, `jitter`, `max_retry_after`, `on_status`, `retry_on(impl IntoIterator<Item = RetryTrigger>)`, `allow_non_idempotent` | 11 |
| `RedirectPolicy`, `RedirectAttempt`, `RedirectAction` | `limited`, `none`, `custom` | 3 |
| `ProxyConfig`, `ProxyRule`, `ProxyUrl`, `NoProxy` | `ProxyConfig::new`, `rule`, `no_proxy`, `env(bool)`; `ProxyRule::all`, `http`, `https`; `ProxyUrl::parse`; `NoProxy::new`; `From<&str>`, `From<&String>`, `From<String>`, `From<ProxyUrl>` | 9 |
| `DnsConfig` | `new`, `resolver`, `resolve_host(host, impl IntoIterator<Item = SocketAddr>)`; `From<Arc<dyn Resolver>>` | 3 |
| `TlsTrustConfig` | `new`, `add_ca_file`, `add_ca_der`, `add_pinned_leaf_sha256`, `env_roots(bool)`, `system_roots(bool)`, `client_identity`, `danger_accept_invalid_certs(bool)` | 8 |
| `ProtocolPolicy` | enum: `Auto`, `Http1`, `Http2`, `Http3`, `Race` | 0 |
| `PoolConfig` | `new`, `idle_timeout`, `max_connections`, `max_h1_conns_per_host`, `keepalive`, `h2_ping_after_idle`, `h2_ping_timeout` | 7 |
| `SocketConfig` | `new`, `local_address`, `local_ipv4`, `local_ipv6`, `tcp_nodelay`, `tcp_keepalive`, `tcp_keepalive_interval`, `tcp_keepalive_retries`, `tcp_user_timeout`, `send_buffer_size`, `recv_buffer_size`, `interface`, `strict`, `happy_eyeballs` | 14 |
| `CompressionConfig`, `ContentEncoding` | `new`, `none`, `gzip`, `deflate`, `brotli`, `zstd`, `max_body_size` | 7 |
| `TcpProfile` | public fields `ttl`, `mss`, `window_size`, `df`, `window_scale`, `no_delay`, `options`, `#[non_exhaustive]` | 0 |
| `DigestAuth` | `new` | 1 |

### Identity

| Type | Functions | Count |
|---|---|---:|
| `Browser` | `get(family, version)`, `latest(Family)`, `all`, `family`, `version`, `profile`, `for_platform`, `identity(Platform, Option<ChromiumBrand>) -> Option<PlatformIdentity>` | 8 |
| `Family`, `Platform`, `ChromiumBrand`, `Preset` | enums; `Platform::detect_host`, `Platform::tcp_profile` | 2 |
| `Identity` | `locked`, `rotate_tls`, `switch_family`, `http`, `tls`, `platform` | 6 |
| `SessionIdentity` | `identity`, `browser`, `platform`, `brand`, `user_agent` | 5 |
| `BrowserProfile` | `from_toml`, `from_fingerprint(FingerprintSpec)`, `expected_ja4`, `expected_h2_fingerprint` | 4 |
| `profile::FingerprintSpec` | `new`, `ja3`, `ja4_r`, `akamai`, `user_agent`, `header_order`, `base(BrowserProfile)`, `name` | 8 |
| `profile::ProfileRegistry` | `global`, `load(dir)`, `get` | 3 |
| `profile::{ProfileMeta, TlsProfile, TlsFingerprint, H2Profile, H2PriorityProfile, H2PlatformOverride, H2Fingerprint, H3Profile, PlatformIdentity, HeaderAnchor, HeaderStyle}` | schema types, public fields | 0 |
| `profile::ProfileError` | error of `load` and `from_toml` | 0 |

### Modules

| Module | Surface | Count |
|---|---|---:|
| `cookie` | `Jar`: `new`, `get_cookie(&Url, &str) -> Option<String>`, `set_cookie(&Url, &str, &str)`, `store_set_cookie(&str, &Url)`, `all_cookies`, `snapshot`, `extend_from(&Jar)`, `remove(&Url, &str) -> usize`, `remove_named(&str) -> usize`, `clear`, `export_cookies(&Url) -> String`, `load_cookies(&str, &Url)`, `cookie_header(&Url)`; `Cookie::is_expired`; `SameSite` | 14 |
| `multipart` | `Form`: `new`, `text`, `part`, `file`, `boundary`; `Part`: `text`, `bytes`, `stream`, `filename`, `mime`, `header` | 11 |
| WebSocket (feature `websocket`) | `WebSocketBuilder`: `header`, `headers`, `proxy`, `config`, `connect`; `WsConnection`: `send(WsMessage)`, `recv`, `close`, `split`, `protocol`, `header`; `WsSink`: `send`, `close`; `WsStream`: `recv`; `WsMessage`; `CloseFrame::new`; `WebSocketConfig`: 7 setters | 23 |
| `trace` | `Trace` (hook methods), events `Dns`, `Connect`, `Tls`, `Sent` (with `method` and `path`), `Head` (with `headers`), `Done`, `TracingTrace` | 0 |
| `audit` | `AuditData`; `compute_ja3(&Ja3Input)`, `compute_ja4(&Ja4Input)`, `compute_ja4h(&Ja4hInput)`, `compute_ja4t(&TcpProfile)`; input types `Ja3Input`, `Ja4Input`, `Ja4hInput` (public fields) | 4 |
| `tls` | `Resolver`, `ResolveFuture`, `SystemResolver`, `ClientIdentity`, `TlsMinVersion`, `TlsError`; `HappyEyeballsConfig`: `new`, `resolve_delay`, `attempt_limit` | 3 |
| `TlsContext` (feature `unstable-bssl`) | `from_profile`, `builder_mut`, `into_inner`; outside semver | 0 |
| `IntoParamPair`, `IntoUrl`, `Result` | trait bound of `headers` and `query`; sealed URL input bound; `Result<T, Error>` alias | 0 |
| tower (feature `tower`) | `LeylineService::new` | 1 |
| `http` | re-export of the `http` crate | 0 |
| `H2Error`, `ErrorCode` | sources reachable from `Error::h2` | 0 |

Total: 264 public functions.

## Semantics

- `Session::new()` and `Session::default()` impersonate the newest bundled
  Chrome captured from a real browser, with a Windows identity. With the `http3` feature they race
  HTTP/3 against HTTP/2 when the profile's `[h3]` table sets `race = true`,
  as the bundled Chrome profiles do. `Session::builder().build()` with no
  browser is a bare session with no impersonation.
- `Browser::latest(family)` returns the newest profile of the family whose
  `[meta] capture` is `"browser"`. A family with no browser capture returns
  its newest profile. `Session::new()` uses `Browser::latest(Family::Chrome)`.
  This choice moves: a patch release may add a newer browser capture. For a fixed fingerprint, pin the browser with
  `Session::builder().browser(Browser::Chrome148)` or
  `Browser::get(Family::Chrome, 148)`.
- `Session::new()` does not fail. The bundled profile data is fixed at
  compile time, and trust-store problems at startup log a warning; a
  request that then cannot verify a certificate fails with `Kind::Tls`.
- `SessionBuilder::browser` and `platform` commute: the browser maps to its
  platform twin whichever call comes first.
- `SessionBuilder::profile` takes a profile from `ProfileRegistry::load` or
  `BrowserProfile::from_toml`. The session sends that profile's TLS, HTTP/2,
  HTTP/3, and `[identity.<platform>]` tables. The last call of `browser`,
  `identity`, or `profile` wins. A loaded profile has no platform twin, so
  `build` returns `Kind::Config` when the profile has no identity table for
  the platform. Without `.platform()`, the platform is Windows.
  `Session::identity().browser()` is `None` for a loaded profile.
- `BrowserProfile::from_fingerprint` builds a profile from a raw JA3 string,
  a raw JA4_r string, and an Akamai HTTP/2 string, on top of the
  `FingerprintSpec::base` profile or the bare profile. It returns the same
  `BrowserProfile` that `from_toml` returns, and `SessionBuilder::profile`
  sends it. The IANA registry that `audit` uses maps the IDs to names, so
  `Response::audit` reports the JA3 and Akamai values of the input strings.
  A hashed JA4, an unknown ID, an extension or SETTINGS ID that leyline
  cannot send, and PRIORITY frames fail with `Kind::Config`. The guide
  section "Build a profile from a JA3 or Akamai string" lists the rules.
- A brand overlay needs `chromium_major` in the profile `[meta]` table.
- Config types (`TimeoutConfig`, `RetryPolicy`, `RedirectPolicy`,
  `ProxyConfig`, `DnsConfig`, `TlsTrustConfig`, `PoolConfig`, `SocketConfig`,
  `HappyEyeballsConfig`, `CompressionConfig`, `WebSocketConfig`) have private
  fields and consuming setters named after the field, with no `with_`
  prefix. A setter for an optional value takes `impl Into<Option<T>>`;
  `None` turns that setting off. A setter that adds to a list starts with
  `add_` or appends one item (`rule`, `on_status`).
- Every `SessionBuilder` config setter replaces the whole value. Start from
  `::new()`, which carries the defaults.
- `TimeoutConfig`: `total` bounds the exchange from send to the last byte of
  a buffered body. It defaults to 300 s; `total(None)` turns it off.
  `read` is an idle limit for each body read, buffered or streamed; a
  streamed body read after `send` returns has only this limit.
  `response_header` bounds the wait for the response head. `connect` bounds
  each TCP, proxy, and TLS connect; it is a session setting, and a request
  value for it has no effect.
- `RequestBuilder::timeout` merges over the session timeouts field by field.
  A field that the request sets wins. Every other field keeps the session
  value. `Deadline` does the merge.
- `RequestBuilder::header` appends, with `http::HeaderMap::append`
  semantics. `json`, `form`, `multipart`, and the auth setters replace the
  header they own.
- Header merge runs in three layers. The profile preset gives the base list
  and its order. `SessionBuilder::headers` replaces a profile header of the
  same name in its slot, or appends a new one; a repeated session name keeps
  the last value. A request header replaces every profile or session header
  of the same name: all request values for that name, in call order, take
  the slot of the replaced header. Repeated `header` calls on one request
  append, so they send every value. A request header with no match goes to
  its anchor slot or to the end.
- `Session::get`, `post`, `put`, `patch`, `delete`, `head`, `request`,
  `websocket`, and `preconnect` take `impl IntoUrl`. `IntoUrl` is sealed.
  A URL that does not parse gives `Kind::Url` with the `url::ParseError` as
  source, at `send()` or `connect()`.
- `Session::execute` reads per-request policy from `http::Extensions`:
  `TimeoutConfig`, `RetryPolicy`, `RedirectPolicy`, `Preset`. Other policy
  comes from the session.
- `RequestBuilder::redirect` overrides the session redirect policy for one
  request. The one redirect loop reads it.
- `Session::with_proxy` derives a clone with the given proxy config. The
  clone shares the pool. Pool entries are keyed by proxy URL, so a clone with
  the same proxy reuses its connections.
- `Session::fresh_pool` derives a clone with a new, empty pool and TLS
  session cache, so the next request opens new connections.
- A proxy set on `RequestBuilder` or `WebSocketBuilder` replaces the session
  proxy config for that request. A `ProxyConfig::new()` with no rule sends
  that request direct.
- `ProxyConfig::proxy_for` picks the proxy for each URL, rules first, then
  `no_proxy`. `send()` checks the proxy URL it picks and fails with
  `Kind::Config` if the URL is invalid. `build()` checks the session rules.
- HTTP/3 does not run over a proxy. `build()` rejects `ProtocolPolicy::Http3`
  when the proxy config sends every URL through a proxy. `send()` rejects an
  HTTP/3 request when `proxy_for` picks a proxy, and a `Race` session sends
  that request over HTTP/2.
- Connection setup has no retry of its own. A failed connect returns the
  error to `RequestBuilder::send`, and `RetryTrigger::ConnectionError`
  decides whether to retry it.
- `Session::with_cookie_jar` derives a clone through the same path as
  `with_proxy`. It shares the pool and every other setting and uses the
  given jar.
- `Session::with_redirect` derives a clone through the same path and uses
  the given redirect policy. `RequestBuilder::redirect` still overrides it
  for one request.
- `Session::identity` returns what the session sends, read from the values
  `build()` resolved: the `Identity` (`None` for a bare session), the HTTP
  browser, the platform, the brand (`None` when no Chromium brand applies),
  and the `User-Agent`.
- The `trace::Head` event carries the response headers as received, before
  decompression, as an `http::HeaderMap` built by the same converter that
  builds `Response::headers()`.
- `Jar::store_set_cookie` is the one Set-Cookie parser. The session calls it
  for every response. `Max-Age=0` or a past `Expires` deletes the cookie.
- `Jar::get_cookie`, `set_cookie`, `store_set_cookie`, `load_cookies`,
  `export_cookies`, and `remove` take a parsed `&Url`, like the reqwest
  cookie store. They do not return a URL parse error.
- `Jar::snapshot` returns a new jar with its own store and a copy of every
  cookie with all attributes, creation order included. A write to one jar
  does not reach the other.
- `Jar::extend_from(other)` copies every cookie of `other` into the jar with
  its attributes. On the same name, domain, and path, the cookie from
  `other` wins. The jar limits apply after the merge. Merging a jar into
  itself does nothing.
- `Cookie::is_expired` is true when the cookie has an expiry that has
  passed. The jar never sends or returns an expired cookie.
- `Jar::remove(url, name)` removes every cookie with that name that the host
  of `url` receives, on every path, and returns the count.
- `Jar::remove_named` removes every cookie with that name on every host and
  returns the count.
- `Browser::identity` returns the identity a session sends for that browser,
  platform, and brand. It calls `profile::resolve_identity`, the same
  function the session builder calls. It
  returns `None` when the profile has no identity for the platform or the
  brand overlay has no capture.
- `Identity::locked(browser, platform)` sends one browser's TLS and HTTP
  identity. `rotate_tls` keeps the HTTP identity and changes the TLS hello
  to another version of the same family. `switch_family` moves the whole
  identity to a browser of another family on the same platform, for example
  to hand a cookie jar from a Chrome session to a Firefox session.
  `rotate_tls` and `switch_family` check their input and return
  `Kind::Config` at the call.
- `Browser` variants are never removed within 0.x. A retired profile keeps
  its variant and its data. Its profile TOML sets `deprecated = "..."` in
  `[meta]`, and the build marks the variant `#[deprecated]` with that note.
  A deprecated profile is never `Browser::latest` or a platform twin target.
- `RetryPolicy::retry_on` replaces the trigger set. `on_status` appends one
  status.
- `DnsConfig::resolve_host` replaces the address list for a host. An empty
  list removes the override.
- The session jar is the only cookie store. `Response::cookies()` parses
  the Set-Cookie headers of that response; it does not store anything.
- `text`, `text_with_charset`, `bytes`, and `json` consume the response,
  so `session.get(url).await?.bytes().await?` is one expression. `bytes`
  returns an owned `bytes::Bytes`. `text` and `text_with_charset` decode
  the declared charset. `into_stream`, `copy_to`, and `read_until` also
  consume the response. Read `status`, `headers`, and other metadata before
  the body call.
- `Response::headers()` returns the `http::HeaderMap`, duplicates included.
  `header(name)` returns the first value as `&str` and skips a value that is
  not UTF-8; read it through `headers()`.
- Reading the body never changes `headers()` or `content_length()`. A
  buffered response that leyline decoded has no `Content-Encoding` and no
  `Content-Length` from the moment it is returned. A `.stream()` response
  keeps the wire values, also after `text`, `bytes`, or `json` decode it.
- `error_for_status` consumes the response. `error_for_status_ref` borrows
  it and returns `Ok(&Response)`, so the headers and the body stay readable
  after a 4xx or 5xx. Both errors carry the status and the URL.
- One decoder handles `Content-Encoding` for buffered and streamed reads:
  gzip, deflate (zlib or raw), brotli, zstd, at most 4 codings.
- `CompressionConfig::max_body_size` is the one response body cap, 100 MiB
  by default. HTTP/1.1, HTTP/2, and HTTP/3 reads and the decoded size all
  stop at it.
- `Response::read_until(limit, done)` calls `done(body, from)` after each
  decoded chunk. `body` is every decoded byte so far; `from` is where the
  newest chunk starts. It stops on `true`, at `limit` decoded bytes, or at
  end of stream.
- `Response::audit` is `None` unless the session was built with
  `audit(true)`.
- Public enums and public structs are `#[non_exhaustive]`, including
  `Cookie` and `SameSite`. Config types have no public fields.
- Builder input errors surface at `build()` or `send()`, never eagerly.
  `Identity` is a value type, not a builder; its checks run at the call.

## Errors

One error type: `leyline::Error`. Read `err.kind()` for the `Kind`.
Downcast the source with `err.tls()`, `err.h2()`, or `err.io()`.
`Kind::as_str()` gives a stable lowercase label equal to the variant name.
`Display` prints the kind, status, message, and URL. It does not repeat the
source; walk `source()` for the cause.
`Error::is_retryable()` is true when `RetryTrigger::Timeout` or
`RetryTrigger::ConnectionError` would match. `RequestBuilder::send` uses the
same function.
`TlsError` is `#[non_exhaustive]`: `Rejected` (peer closed or reset the
handshake), `Handshake`, `HandshakeIo`, `Certificate { verify_code, reason, .. }`,
`Hostname`, `Pinning`, `Dns`, `TcpConnect`, `Proxy { status, .. }`,
`SslConfig`, `Profile`, `TrustStore`.

| Kind | Meaning |
|---|---|
| `Request` | HTTP-level failure (bad header, malformed request) |
| `Redirect` | Redirect policy stopped the chain |
| `Status` | `error_for_status` rejected the status |
| `Body` | Body read or write failed |
| `Decode` | Decompression or charset decode failed |
| `Timeout` | A timeout expired |
| `Connect` | DNS resolution or TCP connect failed |
| `Tls` | TLS handshake or certificate verification failed |
| `Http2` | HTTP/2 protocol failure |
| `Http3` | HTTP/3 protocol failure |
| `Proxy` | Proxy dial, handshake, authentication, or `CONNECT` failed |
| `Io` | Socket I/O |
| `Config` | Invalid session, request, TLS profile, or trust store configuration |
| `Url` | URL parse failure |
| `Json` | JSON serialize or deserialize failure |

## Features

Default: `charset`, `compression-gzip`, `compression-brotli`,
`compression-deflate`, `compression-zstd`, `multipart`, `stream`,
`websocket`, `http3`. Opt-in: `socks`, `tower`, `unstable-bssl`,
`bench-internals` (outside semver). `full` enables every opt-in except `bench-internals` and
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
TOML keys. `profiles/bare.toml` holds the bare profile and
`profiles/platforms.toml` holds the per-platform TCP values. Load custom
profiles with `profile::ProfileRegistry::load(dir)`
or `BrowserProfile::from_toml`.
