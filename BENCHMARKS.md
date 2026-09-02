# Benchmarks

`benches/benches/clients.rs` races Leyline against `reqwest` 0.13 (rustls) over
one TLS origin that the benchmark starts inside its own process.

## Machine

| Field | Value |
| --- | --- |
| CPU | Apple M2 Max, 12 cores |
| OS | macOS 26.5.1 (build 25F80) |
| Toolchain | rustc 1.98.0 (88d9e12ae 2026-08-18) |
| Commit | `e47d399` |
| 1-minute load average during the run | 23.7 to 30.3 |

The machine was not idle: other builds ran on it throughout, sampled once every
5 seconds with `uptime`. Treat the numbers as one contended run, not a quiet-box
figure.

Collected with:

```
sysctl -n machdep.cpu.brand_string
sw_vers
git rev-parse --short HEAD
```

## Command

```
cargo bench --manifest-path benches/Cargo.toml --bench clients
```

## Methodology

The benchmark owns the server. It generates a self-signed leaf with `rcgen`
(SAN `localhost` and `127.0.0.1`) and binds two loopback TLS listeners, each
built from hyper 1.x on `tokio-rustls`:

- one advertising ALPN `http/1.1`, served by `hyper::server::conn::http1` with
  keep-alive on;
- one advertising ALPN `h2`, served by `hyper::server::conn::http2`.

Server-side ALPN is what pins each scenario to one protocol. Both clients trust
the leaf through their own API: `danger_accept_invalid_certs(true)` on the
Leyline `SessionBuilder` and on the `reqwest::ClientBuilder`. That is a
benchmark shortcut. It removes trust-store and chain-verification work from
both stacks equally and keeps the measured delta on the client HTTP path.

**Byte-equivalence.** Before any timing, `verify` runs every scenario once per
client and asserts that the response body is byte-equal to the server's fixture
(`(0..len).map(|i| (i % 251) as u8)`, 16 KiB and 4 MiB), and that the
negotiated protocol version is the one the listener advertised. A mismatch
panics and the benchmark fails; no numbers are produced.

**Numbers.** Criterion times each scenario with `iter_custom`. Median and p95
are computed from the per-sample times in
`benches/target/criterion/clients/<name>/new/sample.json`, as
`times[i] / iters[i]` — the wall time of one scenario execution. The p95 is the
95th percentile of those per-sample values with linear interpolation. The three
scenarios are:

- `h1_16k` — 200 sequential keep-alive GETs of the 16 KiB body on one reused
  client;
- `h2_32x16k` — 32 concurrent GETs of the 16 KiB body, multiplexed over one
  HTTP/2 connection;
- `stream_4mib` — one 4 MiB body consumed chunk by chunk through each client's
  streaming API (`Response::into_stream` for Leyline, `bytes_stream` for
  reqwest).

**wreq is absent.** `wreq` depends on `btls-sys`, which declares
`links = "boringssl"`, and so does `leyline-bssl-sys`. Cargo refuses to resolve
two `links = "boringssl"` packages into one dependency graph, so wreq cannot be
linked into this benchmark binary at all. The out-of-process wreq comparison
lives in `benches/comparison/`.

## Results

Run of 2026-09-01. `median` and `p95` are per-scenario-execution wall times
from `sample.json`; `criterion` is criterion's own point estimate for the same
benchmark.

| Scenario | Client | Median | p95 | Criterion estimate | Samples |
| --- | --- | ---: | ---: | ---: | ---: |
| 200 sequential H1 GETs, 16 KiB | leyline | 12.105 ms | 18.544 ms | 12.978 ms | 20 |
| 200 sequential H1 GETs, 16 KiB | reqwest 0.13 | 35.525 ms | 44.195 ms | 38.133 ms | 20 |
| 32 concurrent H2 GETs, 16 KiB | leyline | 3.855 ms | 5.151 ms | 3.584 ms | 30 |
| 32 concurrent H2 GETs, 16 KiB | reqwest 0.13 | 1.523 ms | 2.049 ms | 1.535 ms | 30 |
| 4 MiB streamed download | leyline | 10.953 ms | 16.966 ms | 7.933 ms | 20 |
| 4 MiB streamed download | reqwest 0.13 | 17.226 ms | 30.582 ms | 22.546 ms | 20 |

Derived from the medians:

| Scenario | leyline | reqwest 0.13 |
| --- | ---: | ---: |
| H1 keep-alive, per request | 60.5 us | 177.6 us |
| H2 multiplexed, per request | 120.5 us | 47.6 us |
| 4 MiB stream, throughput | 365 MiB/s | 232 MiB/s |

Criterion summary lines from the run:

```
clients/leyline_h1_16k      time:   [11.966 ms 12.978 ms 14.599 ms]
                            thrpt:  [13.699 Kelem/s 15.411 Kelem/s 16.714 Kelem/s]
clients/reqwest_h1_16k      time:   [36.515 ms 38.133 ms 39.834 ms]
                            thrpt:  [5.0208 Kelem/s 5.2448 Kelem/s 5.4772 Kelem/s]
clients/leyline_h2_32x16k   time:   [3.1353 ms 3.5838 ms 4.0747 ms]
                            thrpt:  [7.8533 Kelem/s 8.9290 Kelem/s 10.206 Kelem/s]
clients/reqwest_h2_32x16k   time:   [1.4483 ms 1.5351 ms 1.6338 ms]
                            thrpt:  [19.586 Kelem/s 20.846 Kelem/s 22.095 Kelem/s]
clients/leyline_stream_4mib time:   [5.9239 ms 7.9326 ms 10.972 ms]
                            thrpt:  [364.55 MiB/s 504.25 MiB/s 675.23 MiB/s]
clients/reqwest_stream_4mib time:   [16.127 ms 22.546 ms 30.457 ms]
                            thrpt:  [131.33 MiB/s 177.42 MiB/s 248.04 MiB/s]
```

The spread between the median and criterion's estimate for
`leyline_stream_4mib` is wide (10.953 ms against 7.933 ms), and the H2 and
stream intervals are wide as well. Both follow from the machine load recorded
above. Rerun on an idle machine before drawing a conclusion from those two
rows.

## Rerun after HTTP/2 write batching

Commit `d7d095b` (accumulating frame writer, one flush per event-loop turn, persistent read buffer). Same command, same machine, 1-minute load average 17.7 to 23.4 during the run, so treat these as directional.

| Scenario | Client | Criterion estimate | Before |
| --- | --- | ---: | ---: |
| 32 concurrent H2 GETs, 16 KiB | leyline | 2.329 ms | 3.584 ms |
| 32 concurrent H2 GETs, 16 KiB | reqwest 0.13 | 1.290 ms | 1.535 ms |

The write probe in `crates/leyline/tests/h2_write_batching.rs` counts one transport write for the request phase of 8 concurrent requests, where the previous driver made 8.

## Rerun after the per-request allocation pass

Branch `map/p1b-h2-overhead` (shared request head, HPACK static-table index,
`content-length`-sized response buffer). Same command and machine as the write
batching rerun. The 1-minute load average ranged from 11 to 39 across the runs,
so the wall-clock rows below are directional, not a measurement of the change.

| Scenario | Client | Before | After |
| --- | --- | ---: | ---: |
| 32 concurrent H2 GETs, 16 KiB | leyline | 1.139 ms | 1.126 ms |
| 32 concurrent H2 GETs, 16 KiB | reqwest 0.13 | 771 us | 803 us |

Those two runs are 10 minutes apart on a loaded machine and both clients moved
by more than the difference between them: read the pair as "the ratio to
reqwest did not change", not as a speedup. The wall-clock gap on this scenario
is still about 1.4x reqwest.

Two numbers from the run are load-independent:

```
hpack/encode_chrome_headers   before: [2.0318 us 2.0370 us 2.0424 us]
hpack/encode_chrome_headers   after:  [1.6472 us 1.6555 us 1.6669 us]
                                      change: -18.3% (p = 0.00 < 0.05)
allocs/per_request_tiny       before: 17.0 allocs/request   after: 17.0
allocs/per_request_realistic  before: 57.8 allocs/request   after: 57.8
```

The `allocs` bench drives `H2Client::send_request` directly, so it does not
cover the pooled `pool::send_request` path where the pseudo-header and header
clones were removed; those requests now share one `Arc<Head>` between the
pooled attempt and its retry instead of cloning one `String` per pseudo-header
and two per header.
