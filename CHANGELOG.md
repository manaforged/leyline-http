# Changelog

The first public release line of `leyline-http` is **0.1.0**. Versions named
`0.1.0-alpha.N` are previews of it and may change the API between alphas.

Once `0.1.0` ships, updates within the `0.1.x` series preserve API
compatibility. Breaking API changes or a higher minimum Rust version require
a new minor release, such as `0.2.0`. The BoringSSL implementation crates are
versioned separately.

## 0.1.0-alpha.1 - 2026-09-17

### Changed

- `Session::with_proxy` returns `Result<Session>` and rejects an invalid proxy
  URL or an unsupported scheme when it is called, matching `SessionBuilder::build`.
- The README and benchmark summary describe the client-bound cells only.

### Added

- An asynchronous `Session` API for HTTP/1.1, HTTP/2, and HTTP/3 on Tokio.
- Browser profiles for TLS, HTTP/2 settings, and request headers. See the
  [profile reference](crates/leyline/docs/PROFILES.md) for bundled versions and
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
- A [user guide](crates/leyline/docs/guide/README.md) covering requests,
  responses, sessions, and supported targets.

### Changed

- The minimum supported Rust version is 1.98, the current stable at
  release time.
