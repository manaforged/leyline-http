# Benchmarks

This page records client-library comparisons on one machine. Each section links
to its raw results and identifies the configuration under measurement.

## v0.3.0 measurements

All v0.3.0 suites below ran on 2026-11-22 against an isolated loopback network
namespace on a Ryzen 9 9950X3D with the CPU frequency governor set to
`performance`, the origin pinned to CPUs `3-7,19-23` (CCD 0), clients pinned to
CPUs `8-15,24-31` (CCD 1), verified TLS 1.3 (AES-128-GCM, X25519) against a
local test CA, a 10-byte response, and a 30-second request timeout. The
measured core is commit `14ea34f1` (`leyline_source_hash` `0c994244` in every
receipt); peers are hyper-util 0.1.21, hyper 1.12.0, reqwest 0.13.5, h2 0.4.20,
wreq 0.16.3, libcurl 8.22.0, ureq 3.5.0, Go 1.27.1 `net/http`, fasthttp 1.77.0,
and tls-client 1.16.0.

### Browser profiles over HTTP/2

Three independent runs used up to four alternating-order rounds each (`STOP_REF=leyline`
stops decided rivals after round 3 when both req/s and CPU/req differ by more
than 5%), Chrome 146 profiles, verified TLS, and a 30-second timeout.

Requests per second are medians over the 9–12 observations per client and workload:

| Workload | Leyline | wreq | tls-client |
| --- | ---: | ---: | ---: |
| Sequential, one connection | 64,951 | 39,548 | 33,635 |
| 1 connection, 64 streams | 206,790 | 160,585 | 50,255 |
| 8 connections, 8 in flight | 325,332 | 245,517 | 124,920 |
| 8 connections, 256 in flight | 1,173,750 | 754,753 | 211,389 |

Paired throughput ratios use the geometric mean, with a 95% t-interval across
three independent run means. A ratio above 1 favors Leyline; these ratios are
not ratios of the pooled medians above.

| Workload | Leyline / wreq [95% interval] | Leyline / tls-client [95% interval] |
| --- | ---: | ---: |
| Sequential, one connection | 1.646 [1.579, 1.715] | 1.921 [1.892, 1.950] |
| 1 connection, 64 streams | 1.284 [1.246, 1.323] | 4.079 [3.894, 4.272] |
| 8 connections, 8 in flight | 1.322 [1.266, 1.381] | 2.596 [2.552, 2.641] |
| 8 connections, 256 in flight | 1.550 [1.505, 1.595] | 5.577 [5.458, 5.698] |

Leyline completes more requests per second and uses less client CPU per request
than both peers in every cell (0.343–0.373× wreq's CPU and 0.116–0.265×
tls-client's CPU). The
[pooled table](benches/comparison/results/2026-11-22-v030-browser-h2-r1/pooled.txt)
includes CPU intervals, RSS, and instruction counts. Raw observations,
request-header audits, observed connection counts, and binary hashes are in
[run 1](benches/comparison/results/2026-11-22-v030-browser-h2-r1),
[run 2](benches/comparison/results/2026-11-22-v030-browser-h2-r2), and
[run 3](benches/comparison/results/2026-11-22-v030-browser-h2-r3).

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
| Leyline | 68,412 | 245,035 | 350,181 | 1,486,667 |
| Leyline, one runtime per core | 68,357 | 224,961 | 411,926 | 1,458,519 |
| hyper-util client | 55,780 | 214,032 | 334,624 | 1,143,841 |
| libcurl (multi) | 54,911 | 122,914 | 280,668 | 928,490 |
| reqwest | 41,829 | 211,301 | 277,617 | 925,090 |
| Go net/http | 47,551 | 126,193 | 282,244 | 780,005 |
| h2 crate (no client layer) | 68,788 | 207,320 | 352,016 | 1,196,347 |

Paired throughput ratios, Leyline over the peer, with a 95% t-interval across
the three run means:

| Workload | hyper | reqwest | h2 crate |
| --- | ---: | ---: | ---: |
| Sequential | 1.258 [1.209, 1.309] | 1.648 [1.591, 1.707] | 0.997 [0.962, 1.033] |
| 1 conn, 64 streams | 1.139 [1.092, 1.188] | 1.157 [1.118, 1.197] | 1.179 [1.129, 1.231] |
| 8 conns, 8 in flight | 1.074 [1.028, 1.122] | 1.279 [1.206, 1.356] | 0.995 [0.952, 1.041] |
| 8 conns, 256 in flight | 1.297 [1.258, 1.336] | 1.605 [1.547, 1.665] | 1.244 [1.209, 1.281] |

Leyline is ahead of every full client in every measured HTTP/2 workload,
including `8×8` against hyper-util (1.074× default, 1.257× per-core). Against
the bare `h2` crate (which has no pool, headers, or timeouts), Leyline ties on
sequential requests and at 8 in flight (where one runtime per core leads,
1.165 [1.127, 1.204]) and leads by 17.9% at 64 streams and 24.4% at 256 in
flight while executing 3,200–3,600 fewer user-space instructions per request in
every cell. Leyline uses less client CPU per request than every full client in
every workload (0.382–0.645× hyper-util, 0.297–0.481× reqwest) and 0.494–0.537×
the bare `h2` crate's CPU on the concurrent cells. Even on one connection
(`1×64`), single-core `run_per_core` leads multi-threaded hyper-util,
1.062 [1.027, 1.098], at 0.185× its CPU per request.

### General clients over HTTP/1.1

The same method ran the HTTP/1.1 clients. `Leyline blocking` is the blocking
API. Requests per second, medians over 9–12 observations:

| Client | Sequential | 8 workers | 64 workers |
| --- | ---: | ---: | ---: |
| Leyline | 82,965 | 439,293 | 994,230 |
| Leyline, one runtime per core | 80,966 | 490,704 | 1,080,885 |
| Leyline blocking | 84,794 | 511,187 | 971,674 |
| fasthttp | 82,199 | 444,027 | 1,005,630 |
| Go net/http | 54,343 | 645,248 | 640,336 |
| hyper-util client | 73,194 | 387,306 | 942,315 |
| reqwest | 46,948 | 292,165 | 686,472 |
| libcurl (multi) | 66,436 | 127,642 | 124,035 |
| ureq | 40,432 | 71,343 | 47,815 |

A longer follow-up repeated the 8- and 64-worker workloads for the closest
clients with up to 1,000,000 requests per half-pair:

| Client | 8 workers | 64 workers |
| --- | ---: | ---: |
| Leyline | 429,858 | 957,350 |
| Leyline, one runtime per core | 487,703 | 1,055,258 |
| Leyline blocking | 510,445 | 987,747 |
| fasthttp | 441,348 | 1,038,938 |
| Go net/http | 392,768 | 637,366 |
| hyper-util client | 381,560 | 872,054 |

Paired throughput ratios against fasthttp, the fastest peer across both worker
counts:

| Workload | Leyline | One runtime per core | Leyline blocking |
| --- | ---: | ---: | ---: |
| Sequential | 0.986 [0.916, 1.061] | 0.954 [0.842, 1.080] | 1.012 [0.940, 1.090] |
| 8 workers | 0.980 [0.966, 0.995] | 1.112 [1.071, 1.155] | 1.142 [1.057, 1.233] |
| 64 workers | 1.003 [0.855, 1.176] | 1.105 [0.943, 1.295] | 1.001 [0.924, 1.083] |
| 8 workers, long | 0.982 [0.886, 1.089] | 1.116 [1.080, 1.154] | 1.172 [1.153, 1.193] |
| 64 workers, long | 0.925 [0.888, 0.963] | 1.015 [1.000, 1.031] | 0.951 [0.904, 1.000] |

This is not an across-the-board HTTP/1.1 lead. One runtime per core reaches
1.08M req/s and leads fasthttp at 8 workers (+11.2–11.6%) and in the long
64-worker run (+1.5%), using 21–26% less client CPU per request. The blocking
API leads fasthttp at 8 workers (+14.2–17.2%) and ties it at 64. The default
shared runtime ties fasthttp on sequential, 8-worker long, and 64-worker short
runs while trailing it in the long 64-worker run (0.925×) despite using 23%
less CPU per request. At 8 workers Go net/http opened 16–26 connections for 8
workers (`conns.txt`) and led default Leyline in the short run
(0.679 [0.501, 0.920]) before dropping to a tie in the long run
(1.000 [0.873, 1.145], where Leyline blocking leads 1.194 [1.013, 1.407]) and
trailing every Leyline layout at 64 workers by 51–69%. Every Leyline layout is
ahead of hyper-util, reqwest, libcurl, and ureq at every worker count.

The pooled tables for [HTTP/2](benches/comparison/results/2026-11-22-v030-clients-h2-r1/pooled.txt),
[HTTP/1.1](benches/comparison/results/2026-11-22-v030-clients-h1-r1/pooled.txt), and
the [longer HTTP/1.1 runs](benches/comparison/results/2026-11-22-v030-h1-long-r1/pooled.txt),
and the raw runs under `benches/comparison/results/2026-11-22-v030-*`, include
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
| **Leyline** | 17 | 37 | 45 | 11.84 | 10.9 |
| fasthttp | 16 | 37 | 43 | 12.48 | 8.6 |
| hyper-util | 18 | 40 | 49 | 14.44 | 6.5 |
| reqwest | 24 | 43 | 53 | 19.27 | 7.7 |
| wreq | 22 | 43 | 53 | 17.22 | 8.7 |
| ureq | 28 | 45 | 58 | 22.53 | 8.8 |
| libcurl | 21 | 29 | 43 | 10.86 | 8.2 |
| Go net/http | 22 | 48 | 124 | 22.92 | 14.0 |

HTTP/1.1, 64 connections, 20,000 requests per second:

| Client | p50 | p99 | p99.9 | CPU µs | RSS MiB |
| --- | ---: | ---: | ---: | ---: | ---: |
| **Leyline** | 31 | 63 | 73 | 10.14 | 15.4 |
| hyper-util | 32 | 63 | 77 | 12.25 | 13.0 |
| wreq | 35 | 66 | 80 | 17.07 | 15.5 |
| reqwest | 35 | 69 | 85 | 18.11 | 14.6 |
| ureq | 27 | 48 | 60 | 26.75 | 43.5 |
| libcurl | 21 | 33 | 39 | 17.48 | 18.5 |
| fasthttp | 27 | 46 | 54 | 12.90 | 24.9 |
| Go net/http | 29 | 196 | 314 | 27.47 | 20.4 |

HTTP/2, 2 connections, 16 workers, 2,000 requests per second:

| Client | p50 | p99 | p99.9 | CPU µs | RSS MiB |
| --- | ---: | ---: | ---: | ---: | ---: |
| **Leyline** | 19 | 51 | 63 | 11.71 | 11.0 |
| h2 | 20 | 51 | 61 | 12.42 | 6.7 |
| reqwest | 25 | 54 | 69 | 18.68 | 7.5 |
| hyper-util | 27 | 57 | 66 | 15.46 | 7.0 |
| wreq | 25 | 58 | 71 | 18.04 | 8.4 |
| libcurl | 22 | 39 | 54 | 16.33 | 8.2 |

HTTP/2, 8 connections, 256 workers, 50,000 requests per second:

| Client | p50 | p99 | p99.9 | CPU µs | RSS MiB |
| --- | ---: | ---: | ---: | ---: | ---: |
| **Leyline** | 61 | 120 | 143 | 7.29 | 17.7 |
| h2 | 58 | 117 | 138 | 9.79 | 11.6 |
| hyper-util | 78 | 146 | 170 | 13.12 | 13.6 |
| wreq | 76 | 146 | 173 | 18.49 | 17.3 |
| reqwest | 80 | 149 | 175 | 19.10 | 16.0 |
| libcurl | 64 | 104 | 127 | 10.51 | 11.7 |

Paired ratios, Leyline over each client, with a 95% t-interval across the three
run means. Below 1 means Leyline is lower.

| Cell | Client | p99 ratio | CPU per request ratio |
| --- | --- | ---: | ---: |
| HTTP/1.1, 8 conns | hyper-util | 0.941 [0.917, 0.965] | 0.827 [0.813, 0.842] |
| | reqwest | 0.855 [0.792, 0.922] | 0.624 [0.608, 0.640] |
| | wreq | 0.866 [0.809, 0.927] | 0.688 [0.655, 0.724] |
| | ureq | 0.836 [0.625, 1.117] | 0.498 [0.383, 0.648] |
| | libcurl | 1.262 [1.257, 1.268] | 1.093 [1.011, 1.182] |
| | Go net/http | 0.779 [0.729, 0.833] | 0.520 [0.465, 0.580] |
| | fasthttp | 1.098 [0.820, 1.470] | 0.962 [0.858, 1.078] |
| HTTP/1.1, 64 conns | hyper-util | 0.981 [0.942, 1.022] | 0.821 [0.813, 0.830] |
| | reqwest | 0.885 [0.852, 0.920] | 0.544 [0.529, 0.560] |
| | wreq | 0.904 [0.867, 0.943] | 0.573 [0.542, 0.606] |
| | ureq | 1.300 [1.222, 1.382] | 0.382 [0.332, 0.439] |
| | libcurl | 1.915 [1.901, 1.930] | 0.573 [0.568, 0.579] |
| | Go net/http | 0.316 [0.305, 0.328] | 0.366 [0.364, 0.367] |
| | fasthttp | 1.346 [1.214, 1.491] | 0.786 [0.759, 0.814] |
| HTTP/2, 2 conns | hyper-util | 0.913 [0.886, 0.942] | 0.752 [0.718, 0.788] |
| | reqwest | 0.927 [0.834, 1.029] | 0.629 [0.580, 0.682] |
| | wreq | 0.886 [0.850, 0.922] | 0.653 [0.608, 0.701] |
| | h2 | 1.013 [0.941, 1.092] | 0.934 [0.864, 1.010] |
| | libcurl | 1.329 [1.132, 1.561] | 0.714 [0.659, 0.772] |
| HTTP/2, 8 conns | hyper-util | 0.818 [0.743, 0.900] | 0.567 [0.537, 0.598] |
| | reqwest | 0.798 [0.715, 0.891] | 0.390 [0.367, 0.415] |
| | wreq | 0.814 [0.741, 0.894] | 0.401 [0.380, 0.423] |
| | h2 | 1.036 [0.891, 1.206] | 0.756 [0.713, 0.801] |
| | libcurl | 1.263 [0.749, 2.130] | 0.707 [0.670, 0.747] |

Leyline uses less CPU per request than every other client in all four cells
except libcurl at HTTP/1.1 with 8 connections (1.093×) and ties fasthttp there
and `h2` at HTTP/2 with 2 connections. At 50,000 HTTP/2 req/s, Leyline's p99
(120 µs) is 18–20% lower than hyper-util, wreq, and reqwest, and ties both the
bare `h2` crate (1.036×) and libcurl (1.263×) while using 24% less CPU than
`h2` and 29% less CPU than libcurl. On HTTP/1.1 with 64 connections, libcurl,
fasthttp, and ureq have lower p99 than Leyline while using 27–162% more CPU.
Peak RSS stays between 10.9 and 17.7 MiB across all four open-loop cells;
hyper-util, reqwest, and `h2` still have lower peak RSS.

The pooled tables for [HTTP/1.1](benches/comparison/results/2026-11-22-v030-paced-h1-pooled.txt)
and [HTTP/2](benches/comparison/results/2026-11-22-v030-paced-h2-pooled.txt) and
the raw runs under `benches/comparison/results/2026-11-22-v030-paced-*` keep
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
