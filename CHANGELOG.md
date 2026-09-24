# Changelog

The minimum supported Rust version is 1.96, two releases behind stable at
release time.

Within the `0.1.x` series, updates keep API compatibility. A breaking API
change or a higher minimum Rust version needs a new minor release, such as
`0.2.0`. The BoringSSL crates `leyline-bssl`, `leyline-bssl-sys`, and
`leyline-bssl-tokio` share this version and publish as separate crates.

## Unreleased

## 0.1.0 - 2026-09-24

First public release.

### Build

- `leyline-bssl-sys` builds BoringSSL from source with CMake and links it
  statically. The crates ship source, not prebuilt libraries. A build needs
  CMake 3.22 or later, a C and C++ compiler, libclang, and `git`; on Windows,
  also the MSVC build tools and NASM. The BoringSSL crates are trimmed forks
  of Cloudflare's `boring` v5.2.0.

### API contract

- [docs/api.md](docs/api.md) lists every public type and function. The crate
  root exposes only the items on that page. Within `0.1.x`, the page changes
  only by addition.

### Added

- An asynchronous `Session` API for HTTP/1.1, HTTP/2, and HTTP/3 on Tokio.
- Browser profiles for TLS, HTTP/2 settings, and request headers: Chrome 145
  to 153, Brave 146, Firefox 148 to 154, Safari 18 and 26, Safari on iOS 17
  and 18, OkHttp on Android, and CFNetwork on iOS 18 and macOS 26. The
  [profile reference](docs/guide/profiles.md) lists the capture status of
  each.
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
  `webview`, `inferred`, or `self-referential`. `Browser::latest`
  and `Session::new()` select the newest profile with `capture = "browser"`,
  so the default is Chrome 153, not the `chrome-headless-shell` captures of
  Chrome 151 and 152.
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
