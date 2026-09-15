# Benchmarks

This repository measures the current revision with open tooling: Criterion
for in-process work, the separate comparison clients for end-to-end rates,
nghttp2's h2load as an independent reference, and a netem link for
wide-area behavior. Every timed response in the Leyline and wreq Rust
comparison clients is checked against the expected bytes, including the Go
clients when `CMP_BODY` is set. azuretls cannot verify the private test CA
(its verification path requires public roots), so its TLS path is
unverified; its rows are marked.

## Quick start

```sh
cd benches/comparison
CMP_CA=ca.der CMP_CERT=server.der CMP_KEY=server-key.der \
  LEYLINE_CHROME=149 CLIENT_CPUS=2-7,18-23 ROUNDS=20 ./paired.sh
CMP_CA=ca.der CMP_CA_KEY=ca-key.pem RTT_MS=30 LEYLINE_CHROME=149 ./netem.sh
../../scripts/perf_accounting.py --client leyline --preset small
```

`paired.sh` reads `CMP_CA`, `CMP_CERT`, `CMP_KEY`, `LEYLINE_CHROME`,
`ROUNDS`, `CONC`, `CONCURRENCY`, `TARGET_URL`, and `CONTROL`. `LEFT` and
`RIGHT` select the two binaries in `bin/`; `WARM` and `COLD` set the
per-round sequential counts (default `1`, so the loop measures only the
concurrent phase) and `PAIRED_JSON` writes the per-round observations as a
JSON cell. `netem.sh` reads `RTT_MS`, `ORIGIN`, and the `paired.sh`
variables. `control.sh` alone prints the reference rows for a running
origin. `CMP_CONNECTIONS` spreads client tasks over several connections.

## Reference results

Measured on a 16-core x86_64 Linux host (Rust 1.98.1, fat LTO,
one codegen unit). Origin on CPUs 0-1; clients on CPUs 2-7 and 18-23.
`LEYLINE_CHROME=149` request headers, verified TLS with the test CA,
HTTP/2, ten-byte `byte[i] = i % 251` bodies, one connection with 64 streams
unless stated. Client rows are twenty balanced pairs at one connection and
eight balanced pairs at eight connections. The h2load rows use the same
headers and topology and 524,288 requests; h2load does not verify
certificates or check response bodies. Paired 95% intervals use the t
value for the pair count and all exclude zero.

| Cell (concurrent) | h2load | Leyline | wreq 0.16.1 | Leyline vs wreq |
| --- | ---: | ---: | ---: | --- |
| Go 10 B, 1 connection x 64 streams | 93,751 | 90,886 | 93,358 | -2.7% [-3,219, -1,725] |
| Go 10 B, 8 connections x 8 streams | 118,018 | 118,486 | 116,215 | +1.9% [+1,051, +3,491] |
| Hyper 10 B, 1 connection x 64 streams | 225,655 | 211,193 | 192,050 | +10.0% [+17,635, +20,650] |
| Hyper 10 B, 8 connections x 8 streams | 346,472 | 331,748 | 330,177 | +0.5% [-1,541, +4,682], includes zero |

Warm sequential and cold deltas, both origins, one connection, twenty pairs:

| Metric | Go | Hyper |
| --- | ---: | ---: |
| Warm sequential | +4.7% [+1,226, +1,511] | +17.3% [+7,688, +8,242] |
| Cold (full setup and teardown) | +3.6% [+23, +151] | +4.7% [+92, +183] |

The delayed-link cells use the fixed client against wreq. Positive favors
Leyline; intervals that include zero are parity:

| Origin | Body | RTT | Change | 95% interval | Pairs |
| --- | ---: | ---: | ---: | --- | ---: |
| Go | 16 KiB | 30 ms | -0.4% | [-26, +8] | 8 |
| Hyper | 16 KiB | 30 ms | +0.3% | [-8, +21] | 8 |
| Go | 16 KiB | 80 ms | -0.6% | [-9, 0] | 8 |
| Hyper | 16 KiB | 80 ms | +0.6% | [+3, +4] | 4 |
| Go | 4 MiB | 30 ms | +0.6% | [0, +1] | 4 |
| Hyper | 4 MiB | 30 ms | -1.3% | [-1, 0] | 8 |
| Go | 4 MiB | 80 ms | +0.1% | [0, 0] | 8 |
| Hyper | 4 MiB | 80 ms | +0.0% | [0, 0] | 4 |

These are loopback and emulated-link measurements on one host. They do not
establish network behavior, other payloads, or a universal ranking.
Browser-major labels do not prove identical requests or fingerprints;
capture the wire before claiming equivalence. Results are host-specific;
reproduce with the commands above and report the matching control row. The
per-round observations behind every interval are in
[`benches/comparison/results/2026-09-13-reference.json`](benches/comparison/results/2026-09-13-reference.json);
the summaries above are recomputed from those rounds.

## Extended matrix (2026-09-15)

Same host and pins. Eight balanced pairs per cell; every client byte-checks
every response (`CMP_BODY`), and every client except azuretls verifies TLS
against the test CA. `matched` sends the identical thirteen-header Chrome
149 block from both clients (`CMP_HEADERS`); `native` lets each client send
its own profile block. Per-round data lives in
`benches/comparison/results/2026-09-15-*.json`.

Concurrent req/s deltas, Go origin:

| Peer | 1 conn x 64 streams | 8 conns x 8 streams | 8 conns, 8 in-flight | Warm | Cold |
| --- | ---: | ---: | ---: | ---: | ---: |
| wreq 0.16.1 (native) | -1.4% [-4,683, +2,101] | +1.0% [-3,356, +5,667] | -4.2% [-6,682, -528] | +3.5% | +3.0% |
| wreq 0.16.1 (matched) | -2.1% [-3,939, -45] | — | — | +5.6% | +2.8% |
| reqwest 0.13.5 | -6.4% [-9,193, -2,999] | -14.8% [-25,733, -12,474] | — | -3.4% | -8.2% |
| tls-client 1.15.1 | +23.9% [+12,049, +22,118] | -7.5% [-14,780, -3,355] | -16.0% [-17,364, -13,805] | -10.2% | +17.6% |
| azuretls 1.13.2* | +41.0% [+19,731, +31,790] | +8.5% [+3,106, +14,433] | +12.6% [+6,444, +11,744] | +7.8% | +11.9% |

*azuretls runs an unverified TLS path; all its cells favor it slightly on
cold/setup phases.

Leyline without impersonation (`LEYLINE_CHROME=bare`) vs reqwest on the
1-conn cell: +0.3% [-5,407, +5,951] — the engine alone is at reqwest
parity, so the profile carries the -6.4% Chrome-vs-reqwest difference on
this cell.

The Go origin is the limiter in most of these cells: it saturates near
90-115k, so deltas reflect per-request server cost more than client
capacity. Two real exceptions stand out: at 8 conns x 8 streams reqwest
reaches 128.7k where Leyline holds ~110-112k, and tls-client reaches
120.2k — Leyline scales with connections on this origin, but less steeply.
The 8-conns/8-in-flight column keeps one request in flight per connection
and exposes per-request latency instead: Leyline's floor is about 16 µs
above tls-client's there, consistent with the warm-sequential loss.

Concurrent req/s deltas, Hyper origin (server has headroom):

| Peer | 1 conn x 64 streams | 8 conns, 8 in-flight |
| --- | ---: | ---: |
| wreq 0.16.1 (native) | +12.7% [+20,065, +29,317] | +1.1% [-1,344, +7,140] |

### Peak concurrent throughput (Hyper origin, client-bound)

At 8 connections x 256 in-flight requests the origin used about 14% of its
allocation; the clients set the rate. Eight balanced pairs, 10-byte bodies,
verified responses:

| Peer | Delta | 95% interval |
| --- | ---: | --- |
| wreq 0.16.1 (matched headers) | +33.5% | [+230,665, +269,213] |
| wreq 0.16.1 (native headers) | +45.1% | [+338,314, +356,424] |
| reqwest 0.13.5 | +20.4% | [+180,803, +199,007] |

Mean concurrent p50/p99 at that cell: leyline 219/441 µs, wreq 307/638 µs,
reqwest 258/549 µs. A single-run sweep found leyline's observed plateau
near 1.2M req/s (8x512 in-flight) and wreq's near 0.9M; whether that last
ceiling is client, loopback, or kernel is unresolved, so the numbers above
describe the paired delta, not an absolute maximum.

One connection of either client is a serial pipeline (one driver task each
side, ~2 cores at ~170k rps); capacity scales with connections and
in-flight depth, not worker threads.


## Compare with reqwest

`benches/benches/clients.rs` compares Leyline's default session, without
browser impersonation, against reqwest 0.13 with rustls.

```sh
cargo bench --manifest-path benches/Cargo.toml --bench clients
```

The benchmark starts a loopback TLS origin using hyper and tokio-rustls.
Separate listeners advertise HTTP/1.1 and HTTP/2. Both clients disable
certificate verification for the generated self-signed certificate.

| Scenario | Work per iteration |
| --- | --- |
| HTTP/1.1 keep-alive | 200 sequential requests for a 16 KiB body |
| HTTP/2 multiplexing | 32 concurrent requests for a 16 KiB body |
| Streamed download | One 4 MiB response, consumed through the streaming API |

Before timing, `verify` compares each client's response bytes with the
server fixture. It also checks the negotiated protocol for the HTTP/1.1 and
HTTP/2 requests. The fixture is `(0..len).map(|i| (i % 251) as u8)`.
A mismatch fails the benchmark before it produces timings.

Criterion records per-scenario timings under `benches/target/criterion/`.
A concurrent batch duration is not an individual request's latency.

## Separate Rust clients

The programs in `benches/comparison/` include these Rust clients:

| Client | Dependencies | Profile |
| --- | --- | --- |
| `leyline-client` | This checkout | Chrome 152 by default |
| `wreq-client` | wreq 0.16.1, wreq-util 0.2.0 | Chrome 149 |
| `reqwest-client` | reqwest 0.13.5 with rustls and HTTP/2 | No browser impersonation |

Build each client with its own locked manifest and the same Rust toolchain.
Rust 1.98.1 builds all three. Their release profiles use fat LTO and one
codegen unit. Their lockfiles resolve Tokio 1.53.1.

Each program accepts `URL WARM_COUNT COLD_COUNT CONCURRENT_COUNT CONCURRENCY`.
Every timed response must have status 200 and the expected body:
`ok-10byte!` by default, or the `CMP_BODY` fixture when set. The cold loop
creates a client for every request, so it includes client
construction, connection setup, TLS, response consumption, and teardown.
The concurrent phase also prints p50, p90, p99, and p99.9 microseconds over
every request. The `print` and `equiv` commands provide separate response
checks.

Set `LEYLINE_CHROME=149` to select Chrome 149 or `LEYLINE_CHROME=bare`
to disable browser impersonation in the Leyline client. Equal browser version
labels do not establish equal fingerprints: the libraries can select different
platform headers, header order, and TLS settings. Capture those settings when
reporting a browser comparison.

### Certificate verification

Without `CMP_CA`, these clients disable certificate verification for the
self-signed fixture. The disabled modes do different work. Reqwest 0.13.5's
rustls verifier skips handshake-signature checks; Leyline's BoringSSL path
still checks those signatures. Do not use this mode to compare normal,
verified TLS performance.

Set `CMP_CA` to a DER-encoded root CA certificate to enable certificate
and hostname verification. Each client trusts only that root. It reads the
file once before timing; client construction still configures the trust store.

The Go origin in `benches/comparison/go/server` and the Hyper origin in
`benches/examples/origin.rs` accept `CMP_CERT` and `CMP_KEY`.
Use a DER-encoded server certificate and an unencrypted PKCS#8 DER private key.
The certificate must be signed by the test CA and include the request host in
its subject alternative names. Both origins can use the same certificate and
key. With neither variable set, each generates a self-signed certificate.

Before timing, confirm that each client accepts the test CA and rejects an
unrelated CA and a hostname absent from the certificate. Keep the private test
keys out of Git.


Set `CMP_LOG_PROTO=1` on either origin to record the HTTP version, TLS
version, cipher, key-exchange group, and full or resumed handshake state.
Go also records whether a HelloRetryRequest occurred. Keep this logging off
while timing requests.

Set `CMP_TLS=matched` on both origins to restrict key exchange to X25519.
The Hyper origin also restricts the cipher to TLS 1.3 AES-128-GCM. Go selects
its TLS 1.3 cipher internally; confirm AES-128-GCM in the capture before
comparing clients. Report these controlled settings separately from defaults.

The Leyline client also accepts `URL trace COUNT`. It checks every response
and prints mean nanoseconds for session construction, DNS, TCP, TLS, and the
HTTP response. TLS includes time waiting for the server. The reported stages
do not cover all request preparation, scheduling, or teardown work. Tracing
adds overhead, so use it to diagnose costs rather than rank clients.

### Response sizes

Set `CMP_BODY` to a nonnegative byte count on the origin and Rust clients.
The fixture uses `byte[i] = i % 251`, matching the in-process benchmark.
Each process creates the fixture before timing. Clients compare every
response with the complete expected byte sequence, and `equiv` hashes raw
bytes. With `CMP_BODY` unset, the body remains `ok-10byte!`.

Use `equiv` for binary fixtures; `print` decodes the response as text.
Choose request counts and concurrency for the body size, and record both.
These programs measure buffered response consumption. They do not establish
streaming, upload, compression, or network-latency performance.

### Offered-load latency

The Rust comparison clients accept a `paced` mode that sends requests at a
fixed rate and reports service and coordinated-omission-corrected latency
percentiles:

```sh
./bin/leyline URL paced RATE_PER_SECOND SECONDS CONCURRENCY
```

The corrected percentile measures from the intended start time, so it
includes queueing when the offered rate exceeds capacity. Report the
achieved rate beside the corrected percentiles; above capacity the achieved
rate is the capacity estimate.

Set `CMP_CONNECTIONS` on both comparison clients to spread the worker tasks
over several connections. Match the control's `-c` and `-m` at the same
product: eight connections with eight streams is eight connections times
eight concurrent streams.

## Reference control (h2load)

`benches/comparison/control.sh` measures the origin with nghttp2's h2load,
an independent HTTP/2 client. A peer delta alone cannot show whether both
clients are near the transport's ceiling. The control supplies that ceiling
for the same origin, request headers, and connection/stream topology.

Install h2load from the `nghttp2-client` package (Linux) or `nghttp2`
(macOS). h2load does not verify peer certificates (OpenSSL's default is
`SSL_VERIFY_NONE`), so no CA is needed and the control measures an
unverified TLS path. The comparison clients verify; do not read the control
as verified-client evidence.

```sh
TARGET_URL=https://127.0.0.1:8443/ ./control.sh
```

The script sends the same thirteen non-pseudo Chrome 149 headers as the
comparison clients and runs four configurations: the light header set and
the browser header set on one connection with 64 streams, then browser
headers on two and eight connections. The light row shows the transport
ceiling without browser overhead. Do not compare a light-header number with
a browser-profile client number; the origin does less work for the light
request.

Limits: h2load uses OpenSSL, not BoringSSL, does not verify certificates,
and does not compare response bodies. It bounds the origin and the topology,
not the clients. Report the control row beside every published client
result. Treat a client number above the control as parity with the ceiling
unless repeated rounds show a difference outside the paired interval.

## Wide-area behavior (netem)

`benches/comparison/netem.sh` runs a paired comparison over a virtual link
with a real round-trip delay. The origin runs in its own network namespace
and only the veth pair is delayed, so no other service on the host is
affected. Loopback has almost no latency and hides window-management
defects; this cell found a large-response collapse that no loopback test
could see: at 30 ms RTT one four-MiB response measured 0.52 requests/s
against wreq's 28.95, fixed by commit `f49f4e1`.

Requirements: iproute2, openssl, passwordless sudo, and the DER test CA in
`CMP_CA`. The origin certificate must include the virtual link address in
its SAN; set `CMP_CA_KEY` to mint one from the test CA, or set
`NETEM_CERT` and `NETEM_KEY` to a prepared leaf.

```sh
CMP_CA=ca.der CMP_CA_KEY=ca-key.pem RTT_MS=30 ./netem.sh
```

`RTT_MS` defaults to 30. `ORIGIN`, `SERVER_CPUS`, and the `paired.sh`
variables (ROUNDS, CONC, ...) apply. Run the same cells at more than one
RTT; a result at a single delay is a screen, not a ranking.

## Performance accounting

`scripts/perf_accounting.py` runs one comparison client against a local
origin under `perf stat` counters for both processes and prints cycles,
instructions, task-clock, context switches, and page faults per completed
operation. The counters explain where a throughput difference lives; they
do not rank clients by themselves.

```sh
scripts/perf_accounting.py --client leyline --preset small
scripts/perf_accounting.py --client wreq --preset large --syscalls
scripts/perf_accounting.py --client leyline --preset small --record
```

Presets mirror the comparison workloads: small (ten-byte), medium (16 KiB),
large (4 MiB). Override the counts with `--warm`, `--cold`, `--requests`,
`--concurrency`, and `--body`. `--syscalls` adds BPF per-syscall counts
with bpftrace. `--record` captures 199 Hz DWARF call graphs and writes a
text report per round. Raw counters land in a JSON file next to the
captures (`PERF_OUT`, default `/tmp/leyline-perf-accounting`).

Requirements: Linux, perf with passwordless sudo, bpftrace for `--syscalls`.
Pair a profiling run with `paired.sh` for throughput and `netem.sh` for
wide-area behavior. An instrumented rate is diagnostic, not a benchmark.

## Browser-profile comparisons

The separate client programs under `benches/comparison/` compare Leyline,
wreq, reqwest, tls-client, and azuretls. `run.sh` builds and runs all five
against the local Go origin. They run in separate processes. Their browser
profiles and versions can differ: Leyline runs the selected Chrome profile,
wreq runs `Chrome149`, tls-client 1.15.1 runs `Chrome_146` (the newest it
ships), and azuretls 1.13.2 runs its `Chrome` profile. With `CMP_BODY` set,
all five clients byte-check every response; tls-client verifies TLS against
`CMP_CA`, while azuretls runs unverified TLS.

`paired.sh` runs the `LEFT` and `RIGHT` clients in alternating order each
round (default `leyline` and `wreq`) and reports the paired difference with
a 95% interval, the winning rounds, and the mean concurrent p50/p99
latencies. The equivalence gate compares the two clients' complete
responses before any timing. When h2load is installed it also prints the
reference control row from `control.sh`.

### The reqwest baseline

reqwest applies no browser profile, so it anchors the fingerprint tax.
Three pairings decompose it:

- `RIGHT=reqwest ./paired.sh` pairs Leyline's Chrome profile against
  reqwest's natural request.
- `LEYLINE_CHROME=bare RIGHT=reqwest ./paired.sh` pairs Leyline without
  impersonation against reqwest. The distance between this cell and the
  Chrome cell is what the fingerprint itself costs.
- Set `CMP_HEADERS` to a tab-separated name/value file to send the same
  request header block from all three clients; leave it unset so each
  client sends its natural request.

Publish reqwest rows with `CMP_CA` set so both clients run the verified
TLS path.

## Recording a result

Publish results from a committed harness with:

- The full source revision, dependency lockfiles, browser profiles, and build
  options.
- Hardware, OS, toolchain versions, CPU affinity, and background load.
- Request counts, concurrency, payload sizes, connection reuse, and TLS
  verification settings.
- The h2load control row for every origin and body size.
- The response-equivalence output and raw measurements from each round.
- The run order and statistical method, with uncertainty beside the estimate.

Keep non-impersonating HTTP comparisons separate from browser-profile
comparisons. Report each scenario's result, including regressions.
