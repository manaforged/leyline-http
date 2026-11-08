# Benchmarks

This page records client-library comparisons on one machine. Each section links
to its raw results and identifies the configuration under measurement.

## v0.2.0 measurements

### Browser profiles over HTTP/2

Three independent runs used four alternating-order rounds each, Chrome 146
profiles, a 10-byte response, certificate verification against a local test CA,
and a 30-second request timeout. The origin negotiated TLS 1.3 with
AES-128-GCM and X25519 for every client. The host was not dedicated; the server
and clients used separate pinned CPU sets.

Requests per second are medians over the 12 observations per client and workload:

| Workload | Leyline | wreq | tls-client |
| --- | ---: | ---: | ---: |
| Sequential, one connection | 49,106 | 33,355 | 28,799 |
| 1 connection, 64 streams | 176,620 | 147,945 | 52,436 |
| 8 connections, 8 in flight | 260,241 | 200,467 | 150,434 |
| 8 connections, 256 in flight | 1,077,095 | 686,757 | 272,135 |

Paired throughput ratios use the geometric mean, with a 95% t-interval across
three independent run means. A ratio above 1 favors Leyline; these ratios are
not ratios of the pooled medians above.

| Workload | Leyline / wreq [95% interval] | Leyline / tls-client [95% interval] |
| --- | ---: | ---: |
| Sequential, one connection | 1.491 [1.466, 1.516] | 1.724 [1.696, 1.753] |
| 1 connection, 64 streams | 1.196 [1.181, 1.211] | 3.376 [3.338, 3.415] |
| 8 connections, 8 in flight | 1.255 [1.254, 1.256] | 1.695 [1.664, 1.727] |
| 8 connections, 256 in flight | 1.574 [1.460, 1.697] | 3.948 [3.694, 4.220] |

Leyline completes more requests per second and uses less client CPU per request
than both peers in every cell. The
[pooled table](benches/comparison/results/2026-11-03-verified-browser-h2-r1/pooled.txt)
includes CPU intervals and instruction counts. Raw observations, request-header
audits, observed connection counts, and binary hashes are in
[run 1](benches/comparison/results/2026-11-03-verified-browser-h2-r1),
[run 2](benches/comparison/results/2026-11-03-verified-browser-h2-r2), and
[run 3](benches/comparison/results/2026-11-03-verified-browser-h2-r3).
The measured core is `bde05ee7`; the peers are wreq 0.16.3 and tls-client 1.16.0.

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

Three independent runs used four alternating-order rounds each, bare profiles,
identical request-header values, certificate verification against a local test
CA, a 10-byte response, and a 30-second request timeout. The CPU frequency
governor was set to `performance`. Leyline is commit `5bc5331b`. `Leyline` is the
default layout: one multi-threaded Tokio runtime and one shared session.
`Leyline, one runtime per core` is the same client run through
`SessionBuilder::run_per_core`; for HTTP/2 the harness caps its runtime count at
the connection count, so every client holds the stated number of connections.

Requests per second, medians over 12 observations:

| Client | Sequential | 1 conn, 64 streams | 8 conns, 8 in flight | 8 conns, 256 in flight |
| --- | ---: | ---: | ---: | ---: |
| Leyline | 53,256 | 210,127 | 284,188 | 1,397,935 |
| Leyline, one runtime per core | 52,970 | 166,264 | 320,511 | 1,428,376 |
| hyper-util client | 43,329 | 183,297 | 276,706 | 1,148,891 |
| reqwest | 35,733 | 183,052 | 227,841 | 861,386 |
| libcurl (multi) | 44,041 | 98,864 | 226,377 | 653,651 |
| Go net/http | 39,955 | 103,903 | 250,234 | 577,833 |
| h2 crate (no client layer) | 53,344 | 170,783 | 288,118 | 1,225,043 |

Paired throughput ratios, Leyline over the peer, with a 95% t-interval across
the three run means:

| Workload | hyper | reqwest | h2 crate |
| --- | ---: | ---: | ---: |
| Sequential | 1.209 [1.167, 1.253] | 1.489 [1.468, 1.510] | 1.002 [0.969, 1.037] |
| 1 conn, 64 streams | 1.145 [1.094, 1.198] | 1.147 [1.119, 1.175] | 1.243 [1.197, 1.290] |
| 8 conns, 8 in flight | 1.009 [0.906, 1.122] | 1.222 [1.106, 1.351] | 0.984 [0.942, 1.027] |
| 8 conns, 256 in flight | 1.195 [1.098, 1.301] | 1.591 [1.500, 1.688] | 1.112 [1.045, 1.182] |

Leyline is ahead of every full client in every workload except 8 connections
with 8 in flight, where it ties hyper; at that workload one runtime per core is
ahead of hyper, 1.136 [1.085, 1.189]. It also leads libcurl and Go net/http in
every workload. Against the bare h2 crate, which has no pool, headers, or
timeouts, Leyline ties on sequential requests and at 8 in flight and is ahead
with 64 streams and at 256 in flight. Leyline uses less client CPU per request
than every full client in every workload, and slightly more than the bare h2
crate at sequential and 8 in flight.

With one connection, one runtime per core runs on one core and is behind hyper,
0.905 [0.885, 0.926], while using 0.272 of its CPU per request. Use the per-core
layout when a program spreads its requests over at least as many connections as
cores.

### General clients over HTTP/1.1

The same method ran the HTTP/1.1 clients. `Leyline blocking` is the blocking
API. Requests per second, medians over 12 observations:

| Client | Sequential | 8 workers | 64 workers |
| --- | ---: | ---: | ---: |
| Leyline | 60,274 | 325,414 | 812,630 |
| Leyline, one runtime per core | 59,818 | 368,016 | 943,956 |
| Leyline blocking | 61,418 | 376,263 | 928,744 |
| fasthttp | 59,963 | 336,220 | 866,644 |
| Go net/http | 43,104 | 372,120 | 556,023 |
| hyper-util client | 56,416 | 321,075 | 729,825 |
| reqwest | 39,483 | 246,316 | 586,958 |
| libcurl (multi) | 51,639 | 104,240 | 100,958 |
| ureq | 34,532 | 68,507 | 46,400 |

A longer follow-up repeated the 8- and 64-worker workloads for the closest
clients with 1,000,000 and 2,000,000 requests per pair:

| Client | 8 workers | 64 workers |
| --- | ---: | ---: |
| Leyline | 327,702 | 797,737 |
| Leyline, one runtime per core | 364,296 | 917,435 |
| Leyline blocking | 377,466 | 890,194 |
| fasthttp | 334,606 | 876,837 |
| Go net/http | 268,257 | 651,462 |
| hyper-util client | 305,922 | 664,432 |

Paired throughput ratios against fasthttp, the fastest peer here:

| Workload | Leyline | One runtime per core | Leyline blocking |
| --- | ---: | ---: | ---: |
| Sequential | 1.005 [0.969, 1.041] | 0.997 [0.945, 1.051] | 1.032 [0.946, 1.126] |
| 8 workers | 0.981 [0.892, 1.078] | 1.122 [1.086, 1.160] | 1.133 [1.078, 1.191] |
| 64 workers | 0.876 [0.630, 1.218] | 1.038 [1.005, 1.072] | 1.016 [0.906, 1.140] |
| 8 workers, long | 0.987 [0.921, 1.058] | 1.082 [1.053, 1.113] | 1.134 [1.104, 1.164] |
| 64 workers, long | 0.835 [0.719, 0.969] | 1.042 [0.966, 1.125] | 1.023 [0.935, 1.118] |

This is not an across-the-board HTTP/1.1 lead. One runtime per core and the
blocking API are ahead of fasthttp at 8 workers and tie or lead it at 64. The
default shared runtime ties fasthttp at sequential and 8 workers and is behind
it in the long 64-worker run. At 8 workers it is also behind Go net/http in the
short run, 0.867 [0.775, 0.970], and ahead of it in the long run,
1.234 [1.182, 1.289]. Every Leyline layout is ahead of hyper, reqwest, libcurl
and ureq at every worker count where the interval resolves, and the async
layouts use less client CPU per request than fasthttp in every workload. Workers
do not imply identical total TCP connection counts: each client manages its own
HTTP/1.1 pool, and `conns.txt` in each run records what was observed.

A separate Chrome 146 comparison with tls-client passed the header gate and
showed a throughput and CPU lead for async and blocking Leyline at all three
worker counts. Its [pooled table](benches/comparison/results/2026-11-03-verified-browser-h1-matched-r1/pooled.txt)
and raw [run 1](benches/comparison/results/2026-11-03-verified-browser-h1-matched-r1),
[run 2](benches/comparison/results/2026-11-03-verified-browser-h1-matched-r2), and
[run 3](benches/comparison/results/2026-11-03-verified-browser-h1-matched-r3)
record the differing observed connection counts. This does not replace the
excluded wreq comparison or establish an HTTP/1.1 browser-client ranking.

The pooled tables for [HTTP/2](benches/comparison/results/2026-11-05-final-clients-h2-r1/pooled.txt),
[HTTP/1.1](benches/comparison/results/2026-11-05-final-clients-h1-r1/pooled.txt) and
the [longer HTTP/1.1 runs](benches/comparison/results/2026-11-05-final-h1-long-r1/pooled.txt),
and the raw runs under `benches/comparison/results/2026-11-05-final-*`, include
instructions, CPU, connection counts, header audits, and the command for each
run. To reproduce, create the certificates in the [reproduction steps](#reproduce),
set the CPU governor to `performance`, then run from `benches/comparison`:

```bash
export CMP_CA=$PWD/pki/ca.der CMP_CERT=$PWD/pki/server.der CMP_KEY=$PWD/pki/server-key.der
P="PROFILE=bare ROUNDS=4 TIMEOUT=30s SERVER_CPUS=0-6,16-22 CLIENT_CPUS=8-15,24-31"
for run in 1 2 3; do
  env TAG=reproduce-h2-r$run PROTO=h2 $P \
    LIBS="leyline leyline-cores hyper h2 reqwest curl nethttp" bash libs.sh
  env TAG=reproduce-h1-r$run PROTO=h1 $P \
    LIBS="leyline leyline-cores leyline-blocking fasthttp ureq hyper reqwest curl nethttp" bash libs.sh
  env TAG=reproduce-h1-long-r$run PROTO=h1 $P CELLS="8par:1:8:1000000 64par:1:64:1000000" \
    LIBS="leyline leyline-cores leyline-blocking fasthttp nethttp hyper" bash libs.sh
done
python3 libs-table.py results/reproduce-h2-r{1,2,3}/rows.txt --vs hyper
```

`libs.sh` shrinks the request count of a client whose run takes longer than
`TARGET_MS` (default 1,000 ms) after the first round, so slow clients do not
dominate the run time; every row records the counts it used.

### Fresh sessions and closed-loop latency

The existing paired runner measured one connection and 64 streams, with 2,000
warm requests, 200 fresh clients, and 200,000 concurrent requests per round.
Each peer has three batches of four alternating-order rounds. The wreq pair uses
native Chrome 146 profiles; the reqwest pair uses bare profiles and identical
request headers. Both use verified TLS and a 30-second timeout.

| Peer and configuration | Leyline p99 µs | Peer p99 µs | p99 ratio [95% interval] | Fresh-client throughput ratio [95% interval] |
| --- | ---: | ---: | ---: | ---: |
| wreq, Chrome 146 | 557 | 585.5 | 0.946 [0.910, 0.982] | 0.975 [0.871, 1.092] |
| reqwest, bare | 561 | 519.5 | 1.075 [1.036, 1.116] | 0.966 [0.875, 1.067] |

The p99 values are medians of per-round p99 observations, not a percentile of
pooled requests. Ratios use the existing table tool's log-ratio interval across
three batch means. Leyline's p99 is lower than wreq's in this workload but higher
than reqwest's. Fresh-client throughput is unresolved against both peers.
These are closed-loop measurements, without offered-load pacing or
coordinated-omission correction; they do not establish an application tail-latency
lead. The [raw JSON and derived table](benches/comparison/results/2026-11-03-verified-latency)
retain every round.

The [verification probes](benches/comparison/results/2026-11-03-verification)
record trusted-CA acceptance, wrong-CA rejection, and refusal to pool results
with missing trust metadata.

To reproduce this HTTP/2 browser comparison, create the certificates in the
[reproduction steps](#reproduce), then run from `benches/comparison`:

```bash
export CMP_CA=$PWD/pki/ca.der CMP_CERT=$PWD/pki/server.der CMP_KEY=$PWD/pki/server-key.der
for run in 1 2 3; do
  TAG=reproduce-browser-h2-r$run PROFILE=chrome146 PROTO=h2 TIMEOUT=30s ROUNDS=4 \
    LIBS="leyline wreq tlsclient" SERVER_CPUS=0-6,16-22 CLIENT_CPUS=8-15,24-31 \
    bash libs.sh
done
python3 libs-table.py results/reproduce-browser-h2-r{1,2,3}/rows.txt --vs tlsclient
```

### Open-loop latency

Every client got the same offered rate for 10 seconds after a 1-second warm-up,
as the [method](#method) describes. Three independent runs used three
alternating-order rounds each, bare profiles, identical request headers, a
10-byte response, verified TLS against a local test CA, and a 30-second timeout.
The runs used the whole host on 2026-11-07, with the server on CPUs 0-6 and
16-22 and the clients on CPUs 8-15 and 24-31. The Leyline library source is
identical to v0.2.0 (`leyline_source_hash` `3bdc1179` in every receipt). Every
client completed at least 99.9% of its offered requests in every cell.

Latency is in microseconds, CPU is client CPU time per request, and memory is
peak RSS. Each value is the median of 9 observations; lower is better.

HTTP/1.1, 8 connections, 2,000 requests per second:

| Client | p50 | p99 | p99.9 | CPU µs | RSS MiB |
| --- | ---: | ---: | ---: | ---: | ---: |
| **Leyline** | 17 | 36 | 45 | 11.40 | 13.4 |
| hyper-util | 18 | 36 | 46 | 13.62 | 8.1 |
| reqwest | 22 | 43 | 52 | 17.55 | 8.7 |
| wreq | 21 | 41 | 51 | 15.99 | 9.6 |
| ureq | 26 | 43 | 50 | 21.69 | 10.0 |
| libcurl | 21 | 28 | 52 | 10.53 | 10.6 |
| Go net/http | 22 | 49 | 147 | 22.73 | 18.4 |
| fasthttp | 18 | 38 | 44 | 13.17 | 18.0 |

HTTP/1.1, 64 connections, 20,000 requests per second:

| Client | p50 | p99 | p99.9 | CPU µs | RSS MiB |
| --- | ---: | ---: | ---: | ---: | ---: |
| **Leyline** | 33 | 69 | 84 | 9.74 | 29.5 |
| hyper-util | 35 | 71 | 85 | 11.74 | 25.8 |
| reqwest | 37 | 73 | 89 | 17.48 | 27.7 |
| wreq | 37 | 74 | 92 | 16.47 | 28.0 |
| ureq | 26 | 46 | 54 | 25.73 | 58.0 |
| libcurl | 19 | 31 | 38 | 16.26 | 32.5 |
| Go net/http | 29 | 60 | 204 | 22.60 | 97.4 |
| fasthttp | 28 | 52 | 61 | 12.17 | 113.5 |

HTTP/2, 2 connections, 16 workers, 2,000 requests per second:

| Client | p50 | p99 | p99.9 | CPU µs | RSS MiB |
| --- | ---: | ---: | ---: | ---: | ---: |
| **Leyline** | 20 | 51 | 61 | 11.85 | 14.4 |
| hyper-util | 25 | 55 | 65 | 14.94 | 9.2 |
| reqwest | 25 | 55 | 70 | 18.11 | 9.9 |
| wreq | 25 | 56 | 70 | 16.99 | 10.6 |
| h2 | 21 | 51 | 61 | 12.23 | 8.7 |
| libcurl | 22 | 39 | 57 | 15.97 | 9.9 |

HTTP/2, 8 connections, 256 workers, 50,000 requests per second:

| Client | p50 | p99 | p99.9 | CPU µs | RSS MiB |
| --- | ---: | ---: | ---: | ---: | ---: |
| **Leyline** | 60 | 124 | 145 | 7.96 | 50.3 |
| hyper-util | 74 | 139 | 162 | 12.88 | 44.8 |
| reqwest | 78 | 148 | 176 | 18.90 | 47.1 |
| wreq | 73 | 140 | 166 | 18.18 | 48.3 |
| h2 | 56 | 112 | 130 | 9.70 | 42.6 |
| libcurl | 61 | 88 | 114 | 10.16 | 46.4 |

Paired ratios, Leyline over each client, with a 95% t-interval across the three
run means. Below 1 means Leyline is lower.

| Cell | Client | p99 ratio | CPU per request ratio |
| --- | --- | ---: | ---: |
| HTTP/1.1, 8 conns | hyper-util | 0.976 [0.929, 1.025] | 0.835 [0.814, 0.857] |
| | reqwest | 0.832 [0.771, 0.897] | 0.642 [0.620, 0.665] |
| | wreq | 0.880 [0.843, 0.918] | 0.712 [0.682, 0.744] |
| | ureq | 0.856 [0.674, 1.088] | 0.523 [0.497, 0.550] |
| | libcurl | 1.257 [1.085, 1.457] | 1.079 [1.058, 1.101] |
| | Go net/http | 0.719 [0.664, 0.779] | 0.496 [0.467, 0.528] |
| | fasthttp | 0.939 [0.885, 0.996] | 0.860 [0.847, 0.873] |
| HTTP/1.1, 64 conns | hyper-util | 0.969 [0.925, 1.015] | 0.829 [0.806, 0.852] |
| | reqwest | 0.940 [0.890, 0.993] | 0.557 [0.548, 0.566] |
| | wreq | 0.923 [0.868, 0.982] | 0.592 [0.575, 0.610] |
| | ureq | 1.458 [1.392, 1.528] | 0.386 [0.330, 0.451] |
| | libcurl | 2.188 [2.094, 2.286] | 0.604 [0.586, 0.622] |
| | Go net/http | 1.141 [1.101, 1.182] | 0.431 [0.421, 0.441] |
| | fasthttp | 1.317 [1.270, 1.365] | 0.801 [0.780, 0.822] |
| HTTP/2, 2 conns | hyper-util | 0.937 [0.914, 0.960] | 0.778 [0.742, 0.817] |
| | reqwest | 0.924 [0.894, 0.954] | 0.647 [0.618, 0.679] |
| | wreq | 0.904 [0.882, 0.926] | 0.683 [0.645, 0.723] |
| | h2 | 1.004 [0.995, 1.014] | 0.966 [0.948, 0.985] |
| | libcurl | 1.319 [1.312, 1.327] | 0.746 [0.734, 0.758] |
| HTTP/2, 8 conns | hyper-util | 0.888 [0.864, 0.912] | 0.618 [0.613, 0.624] |
| | reqwest | 0.834 [0.779, 0.892] | 0.420 [0.415, 0.424] |
| | wreq | 0.885 [0.835, 0.937] | 0.437 [0.435, 0.440] |
| | h2 | 1.107 [1.035, 1.184] | 0.820 [0.815, 0.824] |
| | libcurl | 1.446 [1.157, 1.807] | 0.781 [0.734, 0.832] |

Leyline uses less CPU per request than every other client in all four cells,
except libcurl at HTTP/1.1 with 8 connections, where it uses 1.079 times as
much. Its p99 is lower than reqwest's and wreq's in every cell, and lower than
or tied with hyper-util's. libcurl has the lowest p99 in every cell. ureq,
fasthttp and Go net/http have a lower p99 than Leyline at HTTP/1.1 with 64
connections, and the h2 crate has a lower p99 at HTTP/2 with 8 connections.
Leyline does not use the least memory: hyper-util, reqwest and wreq have a
lower peak RSS in every cell, and h2 in both HTTP/2 cells.

The pooled tables for [HTTP/1.1](benches/comparison/results/2026-11-07-paced-h1-pooled.txt)
and [HTTP/2](benches/comparison/results/2026-11-07-paced-h2-pooled.txt) and the
raw runs under `benches/comparison/results/2026-11-07-paced-*` keep every round
and receipt. To reproduce one cell, run from `benches/comparison`:

```bash
export CMP_CA=$PWD/pki/ca.der CMP_CERT=$PWD/pki/server.der CMP_KEY=$PWD/pki/server-key.der
for run in 1 2 3; do
  TAG=reproduce-paced-h2-r$run MODE=paced PACED_SECONDS=10 ROUNDS=3 PROFILE=bare PROTO=h2 \
    TIMEOUT=30s CELLS=c8x256:8:256:50000 LIBS="leyline hyper reqwest wreq curl h2" \
    SERVER_CPUS=0-6,16-22 CLIENT_CPUS=8-15,24-31 bash libs.sh
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
