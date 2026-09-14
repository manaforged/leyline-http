# Changelog

The first public release of `leyline-http` will be **0.1.0**. No public
release has shipped yet.

Within the `0.1.x` series, updates preserve API compatibility. Breaking API
changes or a higher minimum Rust version require a new minor release, such as
`0.2.0`. The BoringSSL implementation crates are versioned separately.

## Unreleased

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
- A [user guide](crates/leyline/docs/guide/README.md) covering requests,
  responses, sessions, and supported targets.
