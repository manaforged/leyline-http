# Leyline — measured performance

Generated: 2026-06-09 (full re-run)
Host: Linux x86_64, 4 vCPU, 16 GiB RAM
Toolchain: `rustc 1.94.1 (e408947bf 2026-03-25)`
Criterion version: 0.5

Numbers below come from `cargo bench -p leyline-benches`. Reproduce:

    cd benches
    cargo bench -p leyline-benches

## Headline numbers

| Metric | Value | Bench |
|---|---|---|
| HPACK encode (16 Chrome-147 headers, ≈508 B raw → 306 MiB/s) | 2.05 µs | `hpack::encode_chrome_headers` |
| HPACK decode (same header block) | 9.26 µs (≈49 MiB/s) | `hpack::decode_chrome_headers` |
| HPACK roundtrip (encode + decode) | 11.16 µs | `hpack::roundtrip_chrome_headers` |
| JA3 compute | 1.81 µs | `audit::compute_ja3` |
| JA4 compute | 2.95 µs | `audit::compute_ja4` |
| JA4T compute | 103.1 ns | `audit::compute_ja4t` |
| JA4H compute (16 Chrome-147 nav headers) | 937.8 ns | `audit::compute_ja4h` |
| Chrome extension id derivation | 66.9 ns | `audit::chrome_extension_ids` |
| Frame parse (mixed 6-frame batch, 1082 B) | 293.6 ns (≈3.63 GiB/s) | `frames::parse_mix` |
| Frame header parse (9 B, no payload) | 1.60 ns | `frames::parse_header` |
| Profile lookup (Chrome147) | 27.8 ns | `profile::lookup_chrome147` |
| Profile lookup (all 15 browser variants) | 466.0 ns | `profile::lookup_all_browsers` |
| Session build (Chrome147) | 16.24 ms | `session::build_chrome147` |
| Session build (`Session::chrome`) | 17.18 ms | `session::chrome` |
| `Pool::new()` | 15.13 ns | `pool::new` |
| `Pool::with_limits(...)` | 14.89 ns | `pool::with_limits` |
| Pool checkout-hit equivalent (`H2Client::clone`) | 46.6 ns | `pool::checkout_hit_equivalent` |
| `H2Client::clone` (standalone) | 47.8 ns | `multiplex::handle_clone` |
| Multiplex 1 000 sequential requests (mock peer) | 25.92 ms total / **38 574 req/s** | `multiplex::serial_1k_requests` |
| Multiplex 100 concurrent inflight (mock peer) | 419 µs total / **238 670 req/s** | `multiplex::concurrent_100_inflight` |
| Allocations per `send_request` (warm, serial) | **33 allocations** / 26.16 µs | `allocs::per_request` |
| Peak live-byte delta, 100 concurrent streams | **≈187 KiB (0.18 MiB)** | `allocs::concurrent_footprint` |
| Allocations per `Session::builder().build()?` | **1 640 allocations** | `allocs::session_build` |
| HPACK encode (via `hpack_vs_h2`) | 2.13 µs (≈294 MiB/s) | `hpack_vs_h2::leyline_encode` |
| HPACK decode (via `hpack_vs_h2`) | 9.23 µs (≈49 MiB/s) | `hpack_vs_h2::leyline_decode` |
| Peer comparison vs `h2` crate | deferred — h2 crate does not expose HPACK primitives publicly | `hpack_vs_h2::h2_peer_status` |

The multiplex pair is the headline proof: 100 concurrent requests complete in ~419 µs where 100 sequential requests would require ~2.6 ms; concurrent throughput is **~6.2× serial throughput**, confirming the driver is actually multiplexing and not serialising streams. On a 4-vCPU host with an in-process mock peer, **a single Leyline connection sustains > 230 000 req/s**.


## Scope and caveats

- All benches run against in-process mock peers over `tokio::io::duplex` — zero network latency, zero TLS handshake. Those costs are deliberately excluded so these numbers reflect Leyline's own code paths.
- Single-machine numbers only. Multi-host workloads will shift with kernel TCP, NIC offload, and syscall cost.
- The host is a 4-vCPU Linux VM; higher-end hardware will run these benches meaningfully faster.
- HPACK figures are for a realistic Chrome-147 navigation request header set (16 headers, ≈508 B raw). Larger or smaller header blocks scale roughly linearly.
- Peer comparison against the `h2` crate's HPACK was **deferred** — see `benches/benches/hpack_vs_h2.rs`. The `h2` crate (v0.4) keeps its HPACK module private, so a real peer bench would require either forking `h2` or running the full `h2::client::handshake` (which measures far more than HPACK).
- `allocs::session_build` records 6 419 allocations on iteration 1 (`LazyLock` profile-registry materialisation) and drops to a stable 1 640 allocations per build on every subsequent iteration. The headline value is the steady-state number. (Both grew vs the 2026-04-16 baseline — see the changes note below.)
- `allocs::concurrent_footprint` reports the **delta in live bytes** from just-before-fanout to peak during 100 concurrent streams; it excludes the fixed cost of the connection itself. Absolute RSS is not reported — use `heaptrack` or `dhat` if you need that.

## Methodology

- Criterion.rs with the default 3 s warmup and 100 samples (20–30 samples for the two long multiplex runs so the full suite stays under 10 minutes).
- System allocator (`std::alloc::System`) — no jemalloc / mimalloc. Numbers reflect the stdlib allocator cost users actually see.
- The mock HTTP/2 peer writes the minimum legal response: a HEADERS frame with `:status = 200` followed by a 10-byte DATA frame with END_STREAM. No flow-control stalls, no SETTINGS changes mid-stream, no pushes.
- All benches run the release profile (`cargo bench`, which implies `--release`).
- The allocation-counting global allocator in `allocs.rs` is a wrapper around `std::alloc::System` that tallies allocation count and live-byte watermark with `AtomicU64`s — it is only installed in the `allocs` bench binary, not the rest of the suite, so other numbers are unaffected.
- The excluded sub-workspace at `benches/` mirrors the repo-root `[patch.crates-io] btls-sys = { path = "../crates/btls-sys" }` so the full TLS+H2 stack (and thus `Session`) links the prebuilt BoringSSL shim instead of source-building it. Run `cargo bench -p leyline-benches` from `benches/` (or with `--manifest-path benches/Cargo.toml`).

## How to extend

To add a new bench:

1. Drop a new file in `benches/benches/` that registers a `criterion_group!` and `criterion_main!`.
2. Add a `[[bench]]` entry to `benches/Cargo.toml` (with `harness = false`).
3. For async benches, build a `tokio::runtime::Runtime` once inside the bench function and drive it with `iter_custom` + `rt.block_on(...)` — avoid Criterion's `#[tokio::test]` / `tokio::test` crate integrations.
4. Re-run the full suite and update this document. Benchmarks should be deterministic and complete in under 60 s each; the multiplex benches intentionally run longer because the workload is 1 000 requests per iteration.

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
| `benches/allocs.rs` | `per_request`, `concurrent_footprint`, `session_build` |
| `benches/pool.rs` | `new`, `with_limits`, `checkout_hit_equivalent` |
