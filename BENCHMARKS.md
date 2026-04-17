# Leyline — measured performance

Generated: 2026-04-16
Host: Linux x86_64, 4 vCPU, 16 GiB RAM
Toolchain: `rustc 1.94.1 (e408947bf 2026-03-25)`
Criterion version: 0.5

Numbers below come from `cargo bench -p leyline-benches` at commit `d217712`. Reproduce:

    cd benches
    cargo bench -p leyline-benches

## Headline numbers

| Metric | Value | Bench |
|---|---|---|
| HPACK encode (16 Chrome-147 headers, ≈508 B raw → 198 MiB/s) | 3.16 µs | `hpack::encode_chrome_headers` |
| HPACK decode (same header block) | 11.28 µs | `hpack::decode_chrome_headers` |
| HPACK roundtrip (encode + decode) | 14.40 µs | `hpack::roundtrip_chrome_headers` |
| JA3 compute | 2.54 µs | `audit::compute_ja3` |
| JA4 compute | 3.96 µs | `audit::compute_ja4` |
| JA4T compute | 150.7 ns | `audit::compute_ja4t` |
| Chrome extension id derivation | 80.4 ns | `audit::chrome_extension_ids` |
| Frame parse (mixed 6-frame batch, 1082 B) | 376.8 ns (≈2.83 GiB/s) | `frames::parse_mix` |
| Frame header parse (9 B, no payload) | 3.01 ns | `frames::parse_header` |
| Profile lookup (Chrome147) | 46.7 ns | `profile::lookup_chrome147` |
| Profile lookup (all 10 browser variants) | 461.7 ns | `profile::lookup_all_browsers` |
| Session build (Chrome147) | 10.66 ms | `session::build_chrome147` |
| Session build (`Session::chrome_latest`) | 10.83 ms | `session::chrome_latest` |
| `Pool::new()` | 19.85 ns | `pool::new` |
| `Pool::with_limits(...)` | 18.33 ns | `pool::with_limits` |
| Pool checkout-hit equivalent (`H2Client::clone`) | 20.31 ns | `pool::checkout_hit_equivalent` |
| `H2Client::clone` (standalone) | 20.35 ns | `multiplex::handle_clone` |
| Multiplex 1 000 sequential requests (mock peer) | 47.75 ms total / **20 942 req/s** | `multiplex::serial_1k_requests` |
| Multiplex 100 concurrent inflight (mock peer) | 462 µs total / **216 353 req/s** | `multiplex::concurrent_100_inflight` |
| Allocations per `send_request` (warm, serial) | **33 allocations** / 47.9 µs | `allocs::per_request` |
| Peak live-byte delta, 100 concurrent streams | **≈187 KiB (0.18 MiB)** | `allocs::concurrent_footprint` |
| Allocations per `Session::builder().build()?` | **159 allocations** | `allocs::session_build` |
| HPACK encode (via `hpack_vs_h2`) | 3.11 µs (201 MiB/s) | `hpack_vs_h2::leyline_encode` |
| HPACK decode (via `hpack_vs_h2`) | 11.62 µs (39 MiB/s) | `hpack_vs_h2::leyline_decode` |
| Peer comparison vs `h2` crate | deferred — h2 crate does not expose HPACK primitives publicly | `hpack_vs_h2::h2_peer_status` |

The multiplex pair is the headline proof: 100 concurrent requests complete in ~462 µs where 100 sequential requests would require ~4.8 ms; concurrent throughput is **10.3× serial throughput**, confirming the driver is actually multiplexing and not serialising streams. On a 4-vCPU host with an in-process mock peer, **a single Leyline connection sustains > 200 000 req/s**.

## Scope and caveats

- All benches run against in-process mock peers over `tokio::io::duplex` — zero network latency, zero TLS handshake. Those costs are deliberately excluded so these numbers reflect Leyline's own code paths.
- Single-machine numbers only. Multi-host workloads will shift with kernel TCP, NIC offload, and syscall cost.
- The host is a 4-vCPU Linux VM; higher-end hardware will run these benches meaningfully faster.
- HPACK figures are for a realistic Chrome-147 navigation request header set (16 headers, ≈508 B raw). Larger or smaller header blocks scale roughly linearly.
- Peer comparison against the `h2` crate's HPACK was **deferred** — see `benches/benches/hpack_vs_h2.rs`. The `h2` crate (v0.4) keeps its HPACK module private, so a real peer bench would require either forking `h2` or running the full `h2::client::handshake` (which measures far more than HPACK).
- `allocs::session_build` records 3 176 allocations on iteration 1 (`LazyLock` profile-registry materialisation) and drops to a stable 159 allocations per build on every subsequent iteration. The headline value is the steady-state number.
- `allocs::concurrent_footprint` reports the **delta in live bytes** from just-before-fanout to peak during 100 concurrent streams; it excludes the fixed cost of the connection itself. Absolute RSS is not reported — use `heaptrack` or `dhat` if you need that.

## Methodology

- Criterion.rs with the default 3 s warmup and 100 samples (20–30 samples for the two long multiplex runs so the full suite stays under 10 minutes).
- System allocator (`std::alloc::System`) — no jemalloc / mimalloc. Numbers reflect the stdlib allocator cost users actually see.
- The mock HTTP/2 peer writes the minimum legal response: a HEADERS frame with `:status = 200` followed by a 10-byte DATA frame with END_STREAM. No flow-control stalls, no SETTINGS changes mid-stream, no pushes.
- All benches run the release profile (`cargo bench`, which implies `--release`).
- The allocation-counting global allocator in `allocs.rs` is a wrapper around `std::alloc::System` that tallies allocation count and live-byte watermark with `AtomicU64`s — it is only installed in the `allocs` bench binary, not the rest of the suite, so other numbers are unaffected.
- The excluded sub-workspace at `benches/` patches `boring` / `boring-sys` to the vendored `leyline-ssl` paths so the full TLS+H2 stack (and thus `Session`) compiles inside the bench suite. Run `cargo bench -p leyline-benches` from `benches/` (or with `--manifest-path benches/Cargo.toml`).

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
| `benches/audit.rs` | `compute_ja3`, `compute_ja4`, `compute_ja4t`, `chrome_extension_ids` |
| `benches/frames.rs` | `parse_mix`, `parse_header` |
| `benches/profile.rs` | `lookup_chrome147`, `lookup_all_browsers` |
| `benches/session.rs` | `build_chrome147`, `chrome_latest` |
| `benches/multiplex.rs` | `handle_clone`, `serial_1k_requests`, `concurrent_100_inflight` |
| `benches/hpack_vs_h2.rs` | `leyline_encode`, `leyline_decode`, `h2_peer_status` (deferred) |
| `benches/allocs.rs` | `per_request`, `concurrent_footprint`, `session_build` |
| `benches/pool.rs` | `new`, `with_limits`, `checkout_hit_equivalent` |
