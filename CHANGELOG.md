# Changelog

The first public release of `leyline-http` is **0.1.0**.

Within the `0.1.x` series, updates preserve API compatibility. Breaking API
changes or a higher minimum Rust version require a new minor release, such as
`0.2.0`. The minimum supported Rust version is 1.96, two releases behind
stable. The BoringSSL implementation crates share this version and publish as
separate crates.

## 0.1.0 - 2026-09-17

### Added

- An asynchronous `Session` API for HTTP/1.1, HTTP/2, and HTTP/3 on Tokio.
- Browser profiles for TLS, HTTP/2 settings, and request headers. See the
  [profile reference](docs/guide/profiles.md) for bundled versions and
  capture status.
- Cookies, proxy configuration, redirect policies, opt-in retries, streaming
  request and response bodies, and response decompression.
- WebSocket connections through the session, lifecycle tracing, and optional
  Tower integration.
- Opt-in fingerprint diagnostics derived from the configured profile and
  request.
- Prebuilt BoringSSL libraries and Rust bindings for macOS arm64, Linux x86_64
  and arm64 with glibc, and Windows x86_64 with MSVC.
- `Session::preconnect` opens and pools an HTTP/2 connection before the first
  request.
- The pool sends an HTTP/2 PING before reusing a connection idle for 10 seconds
  and replaces the connection if the PING is not acknowledged within 2 seconds.
  `PoolConfig::h2_ping_after_idle` and `PoolConfig::h2_ping_timeout` change
  the thresholds.
- A [user guide](docs/guide/README.md) covering requests,
  responses, sessions, and supported targets.

### Changed

- The minimum supported Rust version is 1.96, two releases behind the
  current stable at release time.
