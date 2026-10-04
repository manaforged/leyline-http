# Changelog

The minimum supported Rust version is 1.96, two releases behind stable at
release time.

Leyline is experimental: any `0.x` release can change the API and its
behaviour, and each change is listed here. The BoringSSL crates
`leyline-bssl`, `leyline-bssl-sys`, and `leyline-bssl-tokio` share this
version and publish as separate crates.

## Unreleased

### Changed

- `leyline-bssl-sys` ships pre-generated BoringSSL bindings for each
  supported target, so a build no longer compiles or runs `bindgen`, and
  links the BoringSSL source into the build directory instead of copying
  it. Cold builds are faster. To generate the bindings at build time, for
  example for another target, enable the new `bindgen` feature. CMake uses
  the Ninja generator when `ninja` is on `PATH` and `CMAKE_GENERATOR` is
  not set.

## 0.1.1 - 2026-10-02

### Changed

- `Session::new()` and `Session::default()` build a plain session.
  `Session::browser(browser)` is the one-line browser session:
  `Session::browser(Browser::default())` is the newest bundled Chrome on
  Windows, which `Session::new()` built before. A browser takes the first
  platform its own profile covers, so a mobile browser gets its mobile
  platform, with or without the builder.
- A plain session never sends browser-only headers (`sec-*`, client hints,
  `upgrade-insecure-requests`, `priority`), for any preset.
- On a browser session, `SessionBuilder::user_agent` drops the `sec-ch-ua`
  client hint, so the user agent and the hint never disagree.
- Firefox `accept-language` weights follow Firefox: equal steps from 1, as in
  `en,ja;q=0.5` and `de-DE,de;q=0.7,en;q=0.3`.
- A session with a browser and no `protocol` call takes its protocol policy
  from the profile: it races HTTP/3 when the `http3` feature is on and the
  profile's `[h3]` table sets `race = true`.
- Sessions derived with `with_identity` share the parent's connection pool,
  HSTS store, and `Alt-Svc` knowledge, in a partition per profile, instead of
  opening a new pool.
- Converting an `io::Error` that wraps a `leyline::Error` returns the wrapped
  error with its own kind instead of `Kind::Io`.

### Added

#### Sessions and requests

- `leyline::get(url)` sends one GET from a plain session.
- `SessionBuilder::base_url`, `bearer_auth`, `user_agent`, and `languages`.
  With a `base_url`, the session token goes only to the base URL's origin. `Session::with_base_url` derives a session with another base.
- `RequestBuilder::error_for_status()` turns a `4xx` or `5xx` into an error
  after the retries, and keeps the headers and the start of the body.
  `CompressionConfig::max_error_body` caps the kept body (64 KiB by default)
  for `error_for_status` and `download`, and `TimeoutConfig::error_body`
  bounds its read (10 s by default, and never past the `total` timeout).
  Decompression stops at the cap. The kept body is cut at the smaller of
  `max_error_body` and `max_body_size`, with or without content-encoding.
- `RequestBuilder::download(path, limit)` and `Response::download_to` write
  the decoded body through a temporary file, so the path holds the whole
  body or nothing; a declared `Content-Length` over the cap is refused before
  reading. `Response::into_decoded_stream` and `copy_decoded_to` stream the
  decoded body under a limit.
- `RequestBuilder::pages()` follows `Link: rel="next"`, and counts the first
  URL as fetched, so a redirect does not repeat the first page.
  `Pages::limit(max)` stops after `max` pages; there is no default limit;
  `Response::link(rel)` and `links()` parse `Link` headers.
- `RequestBuilder::cookie_jar`, `initiator`, and `tag`. `Response::attempts()`
  and `Response::proxy()`.
- `TimeoutConfig::body(duration)` bounds the time to read a whole body,
  counted from the first read of the body.
- `Session::shutdown()` stops every clone of a session, including bodies
  being streamed; `Error::is_shut_down()` names that error.
- `relay_headers(headers, RelayBody)` and `Response::relay_headers` return the
  headers to forward from a proxy or a service. `proxy-authenticate` and
  `proxy-authorization` are dropped as hop-by-hop. `RelayBody` is
  `#[non_exhaustive]`. With `RelayBody::Decoded`,
  `Response::relay_headers` keeps `content-encoding` when the session did not
  decode the body.
- `RequestBuilder::download` and `Response::download_to` keep the permission
  bits of a target file that already exists, never setuid, setgid, or sticky
  bits.
- `multipart::Part::file(path)` reads a file part that takes a MIME type.

#### Errors and retries

- `Error::category()` returns one `ErrorCategory` per failure, with
  `as_str()` and `gateway_status()`. `Error::is_connect()` agrees with it. `Error::is_dns()`, `is_proxy()`, and
  `is_body_limit()` refine it. `Error::find()` reaches a leyline error inside
  any error chain, such as a tower `BoxError`. `leyline::Error` converts from
  `url::ParseError`.
- A status error keeps `headers()`, `header(name)`, `body()`, `body_text()`,
  `attempts()`, and `proxy()`, and carries the retry policy's view of its last
  response: `retry_after()` and `retries_exhausted()`.
- `RetryPolicy::retry_if(predicate)`, `wait_header(name, WaitFormat)`,
  `retry_unsent(true)`, `skip_blocks(rules)`, `backoff(attempt)`, and
  `rotate_proxies(list)`. A `wait_header` wins over `Retry-After`.
  `retry_unsent(true)` retries any method after an error the server did not
  process: DNS, connect, TLS, proxy, a connect timeout, an HTTP/2
  `REFUSED_STREAM` reset, or an HTTP/3 request the server reports as not
  processed. An HTTP/3 request the server rejects twice is retried on the
  same connection.
- `RetryPolicy::transient()` caps a server-requested wait at 60 s
  (`max_retry_after`). `Error::retries_exhausted()` is `true` when the policy
  wanted another try and did not make it: no retries left, a server wait above
  `max_retry_after`, or a wait or backoff longer than the time left.
- A streaming request body that was never read, for example after a connect
  error, is retried.

#### Proxies, crawling, and blocks

- `SessionBuilder::host_limits(HostLimits)`: requests in flight and per second
  per origin, overrides per host, a total cap, and `pause_on(statuses)` to
  stop new requests to an origin for the wait the server asks for, read from
  the retry policy's wait headers or `Retry-After`. `pause_for` sets the pause
  when the server asks for none (60 s), and `max_pause` caps a requested
  pause (24 h). A streamed request holds its slot until its body ends; a
  streamed body ends at its first error and releases the slot. A failed
  attempt releases its slot before the retry wait.
  `Session::host_stats()` reports requests in flight and waiting per origin.
- Host limits admit each redirect hop against its own origin. `pause_on` and
  proxy-pool strikes apply to the origin that sent the response. A request
  waits for its origin slot, then for any pause and its next rate slot, then
  the total slot. No request holds a total slot while it waits for a pause or the rate spacing. The spacing is
  measured at admission, and a pause is checked again just before admission.
  A request that loses its turn while it waits for the total slot waits
  again. A host name with a trailing dot
  is the same host. Each session built from a `HostLimits` value has its own
  counters, shared by its clones and derived sessions.
- `ProxyPool`: sticky proxies, bans after repeated failures, rotation on block
  rules, and `ProxyPool::identified` to pin a browser identity to each proxy,
  each proxy with its own cookie jar, also when two proxies share an
  identity. `ProxyPool::identified` on a session without a browser fails at
  `build()` with `Kind::Config`. `ProxyPool::stats()` reports health.
  Identity sessions share one connection pool, partitioned by profile.
- `Response::block()` reports a bot-protection challenge from vendors'
  response headers. `BlockRules::from_toml`, `statuses([..])`, `extend`, and
  `check` define and apply your own rules; one `BlockRules` value drives
  `skip_blocks`, `rotate_on_block`, and `check`.
- `PoolStats::busy` and `PoolStats::idle`.

#### Browser sessions, tabs, and devices

- `HeaderAnchor::BeforeCchUa` places a header before `sec-ch-ua`, where
  Chrome sends the client hints a site asked for.
- `Session::tab()` returns a `Tab` that keeps the current page: `open`,
  `follow`, `submit`, `submit_form`, `fetch`, `xhr`, `post_json`, and
  `subresource` send the page as the initiator. A tab with no page refuses
  script requests.
- `Jar::save_to`, `load_from`, and `autosave` keep a jar on disk.
  Jar and device files are versioned, `{ "version": 1, .. }`, and are written
  with mode `0600` on Unix (default ACLs on Windows). `autosave(path,
  interval)` saves at most once per `interval` after a change, and saves
  again when the runtime shuts down.
- `Device` holds a browser device (identity, frozen profile, `profile_id`,
  user agent, proxy and `proxy_password_env`, languages, jar, browser state,
  current page, and caller data in `app`) and reopens it as the same session.
  `Device::check` refuses a session that differs, and `strict` refuses a
  device without a pinned profile or a proxy. `Device::autosave` keeps the
  device and its jar on disk. Serializing a `Device` with
  `proxy_password_env` set never writes the proxy password; `DeviceAutosave::update` and `track` change it.
  It takes a `Duration` or `DeviceAutosaveOptions`, whose `state_interval`
  (300 s by default) saves the connection state on a period.
  `SessionBuilder::expect_profile_id` and `Error::is_profile_changed()` catch
  a changed profile.
- `Session::state()` returns `SessionState`: TLS session tickets, HTTP/3
  `Alt-Svc` knowledge, and the HSTS store. Sessions keep an HSTS store, and
  learn no HSTS entry when certificate verification is off. Saved TLS session
  keys for an authenticated proxy hold the proxy user name, never the
  password or a value derived from it.
- `Browser`, `Family`, `Platform`, `ChromiumBrand`, `Identity`, `ProxyUrl`,
  and `leyline::Url` implement serde; the profile enums have stable string
  ids. `SessionIdentity::to_identity()` and `profile_id()`.
- The `html` feature (on by default, and it enables `multipart`):
  `html::forms`, `Form::find`, `html::meta`, and `html::links`. Forms follow
  the browser rules: `form="id"` controls (the first element with that id),
  disabled fieldsets (only the first `<legend>` that is a direct child is
  exempt), disabled `<optgroup>` options left out, a list-box `<select size>`
  with no selected option left out, one checked radio button per name, line
  breaks in names and values converted to CRLF, and every HTML named
  character reference.
  `Form::set` replaces every value of a name. `Tab::submit_form` encodes the
  fields as the form's `FormEnctype` says (URL-encoded, `multipart/form-data`,
  or `text/plain`) and resolves the action against `<base href>`.

#### Tracing, audit, and testing

- `Trace::summary` receives one `trace::Summary` per request, and
  `Trace::body` one `trace::BodyEnd` per streamed body. `trace::Metrics`
  counts requests, statuses, error categories, attempts, body outcomes, and
  latency; `trace::Fanout` sends events to several traces.
- `AuditData::compare(&audit::Observed)` compares a session's JA4, JA3,
  HTTP/2 fingerprint, request headers, and header order
  (`FingerprintReport::header_order`) with an echo service report. A header
  the service did not echo is `FieldOutcome::Absent`, which counts as a
  mismatch.
- The `test-util` feature adds `leyline::testing::TestServer`, a local HTTP or
  HTTPS server with a private CA that records requests, with delayed and
  chunked responses. `TestServer::http_on(listener, handler)` serves on your
  own listener. `RecordedRequest` has `raw` (the request head as received),
  `request_line`, `header_values`, `header_count`, and `text`;
  `TestResponse::close` closes the connection after the response.
  `leyline::redact_url` exposes the crate's URL redaction.

### Fixed

- `leyline-http` requires `tokio-util` 0.7.5, the first version with the
  owned cancellation future it uses; 0.7.0 to 0.7.4 failed to build.
- A request that follows a URL the server chose (a `pages()` next link,
  `Tab::follow`, or `Tab::submit_form`) drops the session credentials when
  its origin differs from the page it came from, as a redirect does: the
  `Authorization`, `Cookie`, and `Proxy-Authorization` defaults and the
  session bearer token.
- On HTTP/2 and HTTP/3, a `Host` header you set is not sent; the request
  authority carries the host.
- `HeaderStyle::Bare` is the header shape of a plain session. `HeaderStyle`
  variants have fixed discriminants.
- A browser session's connection to an origin that speaks only HTTP/1.1 sends
  the profile's own ClientHello (its ALPN list and ALPS) and carries the
  request on that connection, as a browser does. Before, Leyline dialed a
  second connection that offered only `http/1.1`, a ClientHello no browser
  sends, and a streamed request body to such an origin failed.
- A plain session sends no `accept-language` unless you set languages or the
  header. A `socks5://` proxy set on the builder or in a `ProxyPool` without
  the `socks` feature fails at `build()`.
- A request with an initiator page computes `sec-fetch-site` and `Origin`
  from that page even when the referrer policy sends no `Referer`, as when an
  https page requests an http URL. A navigation with an initiator is a link
  navigation: it sends the computed `sec-fetch-site` and a `Referer` in the
  position each browser sends it. A navigation without one is a typed URL, as
  before.
- A header template value whose placeholders all render empty is not sent. A
  plain session no longer sends empty or Chromium-only client hints when a
  preset is set.
- A plain session's `accept-encoding` lists only the codings that are compiled
  in and enabled in its `CompressionConfig`, and is omitted when none are.
- HTTP/3: a queued request whose caller has gone is not sent, and responses
  report their connection timing and reuse.
- HTTP/2 and HTTP/3 do not send a caller's `host` header next to the
  `:authority`.
- `ProtocolPolicy::Race` skips HTTP/3 for 5 minutes for an origin and proxy
  pair after an HTTP/3 connection to it fails. The `Http3` policy error names
  the missing `socks` feature when that is the cause.
- A proxy that reports it could not reach the origin (SOCKS5 replies 3 to 6,
  `CONNECT` 502 or 504) gives `TlsError::ProxyTargetUnreachable` with a
  `ProxyReply`; `Error::is_proxy()` is false for it. A timeout while reaching
  a proxy on the TLS path is a proxy error and still a timeout.
- A body-limit error names the limit that applied: the session's
  `max_body_size` or the caller's limit.
- Dropping an HTTP/1.1 streamed response releases its connection and the
  per-host connection slot at once, even while the server sends nothing.
- `LeylineService` streams the decoded body, reads a `ProxyConfig` from the
  request extensions, and puts the final `Url`, `HttpVersion`, and
  `ResponseTiming` in the response extensions. `Session::execute` reads the
  same request extensions.
- Saving a jar skips expired cookies, and loading one skips expired cookies,
  keeps the newest of two cookies in one slot, and applies the cookie limits.
  Each cookie's last-access time survives a save, so eviction order survives
  a restart. A jar that 0.1.0 serialized with serde still deserializes.
- HTTP/2: a request that the caller cancels while it waits for a stream slot
  is not sent. Before, the driver sent it when a slot became free.
- HTTP/2: a streaming request body buffers at most 256 KiB ahead of the
  peer's flow-control window, the same limit HTTP/3 uses. Before, a stalled
  peer let the driver buffer the whole body. The body stream is first polled
  when the request starts on the wire, not when it is queued.
- HTTP/2: when the caller drops a request whose upload has not finished, the
  driver resets the stream with `CANCEL` and stops polling the body.
- HTTP/2: when every stream slot is in use, at most one request waits inside
  the driver. Later requests wait in the bounded command queue, and the
  caller's timeout applies to that wait. Pings use their own queue, so a pool
  liveness ping does not wait behind queued requests.
- The pool remembers that an origin negotiated HTTP/1.1 for 10 minutes, then
  offers HTTP/2 again. It keeps at most 1024 such origins.

### Build

- The minimum `tokio` is 1.37 and the minimum `enum_dispatch` for
  `leyline-quiche` is 0.3.13. Leyline did not compile against the older
  versions the 0.1.0 manifests allowed.

## 0.1.0 - 2026-09-30

First public release.

### Build

- `leyline-bssl-sys` supports `x86_64-unknown-linux-musl` and
  `aarch64-unknown-linux-musl`. A musl build needs a musl C and C++
  toolchain. When you cross-compile for Linux, the build script finds the
  target compiler on `PATH` if `CC_<target>` and `CXX_<target>` are not set.
- `leyline-bssl-sys` builds BoringSSL from source with CMake and links it
  statically. The crates ship source, not prebuilt libraries. A build needs
  CMake 3.22 or later, a C and C++ compiler, libclang, and `git`; on Windows,
  also the MSVC build tools and NASM. The BoringSSL crates are trimmed forks
  of Cloudflare's `boring` v5.2.0.
- BoringSSL is pinned to `ac39ea6`, the `boringssl_revision` in Chromium's
  DEPS at tag `154.0.8037.58`, and carries five patches. One restores the
  ClientHello padding extension that Safari sends and BoringSSL removed.
  BoringSSL's own `SSL_CTX_set_grease_sigalgs_enabled` sends the GREASE
  signature algorithm, and `SslContextBuilder::set_grease_sigalgs_enabled`
  wraps it.
- The build reads its settings from `LEYLINE_BSSL_*` variables, such as
  `LEYLINE_BSSL_PATH` and `LEYLINE_BSSL_SOURCE_PATH`. A `BORING_BSSL_*`
  value set for `boring-sys` does not reach Leyline.
- `leyline-quiche` is Cloudflare's `quiche` 0.30.0 linked against
  `leyline-bssl`. It includes the fix for every quiche security advisory
  published before 0.30.0, among them CVE-2025-4820, CVE-2025-4821,
  CVE-2025-7054, CVE-2026-11941, CVE-2026-12523, and CVE-2026-12707.

### API contract

- The [API reference](docs/reference/leyline-http/index.md) lists every
  public item, generated from the compiler. Within `0.1.x`, the list changes
  only by addition.
- Each public type has one path. `Browser`, `BrowserProfile`, `Platform`,
  `TlsTrustConfig`, and the other session types live at the crate root.
- Every URL the API returns is a `url::Url`, re-exported as `leyline::Url`:
  `Response::url`, `Response::redirect_chain`, `Error::url`, and
  `RedirectAttempt::url`.
- Every public type implements `Debug`.
- The BoringSSL `SslContextBuilder` behind `TlsContext` is outside semver. It
  needs `RUSTFLAGS="--cfg leyline_unstable_bssl"` or the `bench-internals`
  feature. Streaming needs no feature.

### Added

- An asynchronous `Session` API for HTTP/1.1, HTTP/2, and HTTP/3 on Tokio.
- Browser profiles for TLS, HTTP/2, HTTP/3, and request headers: Chrome 145
  to 154, Brave 146 and 154, Firefox 148 to 156, Safari 18, 26, and 27, Safari
  on iOS 17, 18, and 27, OkHttp on Android, and CFNetwork on iOS 18, iOS 27,
  and macOS 26, with Edge and Opera brand overlays. Values come from captures
  of signed vendor builds; the [profile reference](docs/guide/profiles.md)
  names the capture behind each.
- Header shapes per request kind (`profiles/headers.toml`): navigate, script,
  XHR, form, form navigation, same-site, and cross-origin. Redirects set
  `sec-fetch-site`, `Origin`, and `Referer` by the Fetch rules. `FetchSite`
  computes the `sec-fetch-site` value.
- HTTP/3 fingerprints: a QUIC ClientHello per profile, transport parameter
  plans with GREASE and order policy, connection ID lengths, HTTP/3 SETTINGS
  plans, the first Initial datagram size, and QUIC v2 with compatible version
  negotiation (RFC 9368, RFC 9369). The QPACK decoder supports the dynamic
  table.
- HTTP/3 stream resets carry the codes that Chrome sends:
  `H3_REQUEST_CANCELLED` for a cancelled request and
  `H3_GENERAL_PROTOCOL_ERROR` for a malformed response. An idle connection
  closes without a CONNECTION_CLOSE frame.
- An HTTP/3 request past the server's stream limit waits for stream credit
  instead of failing. When a response ends or is reset while the request body
  still uploads, Leyline resets the upload with `H3_REQUEST_CANCELLED`, so the
  server returns the stream. A request the server rejects after its GOAWAY is
  resent on a new connection, and a streamed response reset before its end
  fails the body stream instead of ending it early.
- `TlsProfile::key_shares`, a GREASE signature algorithm switch, and
  per-platform TCP profiles captured from each OS's own SYN.
- Cookies, proxy configuration, redirect policies, opt-in retries, streaming
  request and response bodies, and response decompression.
- `RetryPolicy::max_retry_after` caps the wait that a `Retry-After`
  header can request. A longer wait stops the retry and returns the
  response. By default there is no cap.
- `Response::read_until` decodes a compressed body and stops when a predicate
  holds or a byte limit is reached.
- `Session::preconnect` opens and pools an HTTP/2 connection before the
  first request, through the session proxy config.
- The pool sends an HTTP/2 PING before it reuses a connection idle for 10
  seconds. It replaces the connection if the PING is not acknowledged within
  2 seconds. `PoolConfig::h2_ping_after_idle` and
  `PoolConfig::h2_ping_timeout` change the thresholds.
- WebSocket connections through the session. `WebSocketBuilder::header`
  takes the same arguments as `RequestBuilder::header` and appends.
  `WsConnection::header` reads the handshake response headers. A `wss://`
  URL uses the `https` proxy rule, and a `ws://` URL uses the `http` rule.
  As in Chrome, the handshake fails when the response's
  `Sec-WebSocket-Protocol` names a subprotocol the client did not offer,
  appears twice, or is missing after the client offered one.
- Lifecycle tracing and a Tower `Service`, `LeylineService`, that wraps a
  session (feature `tower`).
- Opt-in fingerprint diagnostics derived from the configured profile and
  request (`SessionBuilder::audit`).
- `SocketConfig::tcp_user_timeout` applies on Linux and Android. On other
  systems, Leyline logs one warning per unsupported option per process.
- A [user guide](docs/README.md) and an [API map](docs/api.md).
- `[meta] capture` in each profile records its provenance: `browser`,
  `native` for an OS HTTP stack such as CFNetwork, `headless-shell`,
  `webview`, `emulator`, `inferred`, or `self-referential`. Every Chrome,
  Brave, and Firefox profile is a browser capture. `Browser::latest` and
  `Session::new()` select the newest captured Chrome, Chrome 154 in this
  release.
- `TimeoutConfig::connect` bounds plain `http://` connects. One request spends at
  most one connect timeout on an unreachable host.
- `TimeoutConfig::connect` also bounds the HTTP/3 handshake. A QUIC handshake
  that runs out of time is a `Kind::Connect` error, and `is_timeout()` is true.
- A proxy that answers CONNECT with 502, 503, or 504 fails with a retryable
  error. A retry policy with `RetryTrigger::ConnectionError`, such as
  `RetryPolicy::transient()`, tries again.
- `Session::with_proxy` keeps the shared connection pool, which is keyed by
  proxy. `Session::fresh_pool` takes a new pool, so the next request opens
  new connections.
- `Error::kind` returns a typed `Kind`. DNS, TCP, and proxy failures report
  `Kind::Connect` and `Kind::Proxy`, not `Kind::Tls`. `Error::is_retryable`
  holds the one retry rule that the retry policy also uses.
- When a pooled connection fails before the response, Leyline resends a
  buffered request on a new connection if the method is idempotent or the
  server did not process it (HTTP/2 `REFUSED_STREAM`, or an HTTP/3 request
  that was not sent). A streaming request body cannot be resent, so that
  request fails with `Kind::Body`.
- When the caller's request body stream returns an error, the request fails
  with `Kind::Body` and that error as its source, on HTTP/1.1, HTTP/2, and
  HTTP/3. The pool does not count the connection as dead, and a pooled
  HTTP/2 connection stays open for other requests.
- Request functions take any `IntoUrl` input. A URL that does not parse
  returns `Kind::Url` with its source error.
- `Response::text`, `bytes`, and `json` consume the response and return the
  body. `Response::headers` returns the `HeaderMap`, and
  `Response::error_for_status_ref` checks the status without a move.
- Configuration types use private fields with setters named after the field.
  A per-request timeout overrides the session timeouts one field at a time.
- `Jar::snapshot` copies a cookie jar, `Jar::extend_from` merges one jar into
  another, and `Jar::remove` deletes a cookie.
- Cookie jar methods take a parsed `&Url`, like the reqwest cookie store,
  and do not return a URL parse error.
- `Session::with_redirect` derives a session with another redirect policy and
  the same pool and cookies.
- `Session::with_identity` derives a session that presents another identity.
  The TLS profile, `User-Agent`, client hints, header order, and HTTP/2 and
  HTTP/3 settings switch together, on a new connection pool. An `Http3` or
  `Race` session returns `Kind::Config` for an identity with no HTTP/3
  profile.
- `Session::identity` returns the browser, platform, brand, and user agent
  that the session sends. A `user-agent` header set on the builder is that
  user agent.
- The `trace::Head` event carries the response headers.
- Repeated `Content-Encoding` fields combine in order, as one
  comma-separated list does, and every coding is decoded. A `deflate` body
  is read as zlib when its first two bytes are a valid zlib header for any
  window size, and as raw DEFLATE otherwise.
- `Alt-Svc` keeps HTTP/3 support for an origin until the longest `ma` of its
  `h3` entries, less the response's `Age`, ends on the wall clock (24 hours
  by default). An entry whose `ma` is not a number is ignored. `ma=0` on every
  matching entry, `clear` in any `Alt-Svc` field, or a new value with no `h3`
  alternative for the same authority removes it. Each field is parsed on
  its own, and the pool remembers at most 1024 origins.
- A cookie jar drops a domain when its last cookie is evicted or deleted, so
  a long session over many hosts keeps a bounded domain map.

### Security

- Release builds ignore `SSLKEYLOGFILE` and contain no code that reads it.
  Debug builds keep TLS key logging for development.
- `CompressionConfig::max_body_size` caps a buffered body on HTTP/1.1,
  HTTP/2, and HTTP/3, and the output of every decoder stage. A body over the
  cap fails with `Kind::Body` and is not retried. A `.stream()` body has no
  cap. A truncated compressed body is an error.
- `TlsTrustConfig::min_tls_version` sets a TLS version floor for every TCP
  handshake, including the one with an `https://` proxy. The handshake
  minimum is the higher of the profile's minimum and the floor.
- QPACK header sections are bounded by a local limit that does not depend on
  the peer's SETTINGS, and HTTP/2 PING, WINDOW_UPDATE, and empty CONTINUATION
  floods close the connection.
- An invalid proxy URL in `HTTPS_PROXY`, `HTTP_PROXY`, or `ALL_PROXY` never
  falls back to a direct connection. `SessionBuilder::build` returns a
  `Kind::Config` error that names the variable, and a `Session::new` session
  fails each request with `Kind::Proxy`.
- A plain-HTTP origin cannot shadow a Secure cookie (RFC 6265bis), and
  `ProxyUrl` and error messages redact credentials and query strings.
- `Debug` output never prints a password, token, cookie value, or query
  value. `Response`, `DigestAuth`, `Cookie`, `RedirectAttempt`, the request
  and WebSocket builders, the trace events, and the
  `leyline::trace` log mask them, including a proxy URL written without a
  scheme.
  `Session::proxy_url` returns a `ProxyUrl`, whose `Display` hides the
  password.
- Certificate verification through pins, custom roots on macOS, and HTTP/3
  to an IP address checks that the leaf certificate is issued for TLS
  servers, as the default verifier does.
- A redirect to another origin drops a caller-set `Host` header, as it drops
  `Authorization`, `Proxy-Authorization`, and `Cookie`.
- The HPACK decoder rejects a dynamic table size update that follows a
  header field, or a third update in one header block (RFC 7541 section 4.2),
  as Chrome's decoder does.
- A cookie lives at most 400 days (RFC 6265bis). A longer `Max-Age` or
  `Expires`, an `Expires` date past the platform clock's range, and an expiry
  loaded through serde are cut to 400 days.
- A `TimeoutConfig::total` of `Duration::MAX` sets no deadline, and a
  `Retry-After` date past the platform clock's range is ignored. Neither
  panics.
- `digest_auth` answers a Digest challenge on the request that received it,
  with that request's method and URL, and prefers a challenge it can answer.
  A later same-origin redirect step carries a fresh `Authorization` for its
  own method and URL. A `POST` redirected by a 303 is not sent again, and a
  challenge from an origin that a redirect reached is not answered.
- Cookie `SameSite` rules use the `sec-fetch-site` value the request sends,
  which compares schemes. A cross-site request withholds `Strict` cookies and
  sends `Lax` cookies only on a top-level navigation
  (`sec-fetch-dest: document`) with a safe method. Firefox profiles also
  treat a request as cross-site when its redirect chain crosses sites, as
  Firefox does; Chrome judges the final target only.
- A session `Referer` that is an absolute URL counts as the initiator when
  the request sets no `Referer`. The `Origin` header names the initiator's
  origin.
