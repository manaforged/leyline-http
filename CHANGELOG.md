# Changelog

All notable changes to Leyline. Format loosely follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and
[SemVer](https://semver.org/spec/v2.0.0.html).

Leyline is pre-1.0. Breaking changes will happen between minor releases
until 1.0 — pin exact versions.

## Unreleased

### Changed

- Removed GitHub workflow orchestration; verification and releases now run directly on the supported self-hosted machines.
- Reduced the push gate to package parity and compile sanity; formatting, deeper, and live checks require `--full`.
- Restored the full verification gate on the current Rust toolchain by formatting the merged examples and resolving Clippy and rustdoc failures.

### Added

- **Opera brand overlay covers Chromium 149 (Opera 133 Stable).** `OPERA_PER_CHROMIUM`
  gains the `149 → 133` anchor (Opera 133 Stable on Chromium 149.0.7827.201, per Opera's
  official desktop release blog), so an Opera brand on a Chromium-149 profile emits a real
  `OPR/133` overlay instead of erroring back to stock Chrome.

### Fixed

- **Firefox identities now send a Firefox-shaped request, not a Chrome one.** The
  header presets are Chrome-shaped, so a Firefox session (`Browser::Firefox15x`)
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
  `Session::chrome_latest()` / `Session::new()` now resolve to Chrome 148;
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
  via `Session::builder().pool_limits(idle, max)` /
  `.tcp_keepalive(..)`, or set `SocketConfig.tcp_keepalive = None` to
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
  store is unloadable.** The Windows trust-store hard-fail introduced in
  the hardening wave ran during context build, before
  `set_accept_invalid_certs` was applied — so a machine with a broken
  ROOT hive could not build a `-k` session at all. Verification-disabled
  sessions now skip system trust wiring entirely.

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
  certificate`. `leyline-tls` now enumerates the logical `"ROOT"`
  Windows store via the Win32 crypto API (`CertOpenSystemStoreW`,
  `CertEnumCertificatesInStore`) and loads every cert into the
  BoringSSL `X509_STORE` when neither `SSL_CERT_FILE` nor
  `SSL_CERT_DIR` is set. No dependency on `schannel` or
  `rustls-native-certs` — both are still banned by `deny.toml` as
  alternative TLS backends. macOS Keychain bridging is tracked as a
  follow-up.

## 1.0.0-alpha.1 — 2026-04-17

First public release.

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

C FFI plus Python, Node.js, and Go wrappers — each builds the dynamic
library locally.

### Security hardening

The headline security items:

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
- Four `cargo fuzz` targets with 1,150 seeded corpus inputs.
  `scripts/verify.sh` replays the corpus on every non-quick run and
  supports time-bounded fuzzing via `--fuzz [SECONDS]`.
- Live tests against `tls.peet.ws`, Cloudflare, and Google QUIC
  document JA3 / JA4 / H2 fingerprints per profile.
- `scripts/verify.sh` runs fmt, clippy (`-D warnings`), docs
  (`-D warnings`), tests, `cargo deny`, benches compile, and fuzz
  corpus replay as a release gate.

### Known limitations

- `leyline-h2` is browser-shaped, not a full RFC 9113 server-capable
  stack. See [TESTING.md](TESTING.md) for the covered surface.
- Fingerprints drift. Profiles track the browser versions we captured;
  re-verify against a fresh capture before production use.
- Pre-compiled FFI artifacts are not published; wrappers build the
  dynamic library locally.
