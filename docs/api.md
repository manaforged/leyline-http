# Leyline API

This page and the [API reference](reference/leyline-http/index.md) are the
public contract of `leyline-http` (import name `leyline`). The API reference
lists every public item and maps each task to its one call. It is generated
from the compiler, and `cargo truesight check` fails when it no longer matches
the code. This page covers how the calls fit together, the rules they follow,
and the error model.

The `bench-internals` feature makes internal items public for Leyline's own
tests, benches, and fuzz targets, and the `leyline_unstable_bssl` compiler flag
exposes the BoringSSL context builder. Neither is part of the contract, and the
API reference leaves both out.

## Overview

```text
session     Session::new()                  newest bundled Chrome, Windows
            Session::builder() → SessionBuilder → build() → Session
            session.with_proxy(config)      clone that shares the pool, other proxy
            session.fresh_pool()            clone with a new pool and TLS session cache
            session.with_cookie_jar(jar)    clone that shares the pool, other jar
            session.with_redirect(policy)   clone that shares the pool, other redirect policy
            session.with_identity(id)       clone with another identity and a new pool
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

## Semantics

### Sessions and profiles

- `Session::new()` and `Session::default()` impersonate the newest bundled
  Chrome with a Windows identity. With the `http3` feature they race HTTP/3
  against HTTP/2 when the profile's `[h3]` table sets `race = true`, as the
  bundled Chrome profiles do. The race runs only for an origin that advertised
  `h3` in an `Alt-Svc` header, and every other origin takes `Auto`. See
  [The Alt-Svc gate](guide/http3.md#the-alt-svc-gate).
  `Session::builder().build()` with no browser is a bare session with no
  impersonation.
- `Browser::latest(family)` returns the newest non-deprecated profile of the
  family whose `[meta] capture` value is in the family's `latest_capture` list
  in `profiles/families.toml`. The default list is `["browser"]`. Safari on
  iOS and OkHttp use `["browser", "emulator"]`, and CFNetwork uses
  `["native", "emulator"]`. The crate build fails when a family has no match.
  `Session::new()` uses `Browser::latest(Family::Chrome)`. This choice moves:
  a patch release can add a newer capture. For a fixed fingerprint, pin the
  browser with `Session::builder().browser(Browser::Chrome148)` or
  `Browser::get(Family::Chrome, 148)`.
- `Session::new()` does not fail. The bundled profile data is fixed at
  compile time. Trust-store problems at startup log a warning, and a request
  that then cannot verify a certificate fails with `Kind::Tls`. An invalid
  proxy URL in the environment does not stop `Session::new()` either: every
  request then fails with `Kind::Proxy`, and none connects direct.
- `SessionBuilder::browser` and `platform` give the same result in either
  order. For a bundled Safari or CFNetwork browser, `platform` selects the
  profile of that platform (`Browser::for_platform`, from
  `[meta] platform_browser`).
- `SessionBuilder::profile` takes a profile from `ProfileRegistry::load` or
  `BrowserProfile::from_toml`. The session sends that profile's TLS, HTTP/2,
  HTTP/3, and `[identity.<platform>]` tables. The last call of `browser`,
  `identity`, or `profile` wins. A loaded profile has no per-platform variant,
  so `build()` returns `Kind::Config` when the profile has no
  `[identity.<platform>]` table for the platform. Without `.platform()`, the
  platform is Windows. `Session::identity().browser()` is `None` for a loaded
  profile.
- `BrowserProfile::from_fingerprint` builds a profile from a raw JA3 string,
  a raw JA4_r string, and an Akamai HTTP/2 string, on top of the
  `FingerprintSpec::base` profile or the bare profile. It returns
  `leyline::Result<BrowserProfile>`, and `SessionBuilder::profile` sends the
  profile. Leyline's built-in table of ciphers, signature algorithms, and
  curves, which `audit` also uses, maps the IDs to names, so `Response::audit`
  reports the JA3 and Akamai values that the strings set. A hashed JA4, an
  unknown ID, an extension or SETTINGS ID that Leyline cannot send, and
  PRIORITY frames fail with `Kind::Config`. See
  [Build a profile from a JA3 or Akamai string](guide/profiles.md#build-a-profile-from-a-ja3-or-akamai-string).
- A brand overlay needs `chromium_major` in the profile `[meta]` table.
- A patch release never removes a `Browser` variant. A retired profile keeps
  its variant and its data. Its profile TOML sets `deprecated = "..."` in
  `[meta]`, and the build marks the variant `#[deprecated]` with that note. A
  deprecated profile is never the result of `Browser::latest` or
  `Browser::for_platform`.

### Identities

- `Session::identity` returns what the session sends, read from the values
  `build()` resolved: the `Identity` and the HTTP browser (both `None` for a
  bare or loaded-profile session), the platform, the brand (`None` when no
  Chromium brand applies), and the `User-Agent`.
- `Browser::identity(platform, brand)` returns the `PlatformIdentity` that a
  session built with that browser, platform, and brand sends: `User-Agent`,
  `sec-ch-ua`, and `Accept-Language`. It returns `None` when the profile has
  no identity for the platform or the brand has no overlay for that browser
  and platform.
- `Identity::locked(browser, platform)` sends one browser's TLS and HTTP
  identity. `rotate_tls` keeps the HTTP identity and changes the TLS hello to
  another version of the same family. `rotate_hello` moves to the next
  ClientHello of the family. `switch_family` moves the whole identity to a
  browser of another family on the same platform, for example to hand a cookie
  jar from a Chrome session to a Firefox session. `rotate_tls`,
  `rotate_hello`, and `switch_family` check their input and return
  `Kind::Config` at the call.

### Session clones

- `Session::with_proxy` derives a clone with the given proxy config. The
  clone shares the pool. Pool entries are keyed by proxy URL, so a clone with
  the same proxy reuses its connections.
- `Session::fresh_pool` derives a clone with a new, empty pool and TLS session
  cache, so the next request opens new connections.
- `Session::with_cookie_jar` derives a clone through the same path as
  `with_proxy`. It shares the pool and every other setting and uses the given
  jar.
- `Session::with_redirect` derives a clone through the same path and uses the
  given redirect policy. `RequestBuilder::redirect` still overrides it for one
  request.
- `Session::with_identity` derives a clone that sends another `Identity`. The
  `tls()` browser supplies the ClientHello and the HTTP/2 and HTTP/3
  settings. The `http()` browser supplies `User-Agent`, `sec-ch-ua`,
  `Accept-Language`, and the header shape and order. A `user-agent` header
  from `SessionBuilder::headers` stays the `User-Agent` of the clone. The
  clone gets a new pool
  and TLS session cache, shares the cookie jar, and keeps the brand, proxy,
  timeouts, compression, and TCP profile. A session built without a bundled
  browser, a bare session or a loaded profile, returns `Kind::Config`. So does
  an `Http3` or `Race` session when the `tls()` browser has no HTTP/3 profile.
  See [Mix and rotate identities](guide/fingerprints.md#mix-and-rotate-identities).

### Config types, timeouts, and retries

- Config types (`TimeoutConfig`, `RetryPolicy`, `RedirectPolicy`,
  `ProxyConfig`, `DnsConfig`, `TlsTrustConfig`, `PoolConfig`, `SocketConfig`,
  `HappyEyeballsConfig`, `CompressionConfig`, `WebSocketConfig`) have private
  fields and consuming setters named after the field, with no `with_`
  prefix. A setter for an optional value takes `impl Into<Option<T>>`;
  `None` turns that setting off. `RetryPolicy::max_retry_after` is the
  exception: it takes a `Duration`, and the cap is off until you set it. A setter that adds to a list starts with
  `add_` or appends one item (`rule`, `on_status`).
- Every `SessionBuilder` config setter replaces the whole value. Start from
  `::new()`, which carries the defaults.
- `TlsTrustConfig::min_tls_version(TlsMinVersion)` sets a TLS version floor.
  The handshake minimum is the higher of the profile's minimum and the floor,
  and the default floor, `Tls10`, changes nothing. HTTP/3 always uses TLS 1.3.
  See [Set a TLS version floor](guide/tls-trust.md#set-a-tls-version-floor).
- `TimeoutConfig` has four limits:
  - `total` (300 s by default) bounds the whole `send()`: every redirect hop,
    retry, and backoff, and the buffered body read. `total(None)` turns it
    off.
  - `response_header` (off by default) bounds each redirect hop. On a buffered
    response the hop includes the body read, and on a `.stream()` response it
    covers the head only.
  - `read` (off by default) is an idle limit for each chunk of a `.stream()`
    body that you read after `send()` returns. A buffered response never uses
    it.
  - `connect` (10 s by default) bounds DNS, TCP, the proxy handshake, the TLS
    handshake, and the HTTP/3 handshake. It is a session setting, and a
    request value for it has no effect.
- An expired `read` gives `Kind::Timeout` from `bytes`, `text`, and `json`, an
  `io::Error` of kind `TimedOut` from `into_stream`, and `Kind::Io` from
  `copy_to` and `read_until`. An expired `connect` gives `Kind::Connect`.
  `Error::is_timeout()` is true in each case.
- `RequestBuilder::timeout` merges over the session timeouts field by field.
  A field that the request sets wins, and every other field keeps the session
  value. A bare `Duration` sets `total`.
- `RetryPolicy::retry_on` replaces the trigger set. `on_status` appends one
  status.
- `DnsConfig::resolve_host` replaces the address list for a host. An empty
  list removes the override.
- Connection setup has no retry of its own: a failed connect returns its
  error to `RequestBuilder::send`. There, a timeout, a connect timeout
  included, needs `RetryTrigger::Timeout`, and any other retryable error needs
  `RetryTrigger::ConnectionError`. `RetryPolicy` and the transport resend a
  request only when its body can be replayed, so a request with a streamed
  body is never resent. Outside `RetryPolicy`, the transport also resends in
  these cases:
  - HTTP/1.1 sends an idempotent request once more on a new connection when a
    pooled keep-alive connection fails before the response.
  - HTTP/2 resends when a pooled connection fails mid-request: any method
    after `REFUSED_STREAM`, and an idempotent method after any other failure.
  - HTTP/3 opens a new connection when the request never reached the server.
    It requeues the request once on the same connection after
    `H3_REQUEST_REJECTED` before the response head.
  - `Race` runs the request as `Auto` when both connects fail.

### Requests and headers

- `Session::get`, `post`, `put`, `patch`, `delete`, `head`, `request`,
  `websocket`, and `preconnect` take `impl IntoUrl`. `IntoUrl` is sealed. A
  URL that does not parse gives `Kind::Url` with the `url::ParseError` as
  source, at `send()`, `connect()`, or `preconnect()`.
- `Session::execute` reads per-request policy from `http::Extensions`:
  `TimeoutConfig`, `RetryPolicy`, `RedirectPolicy`, `Preset`. Other policy
  comes from the session.
- `RequestBuilder::redirect` overrides the session redirect policy for one
  request.
- `RequestBuilder::header` appends, with `http::HeaderMap::append` semantics:
  repeated calls with one name add values, and Leyline sends all of them.
  `json`, `form`, `multipart`, and the auth setters replace the header they
  own.
- `WebSocketBuilder::header` and `headers` take the same input and append the
  same way. A `user-agent` or `origin` header replaces the value that the
  handshake sends, so the handshake carries one of each, with the last value.
- Header merge runs in three layers:
  - The profile's header shape in `profiles/headers.toml` gives the base list
    for the preset, its order, and the headers it appends. `HeaderStyle` is
    generated from that file.
  - `SessionBuilder::headers` replaces the value of a profile header of the
    same name in place, or appends a new header at the end. A repeated session
    name keeps the last value.
  - A request header replaces the first header of that name at its position,
    and all request values for the name take that position in call order. A
    request name skips the session value. A request header with no match goes
    to the position its name implies (`Authorization` after `User-Agent`, for
    example) or to the end.

### Proxies

- `Session::proxy_url` returns the proxy of the session's `all` rule, or of
  its first rule when it has none, as a `ProxyUrl`. It returns `None` when
  the session has no rule, and after a rejected environment proxy. Its
  `Display` and `Debug` hide the password; `with_proxy` takes it back, and
  `String::from` gives the full URL.
- A proxy set on `RequestBuilder` or `WebSocketBuilder` replaces the session
  proxy config for that request. A `ProxyConfig::new()` with no rule sends
  that request direct.
- The first rule whose scheme matches the URL picks the proxy.
  `ProxyRule::all` matches every URL, `ProxyRule::http` matches `http://` and
  `ws://`, and `ProxyRule::https` matches `https://` and `wss://`. Then the
  `NoProxy` list can send the request direct. The list applies when you set it
  with `ProxyConfig::no_proxy` and when the proxy came from the environment.
  `send()` fails with `Kind::Config` if the chosen URL is invalid, and
  `build()` checks the session rules.
- With no rule and `env(true)`, the default, `build()` reads the first
  non-empty of `HTTPS_PROXY`, `https_proxy`, `HTTP_PROXY`, `http_proxy`,
  `ALL_PROXY`, and `all_proxy`, and applies it to every scheme. An invalid
  value never falls back to a direct connection. `build()` fails with
  `Kind::Config`. `Session::new()` builds anyway, and then every request that
  uses the session's proxy config fails with `Kind::Proxy`, `Display` prints
  `proxy=rejected`, and `proxy_url()` returns `None`. A proxy that you set
  with `SessionBuilder::proxy`, with `with_proxy`, or on one request is not
  affected. The message names the variable and not its value. See
  [Environment discovery](guide/proxies.md#environment-discovery).
- HTTP/3 runs over a `socks5://` or `socks5h://` proxy through SOCKS5
  `UDP ASSOCIATE` when the `socks` feature is on. The pool keys HTTP/3
  connections by proxy. An `http://` or `https://` proxy cannot carry HTTP/3:
  `build()` rejects `ProtocolPolicy::Http3` when such a proxy takes every URL,
  `send()` rejects an HTTP/3 request when the proxy picked for that URL is
  such a proxy, and a `Race` session sends that request over HTTP/2.

### Cookies

- One parser reads every `Set-Cookie` value: the session uses it for each
  response, `Jar::store_set_cookie` uses it for a value you supply, and
  `Response::cookies()` uses it for one response. `Max-Age=0` or a past
  `Expires` deletes the stored cookie with the same name, domain, and path.
- The session jar is the only cookie store. `Response::cookies()` parses the
  `Set-Cookie` headers of that response and stores nothing.
- `Jar::get_cookie`, `set_cookie`, `store_set_cookie`, `load_cookies`,
  `cookie_header`, and `remove` take a parsed `&Url`, so they cannot return a
  URL parse error.
- `Jar::snapshot` returns a new jar with its own store and a copy of every
  cookie with all attributes, creation order included. A write to one jar
  does not reach the other.
- `Jar::extend_from(other)` copies every cookie of `other` into the jar with
  its attributes. On the same name, domain, and path, the cookie from
  `other` wins. The jar limits apply as each cookie is added. Merging a jar
  into itself does nothing.
- `Cookie::is_expired` is true when the cookie has an expiry that has
  passed. The jar skips expired cookies when it builds a `Cookie` header and
  in `get_cookie`. `all_cookies` and serialization keep them until a
  `Set-Cookie` or a `remove` call deletes them or a jar limit evicts them.
  Filter with `is_expired`.
- `Jar::remove(url, name)` removes every cookie with that name that the host
  of `url` receives, on every path, and returns the count.
- `Jar::remove_named` removes every cookie with that name on every host and
  returns the count.

### Responses and bodies

- `text`, `text_with_charset`, `bytes`, and `json` consume the response,
  so `session.get(url).await?.bytes().await?` is one expression. `bytes`
  returns an owned `bytes::Bytes`. `text_with_charset(default)` decodes with
  the charset in the `Content-Type` header, else with `default`, and `text` is
  `text_with_charset("utf-8")`. An unknown charset label falls back to UTF-8.
  Without the `charset` feature, both decode lossy UTF-8 and ignore the
  declared charset. `into_stream`, `copy_to`, and `read_until` also consume
  the response. Read `status`, `headers`, and other metadata before the body
  call.
- `Response::headers()` returns the `http::HeaderMap`, duplicates included.
  `header(name)` returns the first value as `&str`, and `None` when the header
  is absent or the first value has a byte outside visible ASCII. Read such a
  value through `headers()`.
- Reading the body never changes `headers()` or `content_length()`. A
  buffered response that Leyline decoded has no `Content-Encoding` and no
  `Content-Length` from the moment it is returned. A `.stream()` response
  keeps the wire values, also after `text`, `bytes`, or `json` decode it.
- `error_for_status` consumes the response. `error_for_status_ref` borrows
  it and returns `Ok(&Response)`, so the headers and the body stay readable
  after a 4xx or 5xx. Both errors carry the status and the URL.
- One decoder handles `Content-Encoding` for buffered and streamed reads:
  gzip, deflate (zlib or raw), brotli, and zstd. More than 4 codings fail with
  `Kind::Decode`. When `CompressionConfig` turns off any coding in the list,
  Leyline returns the body as received, with its `Content-Encoding` header.
- `CompressionConfig::max_body_size` (100 MiB by default) caps a buffered body
  on HTTP/1.1, HTTP/2, and HTTP/3. The cap applies to the bytes that `bytes`,
  `text`, and `json` collect, and to the decoded output. A body over the cap
  fails with `Kind::Body`. `into_stream` and `copy_to` on a `.stream()`
  response return the wire bytes with no cap and no decoding. `bytes`,
  `text`, and `json` on a `.stream()` response still apply the cap and
  decode the body.
- `Response::read_until(limit, done)` calls `done(body, from)` after each
  decoded chunk. `body` is every decoded byte so far, and `from` is where the
  newest chunk starts. The call stops when `done` returns `true`, at `limit`
  decoded bytes, or at end of stream.
- `Response::audit` is `None` unless the session was built with
  `audit(true)`.
- `Response::request_headers`, the `request_headers` field of the `Response`
  `Debug` output, and a JA4H input built from those headers are empty unless
  the session was built with `SessionBuilder::audit(true)`. The values are the
  headers the session prepared, not a capture of the wire.
- The `trace::Head` event carries the response headers as received, before
  decompression, as an `http::HeaderMap`.

### API shape

- Public enums, and public structs with public fields, are `#[non_exhaustive]`,
  including `Cookie` and `SameSite`. The exceptions are `audit::Ja3Input`,
  `Ja4Input`, and `Ja4hInput`, which callers fill with a struct literal. Other
  public structs have private fields, and config types use consuming setters.
- Builder input errors surface at `build()`, `send()`, or `connect()`, never
  eagerly.
  `Identity` is a value type, so its checks run at the call.

## Errors

One error type: `leyline::Error`. Read `err.kind()` for the `Kind`.
Downcast the source with `err.tls()`, `err.h2()`, or `err.io()`.
`Kind::as_str()` gives a stable lowercase label equal to the variant name.
`Display` prints the kind, the status and message when present, and the URL
when the error carries one, with its password and query masked. It does not
repeat the source; walk `source()` for the cause.

`Error::is_retryable()` is true for a timeout, a failed or closed connection,
and a proxy `CONNECT` that fails on I/O or is answered with 502, 503, or 504.
`RetryPolicy::transient()` also retries a response with one of those three
statuses. `RequestBuilder::send` retries a timeout under
`RetryTrigger::Timeout` and every other retryable error under
`RetryTrigger::ConnectionError`.

`TlsError` is `#[non_exhaustive]`: `Rejected` (peer closed or reset the
handshake), `Handshake`, `HandshakeIo`, `Certificate { verify_code, reason, .. }`,
`Hostname`, `Pinning`, `Dns`, `TcpConnect`, `Proxy { status, .. }`,
`SslConfig`, `Profile`, `TrustStore`.

The profile loaders, `ProfileRegistry::load` and `BrowserProfile::from_toml`,
return `profile::ProfileError` (`Empty`, `Io`, `Parse`), which has no `Kind`.
`multipart::Form::file` returns `std::io::Result`.

A WebSocket `send`, `recv`, or `close` that fails on the transport, or on a
closed connection, gives `Kind::Io`. A protocol violation or a message over
`WebSocketConfig` limits gives `Kind::Body`.

| Kind | Meaning |
|---|---|
| `Request` | HTTP-level failure (bad header, malformed request) |
| `Redirect` | A redirect could not be followed: a custom policy passed 32 hops, a `Location` scheme other than http or https, or a redirect that must resend a streamed request body. A policy that stops returns the 3xx response and is not an error |
| `Status` | `error_for_status` rejected the status |
| `Body` | A body cannot be used: a response body over `max_body_size`, a response body already taken, a request body that cannot be compressed or resent, or a WebSocket protocol violation or oversized message |
| `Decode` | Decompression failed: bad data, a truncated stream, or more than 4 codings |
| `Timeout` | `total` or `response_header` expired, or `read` expired in `bytes`, `text`, or `json` |
| `Connect` | DNS resolution or TCP connect failed, or the `connect` limit expired during DNS, TCP, a proxy handshake, or a TLS or HTTP/3 handshake. An expired limit also makes `is_timeout()` true |
| `Tls` | TLS handshake or certificate verification failed |
| `Http2` | HTTP/2 protocol failure |
| `Http3` | HTTP/3 protocol or handshake failure |
| `Proxy` | Proxy dial, handshake, authentication, or `CONNECT` failed, or a request of a session that rejected an invalid environment proxy |
| `Io` | Socket I/O, or an HTTP/1.1 response that breaks the wire format; `err.io()` then has kind `InvalidData` |
| `Config` | Invalid session, request, TLS profile, trust store, or proxy configuration, including an invalid environment proxy at `build()` |
| `Url` | URL parse failure |
| `Json` | JSON serialize or deserialize failure |

## Features

Default: `charset`, `compression-gzip`, `compression-brotli`,
`compression-deflate`, `compression-zstd`, `multipart`, `websocket`,
`http3`. Opt-in: `socks`, `tower`, `bench-internals` (outside semver). `full`
enables every opt-in except `bench-internals`. Build with
`RUSTFLAGS="--cfg leyline_unstable_bssl"`, or enable `bench-internals`, to
reach the BoringSSL `SslContextBuilder` behind `TlsContext`. Both, and the
BoringSSL crates, are outside the semver promise.

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
- A getter that copies session configuration back out. The exceptions report
  the state of a built session. `Session::identity` and `Session::proxy_url`
  report what it sends, and `proxy_url` returns the exit as a `ProxyUrl` whose
  `Display` hides the password. `Session::cookies` returns the session's
  cookie jar, and `Session::pool_stats` returns a `PoolStats` snapshot of the
  pool.
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

The keys `capture`, `variant`, `hello`, `platform_browser`, `deprecated`, and
`verified_at` are valid in `[meta]` and ignored at runtime. They are not
fields of `ProfileMeta`. The build reads the first five to generate `Browser`.

The schema is part of the API. A patch release can add fields; removing or
renaming one needs a minor release. `Browser` gains a variant for each captured
release. A retired profile keeps its variant, marked `#[deprecated]` through its
`[meta] deprecated` note, until a minor release removes it.
