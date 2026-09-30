# Changelog

The minimum supported Rust version is 1.96, two releases behind stable at
release time.

Within the `0.1.x` series, updates keep API compatibility. A breaking API
change or a higher minimum Rust version needs a new minor release, such as
`0.2.0`. The BoringSSL crates `leyline-bssl`, `leyline-bssl-sys`, and
`leyline-bssl-tokio` share this version and publish as separate crates.

## Unreleased

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
  by default). `ma=0`, `clear` in any `Alt-Svc` field, or a new value with no
  `h3` alternative for the same authority removes it. Each field is parsed on
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
