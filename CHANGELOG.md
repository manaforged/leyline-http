# Changelog

All notable changes to Leyline. Format loosely follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and
[SemVer](https://semver.org/spec/v2.0.0.html).

Leyline is pre-1.0. Breaking changes will happen between minor releases.
Pin exact versions.

## Unreleased

### Added

- `leyline-h2` now supports concurrent stream multiplexing. A new
  `H2Client` handle is cloneable and multiple concurrent
  `send_request` calls on the same handle run over independent H2
  streams on one TCP connection. A stream parked on flow control
  does not block siblings.
- Streaming request and response bodies: `Body::stream(...)` and
  `RequestBuilder::stream()` → `Response::into_stream()`. Large
  uploads and downloads no longer buffer into memory.
- WebSocket over HTTP/2 via RFC 8441 extended CONNECT
  (`H2ConnectStream`, `Session::websocket` auto-negotiates H2/H1).
- RFC 9113 §5.1 stream state machine, RFC 9113 §6.5.3 SETTINGS ACK
  timeout, RFC 9113 §5.1.1 stream-id parity enforcement.
- CVE-2023-44487-shaped inbound RST_STREAM flood guard
  (configurable threshold / window in `H2Config`).
- Idempotent retry with exponential backoff and jitter
  (`RetryPolicy`, `RequestBuilder::retry`).
- `multipart/form-data` request bodies (`leyline::multipart::{Form,
  Part}`) — text parts, bytes parts, file parts, streaming parts.
- Digest authentication (RFC 7616) — MD5, SHA-256, SHA-512-256,
  `-sess` variants, qop=auth — via `RequestBuilder::digest_auth`.
- `leyline-tower` crate: `tower::Service` adapter so Leyline drops
  into axum / tower middleware stacks.
- Happy Eyeballs (RFC 8305 §5) + pluggable `Resolver` trait for
  custom DNS.
- Connection pool grows an LRU cap
  (`Pool::with_limits(idle_timeout, max_conns)`, default 256).
- `CookieJar::clear` / `is_empty` / `len` for identity-switching
  without rebuilding the Session or pool.
- `cargo-fuzz` targets run and committed corpora (1150 entries
  across four targets); `./scripts/verify.sh` runs every release
  release gate.
- Measured perf numbers in [BENCHMARKS.md](BENCHMARKS.md).

### Fixed

- Integer overflow in the cookie `Expires` parser for pre-1970 dates
  Found by the `cookie_set`
  fuzz target.
- Response body cap changed from a hard-coded 100 MiB connection
  error to a configurable stream-level error
  (`H2Config::max_response_body_bytes`). Exceeding the cap kills
  the offending stream, not the whole connection.
- Hard-coded 64 KiB CONTINUATION reassembly cap replaced with
  `H2Config::max_header_block_bytes` (default 256 KiB).

### Changed

- Vendored `boring` / `boring-sys` / `tokio-boring` crates now set
  `publish = false`, so the release recipe cannot accidentally
  attempt to publish them over Cloudflare's real `boring`.
- Workspace `missing_docs` lint promoted from `warn` to `deny`.
- Hand-rolled base64 in `crates/core/src/request.rs` replaced with
  the `base64` crate's STANDARD engine.
- `deny.toml` swaps the bare `OpenSSL` SPDX allow for scoped
  `[[licenses.clarify]]` entries on the vendored crates only.

## 2.0.0-alpha.1 — Initial public release

First public preview of the Leyline HTTP client. The `2.0`
version marks the re-architecture around a vendored BoringSSL fork and a
purpose-built HTTP/2 crate.

### Added

- Browser profiles for Chrome 145/146/147, Firefox 148, Safari macOS 18,
  Safari iOS 15/17/18, OkHttp Android 7/10.
- `Session` / `Request` / `Response` API shaped like `reqwest`, async on
  tokio.
- Per-response audit block: JA3, JA4, JA4T, JA4H, H2 Akamai fingerprint.
- HTTP/1.1, HTTP/2, and HTTP/3 (over `quiche`) transports with automatic
  ALPN-driven selection and explicit override.
- HTTP and SOCKS5 proxy support with username/password auth.
- Cookie jar (RFC 6265) and connection pool with H2 multiplexing and
  TLS 1.3 session resumption.
- `leyline` CLI (httpie-shaped) with `get` / `post` / `inspect` /
  `profile` / `completions` subcommands.
- C FFI and Python / Node / Go wrappers (local build required; see each
  wrapper's README).
- `cargo deny` supply-chain gate with TLS-backend ban list.
- Claim guard test that scans README and public crate docs for
  unsupported marketing language.
- Live and offline evidence matrix documented in `TESTING.md`.

### Known limitations

- The `leyline-h2` crate is browser-shaped, not a full RFC 9113
  implementation. See `TESTING.md` for the specific surface covered.
- Browser fingerprints drift between releases. Profiles track the
  versions we captured; re-verify before production use.
- Pre-compiled FFI artifacts are not published; wrappers build the
  dynamic library locally.
