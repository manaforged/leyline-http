# Leyline API

**Locked.** This page and the [API reference](reference/leyline-http/index.md)
are the public contract of `leyline-http` (import name `leyline`). Code,
guides, bindings, and reviews follow them. The API reference lists every
public item and maps each task to its one call. It is generated from the
compiler, and `cargo truesight check` fails when it no longer matches the
code.

The `bench-internals` feature makes internal items public for Leyline's own
tests, benches, and fuzz targets, and the `leyline_unstable_bssl` compiler flag
exposes the BoringSSL context builder. Neither is part of the contract, and the
API reference leaves both out.

## Frame (one)

```text
session     Session::new()                  newest captured Chrome, Windows
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

## Semantics

- `Session::new()` and `Session::default()` impersonate the newest captured
  Chrome, currently Chrome 154, with a Windows identity. With the `http3` feature they race
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
- Header merge runs in three layers. The profile's header shape in
  `profiles/headers.toml` gives the base list for the preset, its order, and
  the headers it appends. `HeaderStyle` is generated from that file. `SessionBuilder::headers` replaces a profile header of the
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
- `Session::proxy_url` returns the exit a session is bound to as a
  `ProxyUrl`. Its `Display` and `Debug` hide the password; `with_proxy`
  takes it back, and `String::from` gives the full URL.
- A proxy set on `RequestBuilder` or `WebSocketBuilder` replaces the session
  proxy config for that request. A `ProxyConfig::new()` with no rule sends
  that request direct.
- `ProxyConfig::proxy_for` picks the proxy for each URL, rules first, then
  `no_proxy`. `send()` checks the proxy URL it picks and fails with
  `Kind::Config` if the URL is invalid. `build()` checks the session rules.
- HTTP/3 runs over a `socks5://` or `socks5h://` proxy through SOCKS5
  `UDP ASSOCIATE` when the `socks` feature is on. The pool keys HTTP/3
  connections by proxy. An `http://` or `https://` proxy cannot carry HTTP/3:
  `build()` rejects `ProtocolPolicy::Http3` when such a proxy takes every URL,
  `send()` rejects an HTTP/3 request when `proxy_for` picks one, and a `Race`
  session sends that request over HTTP/2.
- Connection setup has no retry of its own. A failed connect returns the
  error to `RequestBuilder::send`, and `RetryTrigger::ConnectionError`
  decides whether to retry it. When a pooled keep-alive connection fails
  before the response, the pool sends an idempotent request with a buffered
  or empty body once more on a new connection, outside `RetryPolicy`.
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
  `cookie_header`, and `remove` take a parsed `&Url`, like the reqwest
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
- A WebSocket `send`, `recv`, or `close` that fails on the transport, or on a
  closed connection, gives `Kind::Io`. A protocol violation or a message over
  `WebSocketConfig` limits gives `Kind::Body`.
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
`Display` prints the kind, the status and message when present, and the URL
when the error carries one, with its password and query masked. It does not
repeat the source; walk `source()` for the cause.
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
`compression-deflate`, `compression-zstd`, `multipart`, `websocket`,
`http3`. Opt-in: `socks`, `tower`, `bench-internals` (outside semver). `full`
enables every opt-in except `bench-internals`. Build with
`RUSTFLAGS="--cfg leyline_unstable_bssl"` to reach the BoringSSL
`SslContextBuilder` behind `TlsContext`. The BoringSSL crates are outside the
semver promise.

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

The schema is part of the API. A patch release can add fields; removing or
renaming one needs a minor release. `Browser` gains a variant for each captured
release. A retired profile keeps its variant, marked `#[deprecated]` through its
`[meta] deprecated` note, until a minor release removes it.
