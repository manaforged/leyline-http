# Changelog

All notable changes to Leyline. Format loosely follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and
[SemVer](https://semver.org/spec/v2.0.0.html).

## Unreleased

### Fixed

- `Session::with_proxy` opens a new connection for the rebound proxy. It
  evicts the pool entries, in-flight HTTP/2 connects, HTTP/1 permits, and
  ALPN memory for that proxy key before it rebinds, so the next request opens a new connection
  through the proxy instead of reusing the pooled
  socket.

### Changed

- The HTTP/2 request path does less work per request: a pooled send and its
  retry share one request head instead of cloning the pseudo-headers and the
  header list, the HPACK encoder indexes the static table by name length
  instead of scanning all 61 entries (encoding a Chrome header set is 18%
  faster), and a buffered response body is sized once from `content-length`.
- `Response::bytes`, `text`, `text_utf8`, `text_with_charset`, `into_bytes`,
  `into_text`, and `json` are async and work on a streamed body: they drain it,
  decompress it, and keep the bytes for later calls. Draining honors the
  session `read_timeout` per chunk and the same 100 MiB cap buffered mode
  applies. The `&mut self` calls need `let mut resp`. The "response body is
  streaming or already taken" error is gone.
- `BindingResponse::from_leyline` is async, because it drains the body.
- `BrowserProfile::from_toml` returns `ProfileError` instead of
  `toml::de::Error`, and `ProfileError::Parse` now carries
  `path: Option<PathBuf>` with a boxed source. The parser message stays
  reachable through `Display` and `Error::source`, and `toml` is off the
  public API.
- `From<leyline_bssl::ssl::Error>` and `From<leyline_bssl::error::ErrorStack>`
  for `TlsError` are gone; the conversions are crate-private, so BoringSSL
  types are off the public API.
- The external-type allowlist admits `url::*` and `tokio::io::AsyncWrite`.
  `url` is the shared URL type of the Rust HTTP ecosystem, so `Jar` takes
  `url::Url` rather than a wrapper, and `AsyncWrite` is the sink type of
  `Response::copy_to` in a tokio-only crate.
  `cargo check-external-types` reports no unapproved type.

### Added

- `Preset::Native` keeps the session transport profile and sends native app
  requests without browser Origin, Referer, fetch metadata, or client hints.

- `Response::as_bytes` and `Response::as_text` read an already-buffered body
  with no await; both return `None` while the body is still a stream.
- `Response::error_for_status` does not await, so it attaches a body prefix
  only when the body is already buffered.
- `SessionBuilder::trace` installs a per-request lifecycle listener. The
  `leyline::trace::Trace` trait reports `dns`, `connect`, `tls`, `sent`,
  `head`, and `done`, each event carrying an attempt id, the phase duration,
  and the phase's own fields. `TracingTrace` writes the events to `tracing`;
  `Timing` collects the same numbers as `ResponseTiming`. See
  `examples/trace.rs`.
- `SessionBuilder::layer`, behind the `tower` feature, wraps every request
  attempt in a Tower stack. The session hands the layer a `leyline::layer::Call`
  after it resolved headers, body, and proxy and before it picks a transport,
  and takes back a `leyline::layer::Reply`; each redirect leg is one call, and
  redirects, retries, cookies, and tracing stay in the session. A layer can edit
  request headers or answer without calling the inner service. `layer::Log`
  writes one `tracing` line per call. See `examples/layer.rs`.
- A user guide under `docs/guide/`, twelve chapters whose code blocks run as
  doctests of `leyline-http`.
- Builder setters on every config struct, one method per field:
  `TimeoutConfig::default().total(d).connect(d)`, and the same shape on
  `PoolConfig`, `SocketConfig`, `CompressionConfig`, `WebSocketConfig`,
  `HappyEyeballsConfig`, and `RetryPolicy`. The fields stay `pub` for reading.
- `unstable-bssl`, off by default, exposes `TlsContext::builder_mut` and
  `TlsContext::into_inner`. The BoringSSL types they return carry no semver
  promise.
- `package.metadata.cargo_check_external_types.allowed_external_types` in
  `crates/leyline/Cargo.toml` records which foreign crates the public API may
  speak.

### Changed

- Config structs and public data records are `#[non_exhaustive]`, so a new
  field is no longer a breaking change: `TimeoutConfig`, `PoolConfig`,
  `SocketConfig`, `CompressionConfig`, `WebSocketConfig`, `RetryPolicy`,
  `ResponseTiming`, `Request`, `HappyEyeballsConfig`, `TcpProfile`, and
  `ProxyRule`. Build them from `default()` (or `new()`) plus setters.
- `WsMessage` is a Leyline enum (`Text`, `Binary`, `Ping`, `Pong`,
  `Close(Option<CloseFrame>)`), not a re-export of tungstenite's `Message`.
  `CloseFrame` carries a `u16` code and a `String` reason.
- `RedirectAttempt::url` is an `&http::Uri`, not an `&url::Url`.
- `TcpProfile::apply` is crate-internal; it took a `socket2::Socket`.
- `leyline::fuzz::parse_set_cookie` takes the request URL as a `&str`.

### Changed

- MSRV is 1.88: the crate uses let chains, `is_multiple_of`, and
  `as_chunks`, which 1.86 does not have. Verified with `rustup run 1.88.0`.

### Added

- `RequestBuilder::timeouts` overrides the session `total`, `read`, and
  `response_header` caps for one request. `connect` stays session-wide,
  because connections are pooled and coalesced across requests.
- `docs/MSRV.md` states the MSRV policy: the MSRV is `rust-version` in
  `Cargo.toml`, a bump needs a feature that requires it, and a bump ships as
  its own minor release.
- `SECURITY.md` names the pinned BoringSSL revision and the 14-day window for
  picking up an upstream BoringSSL or quiche security fix.
- `benches/benches/clients.rs` compares Leyline against `reqwest` 0.13 over an
  in-process TLS origin for HTTP/1.1 keep-alive, HTTP/2 multiplexing, and a
  4 MiB streamed download; `BENCHMARKS.md` records the machine, the command,
  and the numbers.

### Changed

- `leyline::Error` is a struct, not an enum. `err.kind()` returns a
  `leyline::Kind` (`Builder`, `Request`, `Redirect`, `Status`, `Body`,
  `Decode`, `Timeout`, `Connect`, `Tls`, `Http2`, `Http3`, `Proxy`, `Io`,
  `Config`, `Url`, `Json`); the wrapped `TlsError`, `H2Error`,
  `std::io::Error`, `url::ParseError`, or `serde_json::Error` is reachable
  through `std::error::Error::source()`. `status()` returns a `StatusCode`,
  `url()` returns the request URI, `body_prefix()` returns the retained
  response body, and `without_url()` drops the URL. `Debug` replaces
  userinfo in the URL with `***`. `is_timeout`, `is_connect`,
  `is_connection_closed`, and `is_status` keep their meaning, and
  `is_redirect`, `is_body`, and `is_decode` join them. Code that matched on
  `Error::Config(..)` or the other variants now reads `err.kind()`.

- Every `TimeoutConfig` field documents what it caps and when it fires.
- The public surface uses the `http` crate's types, as reqwest and hyper do.
  `Response::status` and `Error::status` return `http::StatusCode`.
  `Response::headers` and
  `Response::trailers` yield `(&http::HeaderName, &http::HeaderValue)` in wire
  order, and `Response::header_map` copies them into an `http::HeaderMap`.
  `Request::method` is an `http::Method` and `Request::url` an `http::Uri`;
  `Session::request` takes a `Method` and anything that converts to a `Uri`.
  `RequestBuilder::header` and the other header setters take
  `impl TryInto<HeaderName>` and `impl TryInto<HeaderValue>`, so strings keep
  working and an invalid name or value surfaces from `send`. `HeaderList`
  stores `(HeaderName, HeaderValue)` and keeps insertion order and duplicates.
  The crate re-exports `http`, so callers share one version of these types.
- `LeylineService` also implements `tower::Service<http::Request<Body>>` with
  `Response = http::Response<Body>`; `From<http::Request<Body>> for Request`
  carries method, URI, headers, and body across so both paths share one
  dispatch.
- `ProfileRegistry::load(dir)` reads a `<family>/<version>.toml` profile
  directory and runs the same parse and extension-order validation as the
  compiled-in set, so you can author and validate a profile for a browser
  release the installed crate does not bundle. Failures come back as
  `ProfileError::Io`, `Parse`, or `Empty`.
- `Browser::latest(Family)` returns the highest bundled version of a product
  line, so a caller pins the family instead of a version.
- `docs/PROFILES.md` lists every bundled profile with its `captured_against`
  build and whether its JA4 is gated or reconnaissance, and states the
  capture cadence.

### Changed

- The OkHttp Android 10 profile declares `padding = true`. BoringSSL appends
  RFC 7685 padding to this ClientHello, so the reconstructed JA4 now matches
  the profile's wire golden.

- `flate2`, `brotli`, and `zstd` are optional and follow the
  `compression-*` features, which now also gate the matching RFC 8879
  certificate decompressor. A profile that lists a cert-compression
  algorithm whose feature is off fails at session build and names the
  feature. Default build: 162 crates, was 168; a build with no compression
  features pulls 126.
- The crate builds on one `rand` (0.9) and one `md5` (`md-5`), and the
  library no longer enables the `tokio` `rt-multi-thread` feature.
- HTTP/2 writes one buffer per event-loop turn instead of one per frame.
  Eight concurrent requests now leave the client in a single transport write
  rather than eight, so each turn costs one TLS record and one syscall. The
  frame reader also reads into one persistent buffer and hands out slices of
  it, instead of allocating and zeroing a buffer per frame.
- `Session::chrome()` and the other Chromium constructors race HTTP/3 against
  HTTP/2 only for origins that advertised `h3` in `Alt-Svc` or already
  completed a QUIC handshake. A cold origin gets one TCP handshake, as in
  Chrome. The race prefers a pooled HTTP/3 connection instead of picking at
  random.
- `Response::trailers()` returns the trailers of buffered HTTP/2 and HTTP/3
  responses. It was always empty.
- `Response::audit().ja4` is derived from the profile's fixed extension order
  when it has one, with the correct ALPS codepoints. The offline conformance
  test gates JA4 for every profile with a fixed order. Profiles without one
  are reported, not gated.
- `Debug` output of `Session`, `ProxyRule`, and `ProxyUrl` replaces the proxy
  password with `***`.
- HTTP/1.1 requests reject `Transfer-Encoding` beside `Content-Length` and
  duplicate `Transfer-Encoding`, the same check HTTP/2 and HTTP/3 already ran.
- An HTTP/3 request rejected with `H3_REQUEST_REJECTED` is replayed once;
  a second rejection returns the error.
- A profile with an unknown `min_tls_version` fails at session build instead
  of silently using the default floor.
- `leyline::core` is private; every type it held is at the crate root.
  `leyline::h2` and `leyline::pool` stay hidden and carry no semver promise.
- `From<multipart::Form> for Body`.
- One owner per decision: a request's preset is inferred from `content-type`
  in one place for both `RequestBuilder` and `Session::execute`; retry
  classification uses `Error::is_connect`, `is_connection_closed`, and
  `is_timeout`, so a `PermissionDenied` from a body stream is not retried;
  a per-request `header_order` applies on every protocol and wins over the
  identity order; proxy state lives in `ProxyConfig` only, and every proxy
  URL is validated at `SessionBuilder::build`.
- HTTP/2: a streaming response consumer that reads late no longer gets
  `RST_STREAM(CANCEL)`; the driver queues the chunks and stops crediting the
  stream window until the consumer drains. A peer that ends its side while
  the request body is still open gets `RST_STREAM(NO_ERROR)`. A peer that
  sends on a closed stream gets `RST_STREAM(STREAM_CLOSED)`. `GOAWAY` with
  `NO_ERROR` fails the streams above `last_stream_id` with `REFUSED_STREAM`,
  which `RetryPolicy::transient` retries.
- `ProtocolPolicy::Auto` sends a streaming request body straight to HTTP/2
  instead of buffering it for a fallback. When an origin negotiates
  `http/1.1`, the pool remembers it per host, port, and proxy, and later
  requests dial HTTP/1.1 directly; an ALPN mismatch is not retried.
- `SessionBuilder` has one method per config struct plus a few shortcuts.
  Removed: `read_timeout`, `response_header_timeout`, `pool_idle_timeout`,
  `disable_keepalive`, `local_address`, `tcp_nodelay`, `tcp_keepalive`.
  Use `timeouts(TimeoutConfig { .. })`, `pool_config(PoolConfig { .. })`,
  and `socket_config(SocketConfig { .. })`. `timeouts` replaces every
  timeout, including values set earlier by `timeout` and `connect_timeout`.

### Removed

- Process-global response and error observers (`leyline::observe`).
  `SessionBuilder::audit` covers the same need per session.
- `Error::Proxy`, which nothing constructed.
- `H2Config::settings_ack_timeout`, which nothing read.

## 0.1.0

### Changed

- Crate package name is `leyline-http` (lib still `leyline`).
- Bundled Chrome 152, Firefox 154, Safari 26 (WKWebView TLS/H2). Edge is a Chrome TLS overlay.
- `Session::chrome()` uses `ProtocolPolicy::Race` when `http3` is on.
- HTTP/3 QPACK capacity is advertised as 0 until the decoder has a
  dynamic table.
- Node/Python Chrome clients race H3 against H2 like `Session::chrome()`.
- `Session::get` / `head` / `post` return a `RequestBuilder`. Await the
  builder (`session.get(url).await`) or chain (`.header(...).json(...).await`).
  `Session::chrome().get(url).await` is the oneshot. There is no crate-root
  `leyline::get`.
- `Session::profile(browser, platform)` returns `Result`. `Response::text`
  and `bytes` return `Result`. JSON/form bodies infer Xhr/Form on POST/PUT/PATCH
  when the session impersonates a browser. `Session::execute` infers from
  `content-type` unless `Request.preset` is set.
- WebSocket is one door: `session.websocket(url).await`. `wss://` only.
  The handshake does not advertise `permessage-deflate`.
- Header names, values, and methods are RFC 9110 tokens. CR/LF is rejected
  before the request is sent.
- A `v*` tag runs hosted `scripts/verify.sh --full`. Publishing is
  manual. Local default `verify.sh` is package sanity; `--full` is the
  release gate.
- **Application retries are opt-in.** A new `Session` uses
  `RetryPolicy::none()` (`Default` is none). Use `RetryPolicy::transient()`
  for 429/502/503/504, connect errors, and timeouts (3 retries, 4 attempts).
  Set it on the session or one request. A streaming body that cannot replay
  returns the attempt's error instead of a synthetic message.

- **Default connect timeout is now 10 seconds** (was: `None` / bounded only by
  the request-wide `total` of 300s). A faulty provider that completes the TCP
  connection but stalls the TLS handshake now fails the connect fast — as a
  retryable connection error — instead of tying the request up for the full
  `total`. Override with `connect_timeout(...)`, or `connect: None` on
  `TimeoutConfig` to disable.

### Security

- **`TlsTrustConfig::without_system_roots()` now actually excludes the platform
  trust store.** The connector is built from BoringSSL's `SslConnector::builder`,
  which unconditionally calls `set_default_verify_paths()`; opting out of system
  roots previously only skipped *adding* them, leaving the OS default CAs (the
  public web PKI, on Linux) in the store. A caller who trusted only a private CA
  therefore still trusted every public CA — an attacker holding any public-CA
  cert for the target host could MITM them. The no-system-roots path now builds
  from `bare_builder` (which never calls `set_default_verify_paths`), so only the
  explicitly-configured roots are trusted.

### Added

- **Opera brand overlay covers Chromium 149 (Opera 133 Stable).** `OPERA_PER_CHROMIUM`
  gains the `149 → 133` anchor (Opera 133 Stable on Chromium 149.0.7827.201, per Opera's
  official desktop release blog), so an Opera brand on a Chromium-149 profile emits a real
  `OPR/133` overlay instead of erroring back to stock Chrome.

### Fixed

- **Firefox identities now send a Firefox-shaped request, not a Chrome one.** The
  header presets are Chrome-shaped, so a Firefox session (`Browser::Firefox148` through `Firefox154`)
  sent `Sec-CH-UA*` Client Hints (which no Firefox build emits), the Chrome
  document `Accept`, no `priority` / `te` headers, and Chrome's header order — all
  on the same connection as the Firefox JA4, a hard TLS-vs-header contradiction.
  A Firefox identity now: strips every `sec-ch-ua*` header; uses the Gecko document
  `Accept` (`text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8`); adds
  `priority` (`u=0, i` on document loads, `u=1, i` on subresources) and
  `te: trailers`; and reorders the full request (including `cookie`) to the real
  Firefox sequence. Values captured live from Firefox 153 via tls.peet.ws; the
  navigate order is capture-exact (subresource `content-type`/`origin`/`cookie`
  positions are Firefox-conventional pending a cookied-XHR capture).

- **Chrome 150 profile GREASE brand corrected to `"Not;A=Brand";v="8"`.** The
  `chrome/150.toml` `sec_ch_ua` carried Chrome 149's GREASE
  (`"Not)A;Brand";v="24"`) across all four platform identities — a stale copy —
  so a Chrome-150 session advertised a brand token no real Chrome 150 emits,
  splitting the wire `sec-ch-ua` from the browser's own `navigator.userAgentData`.
  Corrected to the value a real Chromium 150.0.7871.47 build emits.

- **TLS handshake failures are retryable.** Connect-phase TLS errors
  (`Handshake`, `HandshakeIo`, `SslConnect`) join `TcpConnect` / `Dns`.
  A peer that resets, alerts, or speaks garbage during the handshake is
  retried on a fresh connection. Certificate, hostname, and pin failures
  stay permanent. `Error::is_connect` is handshake/DNS/proxy only.
  `Error::is_connection_closed` is a drop after a connection existed
  (not DNS or TCP-connect failure).

- **307/308 redirects no longer drop a buffered request body.** The hop body was
  moved into the send and `current_body` left empty, so a method+body-preserving
  redirect re-sent the request with **no body** — silently corrupting e.g. a
  307-redirected login/checkout POST. The buffered body is now kept as a cheap
  refcounted clone and replayed on the next hop. Streaming bodies still fail
  loudly (they can't be replayed); an empty body stays empty.

- **HTTP/2 inbound frame-size cap now uses our advertised `MAX_FRAME_SIZE`, not
  the peer's.** The reader was capped by `peer_settings.max_frame_size` — the
  peer's *receive* limit — so a hostile server advertising a 16 MB frame size
  could make the client accept (and eagerly pre-allocate) frames far larger than
  the 16384 it actually advertised. The reader now enforces our own value.

- **Malformed or absent HTTP/2 and HTTP/3 `:status` no longer mis-behaves.** In
  HTTP/2 a malformed `:status` propagated out of the event loop and tore down
  every stream on the connection, and an absent one was delivered to the caller
  as a successful `status = 0`. Both now fail just that stream (PROTOCOL_ERROR);
  the HTTP/3 path, which coerced a bad `:status` to `0`, now resets the stream
  with `H3_MESSAGE_ERROR`.

- **HTTP/2 trailers now require END_STREAM.** A trailer HEADERS block without
  END_STREAM (a malformed response per RFC 9113 §8.1) was completing the stream
  as if the response ended cleanly; it now fails the stream.

- **Redirects to a non-HTTP(S) scheme are refused.** A `Location:` of `file:`,
  `data:`, `javascript:`, etc. was passed into the transport to fail with a
  confusing downstream error; it is now rejected with a clear `Error::Redirect`.

- **SOCKS5 no longer follows an auth method it never offered.** A server that
  selected USERNAME/PASSWORD when the client offered only NO_AUTH would proceed
  to authenticate (with empty credentials) — an unsolicited-auth downgrade. The
  client now rejects a selected method it did not advertise, and offers auth when
  the proxy URL carries a username **or** a password (a password-only URL was
  previously not offering auth).

- **`tcp::log_once` no longer panics on a poisoned lock.** It now recovers via
  `PoisonError::into_inner`, matching the rest of the crate — a best-effort
  diagnostic log must never abort a connection.

- **HTTP/3 driver now reaps cancelled request streams (cancel-safety parity with
  HTTP/2).** When a caller dropped its response receiver before the peer replied
  — e.g. an outer `response_header` / `total` timeout firing on a silent upstream
  — the H3 driver left the stream in its table with no peer event to remove it,
  holding `max_concurrent_bidi_streams` credit and flow-control window until the
  connection's idle timeout. The driver now sweeps such streams each loop
  iteration (and caps its select wait to a bounded window while any stream is in
  flight), STOP_SENDING + RESET_STREAM to free the slot at once — mirroring the
  H2 driver's `sweep_cancelled_streams`. Off the default `Auto`→H2 path; affects
  explicit `.http3()` / `.race()` callers.

- **`tcp_keepalive_retries` no longer warns (or, under `strict`, hard-fails) on
  platforms without per-socket `TCP_KEEPCNT`.** `SocketConfig` defaults the
  retry count to `Some(3)`, but `socket2` only exposes `with_retries` on
  Linux/Android/Apple/BSD. On other targets (Windows) every connection logged
  `WARN socket option not supported option="tcp_keepalive_retries"`, and a
  `strict: true` caller would have hard-failed every connect on a value it never
  set. Keepalive idle time + interval still apply there, so the missing
  retry-count is now a benign `debug!` no-op and the OS default probe count is
  used. `tcp_user_timeout` / `interface` (explicit opt-ins) still error under
  `strict`.

### Added

- **Post-send response timeout (`SessionBuilder::response_header_timeout`,
  `TimeoutConfig::response_header`).** Caps the wait from request-sent until the
  transport response resolves, per redirect hop. Bounds an upstream — typically
  a proxy — that completes the handshake and then goes silent: `connect_timeout`
  has already elapsed and `read_timeout` only arms on a streamed body, so before
  this the silent phase was bounded only by the request-wide `total`. For a
  **streamed** response (`stream_response`) it resolves at headers — a true
  time-to-first-byte cap, with `read` then governing the body. For the
  **buffered** default it resolves only after the full body (H1/H2/H3 alike), so
  it bounds the whole response per hop. Opt-in — `None` leaves the phase bounded
  by `total`, so existing sessions are unchanged.

- **Chrome 148 profile (`Browser::Chrome148`).** Verified against
  tls.peet.ws on 2026-06-09 (Windows capture): wire-identical to Chrome
  147 — same cipher list, extension set, signature algorithms, supported
  groups, JA4 (`t13d1516h2_8daaf6152771_d8a2da3f94cd` cold), and Akamai-H2
  fingerprint (`1:65536;2:0;4:6291456;6:262144|15663105|0|m,a,s,p`). Only
  HTTP identity differs: the `Chrome/148` UA token and the rotated
  `sec-ch-ua` brand list (`"Chromium";v="148", "Google Chrome";v="148",
  "Not/A)Brand";v="99"`). `Browser::default_browser()` /
  `Session::chrome()` now resolves to Chrome 148;
  pin `Browser::Chrome147` via the builder for the prior version.

### Changed

- **Response fingerprint introspection is now opt-in (`SessionBuilder::audit`).**
  Off by default. When off, the execute path skips cloning the request
  headers, `Response::request_headers()` returns empty, and
  `Response::audit()` returns `None` — so high-throughput callers that never
  introspect pay nothing on the hot path (previously every response cloned its
  request header vec and built the audit block eagerly). Call
  `Session::builder().audit(true)` to populate `audit()` (JA3/JA4/JA4H/JA4T/H2)
  and `request_headers()`. A registered `observe` response observer implies
  header retention regardless, since its snapshot borrows them. **Migration:**
  any caller that reads `resp.audit()` or `resp.request_headers()` must now set
  `.audit(true)` on the session builder.

- **Pool + socket defaults tuned for long-lived sessions.**
  `DEFAULT_IDLE_TIMEOUT` 90s → 300s (matches Chrome's
  `kUsedIdleSocketTimeout`), `DEFAULT_MAX_CONNECTIONS` 256 → 2048, and
  `SocketConfig` now enables kernel TCP keepalive by default
  (60s idle / 30s interval / 3 probes). Long-lived, session-persistent
  pooled workloads keep one pool entry per
  `(host, proxy)` pair and previously had to run app-level keep-alive
  pings just to outrun the 90s reaper and the 256-entry LRU. Override
  via `Session::builder().pool_idle_timeout(...)` / `.pool_config(...)` /
  `.tcp_keepalive(...)`, or set `SocketConfig.tcp_keepalive = None` to
  disable kernel keepalive. ~2048 × ~50 KB ≈ 100 MB pool ceiling per
  session.

### Fixed

- **H1 pool probes a pooled connection for liveness before reuse.**
  `checkout_h1` only saw the idle deque, so a keep-alive peer that closed
  its half while the connection sat pooled (e.g. a Node server whose
  default `keepAliveTimeout` is 5s vs the pool's 300s idle) was handed out
  dead — the next write failed mid-request (Windows os error 10053 /
  `WSAECONNABORTED`), spamming `pool stale hit` and hard-failing streaming
  request bodies that cannot be replayed. Checkout now does a non-blocking
  `poll_read` probe and drains any pooled entry already at EOF / error /
  pre-request desync, turning a stale keep-alive socket into a clean cache
  miss instead of a failed exchange. Buffered and one-shot streaming request
  bodies both benefit; the irreducible probe-to-write race (a peer FIN landing
  in the microseconds before the first write) can still surface for one-shot
  streaming bodies, which cannot be replayed. Probe catches are counted in the
  new `PoolStats::stale_probed`, distinct from `evictions_dead` (mid-exchange
  failures), so the probe's effectiveness is observable.

- **Env-inherited `NO_PROXY` no longer bypasses explicitly-set proxies.**
  `ProxyConfig::proxy_for` checked the no-proxy matcher before any proxy
  resolution, and the matcher defaults to the `NO_PROXY` env var — so a
  stray `NO_PROXY` on the box silently turned per-request and session
  proxies DIRECT (a real-IP leak). No-proxy gating is
  now scoped by provenance: env-inherited patterns only bypass
  env-discovered proxies; a matcher set via `.no_proxy(...)` keeps the
  old bypass-everything semantics.

- **`danger_accept_invalid_certs` builds even when the system trust
  store is unloadable.** Verification-disabled sessions skip system
  trust wiring. A machine with a broken ROOT hive can still build a
  `-k` session.

- **TLS session-ticket cache recovers from mutex poison on the write
  side too.** The read side recovered but the `new_session_callback`
  still dropped tickets on a poisoned lock, so one poison event
  permanently downgraded every later handshake to the cold
  (non-resumed) JA4. Both sites now share one recovering lock helper.

- **`H2Config::from_profile` rejects duplicate `pseudo_order` tokens.**
  Four known tokens with a duplicate (e.g. `:method` twice, no
  `:authority`) passed validation and would emit a malformed
  pseudo-header list.

- **h2: `FrameReader::next` is now cancel-safe.** The actor-model
  driver polls `reader.next()` in a biased `tokio::select!` alongside a
  command channel, a body channel, and a sweep tick. The previous
  implementation used `AsyncReadExt::read_exact`, which is not
  cancel-safe: when a sibling branch won the race, `read_exact` was
  dropped mid-read and the bytes it had already pulled off the socket
  were lost. The reader then restarted `read_exact` from scratch and
  parsed the first 9 bytes it saw — which were payload bytes offset
  into the real frame — as a new frame header. The wire was desynced
  and the next "frame length" was a random 24-bit integer, surfacing
  to callers as
  `http: request: frame size <~4–9 MB> exceeds max 16384`. `FrameReader`
  now keeps a header cursor and an optional payload state on the
  struct, does single-call `AsyncReadExt::read` (cancel-safe per
  Tokio's contract: no bytes moved on Pending), and resumes from the
  cursor on the next call. Regression test
  `codec::tests::next_resumes_after_cancellation` races `next()`
  against `tokio::task::yield_now()` with a server feeding one byte at
  a time.
- **h2 HPACK: auto-lowercase header names on encode.** RFC 7540
  §8.1.2 requires HTTP/2 header field names to be lowercase, and
  §8.1.2.6 requires peers to treat uppercase names as a stream error
  (`PROTOCOL_ERROR`). The previous `Encoder::encode_header` passed the
  caller's name through unchanged, which meant a caller with a
  vendor-supplied mixed-case header name (some SDKs emit non-lowercase
  header names) sent a malformed frame and the peer returned
  400 with `"found an invalid character in header name"`. The
  encoder now lowercases names at the top of `encode_header`,
  allocating only when an uppercase byte is present.
  Pseudo-headers (`:method`, `:path`, …) are already lowercase.
  Regression test `hpack::encoder::tests::uppercase_name_encoded_as_lowercase`.
- **Windows: bridge the system ROOT certificate store into BoringSSL.**
  BoringSSL's `X509_STORE_set_default_paths()` points at Unix-style
  locations (`/etc/ssl/certs`) that do not exist on Windows, so a
  default `Session` on Windows had zero trust roots and every HTTPS
  request failed at handshake with `unable to get local issuer
  certificate`. `leyline-http` now enumerates the logical `"ROOT"`
  Windows store via the Win32 crypto API (`CertOpenSystemStoreW`,
  `CertEnumCertificatesInStore`) and loads every cert into the
  BoringSSL `X509_STORE` when neither `SSL_CERT_FILE` nor
  `SSL_CERT_DIR` is set. No dependency on `schannel` or
  `rustls-native-certs` — both are still banned by `deny.toml` as
  alternative TLS backends. macOS Keychain bridging is tracked as a
  follow-up.

## 1.0.0-alpha.1 — 2026-04-17

Historical version line. First crates.io publish is 0.1.0.

### What's here

The API is shaped like `reqwest`, async on tokio. `Session` builds a client
pinned to a browser profile. `RequestBuilder` handles the request side
(headers, query, form, JSON, multipart, streaming bodies, digest auth,
retry). `Response` exposes status, headers, body (buffered or streamed),
cookies, and a per-connection audit block (JA3, JA4, JA4T, JA4H, H2
Akamai fingerprint).

Transports: HTTP/1.1 with a keep-alive pool, HTTP/2 with a custom
concurrent-multiplex driver, HTTP/3 over quiche. Automatic ALPN-driven
protocol selection, explicit override via `SessionBuilder::http1/http2/http3`.

Proxies: HTTP CONNECT tunnel, SOCKS5, both with username/password auth.
`HTTP_PROXY` / `HTTPS_PROXY` / `NO_PROXY` environment support.

WebSocket over HTTP/1.1 upgrade and RFC 8441 extended CONNECT over HTTP/2.
`Session::websocket` auto-negotiates.

Browser profiles: Chrome 145/146/147, Firefox 148, Safari macOS 18,
Safari iOS 15/17/18, OkHttp Android 7/10. Adding a version is a TOML
copy-and-edit.

Python and Node.js wrappers. Each builds the native library locally.

### Security hardening

- **CWE-93** — H1 request smuggling. `send_request_h1_pooled` validates
  method, request-target, header names (RFC 9110 §5.6.2 `tchar`), and
  header values (§5.5 `field-value`) before any TCP connect.
- **CWE-93** — Multipart header injection. Control chars rejected;
  filename / name are properly quoted-pair-escaped.
- **RFC 9112 §6.1** — H1 response framing conflicts. Multiple
  `Content-Length`, comma-list CL, CL plus TE, and TE where `chunked`
  is not the final coding are all rejected before body read. Present
  but non-decimal CL (`+10`, `10 foo`, hex, overflow) is rejected
  instead of falling through to read-to-close.
- **RFC 9113 §6.9.1** — H2 flow-control window overflow. `WINDOW_UPDATE`
  that would push any window past 2³¹−1 surfaces `FLOW_CONTROL_ERROR`
  per-stream or per-connection. Applies to the handshake path.
- **CVE-2023-44487 shape** — H2 RST_STREAM flood guard. Configurable
  threshold and window on `H2Config`; trips `ENHANCE_YOUR_CALM`.
  Separate identical guard for SETTINGS frames.
- **httpoxy** — `HTTP_PROXY` is ignored when any of ten CGI signals is
  present. `HTTPS_PROXY` is unaffected — no request header maps to it.
- **H3 body cap** — `H3Config::max_response_body_bytes` (default 100 MiB).
  Exceeding it closes the stream with `H3_EXCESSIVE_LOAD`.
- **SSL_CERT_DIR** — Follows symlinks, so Debian / Ubuntu / RHEL systems
  (where `/etc/ssl/certs` is entirely symlinks) actually pick up their
  CAs when the env var is set.
- **NO_PROXY** — Bare IPv6 patterns (`::1`, `fe80::1`, `2001:db8::1`) no
  longer have their trailing hextet stripped as a port.
- **Proxy CONNECT** — Strict status-line parse. CL / TE on 2xx CONNECT
  responses are rejected (RFC 9110 §9.3.6). Trailing bytes past the
  terminator are rejected — a proxy pre-stuffing bytes is attempting
  to inject into the TLS handshake stream.
- **Chunked-transfer DoS** — Chunk size is parsed as `u64` first and
  rejected if it exceeds 100 MiB before allocation.
- **Cookie parser overflow** — `Expires` dates before 1970 no longer
  wrap. Found by the `cookie_set` fuzz target.

### Observability

- `PoolStats` (hits, misses, installs, evictions) per `Pool`.
- `tracing` spans on the hot request path.
- Per-response audit block (JA3 / JA4 / JA4T / JA4H / H2 Akamai).

### Testing and verification

- 780 tests across the workspace.
- Three `cargo fuzz` targets (`h2_frame`, `hpack`, `h2_continuation`).
  `scripts/verify.sh --full` replays the corpus when cargo-fuzz and
  nightly are installed. Time-bounded fuzzing is `--fuzz [SECONDS]`.
- Live tests against `tls.peet.ws`, Cloudflare, and Google QUIC
  document JA3 / JA4 / H2 fingerprints per profile.
- `scripts/verify.sh` runs fmt, clippy (`-D warnings`), docs
  (`-D warnings`), tests, `cargo deny`, benches compile, and fuzz
  corpus replay as a release gate.

### Known limitations

- The HTTP/2 stack is browser-shaped, not a full RFC 9113 server.
- Fingerprints drift. Profiles track the browser versions we captured;
  re-verify against a fresh capture before production use.
- `leyline-bssl-sys` ships prebuilt BoringSSL libraries for four
  targets. Node and Python still need a native addon or wheel.
