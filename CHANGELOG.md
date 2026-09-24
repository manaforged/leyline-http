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

### Added

- An asynchronous `Session` API for HTTP/1.1, HTTP/2, and HTTP/3 on Tokio.
- Browser profiles for TLS, HTTP/2 settings, and request headers: Chrome 145
  to 152, Brave 146, Firefox 148 to 154, Safari 18 and 26, Safari on iOS 17
  and 18, OkHttp on Android, and CFNetwork on iOS 18 and macOS 26. The
  [profile reference](docs/guide/profiles.md) lists the capture status of
  each.
- Cookies, proxy configuration, redirect policies, opt-in retries, streaming
  request and response bodies, and response decompression.
- `RetryPolicy::with_max_retry_after` caps the wait that a `Retry-After`
  header can request. A longer wait stops the retry and returns the
  response. By default there is no cap.
- `Response::read_until` decodes a compressed body and stops when a predicate
  holds or a byte limit is reached.
- `Session::preconnect` and `Session::preconnect_via` open and pool an
  HTTP/2 connection before the first request, directly or through a given
  proxy.
- The pool sends an HTTP/2 PING before it reuses a connection idle for 10
  seconds. It replaces the connection if the PING is not acknowledged within
  2 seconds. `PoolConfig::h2_ping_after_idle` and
  `PoolConfig::h2_ping_timeout` change the thresholds.
- WebSocket connections through the session. `WsConnection::header` reads the
  handshake response headers.
- Lifecycle tracing and optional Tower integration (feature `tower`).
- Opt-in fingerprint diagnostics derived from the configured profile and
  request (`SessionBuilder::audit`).
- `SocketConfig::tcp_user_timeout` applies on Linux and Android. On other
  systems, Leyline logs one warning per unsupported option per process.
- Prebuilt BoringSSL libraries and Rust bindings for macOS arm64, Linux x86_64
  and arm64 with glibc, and Windows x86_64 with MSVC.
- A [user guide](docs/README.md) and an [API map](docs/api.md).
- `connect_timeout` bounds plain `http://` connects. One request spends at
  most one connect timeout on an unreachable host.
- `Session::with_proxy` keeps the shared connection pool, which is keyed by
  proxy. A rebind to the session's current proxy URL takes a new pool, so a
  next request opens new connections.

### Security

- Release builds ignore `SSLKEYLOGFILE` and contain no code that reads it.
  Debug builds keep TLS key logging for development.
