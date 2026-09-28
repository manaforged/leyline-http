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
- `leyline-quiche` is Cloudflare's `quiche` 0.30.0 linked against
  `leyline-bssl`. It includes the upstream fixes for CVE-2025-4820,
  CVE-2025-4821, CVE-2025-7054, CVE-2026-12523, and CVE-2026-12707.

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
- The BoringSSL `SslContextBuilder` behind `TlsContext` is outside semver and
  needs `RUSTFLAGS="--cfg leyline_unstable_bssl"`, so a dependency cannot
  turn it on for you. Streaming needs no feature.

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
- WebSocket connections through the session. `WsConnection::header` reads the
  handshake response headers.
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
  `Session::new()` select the newest captured Chrome, currently Chrome 154.
- `TimeoutConfig::connect` bounds plain `http://` connects. One request spends at
  most one connect timeout on an unreachable host.
- `Session::with_proxy` keeps the shared connection pool, which is keyed by
  proxy. `Session::fresh_pool` takes a new pool, so the next request opens
  new connections.
- `Error::kind` returns a typed `Kind`. DNS, TCP, and proxy failures report
  `Kind::Connect` and `Kind::Proxy`, not `Kind::Tls`. `Error::is_retryable`
  holds the one retry rule that the retry policy also uses.
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
- `Session::identity` returns the browser, platform, brand, and user agent
  that the session sends.
- The `trace::Head` event carries the response headers.

### Security

- Release builds ignore `SSLKEYLOGFILE` and contain no code that reads it.
  Debug builds keep TLS key logging for development.
- Decompression is bounded by `CompressionConfig::max_body_size` at every
  decoder stage, and a truncated compressed body is an error.
- QPACK header sections are bounded by a local limit that does not depend on
  the peer's SETTINGS, and HTTP/2 PING, WINDOW_UPDATE, and empty CONTINUATION
  floods close the connection.
- A plain-HTTP origin cannot shadow a Secure cookie (RFC 6265bis), and
  `ProxyUrl` and error messages redact credentials and query strings.
- `Debug` output never prints a password, token, cookie value, or query
  string. `Response`, `DigestAuth`, `Cookie`, `HeaderList`, the builders, the
  trace events, and the `leyline::trace` log mask them.
  `Session::proxy_url` returns a `ProxyUrl`, whose `Display` hides the
  password.
- Certificate verification through pins, custom roots on macOS, and HTTP/3
  to an IP address checks that the leaf certificate is issued for TLS
  servers, as the default verifier does.
