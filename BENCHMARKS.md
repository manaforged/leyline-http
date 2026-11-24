# Benchmarks

This page records client-library comparisons on one machine. Each section links
to its raw results and identifies the configuration under measurement.

## v0.3.0 measurements

All v0.3.0 suites below ran on 2026-11-23 over loopback on a Ryzen 9 9950X3D
with the CPU frequency governor set to `performance`, the origin pinned to CPUs
`3-7,19-23` (CCD 0), clients pinned to CPUs `8-15,24-31` (CCD 1), verified TLS
1.3 (AES-128-GCM, X25519) against a local test CA, a 10-byte response, and a
30-second request timeout. The measured core is commit `c75e85e8` (`leyline_source_hash` `8381ed83` in
throughput and HTTP/1.1 open-loop receipts) and commit `56e95e4c`
(`leyline_source_hash` `220ac1b6` in the re-run HTTP/2 open-loop receipts,
which differ only outside the 10-byte buffered HTTP/2 path); peers are
hyper-util 0.1.21, hyper 1.12.0, reqwest 0.13.5, h2 0.4.20, wreq 0.16.3,
libcurl 8.22.0, ureq 3.5.0, Go 1.27.1 `net/http`, fasthttp 1.77.0, and
tls-client 1.16.0.

### Browser profiles over HTTP/2

Three independent runs used up to four alternating-order rounds each (`STOP_REF=leyline`
stops decided rivals after round 3 when both req/s and CPU/req differ by more
than 5%), Chrome 146 profiles, verified TLS, and a 30-second timeout.

Requests per second are medians over the 9–12 observations per client and workload:

| Workload | Leyline | wreq | tls-client |
| --- | ---: | ---: | ---: |
| Sequential, one connection | 56,549 | 37,122 | 31,641 |
| 1 connection, 64 streams | 188,041 | 158,536 | 47,161 |
| 8 connections, 8 in flight | 290,308 | 222,425 | 116,881 |
| 8 connections, 256 in flight | 1,129,848 | 724,707 | 195,040 |

Paired throughput ratios use the geometric mean, with a 95% t-interval across
three independent run means. A ratio above 1 favors Leyline; these ratios are
not ratios of the pooled medians above.

| Workload | Leyline / wreq [95% interval] | Leyline / tls-client [95% interval] |
| --- | ---: | ---: |
| Sequential, one connection | 1.552 [1.533, 1.571] | 1.815 [1.746, 1.887] |
| 1 connection, 64 streams | 1.194 [1.185, 1.202] | 4.019 [3.978, 4.060] |
| 8 connections, 8 in flight | 1.295 [1.261, 1.330] | 2.444 [2.300, 2.597] |
| 8 connections, 256 in flight | 1.552 [1.521, 1.584] | 5.789 [5.696, 5.883] |

Leyline completes more requests per second and uses less client CPU per request
than both peers in every cell (0.367–0.393× wreq's CPU and 0.117–0.293×
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
| Leyline | 59,009 | 218,489 | 305,313 | 1,438,580 |
| Leyline, one runtime per core | 58,860 | 179,680 | 351,975 | 1,409,655 |
| hyper-util client | 46,667 | 194,084 | 288,955 | 1,124,148 |
| libcurl (multi) | 48,412 | 107,475 | 247,015 | 793,204 |
| reqwest | 39,015 | 192,658 | 246,936 | 891,623 |
| Go net/http | 42,896 | 98,901 | 248,915 | 615,231 |
| h2 crate (no client layer) | 59,086 | 184,407 | 305,951 | 1,172,176 |

Paired throughput ratios, Leyline over the peer, with a 95% t-interval across
the three run means:

| Workload | hyper | reqwest | h2 crate |
| --- | ---: | ---: | ---: |
| Sequential | 1.273 [1.168, 1.388] | 1.550 [1.471, 1.632] | 1.012 [0.996, 1.028] |
| 1 conn, 64 streams | 1.128 [1.113, 1.145] | 1.138 [1.126, 1.150] | 1.195 [1.145, 1.248] |
| 8 conns, 8 in flight | 1.050 [1.020, 1.082] | 1.253 [1.222, 1.284] | 0.997 [0.934, 1.064] |
| 8 conns, 256 in flight | 1.279 [1.261, 1.298] | 1.614 [1.569, 1.660] | 1.227 [1.218, 1.236] |

Leyline leads every full client across all four HTTP/2 workloads on the default
runtime (including 1.050 [1.020, 1.082] over hyper-util at `8×8`, where one
runtime per core extends the lead to 1.221 [1.170, 1.275] over hyper-util and
1.159 [1.088, 1.235] over the bare `h2` crate). Against the bare `h2` crate
(which has no pool, headers, or timeouts), Leyline ties on sequential requests
and at 8 in flight and leads by 19.5% at 64 streams and 22.7% at 256 in flight
while executing 2,805–3,354 fewer user-space instructions per request in every
cell. Leyline uses less client CPU per request than every full client in every
workload (0.431–0.670× hyper-util, 0.334–0.505× reqwest) and 0.549–0.559× the
bare `h2` crate's CPU on the concurrent cells. On one connection (`1×64`),
single-core `run_per_core` trails multi-threaded hyper-util in throughput
(0.928 [0.904, 0.952]) while using 0.241× its CPU per request.

### General clients over HTTP/1.1

The same method ran the HTTP/1.1 clients. `Leyline blocking` is the blocking
API. Requests per second, medians over 9–12 observations:

| Client | Sequential | 8 workers | 64 workers |
| --- | ---: | ---: | ---: |
| Leyline | 66,922 | 355,862 | 878,877 |
| Leyline, one runtime per core | 64,579 | 412,319 | 921,370 |
| Leyline blocking | 68,971 | 423,685 | 891,526 |
| fasthttp | 69,388 | 367,732 | 888,039 |
| Go net/http | 47,259 | 423,637 | 604,988 |
| hyper-util client | 62,165 | 327,527 | 807,188 |
| reqwest | 41,247 | 266,552 | 638,486 |
| libcurl (multi) | 56,988 | 111,592 | 108,432 |
| ureq | 37,425 | 69,879 | 48,339 |

A longer follow-up repeated the 8- and 64-worker workloads for the closest
clients with up to 1,000,000 requests per half-pair:

| Client | 8 workers | 64 workers |
| --- | ---: | ---: |
| Leyline | 355,962 | 883,851 |
| Leyline, one runtime per core | 410,765 | 903,287 |
| Leyline blocking | 417,434 | 863,006 |
| fasthttp | 366,581 | 886,988 |
| Go net/http | 370,574 | 627,868 |
| hyper-util client | 330,601 | 822,912 |

Paired throughput ratios against fasthttp, the fastest peer across both worker
counts:

| Workload | Leyline | One runtime per core | Leyline blocking |
| --- | ---: | ---: | ---: |
| Sequential | 0.998 [0.913, 1.092] | 0.960 [0.891, 1.034] | 1.016 [0.953, 1.083] |
| 8 workers | 0.960 [0.935, 0.986] | 1.115 [1.088, 1.142] | 1.140 [1.026, 1.266] |
| 64 workers | 0.979 [0.906, 1.058] | 1.025 [0.966, 1.087] | 0.979 [0.894, 1.073] |
| 8 workers, long | 0.962 [0.922, 1.005] | 1.111 [1.027, 1.202] | 1.131 [1.113, 1.149] |
| 64 workers, long | 0.994 [0.962, 1.028] | 1.020 [0.954, 1.090] | 0.960 [0.869, 1.059] |

This is not an across-the-board HTTP/1.1 lead. One runtime per core reaches
921,370 req/s at 64 workers (903,287 in the 1M-request runs), leads fasthttp at
8 workers (+11.1–11.5%), and ties it at 64 workers while using 16–21% less
client CPU per request. The blocking API leads fasthttp at 8 workers
(+13.1–14.0%) and ties it at 64. The default shared runtime ties fasthttp on
sequential, 64-worker, and long 8-worker runs while trailing it in the short
8-worker run (0.960×) despite using 15–19% less CPU per request. At 8 workers Go
net/http opened 12–26 connections for 8 workers (`conns.txt`) and tied every
Leyline layout while trailing every Leyline layout at 64 workers by 37–52%.
Every Leyline layout is ahead of hyper-util, reqwest, libcurl, and ureq at every
worker count.

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
| **Leyline, one runtime per core** | 20 | 34 | 47 | 16.10 | 10.4 |
| **Leyline** | 20 | 43 | 53 | 12.43 | 10.1 |
| libcurl | 25 | 33 | 47 | 11.80 | 8.3 |
| hyper-util | 22 | 44 | 54 | 14.78 | 6.5 |
| ureq | 29 | 46 | 56 | 23.56 | 8.8 |
| wreq | 24 | 48 | 58 | 16.89 | 8.6 |
| reqwest | 25 | 50 | 59 | 18.09 | 7.4 |
| fasthttp | 22 | 50 | 54 | 15.11 | 8.6 |
| Go net/http | 27 | 56 | 128 | 24.75 | 13.7 |

HTTP/1.1, 64 connections, 20,000 requests per second:

| Client | p50 | p99 | p99.9 | CPU µs | RSS MiB |
| --- | ---: | ---: | ---: | ---: | ---: |
| **Leyline, one runtime per core** | 18 | 28 | 37 | 9.66 | 13.2 |
| **Leyline** | 33 | 62 | 72 | 11.81 | 15.3 |
| libcurl | 23 | 37 | 45 | 18.85 | 18.2 |
| fasthttp | 29 | 50 | 58 | 13.52 | 24.8 |
| ureq | 29 | 52 | 59 | 28.00 | 43.5 |
| hyper-util | 35 | 64 | 76 | 13.98 | 13.0 |
| reqwest | 38 | 65 | 79 | 20.10 | 14.8 |
| wreq | 37 | 65 | 78 | 19.13 | 15.2 |
| Go net/http | 33 | 213 | 323 | 29.31 | 20.0 |

HTTP/2, 2 connections, 16 workers, 2,000 requests per second:

| Client | p50 | p99 | p99.9 | CPU µs | RSS MiB |
| --- | ---: | ---: | ---: | ---: | ---: |
| **Leyline, one runtime per core** | 19 | 35 | 56 | 9.89 | 10.9 |
| **Leyline** | 29 | 57 | 70 | 13.56 | 10.9 |
| libcurl | 23 | 50 | 67 | 18.55 | 8.2 |
| h2 | 29 | 57 | 70 | 13.89 | 6.4 |
| hyper-util | 33 | 61 | 70 | 16.08 | 6.9 |
| reqwest | 30 | 65 | 79 | 19.86 | 7.6 |
| wreq | 31 | 66 | 77 | 19.27 | 8.7 |

HTTP/2, 8 connections, 256 workers, 50,000 requests per second:

| Client | p50 | p99 | p99.9 | CPU µs | RSS MiB |
| --- | ---: | ---: | ---: | ---: | ---: |
| **Leyline, one runtime per core** | 45 | 78 | 98 | 4.31 | 14.3 |
| **Leyline** | 66 | 115 | 140 | 8.13 | 17.3 |
| h2 | 66 | 119 | 142 | 10.78 | 11.0 |
| libcurl | 71 | 125 | 154 | 12.09 | 11.7 |
| wreq | 81 | 152 | 178 | 19.82 | 16.7 |
| hyper-util | 84 | 153 | 177 | 13.81 | 13.5 |
| reqwest | 90 | 158 | 187 | 19.95 | 15.8 |

Paired ratios, Leyline (default shared runtime) over each client, with a 95%
t-interval across the three run means. Below 1 means Leyline is lower.

| Cell | Client | p99 ratio | CPU per request ratio |
| --- | --- | ---: | ---: |
| HTTP/1.1, 8 conns | hyper-util | 0.970 [0.905, 1.039] | 0.843 [0.825, 0.861] |
| | reqwest | 0.867 [0.835, 0.900] | 0.681 [0.653, 0.711] |
| | wreq | 0.899 [0.851, 0.949] | 0.736 [0.684, 0.792] |
| | ureq | 0.932 [0.863, 1.005] | 0.537 [0.528, 0.545] |
| | libcurl | 1.345 [1.202, 1.505] | 1.055 [1.032, 1.079] |
| | Go net/http | 0.759 [0.686, 0.839] | 0.506 [0.480, 0.534] |
| | fasthttp | 0.858 [0.818, 0.900] | 0.827 [0.784, 0.872] |
| HTTP/1.1, 64 conns | hyper-util | 0.954 [0.918, 0.992] | 0.843 [0.833, 0.854] |
| | reqwest | 0.941 [0.914, 0.969] | 0.587 [0.581, 0.593] |
| | wreq | 0.946 [0.916, 0.976] | 0.615 [0.606, 0.625] |
| | ureq | 1.181 [1.156, 1.206] | 0.428 [0.387, 0.475] |
| | libcurl | 1.676 [1.596, 1.760] | 0.619 [0.607, 0.631] |
| | Go net/http | 0.293 [0.287, 0.300] | 0.403 [0.397, 0.409] |
| | fasthttp | 1.243 [1.191, 1.296] | 0.872 [0.868, 0.876] |
| HTTP/2, 2 conns | hyper-util | 0.947 [0.927, 0.968] | 0.827 [0.798, 0.857] |
| | reqwest | 0.892 [0.831, 0.957] | 0.684 [0.624, 0.749] |
| | wreq | 0.879 [0.861, 0.896] | 0.706 [0.654, 0.762] |
| | h2 | 1.000 [0.929, 1.077] | 0.966 [0.816, 1.144] |
| | libcurl | 1.213 [0.800, 1.838] | 0.727 [0.709, 0.745] |
| HTTP/2, 8 conns | hyper-util | 0.759 [0.695, 0.829] | 0.589 [0.585, 0.592] |
| | reqwest | 0.733 [0.676, 0.795] | 0.408 [0.401, 0.415] |
| | wreq | 0.759 [0.716, 0.805] | 0.410 [0.407, 0.414] |
| | h2 | 0.979 [0.842, 1.138] | 0.753 [0.743, 0.764] |
| | libcurl | 0.959 [0.825, 1.116] | 0.679 [0.658, 0.700] |

On the default shared Tokio runtime, Leyline uses less CPU per request than
every other client in all four cells except libcurl at HTTP/1.1 with 8
connections (1.055×) and ties `h2` at HTTP/2 with 2 connections. At 50,000
HTTP/2 req/s, default Leyline has 115 µs p99 at 8.13 µs/req CPU: 24–27% lower
p99 than hyper-util, wreq, and reqwest, and a tie on p99 with bare `h2`
(0.979×) and libcurl (0.959×) while using 25% less CPU than `h2` and 32% less
CPU than libcurl. On HTTP/1.1 with 64 connections, shared-runtime Tokio
cross-thread wakeups leave default Leyline at 62 µs p99 behind libcurl,
fasthttp, and ureq (while using 13–57% less CPU).

With `SessionBuilder::run_per_core`, eliminating cross-thread wakeups cuts p99
latency sharply across every open-loop cell: at 50,000 HTTP/2 req/s,
`leyline-cores` drops p99 to 78 µs at 4.31 µs/req CPU (0.491–0.508× the p99 of
hyper-util, reqwest, and wreq at 0.216–0.311× their CPU, and 0.642–0.655× the
p99 of libcurl and bare `h2` at 0.359–0.398× their CPU); at 20,000 HTTP/1.1
req/s (`c64`), `leyline-cores` leads all nine clients in both latency and CPU
at 28 µs p99 and 9.66 µs/req CPU (0.748 [0.710, 0.788]× libcurl's p99 at
0.509× its CPU, 0.555 [0.530, 0.580]× fasthttp's p99 at 0.717× its CPU, and
0.420–0.426× hyper-util, reqwest, and wreq). Peak RSS stays between 10.1 and
17.3 MiB across all four open-loop cells; hyper-util, reqwest, and `h2` still
have lower peak RSS on low-concurrency cells.

The pooled tables for [HTTP/1.1](benches/comparison/results/2026-11-23-v030-paced-h1-pooled.txt)
and [HTTP/2](benches/comparison/results/2026-11-23-v030-paced-h2-pooled.txt) and
the raw runs under `benches/comparison/results/2026-11-23-v030-paced-*` keep
every round and receipt. To reproduce one cell, run from `benches/comparison`:

```bash
export CMP_CA=$PWD/pki/ca.der CMP_CERT=$PWD/pki/server.der CMP_KEY=$PWD/pki/server-key.der
for run in 1 2 3; do
  TAG=reproduce-paced-h2-r$run MODE=paced PACED_SECONDS=5 ROUNDS=3 PROFILE=bare PROTO=h2 \
    TIMEOUT=30s CELLS=c8x256:8:256:50000 LIBS="leyline leyline-cores hyper reqwest wreq curl h2" \
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
