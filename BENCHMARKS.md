# Leyline — measured performance

Generated: 2026-06-22
Host: 8-core x86_64 desktop, Windows 11
(10.0.26200), native (no WSL2)
Toolchain: `rustc 1.95.0 (59807616e 2026-04-14)`
Criterion version: 0.5

Numbers below come from `cargo bench -p leyline-benches` at commit `2802f6c`. Reproduce:

    cd benches
    cargo bench -p leyline-benches

Same physical CPU as earlier revisions of this file, now measured on native
Windows rather than WSL2. Two consequences for compute-bound rows: allocation-
heavy benches (HPACK encode, JA3/JA4 audit) run slower than the prior Linux
numbers because the Windows system allocator is costlier for small allocations
than glibc; conversely the duplex-pipe IO benches and `Session::build` run far
faster without WSL2's per-syscall overhead. The controlled before/after
sections below git-toggle each change on this one host, so they isolate the
real per-change deltas independent of these cross-host shifts.

## Headline numbers

| Metric | Value | Bench |
|---|---|---|
| HPACK encode (16 Chrome-147 headers) | 2.21 µs (≈219 MiB/s) | `hpack::encode_chrome_headers` |
| HPACK decode (same header block) | 5.06 µs (≈96 MiB/s) | `hpack::decode_chrome_headers` |
| HPACK roundtrip (encode + decode) | 7.18 µs | `hpack::roundtrip_chrome_headers` |
| JA3 compute | 2.17 µs | `audit::compute_ja3` |
| JA4 compute | 4.18 µs | `audit::compute_ja4` |
| JA4T compute | 146.3 ns | `audit::compute_ja4t` |
| JA4H compute (16 Chrome-147 nav headers) | 881.7 ns | `audit::compute_ja4h` |
| Chrome extension id derivation | 96.4 ns | `audit::chrome_extension_ids` |
| Frame parse (mixed 6-frame batch, 1082 B) | 153.4 ns (≈6.73 GiB/s) | `frames::parse_mix` |
| Frame header parse (9 B, no payload) | 1.01 ns | `frames::parse_header` |
| Profile lookup (Chrome147) | 41.3 ns | `profile::lookup_chrome147` |
| Profile lookup (all 16 browser variants) | 717.2 ns | `profile::lookup_all_browsers` |
| Session build (Chrome147) | 53.6 µs | `session::build_chrome147` |
| Session build (`Session::chrome`) | 46.3 µs | `session::chrome` |
| `Pool::new()` | 77.5 ns | `pool::new` |
| `Pool::with_limits(...)` | 89.5 ns | `pool::with_limits` |
| Pool checkout-hit equivalent (`H2Client::clone`) | 34.2 ns | `pool::checkout_hit_equivalent` |
| Pool warm checkout at occupancy 1 / 2048 | 169.9 ns / 172.8 ns (flat — O(1)) | `pool::checkout_at_occupancy` |
| `H2Client::clone` (standalone) | 32.5 ns | `multiplex::handle_clone` |
| Multiplex 1 000 sequential requests (mock peer) | 16.09 ms total / **62 150 req/s** | `multiplex::serial_1k_requests` |
| Multiplex 100 concurrent inflight (mock peer) | 249.8 µs total / **400 400 req/s** | `multiplex::concurrent_100_inflight` |
| Allocations per `send_request` (warm), tiny / Chrome-sized response | **20 / 61 allocations** | `allocs::per_request_{tiny,realistic}` |
| Peak live-byte delta, 100 concurrent streams | **≈226 KiB (0.22 MiB)** | `allocs::concurrent_footprint` |
| Allocations per `Session::builder().build()?` | **203 allocations** | `allocs::session_build` |
| Response-header materialize (8-header response), clone / move | **29 / 29 allocations** | `allocs::response_headers_*` |
| HPACK encode (via `hpack_vs_h2`) | 2.02 µs (≈239 MiB/s) | `hpack_vs_h2::leyline_encode` |
| HPACK decode (via `hpack_vs_h2`) | 6.69 µs (≈68 MiB/s) | `hpack_vs_h2::leyline_decode` |
| Peer comparison vs `h2` crate | deferred — h2 crate does not expose HPACK primitives publicly | `hpack_vs_h2::h2_peer_status` |

The multiplex pair is the headline proof of real multiplexing: 100 concurrent requests complete in ~250 µs where 100 sequential requests would take ~1.6 ms; concurrent throughput is **~6.4× serial throughput**, confirming the driver multiplexes streams rather than serialising them. (The ratio is smaller than on the prior WSL2 host because native serial IO is much faster — WSL2's per-wakeup overhead inflated the serial baseline; it does not mean multiplexing helps less.)

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
| 1 000 sequential GETs, one reused client | 46.73 ms (**21 400 req/s**) | 58.78 ms (**17 010 req/s**) | Leyline 1.26× |
| 100 concurrent GETs | 1.27 ms (**78 700 req/s**) | 1.11 ms (**90 100 req/s**) | reqwest 1.15× |

Both clients ran in the same process, on the same host, in the same run. The
**serial** result — the per-request client-stack cost — favours Leyline by
1.26×, consistent with the 1.29× measured on the prior Linux host. The
**concurrent** result is close and host-sensitive: on this Windows host reqwest
edges ahead by 1.15× where Leyline led on Linux, because the two pools fan out
across real loopback TCP sockets whose contention behaves differently per OS.
The serial number is the per-request stack cost (full connection reuse, no
multiplexing — H1 is one request per connection); the concurrent number fans out
across each client's connection pool. This is a loopback measurement with no
network latency and no TLS handshake. Over a real network, round-trip time
dominates and the two converge.

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
median of 3 interleaved runs on the 8-core desktop under WSL2, with the leyline and
wreq clients rebuilt from source at commit `e80fa9d`:

| Client | warm (seq latency) | **concurrent throughput** | cold (handshake) |
|---|---|---|---|
| leyline | 2 934 req/s | **46 724 req/s** | 746 req/s |
| wreq | 2 950 req/s | **42 880 req/s** | 861 req/s |
| bogdanfinn/tls-client | 3 537 req/s | 23 776 req/s | 756 req/s |
| azuretls | 3 462 req/s | 17 578 req/s | 809 req/s |

- **Concurrent throughput** — 64 requests in flight over one multiplexed H2
  connection — is the metric that compares these clients at the scale they are
  actually used. The two Rust BoringSSL clients lead: leyline sustains
  **~46 700 req/s, ~2.0× bogdanfinn/tls-client and ~2.7× azuretls**, neck-and-neck
  with wreq (its closest peer, same BoringSSL lineage). leyline and wreq are a
  statistical tie at the top: leyline takes the median this measurement (46.7k vs
  42.9k) and is faster in 2 of 3 interleaved rounds, but their ranges overlap
  (leyline 41.7–49.1k, wreq 42.8–48.0k), so the per-round lead trades inside the
  variance. The equivalence gate confirms leyline and wreq returned a
  byte-identical body (FNV-1a `5df04559e15e1bf4`, status 200), so the throughput
  is a comparison of equal work. Absolute throughput is host- and
  kernel-sensitive — the portable result is the **relative** standing (the two
  BoringSSL clients tied at the top, both clear of the Go utls clients), not the
  raw req/s. For a drift-cancelled, deployment-representative number the **paired
  harness** (`benches/comparison/paired.sh`) runs interleaved ABAB rounds with a
  paired t-test + 95% CI and the same equivalence gate against a real remote
  target through a real proxy (`TARGET_URL` + `PROXY`).
- **warm** is single-request-at-a-time latency. All four cluster in a narrow band
  and the Go clients edge ahead, but this number is **server-bound** — against its
  own mock peer leyline does a request in ~97 µs vs ~340 µs here, so most of the
  warm time is the shared Go server + socket + TLS, not the client. It is reported
  for completeness, not as a client ranking; sequential is the wrong regime for
  HTTP/2, whose entire purpose is multiplexing.
- **cold** builds a fresh client and a new TLS handshake per request. leyline is a
  touch behind. Against real public servers the per-build cost is cut sharply by
  the cached system-trust store (see the controlled A/B section above), but this
  harness hits a local self-signed server with non-system trust, so that path is
  not exercised here and the TLS handshake itself dominates. With the
  reuse-a-client norm cold never shows; it matters only if you discard clients.

Combined with the reqwest result: leyline tracks wreq on impersonated H2
throughput (statistical tie), beats the Go utls clients by 2.0–2.7×, and leads
reqwest on plain-HTTP serial — one library in the leading group at both jobs.
leyline's allocation and HPACK optimizations target specific per-request costs;
they are proven in the controlled A/B and allocation sections above and do not
visibly move this concurrent-loopback comparison, which is bottlenecked on the
shared TLS / socket / scheduler path the four clients exercise alike — the
deterministic alloc counts, not this number, are their receipt.

Caveats: loopback only (no network RTT, no real-world TLS endpoints); each library
impersonates its own newest Chrome, so the JA4s are Chrome-class but not identical;
warm/cold are sequential and warm is server-bound as noted. Reproduce with
[`benches/comparison/run.sh`](benches/comparison/run.sh).

## Connection-pool checkout scaling (controlled A/B)

`pool::checkout_at_occupancy/{1,64,512,2048}` drives `checkout_handle`'s exact
per-request body — `make_key` → `evict_idle` → `checkout_h2` — against a pool
pre-filled with N live entries (one mock H2 driver, handle cloned into every
entry, so each reports live and the idle sweep keeps them all). `evict_idle`
runs on every checkout and gates its O(entries) idle sweep behind a 250 ms reap
deadline, so warm checkout is **O(1) in pool occupancy**:

| Pool occupancy | Throttled (default) | Sweep-every-checkout (control) | Cost removed |
|---|---|---|---|
| 1 | 165.7 ns | 184.8 ns | −5.9% |
| 64 | 161.3 ns | 405.4 ns | −60% (2.5×) |
| 512 | 160.9 ns | 2 089 ns | −92.5% (13×) |
| 2 048 | 162.3 ns | 8 070 ns | −98.0% (50×) |

The control column disables the reap deadline so `evict_idle` scans on every
checkout; the gap is the per-request scan cost the throttle elides. At the
2048-entry LRU default — the warm-proxy fan-out the cap is sized for
([`pool.rs`](crates/leyline/src/pool/pool.rs)) — checkout holds flat at ~162 ns
(≈6.1 M checkouts/s) instead of climbing to ~8 µs.

Controlled A/B: both columns are measured back-to-back in one session via
`--save-baseline`, toggling only the throttle. The `pool::new` (≈53 ns),
`pool::with_limits` (≈53 ns), and `checkout_hit_equivalent` (≈32 ns) controls —
none of which touch the sweep — stay within ±3% across both runs, so the deltas
are attributable to the throttle, not host drift. Absolute ns here are from the
same native Windows host as the headline table; the **ratio** is host-independent
regardless.

## HPACK Huffman decode fast table (controlled A/B)

The HPACK Huffman decoder resolves codes of length 5..=8 — the common ASCII
case in URLs, tokens, and header text — through an 8-bit fast table (one index
per symbol), falling back to a per-length binary search only for the rare long
codes. Decode runs on every inbound response and carries no fingerprint surface
(leyline emits encoded headers, never decoded ones), so the only questions are
speed and byte-exact output.

| Bench (16 Chrome-147 headers) | Fast table | Per-length search (control) | Change |
|---|---|---|---|
| `hpack::decode_chrome_headers` | 5.18 µs | 7.45 µs | **−30.1%** |
| `hpack::roundtrip_chrome_headers` | 7.34 µs | 9.80 µs | −24.7% |
| `hpack::encode_chrome_headers` (control) | 2.25 µs | 2.22 µs | +1.3% (within noise) |

Controlled A/B: both sides measured back-to-back via `--save-baseline` with the
decoder git-toggled. `encode` — which the decode change does not touch — holds
within noise across both runs, so the decode delta is attributable to the fast
table, not host drift. Output is byte-identical, proven by encode→decode
roundtrip over all 256 bytes, all 65 536 byte pairs, control chars (30-bit
codes), and realistic header strings. Absolute µs are from the same native
Windows host as the headline table; the ratio is host-independent.

## Response-header materialization allocations

The HPACK dynamic/static table stores `Bytes`, so an indexed (table-hit)
response header materializes by a refcount clone (dynamic) or `Bytes::from_static`
(static) with no heap copy; only a literal value allocates. Response headers
flow to `Response::headers()` as `HeaderStr` (a `Bytes`-backed, `Deref<str>`
type), so the per-header `String` allocation is gone end-to-end. Counts are
exact and host-independent.

| Materializing an 8-header response (fresh decoder) | Allocations |
|---|---|
| `allocs::response_headers_clone` | 29 |
| `allocs::response_headers_move` | 29 |

Clone and move now cost the same — a `Bytes` clone is an atomic refcount bump,
not a buffer copy — so the old clone-vs-move gap (57 vs 43) collapses. On a
**warm** connection the win is larger: repeated response headers become
dynamic-table hits, dropping `per_request_realistic` (12 headers + 2 KiB body)
from 101 to **61 allocations** — measured against a persistent driver decoder
(the Bytes decoder plus fusing the request encode into one presized pass).

## Session build: cached system trust store (controlled A/B, Windows)

`Session::build` loads the OS trust roots into the connector. On Windows that
enumerates the system ROOT store via Win32 and DER-parses every cert — a
per-build cost dominated by the store walk, not allocation. Sharing one
pre-parsed, refcounted root store across builds (pure system trust, no additive
roots) collapses it.

| `Session::builder().browser(Chrome147).build()` | Cached | Per-build load (control) |
|---|---|---|
| `session::build_chrome147` (time) | 49 µs | 3.09 ms |
| `allocs::session_build` (count) | 203 | 279 |

Time drops ~63×; the gap is the Win32 ROOT-store enumeration the cache elides.
Controlled A/B with the cache git-toggled, same session. The shared store holds
only public CA roots (no per-session state) and trust roots are never on the
wire, so JA4 is unchanged — the live 16-profile JA4 + Akamai matrix passes with
the cache active. Absolute numbers are from a Windows 11 native host.

## Host sensitivity

These numbers are from an 8-core desktop on native Windows. Two classes of bench
scale differently with the host:

- **Allocation-bound** (HPACK encode, JA3/JA4 audit math — these build strings)
  run somewhat slower than the same CPU under Linux because the Windows system
  allocator is costlier for small allocations. Pure-compute benches (frame
  parsing, `H2Client::clone`) are unaffected.
- **IO-bound** (`multiplex::serial_1k_requests`, `concurrent_100_inflight`) run
  materially faster than under WSL2, which carried per-wakeup overhead on the
  `tokio::io::duplex` round-trips. The allocation **counts** these benches
  report (33 per request, 203 per session build) are host-independent and are
  the figures to track for regressions — wall-time is not comparable across
  hosts.

## Scope and caveats

- The H2 benches run against in-process mock peers over `tokio::io::duplex` — zero network latency, zero TLS handshake. The `http_vs_reqwest` bench uses a real loopback TCP socket (reqwest cannot attach to a duplex pipe) but still no external network or TLS. Real-network and handshake costs are excluded so these numbers reflect each client's own code paths.
- Single-machine numbers only. Multi-host workloads shift with kernel TCP, NIC offload, and syscall cost.
- HPACK figures are for a realistic Chrome-147 navigation request header set (16 headers, ≈508 B raw). Larger or smaller header blocks scale roughly linearly.
- Peer comparison against the `h2` crate's HPACK is **deferred** — see `benches/benches/hpack_vs_h2.rs`. The `h2` crate (v0.4) keeps its HPACK module private, so a real peer bench would require either forking `h2` or running the full `h2::client::handshake` (which measures far more than HPACK).
- The `reqwest` comparison is plaintext HTTP/1.1 only. It does not compare TLS handshake, HTTP/2, or fingerprint control — leyline's reason to exist — because reqwest does not expose those as a like-for-like surface. It answers one question: bare HTTP request overhead, client stack vs client stack.
- `allocs::session_build` records 5 364 allocations on iteration 1 (`LazyLock` profile-registry materialisation plus the one-time system-trust-store cache population) and drops to a stable 203 allocations on every subsequent iteration. The headline value is the steady-state number.
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
| `benches/allocs.rs` | `per_request_tiny`, `per_request_realistic`, `concurrent_footprint`, `session_build`, `response_headers_clone`, `response_headers_move` |
| `benches/pool.rs` | `new`, `with_limits`, `checkout_hit_equivalent`, `checkout_at_occupancy/{1,64,512,2048}` |
