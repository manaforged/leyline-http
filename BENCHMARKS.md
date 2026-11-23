# Benchmarks

This page records client-library comparisons on one machine. Each section links
to its raw results and identifies the configuration under measurement.

## v0.3.0 measurements

All v0.3.0 suites below ran on 2026-11-23 over loopback on a Ryzen 9 9950X3D
with the CPU frequency governor set to `performance`, the origin pinned to CPUs
`3-7,19-23` (CCD 0), clients pinned to CPUs `8-15,24-31` (CCD 1), verified TLS
1.3 (AES-128-GCM, X25519) against a local test CA, a 10-byte response, and a
30-second request timeout. The measured core is commit `e6030d83`
(`leyline_source_hash` `8ee13109` in every receipt); peers are hyper-util
0.1.21, hyper 1.12.0, reqwest 0.13.5, h2 0.4.20, wreq 0.16.3, libcurl 8.22.0,
ureq 3.5.0, Go 1.27.1 `net/http`, fasthttp 1.77.0, and tls-client 1.16.0.

### Browser profiles over HTTP/2

Three independent runs used up to four alternating-order rounds each (`STOP_REF=leyline`
stops decided rivals after round 3 when both req/s and CPU/req differ by more
than 5%), Chrome 146 profiles, verified TLS, and a 30-second timeout.

Requests per second are medians over the 9–12 observations per client and workload:

| Workload | Leyline | wreq | tls-client |
| --- | ---: | ---: | ---: |
| Sequential, one connection | 55,453 | 35,499 | 30,240 |
| 1 connection, 64 streams | 186,302 | 157,459 | 45,062 |
| 8 connections, 8 in flight | 286,466 | 216,059 | 114,252 |
| 8 connections, 256 in flight | 1,128,037 | 720,060 | 190,064 |

Paired throughput ratios use the geometric mean, with a 95% t-interval across
three independent run means. A ratio above 1 favors Leyline; these ratios are
not ratios of the pooled medians above.

| Workload | Leyline / wreq [95% interval] | Leyline / tls-client [95% interval] |
| --- | ---: | ---: |
| Sequential, one connection | 1.548 [1.495, 1.604] | 1.801 [1.767, 1.836] |
| 1 connection, 64 streams | 1.186 [1.148, 1.226] | 4.078 [3.879, 4.288] |
| 8 connections, 8 in flight | 1.326 [1.312, 1.341] | 2.522 [2.480, 2.564] |
| 8 connections, 256 in flight | 1.571 [1.495, 1.651] | 5.917 [5.687, 6.156] |

Leyline completes more requests per second and uses less client CPU per request
than both peers in every cell (0.364–0.395× wreq's CPU and 0.114–0.297×
tls-client's CPU). The
[pooled table](benches/comparison/results/2026-11-23-v030-browser-h2-r1/pooled.txt)
includes CPU intervals, RSS, and instruction counts. Raw observations,
request-header audits, observed connection counts, and binary hashes are in
[run 1](benches/comparison/results/2026-11-23-v030-browser-h2-r1),
[run 2](benches/comparison/results/2026-11-23-v030-browser-h2-r2), and
[run 3](benches/comparison/results/2026-11-23-v030-browser-h2-r3).

These are native browser-profile defaults, not identical browser identities:
Leyline uses Windows headers, wreq uses macOS headers, and tls-client receives
Leyline's captured header values. The gate requires the same end-to-end header
names; values remain visible in each audit. This result does not establish a
universal client ranking, a tail-latency lead, or performance on HTTP/3, large
bodies, or other hardware.

The HTTP/1.1 browser-profile comparison with wreq was not measured: its
`priority` header differs from Leyline's header set, so the equality gate stopped
the run. The [failed header audit](benches/comparison/results/2026-11-03-verified-browser-h1-audit/headers.txt)
is retained rather than changing either fingerprint for the benchmark.

### General clients over HTTP/2

Three independent runs used up to four alternating-order rounds each, bare
profiles, identical request-header values, verified TLS, a 10-byte response,
and a 30-second request timeout. `Leyline` is the default layout: one
multi-threaded Tokio runtime and one shared session. `Leyline, one runtime per
core` is the same client run through `SessionBuilder::run_per_core`; for HTTP/2
the harness caps its runtime count at the connection count, so every client
holds the stated number of connections.

Requests per second, medians over 9–12 observations:

| Client | Sequential | 1 conn, 64 streams | 8 conns, 8 in flight | 8 conns, 256 in flight |
| --- | ---: | ---: | ---: | ---: |
| Leyline | 57,985 | 212,966 | 301,780 | 1,430,815 |
| Leyline, one runtime per core | 56,530 | 174,964 | 350,655 | 1,412,832 |
| hyper-util client | 45,010 | 191,293 | 285,213 | 1,120,042 |
| libcurl (multi) | 46,682 | 103,161 | 246,353 | 786,210 |
| reqwest | 36,318 | 189,194 | 244,532 | 898,811 |
| Go net/http | 41,835 | 95,388 | 244,032 | 619,826 |
| h2 crate (no client layer) | 56,117 | 181,353 | 302,532 | 1,166,876 |

Paired throughput ratios, Leyline over the peer, with a 95% t-interval across
the three run means:

| Workload | hyper | reqwest | h2 crate |
| --- | ---: | ---: | ---: |
| Sequential | 1.278 [1.070, 1.527] | 1.615 [1.538, 1.695] | 1.033 [0.999, 1.069] |
| 1 conn, 64 streams | 1.120 [1.113, 1.128] | 1.125 [1.102, 1.149] | 1.176 [1.143, 1.209] |
| 8 conns, 8 in flight | 1.079 [0.993, 1.172] | 1.238 [1.155, 1.327] | 1.001 [0.924, 1.085] |
| 8 conns, 256 in flight | 1.281 [1.255, 1.307] | 1.592 [1.561, 1.625] | 1.228 [1.187, 1.271] |

Leyline leads every full client on sequential, 64-stream, and 256-in-flight
HTTP/2 workloads and ties hyper-util at `8×8` on the default runtime (where
one runtime per core leads hyper-util by 1.239 [1.200, 1.279] and the bare
`h2` crate by 1.149 [1.136, 1.163]). Against the bare `h2` crate (which has no
pool, headers, or timeouts), Leyline ties on sequential requests and at 8 in
flight and leads by 17.6% at 64 streams and 22.8% at 256 in flight while
executing 2,900–3,250 fewer user-space instructions per request in every cell.
Leyline uses less client CPU per request than every full client in every
workload (0.425–0.676× hyper-util, 0.329–0.507× reqwest) and 0.540–0.564× the
bare `h2` crate's CPU on the concurrent cells. On one connection (`1×64`),
single-core `run_per_core` trails multi-threaded hyper-util in throughput
(0.907 [0.885, 0.931]) while using 0.249× its CPU per request.

### General clients over HTTP/1.1

The same method ran the HTTP/1.1 clients. `Leyline blocking` is the blocking
API. Requests per second, medians over 9–12 observations:

| Client | Sequential | 8 workers | 64 workers |
| --- | ---: | ---: | ---: |
| Leyline | 66,674 | 352,853 | 857,582 |
| Leyline, one runtime per core | 66,430 | 408,288 | 857,194 |
| Leyline blocking | 67,339 | 417,795 | 875,486 |
| fasthttp | 67,293 | 362,396 | 911,555 |
| Go net/http | 45,691 | 452,981 | 577,307 |
| hyper-util client | 61,332 | 328,592 | 832,750 |
| reqwest | 39,436 | 271,419 | 627,133 |
| libcurl (multi) | 55,315 | 111,519 | 106,417 |
| ureq | 37,022 | 68,185 | 46,844 |

A longer follow-up repeated the 8- and 64-worker workloads for the closest
clients with up to 1,000,000 requests per half-pair:

| Client | 8 workers | 64 workers |
| --- | ---: | ---: |
| Leyline | 359,940 | 877,847 |
| Leyline, one runtime per core | 410,715 | 911,560 |
| Leyline blocking | 409,281 | 857,302 |
| fasthttp | 359,943 | 888,257 |
| Go net/http | 361,839 | 615,786 |
| hyper-util client | 328,702 | 789,854 |

Paired throughput ratios against fasthttp, the fastest peer across both worker
counts:

| Workload | Leyline | One runtime per core | Leyline blocking |
| --- | ---: | ---: | ---: |
| Sequential | 0.994 [0.969, 1.019] | 0.974 [0.923, 1.028] | 1.001 [0.951, 1.053] |
| 8 workers | 0.944 [0.827, 1.079] | 1.119 [1.082, 1.157] | 1.147 [1.133, 1.162] |
| 64 workers | 0.952 [0.932, 0.973] | 0.962 [0.836, 1.108] | 0.965 [0.847, 1.099] |
| 8 workers, long | 1.009 [0.943, 1.079] | 1.124 [1.096, 1.153] | 1.142 [1.105, 1.180] |
| 64 workers, long | 0.978 [0.925, 1.034] | 1.018 [0.982, 1.056] | 0.965 [0.924, 1.009] |

This is not an across-the-board HTTP/1.1 lead. One runtime per core reaches
911,560 req/s in the long 64-worker run, leads fasthttp at 8 workers
(+11.9–12.4%), and ties it at 64 workers while using 17–21% less client CPU
per request. The blocking API leads fasthttp at 8 workers (+14.2–14.7%) and
ties it at 64. The default shared runtime ties fasthttp on sequential, 8-worker,
and long 64-worker runs while trailing it in the short 64-worker run (0.952×)
despite using 18% less CPU per request. At 8 workers Go net/http opened 15–25
connections for 8 workers (`conns.txt`) and led default Leyline in the short
run (0.831 [0.737, 0.938]) before dropping to a tie in the long run
(0.913 [0.683, 1.220]) and trailing every Leyline layout at 64 workers by
48–52%. Every Leyline layout is ahead of hyper-util, reqwest, libcurl, and
ureq at every worker count.

The pooled tables for [HTTP/2](benches/comparison/results/2026-11-23-v030-clients-h2-r1/pooled.txt),
[HTTP/1.1](benches/comparison/results/2026-11-23-v030-clients-h1-r1/pooled.txt), and
the [longer HTTP/1.1 runs](benches/comparison/results/2026-11-23-v030-h1-long-r1/pooled.txt),
and the raw runs under `benches/comparison/results/2026-11-23-v030-*`, include
instructions, CPU, RSS, connection counts, header audits, and receipts for each
run. To reproduce all six suites and regenerate the charts from `benches/comparison`:

```bash
export CMP_CA=$PWD/pki/ca.der CMP_CERT=$PWD/pki/server.der CMP_KEY=$PWD/pki/server-key.der
SERVER_CPUS=3-7,19-23 CLIENT_CPUS=8-15,24-31 ./release-matrix.sh
```

`libs.sh` sizes each client's request count to `TARGET_MS` (500 ms in
`release-matrix.sh`, default 1,000 ms) during calibration and caches sizes
across runs 1–3; every row records the exact counts it used.

### Fresh sessions and closed-loop latency (v0.2.0 reference)

The paired runner measured one connection and 64 streams on v0.2.0 (`bde05ee7`),
with 2,000 warm requests, 200 fresh clients, and 200,000 concurrent requests per
round. Each peer has three batches of four alternating-order rounds. The wreq
pair uses native Chrome 146 profiles; the reqwest pair uses bare profiles and
identical request headers. Both use verified TLS and a 30-second timeout.

| Peer and configuration | Leyline p99 µs | Peer p99 µs | p99 ratio [95% interval] | Fresh-client throughput ratio [95% interval] |
| --- | ---: | ---: | ---: | ---: |
| wreq, Chrome 146 | 557 | 585.5 | 0.946 [0.910, 0.982] | 0.975 [0.871, 1.092] |
| reqwest, bare | 561 | 519.5 | 1.075 [1.036, 1.116] | 0.966 [0.875, 1.067] |

The p99 values are medians of per-round p99 observations, not a percentile of
pooled requests. Ratios use the table tool's log-ratio interval across three
batch means. Leyline's p99 is lower than wreq's in this workload but higher
than reqwest's. Fresh-client throughput is unresolved against both peers. These
are closed-loop measurements without offered-load pacing or coordinated-omission
correction. The [raw JSON and derived table](benches/comparison/results/2026-11-03-verified-latency)
retain every round.

The [verification probes](benches/comparison/results/2026-11-03-verification)
record trusted-CA acceptance, wrong-CA rejection, and refusal to pool results
with missing trust metadata.

### Open-loop latency

Every client got the same offered rate after a 1-second warm-up (10 seconds at
2,000 req/s; 5 seconds at 20,000 and 50,000 req/s, giving 100k–250k measured
samples per round and 0.9M–2.25M across 9 rounds), as the [method](#method)
describes. Three independent runs used three alternating-order rounds each, bare
profiles, identical request headers, a 10-byte response, verified TLS against a
local test CA, and a 30-second timeout. Both the Rust (`paced.rs`) and Go
(`cmppaced`) harnesses store compact 8-byte post-warmup samples and compute
exact percentiles in one reused buffer, eliminating the 30–90 MiB harness RSS
overhead seen in the v0.2.0 open-loop harness. Every client completed at least
99.9% of its offered requests in every cell.

Latency is in microseconds, CPU is client CPU time per request, and memory is
peak RSS. Each value is the median of 9 observations; lower is better.

HTTP/1.1, 8 connections, 2,000 requests per second:

| Client | p50 | p99 | p99.9 | CPU µs | RSS MiB |
| --- | ---: | ---: | ---: | ---: | ---: |
| **Leyline** | 20 | 43 | 55 | 13.30 | 10.4 |
| fasthttp | 21 | 49 | 54 | 15.57 | 8.8 |
| hyper-util | 22 | 47 | 54 | 15.69 | 7.1 |
| reqwest | 25 | 51 | 59 | 19.12 | 7.7 |
| wreq | 24 | 48 | 59 | 17.78 | 8.7 |
| ureq | 30 | 48 | 59 | 23.34 | 8.8 |
| libcurl | 25 | 32 | 54 | 12.05 | 8.5 |
| Go net/http | 26 | 56 | 122 | 24.27 | 13.9 |

HTTP/1.1, 64 connections, 20,000 requests per second:

| Client | p50 | p99 | p99.9 | CPU µs | RSS MiB |
| --- | ---: | ---: | ---: | ---: | ---: |
| **Leyline** | 34 | 63 | 76 | 12.24 | 15.8 |
| hyper-util | 35 | 65 | 77 | 14.24 | 13.3 |
| wreq | 38 | 68 | 81 | 19.58 | 15.7 |
| reqwest | 38 | 69 | 81 | 20.52 | 14.9 |
| ureq | 29 | 53 | 62 | 27.44 | 43.6 |
| libcurl | 24 | 38 | 48 | 20.05 | 18.8 |
| fasthttp | 30 | 49 | 58 | 14.17 | 24.6 |
| Go net/http | 33 | 214 | 330 | 29.74 | 20.2 |

HTTP/2, 2 connections, 16 workers, 2,000 requests per second:

| Client | p50 | p99 | p99.9 | CPU µs | RSS MiB |
| --- | ---: | ---: | ---: | ---: | ---: |
| **Leyline** | 28 | 57 | 70 | 14.03 | 10.7 |
| h2 | 28 | 57 | 71 | 14.09 | 6.3 |
| reqwest | 30 | 64 | 79 | 20.04 | 7.6 |
| hyper-util | 33 | 61 | 73 | 16.95 | 6.9 |
| wreq | 31 | 67 | 79 | 19.39 | 8.6 |
| libcurl | 23 | 50 | 66 | 19.37 | 8.3 |

HTTP/2, 8 connections, 256 workers, 50,000 requests per second:

| Client | p50 | p99 | p99.9 | CPU µs | RSS MiB |
| --- | ---: | ---: | ---: | ---: | ---: |
| **Leyline** | 66 | 120 | 145 | 8.18 | 17.6 |
| h2 | 66 | 121 | 145 | 10.89 | 11.1 |
| hyper-util | 84 | 154 | 177 | 13.82 | 13.4 |
| wreq | 82 | 150 | 175 | 19.99 | 17.0 |
| reqwest | 90 | 159 | 192 | 19.92 | 16.4 |
| libcurl | 72 | 134 | 160 | 12.29 | 12.0 |

Paired ratios, Leyline over each client, with a 95% t-interval across the three
run means. Below 1 means Leyline is lower.

| Cell | Client | p99 ratio | CPU per request ratio |
| --- | --- | ---: | ---: |
| HTTP/1.1, 8 conns | hyper-util | 0.916 [0.838, 1.002] | 0.843 [0.796, 0.892] |
| | reqwest | 0.849 [0.779, 0.926] | 0.691 [0.685, 0.698] |
| | wreq | 0.876 [0.754, 1.019] | 0.743 [0.714, 0.773] |
| | ureq | 0.924 [0.689, 1.238] | 0.529 [0.488, 0.573] |
| | libcurl | 1.281 [0.936, 1.754] | 1.084 [1.060, 1.110] |
| | Go net/http | 0.754 [0.644, 0.883] | 0.539 [0.527, 0.551] |
| | fasthttp | 0.863 [0.741, 1.004] | 0.842 [0.805, 0.881] |
| HTTP/1.1, 64 conns | hyper-util | 0.964 [0.892, 1.042] | 0.849 [0.828, 0.871] |
| | reqwest | 0.899 [0.784, 1.030] | 0.596 [0.578, 0.613] |
| | wreq | 0.894 [0.812, 0.983] | 0.620 [0.593, 0.649] |
| | ureq | 1.209 [1.126, 1.299] | 0.464 [0.396, 0.545] |
| | libcurl | 1.650 [1.586, 1.717] | 0.617 [0.601, 0.634] |
| | Go net/http | 0.296 [0.283, 0.310] | 0.413 [0.394, 0.432] |
| | fasthttp | 1.263 [1.201, 1.329] | 0.863 [0.824, 0.904] |
| HTTP/2, 2 conns | hyper-util | 0.923 [0.899, 0.948] | 0.809 [0.780, 0.839] |
| | reqwest | 0.891 [0.823, 0.966] | 0.687 [0.630, 0.748] |
| | wreq | 0.851 [0.788, 0.920] | 0.707 [0.645, 0.775] |
| | h2 | 0.991 [0.913, 1.075] | 0.960 [0.826, 1.117] |
| | libcurl | 1.185 [1.027, 1.368] | 0.673 [0.562, 0.806] |
| HTTP/2, 8 conns | hyper-util | 0.788 [0.757, 0.821] | 0.586 [0.564, 0.608] |
| | reqwest | 0.769 [0.737, 0.802] | 0.411 [0.401, 0.420] |
| | wreq | 0.805 [0.773, 0.839] | 0.413 [0.395, 0.431] |
| | h2 | 1.001 [0.972, 1.032] | 0.752 [0.732, 0.773] |
| | libcurl | 0.957 [0.808, 1.133] | 0.668 [0.633, 0.704] |

Leyline uses less CPU per request than every other client in all four cells
except libcurl at HTTP/1.1 with 8 connections (1.084×) and ties `h2` at HTTP/2
with 2 connections. At 50,000 HTTP/2 req/s, Leyline has the lowest median p99
(120 µs) and lowest client CPU (8.18 µs/req) of all six clients: 20–23% lower
p99 than hyper-util, wreq, and reqwest, and a tie on p99 with the bare `h2`
crate (1.001×) and libcurl (0.957×) while using 25% less CPU than `h2` and 33%
less CPU than libcurl. On HTTP/1.1 with 64 connections, libcurl, fasthttp, and
ureq have lower p99 than Leyline while using 16–116% more CPU. Peak RSS stays
between 10.4 and 17.6 MiB across all four open-loop cells; hyper-util, reqwest,
and `h2` still have lower peak RSS.

The pooled tables for [HTTP/1.1](benches/comparison/results/2026-11-23-v030-paced-h1-pooled.txt)
and [HTTP/2](benches/comparison/results/2026-11-23-v030-paced-h2-pooled.txt) and
the raw runs under `benches/comparison/results/2026-11-23-v030-paced-*` keep
every round and receipt. To reproduce one cell, run from `benches/comparison`:

```bash
export CMP_CA=$PWD/pki/ca.der CMP_CERT=$PWD/pki/server.der CMP_KEY=$PWD/pki/server-key.der
for run in 1 2 3; do
  TAG=reproduce-paced-h2-r$run MODE=paced PACED_SECONDS=5 ROUNDS=3 PROFILE=bare PROTO=h2 \
    TIMEOUT=30s CELLS=c8x256:8:256:50000 LIBS="leyline hyper reqwest wreq curl h2" \
    SERVER_CPUS=3-7,19-23 CLIENT_CPUS=8-15,24-31 bash libs.sh
done
python3 libs-table.py results/reproduce-paced-h2-r{1,2,3}/rows.txt
```

## Method

Each comparison has two passes, both run by `libs.sh`.

- Throughput (closed loop). Each worker sends its next request when the last one ends. A run sends `m` requests, then `2m`, and the rate is `(2m - m)` over the difference in wall time, so process start and connection setup cancel out. `perf stat` counts instructions and CPU time; GNU `time` records peak RSS.
- Latency (open loop, `MODE=paced`). Every client gets the same offered rate. Worker `w` sends request `i` at a fixed time on the schedule and does not wait for a slow response to move the schedule. When the worker's previous request ran past this request's scheduled time, latency counts from the scheduled time, so a client that falls behind shows its queueing delay (coordinated omission). When the worker was idle, latency counts from the send, so timer wake-up error in the harness is not charged to the client. Each worker's gap between requests is at least 2 ms, and a cell that asks for less is refused. The first second is a warm-up and is left out. The row also records CPU time per request at that rate and peak RSS.

In both passes the clients run one at a time, in alternating order each round. A cell is pooled over three independent runs. Ratios are geometric means of same-round pairs with a 95% t-interval over the per-run means. Runs with different Leyline source hashes, certificate settings or protocols are not pooled.

## Client configuration

Each client uses its own recommended pooled setup, with the limits raised where the library's default would cap the cell below its worker count.

| Client | Setting in this harness | Library default it changes |
| --- | --- | --- |
| Leyline | One `Session` per connection, cloned into workers | None |
| hyper-util | Legacy `Client`, `set_nodelay(true)`, `http2_only` for HTTP/2 | `HttpConnector` leaves `TCP_NODELAY` off by default (hyper-util 0.1.21) |
| reqwest | One `Client` per connection, `http1_only` or `http2_prior_knowledge` | None; `tcp_nodelay` is on by default |
| wreq | One `Client` per connection, `http1_only` for HTTP/1.1 | None |
| h2 | One `SendRequest` per connection, cloned into workers | None |
| ureq | One `Agent`, `max_idle_connections` and `max_idle_connections_per_host` set to the worker count | 10 idle connections, 3 per host (ureq 3.5.0) |
| libcurl | One multi handle per connection; HTTP/2 uses `CURLMOPT_PIPELINING` multiplexing, `CURLOPT_PIPEWAIT` and one connection per multi handle | None; `TCP_NODELAY` is on by default since 7.50.2 |
| Go net/http | `MaxIdleConns` and `MaxIdleConnsPerHost` set to the worker count; HTTP/2 uses `ForceAttemptHTTP2` and one connection | `DefaultMaxIdleConnsPerHost` is 2 |
| fasthttp | `MaxConnsPerHost` set to the worker count | `DefaultMaxConnsPerHost` is 512 |

## Limits

- One machine. The HTTP/2 tables ran over loopback only, which removes network latency, so they show client CPU cost.
- One response size (10 bytes). Large bodies measure decompression and copy
  cost, which this table does not cover.
- HTTP/3 is not compared here.

## Reproduce

You need Linux, Rust 1.96 or later, Go 1.26 or later, Python 3, `taskset`,
and OpenSSL.

1. From `benches/comparison`, create a test CA and a server certificate for `127.0.0.1`:

   ```bash
   mkdir -p pki && cd pki
   openssl req -x509 -newkey rsa:2048 -nodes -days 30 -subj "/CN=leyline-cmp-ca" \
     -keyout ca-key.pem -out ca.pem
   openssl req -newkey rsa:2048 -nodes -subj "/CN=127.0.0.1" \
     -keyout server-key.pem -out server.csr
   printf 'basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature\nextendedKeyUsage=serverAuth\nsubjectAltName=IP:127.0.0.1\n' > leaf.ext
   openssl x509 -req -in server.csr -CA ca.pem -CAkey ca-key.pem -CAcreateserial \
     -days 30 -extfile leaf.ext -out server.pem
   openssl x509 -in ca.pem -outform DER -out ca.der
   openssl x509 -in server.pem -outform DER -out server.der
   openssl pkcs8 -topk8 -nocrypt -in server-key.pem -outform DER -out server-key.der
   cd ..
   ```

2. Run the client-library comparison from `benches/comparison`. It also needs `perf`. Install other runtimes only for the clients selected in `LIBS`:

   ```bash
   CMP_CA=$PWD/pki/ca.der CMP_CERT=$PWD/pki/server.der CMP_KEY=$PWD/pki/server-key.der \
     SERVER_CPUS=0-6,16-22 CLIENT_CPUS=8-15,24-31 ./libs.sh
   ```

3. Run the HTTP/1.1 comparison with `PROTO=h1 ROUNDS=10`. For the delayed link, put the origin in a network namespace joined to the host by a veth pair with `tc qdisc add dev <veth> root netem delay 50us` on each end, issue a server certificate that also covers the namespace address, and set `SERVER_NETNS=<namespace>` and `SERVER_ADDR=<address>:0`.

`libs.sh` builds the selected clients, checks their response bodies and request headers, and writes `rows.txt`, `table.txt`, and `receipts.txt` to `results/<date>-libs-<protocol>-<profile>-timeout-<timeout>/`. The receipts record versions, binary hashes, host state, TLS mode, certificate verification, and the CA hash. `LIBS`, `CELLS`, `ROUNDS`, `PROFILE`, `TIMEOUT`, and `TAG` override the defaults. The table tool refuses to pool runs with different or unknown trust settings.
