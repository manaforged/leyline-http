# Leyline — measured performance

Generated: 2026-06-21
Host: 8-core x86_64 desktop, Linux under WSL2
(kernel `6.6.87.2-microsoft-standard-WSL2`)
Toolchain: `rustc 1.89.0 (29483883e 2025-08-04)`
Criterion version: 0.5

Numbers below come from `cargo bench -p leyline-benches` at commit `5018d4b`. Reproduce:

    cd benches
    cargo bench -p leyline-benches

## Headline numbers

| Metric | Value | Bench |
|---|---|---|
| HPACK encode (16 Chrome-147 headers) | 1.54 µs (≈406 MiB/s) | `hpack::encode_chrome_headers` |
| HPACK decode (same header block) | 7.13 µs (≈64 MiB/s) | `hpack::decode_chrome_headers` |
| HPACK roundtrip (encode + decode) | 9.26 µs | `hpack::roundtrip_chrome_headers` |
| JA3 compute | 1.29 µs | `audit::compute_ja3` |
| JA4 compute | 2.38 µs | `audit::compute_ja4` |
| JA4T compute | 126.6 ns | `audit::compute_ja4t` |
| JA4H compute (16 Chrome-147 nav headers) | 733.4 ns | `audit::compute_ja4h` |
| Chrome extension id derivation | 51.6 ns | `audit::chrome_extension_ids` |
| Frame parse (mixed 6-frame batch, 1082 B) | 163.7 ns (≈6.51 GiB/s) | `frames::parse_mix` |
| Frame header parse (9 B, no payload) | 1.25 ns | `frames::parse_header` |
| Profile lookup (Chrome147) | 27.6 ns | `profile::lookup_chrome147` |
| Profile lookup (all 16 browser variants) | 451.1 ns | `profile::lookup_all_browsers` |
| Session build (Chrome147) | 9.61 ms | `session::build_chrome147` |
| Session build (`Session::chrome`) | 9.59 ms | `session::chrome` |
| `Pool::new()` | 20.9 ns | `pool::new` |
| `Pool::with_limits(...)` | 27.0 ns | `pool::with_limits` |
| Pool checkout-hit equivalent (`H2Client::clone`) | 35.9 ns | `pool::checkout_hit_equivalent` |
| `H2Client::clone` (standalone) | 36.7 ns | `multiplex::handle_clone` |
| Multiplex 1 000 sequential requests (mock peer) | 97.22 ms total / **10 286 req/s** | `multiplex::serial_1k_requests` |
| Multiplex 100 concurrent inflight (mock peer) | 437.6 µs total / **228 520 req/s** | `multiplex::concurrent_100_inflight` |
| Allocations per `send_request` (warm, serial) | **33 allocations** / 95.7 µs | `allocs::per_request` |
| Peak live-byte delta, 100 concurrent streams | **≈222 KiB (0.21 MiB)** | `allocs::concurrent_footprint` |
| Allocations per `Session::builder().build()?` | **1 412 allocations** | `allocs::session_build` |
| HPACK encode (via `hpack_vs_h2`) | 1.88 µs (≈334 MiB/s) | `hpack_vs_h2::leyline_encode` |
| HPACK decode (via `hpack_vs_h2`) | 7.22 µs (≈63 MiB/s) | `hpack_vs_h2::leyline_decode` |
| Peer comparison vs `h2` crate | deferred — h2 crate does not expose HPACK primitives publicly | `hpack_vs_h2::h2_peer_status` |

The multiplex pair is the headline proof of real multiplexing: 100 concurrent requests complete in ~438 µs where 100 sequential requests would take ~9.7 ms; concurrent throughput is **~22× serial throughput**, confirming the driver multiplexes streams rather than serialising them.

## Bare HTTP/1.1 vs `reqwest`

`http_vs_reqwest.rs` races Leyline against `reqwest` over a shared in-process
loopback HTTP/1.1 keep-alive server. Leyline's `http://` scheme routes through
its plaintext HTTP/1.1 transport (`send_request_h1`, no TLS); `reqwest` is built
`default-features = false` (no TLS backend, no h2, no gzip) so it is the bare
hyper H1 client. Both clients reuse one pooled client and fully consume the
10-byte response body, so the comparison is pure client-stack cost (request
build + H1 codec + pool checkout + response parse).

| Scenario | Leyline | reqwest | Ratio |
|---|---|---|---|
| 1 000 sequential GETs, one reused client | 125.84 ms (**7 947 req/s**) | 162.84 ms (**6 141 req/s**) | Leyline 1.29× |
| 100 concurrent GETs | 591.84 µs (**168 960 req/s**) | 668.41 µs (**149 610 req/s**) | Leyline 1.13× |

Both clients ran in the same process, on the same host, in the same run, so the
**ratio** is host-independent even though the absolute numbers are not. The
serial number is the per-request stack cost (full connection reuse, no
multiplexing — H1 is one request per connection); the concurrent number fans out
across each client's connection pool. This is a loopback measurement with no
network latency and no TLS handshake: it isolates the code each client runs per
request. Over a real network, round-trip time dominates and the two converge.

## HTTP/2 vs the impersonation libraries

The reqwest comparison above is the bare-HTTP floor. This section is the one that
matters for leyline's actual niche: how it compares to the other browser-
impersonation HTTP clients doing the real job — an HTTP/2 request carrying a
Chrome TLS fingerprint. The harness lives in
[`benches/comparison/`](benches/comparison/) — each client is a separate process
(they each vendor their own TLS stack and cannot share a binary) hitting one
shared local HTTPS/2 server.

| Client | Lang | TLS stack | Chrome | JA4 (tls.peet.ws) |
|---|---|---|---|---|
| leyline | Rust | leyline-bssl (BoringSSL) | 150 | `t13d1517h2_8daaf6152771_cb7bf5808d99` |
| wreq | Rust | boring (BoringSSL) | 137 | `t13d1516h2_8daaf6152771_d8a2da3f94cd` |
| bogdanfinn/tls-client | Go | utls | 146 | `t13d1517h2_8daaf6152771_dcad5a053991` |
| azuretls | Go | utls | latest | `t13d1516h2_8daaf6152771_d8a2da3f94cd` |

All four negotiate HTTP/2 and emit a Chrome-class JA4 with an **identical cipher
hash** (`8daaf6152771`) — the same Chrome cipher list, i.e. equal work. They
differ only in the extension component because each library tracks a different
newest Chrome (no Chrome major is supported by all four). Three metrics, each the
median of repeated runs on the 8-core desktop / WSL2 host:

| Client | warm (seq latency) | **concurrent throughput** | cold (handshake) |
|---|---|---|---|
| leyline | 2 690 req/s | **40 400 req/s** | 672 req/s |
| wreq | 2 700 req/s | **42 200 req/s** | 781 req/s |
| bogdanfinn/tls-client | 3 330 req/s | 24 100 req/s | 755 req/s |
| azuretls | 3 260 req/s | 16 250 req/s | 741 req/s |

- **Concurrent throughput** — 64 requests in flight over one multiplexed H2
  connection — is the metric that compares these clients at the scale they are
  actually used. The two Rust BoringSSL clients lead: leyline sustains
  **~40 400 req/s, ~1.7× bogdanfinn/tls-client and ~2.5× azuretls**, and lands
  within ~4% of wreq (its closest peer, same BoringSSL lineage; wreq is marginally
  ahead). The two Go utls clients trail.
- **warm** is single-request-at-a-time latency. All four cluster in a narrow band
  and the Go clients edge ahead, but this number is **server-bound** — against its
  own mock peer leyline does a request in ~97 µs vs ~370 µs here, so most of the
  warm time is the shared Go server + socket + TLS, not the client. It is reported
  for completeness, not as a client ranking; sequential is the wrong regime for
  HTTP/2, whose entire purpose is multiplexing.
- **cold** builds a fresh client and a new TLS handshake per request. leyline is a
  touch behind: a fresh `Session` rebuilds its connector each time. With the
  reuse-a-client norm this never shows; it matters only if you discard clients.

Combined with the reqwest result: leyline tracks wreq on impersonated H2
throughput (within ~4%), beats the Go utls clients by 1.7–2.5×, and beats reqwest
on plain HTTP — one library that stays in the leading group at both jobs, so a
bare-HTTP path and an impersonation path don't need two different clients.

Caveats: loopback only (no network RTT, no real-world TLS endpoints); each library
impersonates its own newest Chrome, so the JA4s are Chrome-class but not identical;
warm/cold are sequential and warm is server-bound as noted. Reproduce with
[`benches/comparison/run.sh`](benches/comparison/run.sh).

## Host sensitivity

These numbers are from an 8-core desktop under WSL2. Two classes of bench scale
differently with the host:

- **Compute-bound** (HPACK, JA3/JA4 audit math, frame parsing, `Session::build`)
  scale with single-thread CPU performance and are unaffected by virtualization.
- **Async-scheduler / IO-bound** (`multiplex::serial_1k_requests`,
  `allocs::per_request` wall-time) carry WSL2's per-wakeup overhead on the
  `tokio::io::duplex` round-trips; on bare metal the serial multiplex throughput
  is materially higher. The allocation **counts** these benches report (33 per
  request, 1 412 per session build) are host-independent and are the figures to
  track for regressions — wall-time is not comparable across hosts.

## Scope and caveats

- The H2 benches run against in-process mock peers over `tokio::io::duplex` — zero network latency, zero TLS handshake. The `http_vs_reqwest` bench uses a real loopback TCP socket (reqwest cannot attach to a duplex pipe) but still no external network or TLS. Real-network and handshake costs are excluded so these numbers reflect each client's own code paths.
- Single-machine numbers only. Multi-host workloads shift with kernel TCP, NIC offload, and syscall cost.
- HPACK figures are for a realistic Chrome-147 navigation request header set (16 headers, ≈508 B raw). Larger or smaller header blocks scale roughly linearly.
- Peer comparison against the `h2` crate's HPACK is **deferred** — see `benches/benches/hpack_vs_h2.rs`. The `h2` crate (v0.4) keeps its HPACK module private, so a real peer bench would require either forking `h2` or running the full `h2::client::handshake` (which measures far more than HPACK).
- The `reqwest` comparison is plaintext HTTP/1.1 only. It does not compare TLS handshake, HTTP/2, or fingerprint control — leyline's reason to exist — because reqwest does not expose those as a like-for-like surface. It answers one question: bare HTTP request overhead, client stack vs client stack.
- `allocs::session_build` records 6 497 allocations on iteration 1 (`LazyLock` profile-registry materialisation) and drops to a stable 1 412 allocations on every subsequent iteration. The headline value is the steady-state number.
- `allocs::concurrent_footprint` reports the **delta in live bytes** from just-before-fanout to peak during 100 concurrent streams; it excludes the fixed cost of the connection itself. Absolute RSS is not reported — use `heaptrack` or `dhat` if you need that.

## Methodology

- Criterion.rs with the default 3 s warmup and 100 samples (20–30 samples for the long multiplex and `http_vs_reqwest` runs so the full suite stays under ~10 minutes).
- System allocator (`std::alloc::System`) — no jemalloc / mimalloc. Numbers reflect the stdlib allocator cost users actually see.
- The mock HTTP/2 peer writes the minimum legal response: a HEADERS frame with `:status = 200` followed by a 10-byte DATA frame with END_STREAM. The `http_vs_reqwest` loopback server answers each keep-alive GET with a fixed 200 + 10-byte body.
- All benches run the release profile (`cargo bench`, which implies `--release`).
- The allocation-counting global allocator in `allocs.rs` wraps `std::alloc::System` and tallies allocation count and live-byte watermark with `AtomicU64`s — it is installed only in the `allocs` bench binary, so other numbers are unaffected.
- The excluded sub-workspace at `benches/` depends on the in-repo `leyline` crate, so the full TLS+H2 stack (and thus `Session`) links leyline's prebuilt BoringSSL instead of source-building it. Run `cargo bench -p leyline-benches` from `benches/` (or with `--manifest-path benches/Cargo.toml`).

## How to extend

To add a new bench:

1. Drop a new file in `benches/benches/` that registers a `criterion_group!` and `criterion_main!`.
2. Add a `[[bench]]` entry to `benches/Cargo.toml` (with `harness = false`).
3. For async benches, build a `tokio::runtime::Runtime` once inside the bench function and drive it with `iter_custom` + `rt.block_on(...)` — avoid Criterion's `#[tokio::test]` / `tokio::test` crate integrations.
4. Re-run the full suite and update this document. Benchmarks should be deterministic and complete in under 60 s each; the multiplex and `http_vs_reqwest` benches intentionally run longer because the workload is 100–1 000 requests per iteration.

## Bench file inventory

| File | Benches |
|---|---|
| `benches/hpack.rs` | `encode_chrome_headers`, `decode_chrome_headers`, `roundtrip_chrome_headers` |
| `benches/audit.rs` | `compute_ja3`, `compute_ja4`, `compute_ja4t`, `chrome_extension_ids`, `compute_ja4h` |
| `benches/frames.rs` | `parse_mix`, `parse_header` |
| `benches/profile.rs` | `lookup_chrome147`, `lookup_all_browsers` |
| `benches/session.rs` | `build_chrome147`, `chrome` |
| `benches/multiplex.rs` | `handle_clone`, `serial_1k_requests`, `concurrent_100_inflight` |
| `benches/hpack_vs_h2.rs` | `leyline_encode`, `leyline_decode`, `h2_peer_status` (deferred) |
| `benches/http_vs_reqwest.rs` | `leyline_serial_1k`, `reqwest_serial_1k`, `leyline_concurrent_100`, `reqwest_concurrent_100` |
| `benches/allocs.rs` | `per_request`, `concurrent_footprint`, `session_build` |
| `benches/pool.rs` | `new`, `with_limits`, `checkout_hit_equivalent` |
