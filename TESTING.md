# Testing Leyline

Every wire-level property Leyline claims is gated by a test in this repo. The
README's `## Testing` section points here for the full matrix. This document
is the canonical evidence table.

## Running the suites

First-clone developer check:

```bash
./scripts/dev-setup.sh       # Linux / macOS / Git Bash
.\scripts\dev-setup.ps1      # Windows PowerShell
.\scripts\verify.ps1 -Quick  # Windows PowerShell local gate
```

Release and regression gates:

```bash
cargo test --workspace --exclude leyline-quiche           # offline Leyline suite + PQ gate + claim guard
cargo test -p leyline --test tls_peet -- --ignored        # live fingerprint tests
cargo test -p leyline --test smoke -- --ignored --nocapture # 17-gate smoke suite
cargo deny --all-features check                           # supply chain gate
```

The pre-commit hook runs the offline Leyline workspace suite on every commit.
`leyline-quiche` is a vendored Cloudflare quiche fork; its upstream lib tests
are not part of Leyline's release gate, but Leyline's H3 integration path is
covered by the smoke and transport tests below.

## What a plain `cargo test` does and doesn't prove

Rows below whose test is named `live_*` run **only** under `-- --ignored` and
talk to real infrastructure (tls.peet.ws, Cloudflare, Google QUIC). The
pre-commit and pre-push hooks run them; a plain `cargo test` does **not**.
That means an offline run does not anchor Leyline's wire output against an
external browser — the offline fingerprint tests compare the emitter to
Leyline's own TOML (necessary, but tautological by construction). Only the
`live_*` suite, and `chrome_pq_key_shares_use_distinct_x25519_ephemerals`
(a real local ClientHello capture), check against ground truth. JA4 is
externally anchored for the six profiles with a `live_ja4_exact_match_*`
test; the other shipped profiles are TOML-asserted, not live-verified.

Offline rows whose test lives in `tests/http_semantics.rs` run against an
**in-process httpbin-lite mock** (no network) — deterministic, but they
prove Leyline's HTTP semantics, not behaviour against any real server.

## Evidence matrix

| Property | How it is proved | Where |
|---|---|---|
| TLS ClientHello matches profile JA4 | Live capture from tls.peet.ws compared against TOML expectation, per browser | `live_ja4_exact_match_{chrome145,chrome146,chrome147,chrome148,firefox150,safari18}` |
| Every profile's H2 fingerprint matches its TOML value | Akamai H2 fingerprint asserted for all 15 profiles | `h2_fingerprints_match_toml_expectations` (offline, emitter↔TOML), `live_h2_akamai_every_profile` (live anchor) |
| HTTP/2 pseudo-header order matches browser | Live capture, per browser | `live_chrome147_pseudo_header_order`, `live_firefox150_pseudo_header_order` |
| TCP SYN differs by OS (JA4T) | TTL 64 on Linux, 128 on Windows, three-way distinguishable | `live_tcp_linux_ttl_is_64`, `live_tcp_windows_ttl_is_128`, `live_tcp_windows_distinguishable_from_linux` |
| ALPS / cert compression / ALPN extensions present | Live tls.peet.ws inspection, per extension | `live_chrome147_has_alps_extension`, `live_chrome147_has_cert_compression` |
| Cert-compression algorithm set matches profile | Firefox 150/151 advertise zlib+brotli+zstd (not just brotli) | `live_firefox_cert_compression_advertises_zlib_brotli_zstd` |
| Cipher order matches profile | tls.peet.ws cipher list compared to TOML order | `live_chrome147_ciphers_match_profile_order` |
| Post-quantum ephemeral keys are distinct (utls#342 regression) | Local TCP capture of Chrome 147 ClientHello, parse `key_share`, assert X25519 ≠ X25519 inside X25519MLKEM768 | `chrome_pq_key_shares_use_distinct_x25519_ephemerals` |
| TLS 1.3 session resumption works end-to-end | Two requests, second presents a valid pre-shared key | `live_session_resumption_pre_shared_key` |
| Peer certificate is reachable on the response | Response exposes DER-encoded cert + version + cipher | `live_tls_peer_certificate_exposed` |
| HTTP/3 reachable against real QUIC servers | Direct H3 GET against Cloudflare and Google | `live_h3_cloudflare`, `live_h3_cloudflare_firefox_profile`, `live_h3_google` |
| Wire-fidelity: caller-set headers | Raw TCP captures assert caller header replacement, duplicate preservation, redirect auth stripping/preservation, and brand overlay precedence | `crates/leyline/tests/core_wire_fidelity.rs` |
| HTTP/2 connection reuse through the pool | Three sequential requests on one session, second/third are warm | `live_h2_connection_reuse` |
| Cookies persist and flow back | Set-Cookie captured, jar replays it on the next hop (offline mock) | `cookies_set_then_sent` |
| Redirects follow and rewrite the URL | Chain of 302s recorded, final URL is the destination (offline mock) | `redirect_follows_and_rewrites_url`, `redirect_preserves_auth_same_host` |
| Authorization stripped on cross-origin redirect | Raw-TCP capture: header present on hop 1, absent after the origin changes; preserved same-origin | `redirect_cross_origin_strips_authorization_after_first_hop`, `redirect_same_origin_preserves_authorization` (`core_wire_fidelity.rs`) |
| Decompression for gzip / brotli / deflate | Mock compresses the body; full body decodes to valid JSON (offline mock) | `decompression_gzip`, `decompression_brotli`, `decompression_deflate` |
| JSON / form body round-trip | Echoed fields come back unchanged (offline mock) | `post_json_body_roundtrip`, `post_form_body_roundtrip` |
| HTTP CONNECT and SOCKS5 proxy wire shape | Local mock proxies assert exact bytes per RFC 1928 / RFC 7231 | `offline_http_connect_proxy_wire_bytes`, `offline_socks5_proxy_wire_bytes`, `live_http_connect_proxy`, `live_socks5_proxy` |
| WebSocket upgrade succeeds on a fingerprinted H1 stream | Live echo round-trip | `live_websocket_echo` |
| Identity headers vary correctly by platform | UA, sec-ch-ua, sec-ch-ua-platform per Windows / Linux / Android | `live_chrome147_windows_identity_headers`, `live_chrome147_linux_identity_headers`, `live_chrome147_android_identity_headers` |

## The 17-gate smoke suite

`cargo test -p leyline --test smoke -- --ignored --nocapture` runs all of the
gates below end-to-end against public infrastructure. Source:
[`crates/leyline/tests/smoke.rs`](crates/leyline/tests/smoke.rs).

| Gate | Assertion |
|---|---|
| Chrome 148 exact JA4 + H2 | JA4 and H2 match profile expectations |
| Firefox 150 exact JA4 + H2 | JA4 and H2 match profile expectations |
| Connection reuse (3 requests) | All requests succeed through one session |
| Brotli/gzip decompression | Response decodes to valid JSON |
| HTTP/2 CDN GET (httpbin.org) | Request succeeds over HTTPS |
| POST JSON | Body echoes through the server |
| POST form | Form fields echo through the server |
| GET with query params | Query fields echo through the server |
| Bearer auth header | Authorization header reaches the server |
| `error_for_status` on 404 | 404 maps to an error |
| Redirect following (302 x2) | Redirect chain is recorded |
| Chrome/Firefox/Safari differ | Three profiles produce three H2 fingerprints |
| HTTP/1.1 browser wire shape | `Host`, `User-Agent`, `Accept-Encoding`, keep-alive |
| HTTP/3 Chrome QUIC GET | Direct H3 request succeeds |
| HTTP/3 Firefox QUIC GET | Direct H3 request succeeds |
| HTTP/3 POST body round-trip | Body echoes over H3 |
| Large response (50KB) | Full body is received |

## Offline unit coverage

| Crate | What they cover |
|---|---|
| `leyline-h2` | Frame encode/decode, HPACK roundtrip, Huffman coverage, SETTINGS fingerprint, **stream state machine (42 tests in `crates/leyline/src/h2/stream_state.rs` unit tests)**, **CONNECT / extended CONNECT pseudo-header shape (`crates/leyline/tests/h2_connect_method.rs`)**, **RST_STREAM flood guard (`crates/leyline/tests/h2_rst_flood.rs`)**, **outbound PRIORITY on HEADERS (`crates/leyline/tests/h2_frame_roundtrip.rs`)** |
| `leyline-cookies` | Set-Cookie parsing, jar ordering, eviction, SameSite, prefix validation |
| `leyline-audit` | JA3/JA4 section computation, cipher ID mapping, GREASE detection |
| `leyline-profile` | Profile loading, builder combinations, browser shortcuts |
| `leyline-core` | Request building, response handling, protocol policy, HTTP/1.1 wire shape |
| `leyline-tcp` | Platform profiles apply correctly |
| `leyline-pool` | Connection reuse |

## HTTP/2 correctness surface

`leyline-h2` is intentionally small. What it currently guarantees:

- Per-stream state tracking (RFC 9113 §5.1): `Idle` → `Open` →
  `HalfClosedLocal` → `Closed`, with illegal transitions surfaced as
  `H2Error::Stream { code: ProtocolError }`. See
  `crates/leyline/src/h2/stream_state.rs`.
- `MAX_CONCURRENT_STREAMS` enforced outbound against the peer's
  advertised value.
- Inbound RST_STREAM flood guard — default 100 RSTs in 10 seconds
  trips an `EnhanceYourCalm` connection error. Defense-in-depth
  against CVE-2023-44487-shaped server behaviour. Configurable via
  `H2Config::rst_stream_flood_threshold` /
  `rst_stream_flood_window`.
- Outbound PRIORITY field on the first HEADERS frame, driven by
  `H2Config::default_priority`. Needed for Chrome/Firefox fingerprint
  parity; can be left `None` for a clean minimal HEADERS.
- Outbound trailers via `send_request_with_trailers`. Tiny trailer
  blocks only — blocks larger than `max_frame_size` return
  `InternalError` rather than splitting to CONTINUATION.
- Classic CONNECT (RFC 9113 §8.5) and extended CONNECT (RFC 8441)
  pseudo-header shapes. `:scheme` / `:path` are dropped for classic
  CONNECT; `:protocol` is emitted for extended.

**Concurrent multiplexing.** `H2Client` is a cloneable handle to a
driver task that owns the connection. Two concurrent
`H2Client::send_request` calls run over independent streams on the
same TCP connection; a stream parked on flow control does not block
other streams. See `crates/leyline/src/h2/client.rs` and
`crates/leyline/tests/h2_multiplex.rs` for the four concurrency gates:

- Concurrent responses delivered out of order.
- Parked-on-WINDOW_UPDATE stream does not block sibling stream.
- Graceful GOAWAY on last-handle-drop.
- Reader EOF fans an error out to every pending request.

What `leyline-h2` explicitly does **not** do:
- Server-side support or server-initiated streams.
- PUSH_PROMISE accepts. Any PUSH_PROMISE is RST_STREAM'd with
  `Cancel`. This matches Chrome's `SETTINGS_ENABLE_PUSH = 0` policy.

## Fuzzing

When present, `fuzz/` provides four `cargo-fuzz` targets (nightly):

- `hpack_integer` / `hpack_header_block` — HPACK decoder,
  single-block and two-block-with-shared-table scenarios.
- `h2_frame` — `Frame::parse` across every frame type.
- `cookie_set` — `CookieJar::store_set_cookie` parsing.

See `fuzz/README.md` for run instructions in branches that carry the fuzz
workspace. These are defensive targets for panics and integer overflows;
correctness is covered by the unit and round-trip tests above.

## Regression gates

| Test | What it guards |
|---|---|
| `chrome_pq_key_shares_use_distinct_x25519_ephemerals` | `crates/leyline/tests/pq_key_shares.rs` — utls#342 PQ ephemeral-key reuse distinguisher |
| `public_claims_stay_bounded` | `crates/leyline/tests/claim_guard.rs` — scans README and every public lib.rs for marketing superlatives |
| `readme_keeps_public_evidence_commands` | `crates/leyline/tests/claim_guard.rs` — pins the three `cargo test` / `cargo run` commands in the README |
