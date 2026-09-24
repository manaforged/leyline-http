# Leyline API map

The supported surface below is stable within semver.
The modules `leyline::h2`, `leyline::pool`, and `leyline::fuzz` exist only
with the `bench-internals` feature, for leyline's own tests, benches, and
fuzz targets. `leyline::tls::FingerprintConnector` is a `#[doc(hidden)]`
internal. None of these are part of the contract; they change without
notice. Do not call them.

Crate: `leyline-http` on crates.io. Import name: `leyline`.

## Core types

| Job | Symbol |
|---|---|
| Client | `leyline::Session` |
| Configure | `leyline::SessionBuilder` (via `Session::builder()`) |
| Request | `leyline::RequestBuilder` (via `session.get(url)` etc.), `leyline::Request` for `Session::execute` (`method`/`url`/`headers` getters; `retry`, `stream`, `timeout`, `timeouts` setters match `RequestBuilder`) |
| Response | `leyline::Response`, `leyline::BodyStream`, `leyline::ResponseTiming`, `leyline::HttpVersion` |
| Error | `leyline::Error`, `leyline::Kind`, `leyline::Result<T>` |

## Quick paths

A single request:

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::chrome();
let mut resp = session.get("https://example.com/").await?;
let body = resp.text().await?;
# Ok(())
# }
```

A configured session:

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::builder()
    .firefox()
    .macos()
    .timeout(std::time::Duration::from_secs(15))
    .proxy("http://user:pass@host:8080")
    .build()?;
# Ok(())
# }
```

`RequestBuilder` implements `IntoFuture`; `.await` and `.send()` are the same
call. Builder methods never fail eagerly; invalid input is deferred to
`.build()` or `.send()` as a `Kind::Builder` or `Kind::Request` error.

## Jobs to symbols

Unqualified names are re-exported at `leyline::`. Module-qualified names
(`profile::`, `tls::`, `cookie::`, `audit::`, `trace::`, `multipart::`) are
not at the root.

| Job | Symbols |
|---|---|
| Pick a browser | `Session::chrome`/`firefox`/`safari`/`edge`/`brave`/`opera`/`vivaldi`, `Session::profile(Browser, Platform)`, `SessionBuilder::browser`, `Browser`, `Browser::latest(Family)`, `Preset`, `Browser::all`, `profile::Family` |
| Pick a platform | `SessionBuilder::platform`/`windows`/`macos`/`linux`/`android`/`ios`, `Platform::Host` |
| Brand overlay | `SessionBuilder::brand`, `ChromiumBrand`, `profile::BrandOverlayError` |
| Composed identity | `Identity::locked`, `Identity::rotate_tls`, `Identity::rotate_hello`, `Identity::pass`, `SessionBuilder::identity`, `SessionBuilder::http_identity` |
| HTTP methods | `Session::get`/`post`/`put`/`patch`/`delete`/`head`, `Session::request(Method, url)`, `Session::execute(Request)` |
| Headers | `RequestBuilder::header`/`append_header`/`headers`/`append_headers`, `accept`/`accept_language`/`user_agent`/`referer`/`origin`/`content_type`, `anchored(profile::HeaderAnchor, ..)`, `header_order`, `HeaderList` |
| Body out | `RequestBuilder::body`/`json`/`form`/`form_str`/`multipart`, `Body::stream`/`stream_with_length`, `Body` as a `Stream`, `multipart::Form`, `multipart::Part` |
| Query | `RequestBuilder::query`, `IntoParamPair` |
| Session defaults | `SessionBuilder::accept_language`/`extra_headers`/`https_only`, `Session::default_timeout`/`response_header_timeout` |
| Auth | `RequestBuilder::basic_auth`/`bearer_auth`/`digest_auth`, `DigestAuth` |
| Read the response | `Response::status`/`headers`/`header`/`header_all`/`cookies`/`cookie`, `text`/`text_utf8`/`text_with_charset`/`bytes`/`json`/`into_text`/`into_bytes`, `as_bytes`/`as_text`, `content_length`, `header_map`, `is_success`/`is_client_error`/`is_server_error`, `error_for_status` |
| Inspect the wire | `Response::request_headers`, `tls_peer_certificate`, `tls_cipher`, `tls_version`, `tls_alpn`, `trailers`, `redirect_chain`, `url`, `timing` |
| Streaming | `RequestBuilder::stream`, `Response::into_stream` produces `BodyStream` (a `futures_util::Stream`), `copy_to`, `download_to`, `read_until` |
| Retries and timeouts | `RetryPolicy::none`/`transient`/`with_max_retry_after`, `RetryTrigger`, `TimeoutConfig`, `SessionBuilder::retry`/`timeout`/`timeouts`/`connect_timeout`, per-request `RequestBuilder::retry`/`timeout`/`timeouts`, `allow_non_idempotent_retry` |
| Redirects | `SessionBuilder::max_redirects`/`redirect_policy`, `RedirectPolicy::limited`/`none`/`custom`, `RedirectAttempt`, `RedirectAction`, `Session::with_redirect_policy`, `Response::redirect_chain` |
| Proxies | `SessionBuilder::proxy`/`proxies`/`no_proxy`/`disable_env_proxies`, `ProxyConfig`, `ProxyRule`, `ProxyUrl`, `NoProxy`, `Session::with_proxy`, per-request `RequestBuilder::proxy` |
| DNS | `SessionBuilder::resolver`/`dns`/`resolve_host`/`resolve_host_to_addrs`, `DnsConfig`, `tls::Resolver`, `tls::SystemResolver` |
| TLS trust | `SessionBuilder::tls_trust`/`add_root_certificate_file`/`add_root_certificate_der`/`add_pinned_leaf_sha256`/`without_env_roots`/`without_system_roots`/`client_identity_files`/`danger_accept_invalid_certs`, `TlsTrustConfig`, `tls::ClientIdentity`, `TlsMinVersion`, `TlsContext`, `TlsError` |
| Protocol | `SessionBuilder::http1`/`http2`/`http3`/`race`/`protocol_policy`, `ProtocolPolicy`, `H3Config` (feature `http3`, on by default) |
| WebSocket | `Session::websocket` gives `WebSocketBuilder`; `connect()` gives `WsConnection`; `split()` gives `WsSink` and `WsStream`; `WsMessage`, `CloseFrame`, `WebSocketConfig` through `SessionBuilder::websocket_config` (feature `websocket`, on by default) |
| Cookies | `Session::cookies`, `Session::with_cookie_jar`, `SessionBuilder::cookie_jar`, `cookie::Jar`, `cookie::Cookie`, `cookie::SameSite` |
| Fingerprints | `SessionBuilder::audit(true)` then `Response::audit` gives `audit::AuditData`; `audit::compute_ja3`/`compute_ja4`/`compute_ja4h`/`compute_ja4t` with `Ja3Input`/`Ja4Input`/`Ja4hInput` for standalone computation; `Browser::profile` gives `BrowserProfile` |
| Tracing and timing | `SessionBuilder::trace(impl trace::Trace)`, events `trace::Dns`/`Connect`/`Tls`/`Sent`/`Head`/`Done`, `trace::TracingTrace`, `trace::Timing`, `Response::timing` gives `ResponseTiming` |
| Pool observability | `Session::pool_stats` gives `PoolStats`, `Session::preconnect`/`preconnect_via`, `SessionBuilder::pool_config` gives `PoolConfig` |
| Transport knobs | `SessionBuilder::tcp_profile` takes `TcpProfile`, `socket_config` takes `SocketConfig` (`tcp_user_timeout` applies on Linux and Android; `interface` is accepted but not applied), `happy_eyeballs` takes `tls::HappyEyeballsConfig`, `compression` takes `CompressionConfig` |
| Request compression | `CompressionConfig`, `RequestBuilder::compress(ContentEncoding)` |
| Tower | `SessionBuilder::layer`, `LeylineService`, `layer::Call`/`Log`/`Logged`/`Pending`/`Reply`/`Transport` (feature `tower`, off by default) |
| HTTP/2 errors | `H2Error`, `ErrorCode` (root re-exports; the rest of `leyline::h2` is internal) |
| http types | `leyline::http` re-exports the `http` crate: `Method`, `Uri`, `HeaderName`, `HeaderValue`, `StatusCode` |

## Errors

One error type: `leyline::Error`. Read `err.kind()` for the `Kind`, or the
predicates `is_timeout`/`is_connect`/`is_status`/`is_redirect`/`is_body`/
`is_decode`/`is_connection_closed`. Downcast the source with `err.tls()` for
`TlsError`, `err.h2()` for `H2Error`, `err.io()` for `std::io::Error`.
`Kind::as_str()` gives a stable lowercase label.

| Kind | Meaning |
|---|---|
| `Builder` | Configuration rejected at `build()` or `send()` |
| `Request` | HTTP-level failure (bad header, malformed request) |
| `Redirect` | Redirect policy stopped the chain |
| `Status` | `error_for_status` rejected the status |
| `Body` | Body read or write failed |
| `Decode` | Decompression or charset decode failed |
| `Timeout` | A timeout expired |
| `Connect` | TCP or TLS connect failed |
| `Tls` | TLS handshake or verification; `err.tls()` for detail |
| `Http2` | HTTP/2 protocol failure; `err.h2()` for `H2Error` |
| `Http3` | HTTP/3 protocol failure |
| `Proxy` | Proxy connect or tunnel failed |
| `Io` | Socket I/O; `err.io()` for `std::io::Error` |
| `Config` | Invalid configuration |
| `Url` | URL parse failure |
| `Json` | JSON serialize or deserialize failure |

## Conventions

- `Session::new()` and `Session::default()` build a bare session with no
  impersonation. Every named constructor (`chrome()`, `firefox()`, and the
  rest) impersonates.
- `Session::with_*` derives a clone that shares the pool. Pool entries are
  keyed by proxy, so sessions on different proxies never share a connection.
  `with_proxy` with the session's current proxy URL takes a fresh pool, so a
  next request opens new connections.
- `Response` body readers (`text`, `bytes`, `json`) buffer on first call.
  `into_stream`, `copy_to`, `download_to`, and `read_until` consume the
  response. `into_stream` returns `Err` with `Kind::Body` if the body was
  already taken.
- Public enums and config structs are `#[non_exhaustive]`: match with a `_`
  arm and construct through builders or `Default`, not literals.
- `SessionBuilder::socket_config`, `tls_trust`, `dns`, and `pool_config`
  replace the whole value. Start from `::new()`, which carries the defaults.
- `Response::read_until(limit, done)` calls `done(body, from)` after each
  decoded chunk: `body` is every decoded byte so far, `from` is where the
  newest chunk starts. It stops on `true`, at `limit` decoded bytes, or at
  end of stream, and returns the decoded prefix.
- `Response::audit` is `None` unless the session was built with
  `audit(true)`.
- Default features: `cookies`, `charset`, `compression-gzip`,
  `compression-brotli`, `compression-deflate`, `compression-zstd`,
  `multipart`, `stream`, `websocket`, `http3`, `system-trust`. Opt-in
  features: `socks`, `tower`, `native-interface-bind`, `unstable-bssl`
  (exposes BoringSSL types on `TlsContext`), `bench-internals` (HTTP/2, pool, and fuzz
  internals for tests, benches, and fuzz targets; outside semver). `full` enables every opt-in except `bench-internals` and
  `unstable-bssl`.
- The supported public surface is `leyline-http`. The BoringSSL crates it
  depends on are outside the semver promise.

## Profile schema

`BrowserProfile`, `profile::TlsProfile`, `profile::H2Profile`,
`profile::PlatformIdentity`, and the other `leyline::profile` types mirror
the bundled `profiles/<family>/<version>.toml` files; field names are the
TOML keys. Load custom profiles with `profile::ProfileRegistry::load(dir)`
or `BrowserProfile::from_toml`.
