# Benchmarks

This page compares Leyline with wreq, reqwest, and tls-client on one machine. All numbers
come from the files in
[`benches/comparison/results/2026-09-24`](benches/comparison/results/2026-09-24).

## Results

Each row is 20 paired rounds. In a round, both clients run the same workload
against the same server, and the order alternates between rounds. The delta is
Leyline's mean request rate minus the peer's, divided by the peer's. The
interval is the paired 95% confidence interval of that delta. "Rounds won"
counts the rounds in which Leyline completed more requests per second.

| Scenario | Peer | Leyline req/s | Peer req/s | Delta | 95% interval | Rounds won | Leyline p50 / p99 µs | Peer p50 / p99 µs |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Sequential, one connection | wreq | 39,232 | 34,027 | +15.3% | +13.6% to +17.0% | 20/20 | | |
| Sequential, one connection | reqwest | 39,103 | 35,556 | +10.0% | +8.2% to +11.7% | 20/20 | | |
| New client per request | wreq | 1,949 | 1,762 | +10.6% | +8.1% to +13.1% | 20/20 | | |
| New client per request | reqwest | 1,920 | 2,334 | -17.7% | -20.3% to -15.2% | 0/20 | | |
| 1 connection, 64 streams | wreq | 172,132 | 155,458 | +10.7% | +10.2% to +11.2% | 20/20 | 349 / 589 | 404 / 582 |
| 1 connection, 64 streams | reqwest | 173,684 | 181,194 | -4.1% | -4.6% to -3.7% | 0/20 | 347 / 579 | 341 / 536 |
| 8 connections, 8 in flight | wreq | 257,134 | 234,088 | +9.8% | +4.0% to +15.7% | 19/20 | 28 / 68 | 31 / 81 |
| 8 connections, 8 in flight | reqwest | 259,046 | 262,656 | -1.4% | -2.6% to -0.1% | 7/20 | 27 / 66 | 28 / 67 |
| 8 connections, 256 in flight | wreq | 1,059,129 | 815,119 | +29.9% | +28.2% to +31.7% | 20/20 | 221 / 634 | 298 / 612 |
| 8 connections, 256 in flight | reqwest | 1,090,449 | 1,031,505 | +5.7% | +5.2% to +6.2% | 20/20 | 217 / 578 | 236 / 479 |
| Sequential, one connection | tls-client | 39,031 | 42,294 | -7.7% | -9.1% to -6.4% | 1/20 | | |
| New client per request | tls-client | 1,921 | 1,378 | +39.5% | +37.3% to +41.6% | 20/20 | | |
| 1 connection, 64 streams | tls-client | 169,089 | 82,339 | +105.4% | +103.6% to +107.1% | 20/20 | 356 / 598 | 912 / 2015 |
| 8 connections, 8 in flight | tls-client | 249,586 | 211,296 | +18.1% | +16.1% to +20.2% | 20/20 | 28 / 75 | 31 / 147 |
| 8 connections, 256 in flight | tls-client | 1,014,179 | 511,152 | +98.4% | +93.5% to +103.3% | 20/20 | 228 / 711 | 483 / 1655 |

The latency columns are the per-round concurrent p50 and p99, averaged over
the 20 rounds. The sequential and new-client rows come from the
1-connection runs, which time 2,000 sequential requests and 200 new clients
per round.

What the table shows:

- Against wreq, Leyline completes more requests per second in every scenario.
  Its p99 latency is 7 µs higher at 1 connection and 22 µs higher at 256 in
  flight.
- Against reqwest, Leyline is faster on sequential requests and at 256 in
  flight. It is slower when it opens a new client per request, at 64 streams
  on one connection, and by a small margin at 8 in flight. Its p99 latency is
  higher at 64 streams and at 256 in flight.
- Against tls-client, Leyline is faster in every scenario except sequential
  requests on one connection, where Leyline is 7.7% slower (39,031 requests
  per second against 42,294). Leyline's p99 latency is lower in every
  concurrent scenario.
- reqwest sends no browser profile, so it does less work per request and per
  new connection than Leyline or wreq. The comparison with reqwest includes
  that difference.

## Client libraries

Thirteen HTTP/2 clients ran the same requests against the same local origin on 2026-10-31. The files are in [`benches/comparison/results/2026-10-31-libs`](benches/comparison/results/2026-10-31-libs), and [`benches/comparison/libs.sh`](benches/comparison/libs.sh) reproduces them. Each value is the median of 3 rounds in alternating order. Every client checks the status and full body of every response and holds exactly the stated number of connections. Leyline is commit `beef1b9d`, with no browser profile.

Requests per second:

| Client | Sequential | 1 conn, 64 streams | 8 conns, 8 in flight | 8 conns, 256 in flight |
| --- | ---: | ---: | ---: | ---: |
| Leyline | 32,481 | 206,727 | 228,290 | 982,506 |
| hyper-util client | 35,247 | 172,041 | 255,506 | 1,106,986 |
| h2 crate (no client layer) | 38,455 | 156,048 | 271,817 | 1,197,853 |
| reqwest | 31,978 | 168,936 | 212,142 | 822,686 |
| wreq | 30,775 | 142,765 | 193,803 | 648,983 |
| libcurl (multi) | 39,312 | 95,651 | 197,486 | 566,666 |
| Go net/http | 37,332 | 95,124 | 204,348 | 489,990 |
| Go tls-client | 36,848 | 68,854 | 185,206 | 383,916 |
| OkHttp | 22,556 | 120,870 | 161,806 | 235,455 |
| JDK HttpClient | 17,459 | 98,228 | 117,919 | 244,982 |
| Node http2 (no client layer) | 20,699 | 61,936 | 39,927 | 62,242 |
| undici | 14,121 | 45,351 | 30,370 | 40,412 |
| httpx | 2,726 | 2,835 | 3,163 | 2,672 |

User-space instructions per request, measured as the difference between a run of n requests and a run of 2n, so process startup and setup cancel out:

| Client | Sequential | 1 conn, 64 streams | 8 conns, 8 in flight | 8 conns, 256 in flight |
| --- | ---: | ---: | ---: | ---: |
| Leyline | 67.2k | 47.3k | 62.5k | 44.4k |
| hyper-util client | 51.9k | 39.6k | 46.0k | 35.1k |
| h2 crate (no client layer) | 35.7k | 26.5k | 31.0k | 22.8k |
| reqwest | 89.0k | 74.0k | 80.5k | 69.5k |
| wreq | 87.6k | 70.6k | 79.4k | 64.3k |
| libcurl (multi) | 100.6k | 83.4k | 100.7k | 82.6k |
| Go net/http | 89.8k | 83.2k | 64.6k | 70.5k |
| Go tls-client | 106.2k | 105.1k | 97.5k | 99.4k |
| OkHttp | 103.2k | 91.1k | 93.5k | 86.7k |
| JDK HttpClient | 1,601.4k | 115.1k | 108.0k | 111.0k |
| Node http2 (no client layer) | 293.7k | 236.6k | 256.2k | 246.0k |
| undici | 392.6k | 335.3k | 339.1k | 383.6k |
| httpx | 3,380.2k | 3,216.8k | 3,296.3k | 3,196.5k |

The h2 crate and Node http2 rows are HTTP/2 engines with no client layer on top: no URL parsing, pooling, redirects, cookies or response object. They show what the client layer costs. hyper-util is the layer that reqwest builds on. Leyline uses fewer instructions per request than every client except hyper-util, and it leads in requests per second with 1 connection and 64 streams. hyper-util is ahead of it with 8 connections, by 11.9% at 8 in flight and 12.7% at 256 in flight. In the sequential row, which varied 11% to 26% between rounds, hyper-util, libcurl, Go net/http, and tls-client are ahead of it. The JDK sequential instruction count includes JIT compilation that the short run does not cancel.

## HTTP/1.1 client libraries

Five HTTP/1.1 clients ran the same requests on 2026-11-01, once over loopback and once over a link with added delay. The files are in [`benches/comparison/results/2026-11-01-h1-r10b`](benches/comparison/results/2026-11-01-h1-r10b) and [`benches/comparison/results/2026-11-01-h1-net100us-b`](benches/comparison/results/2026-11-01-h1-net100us-b). Each value is the median of 10 rounds in alternating order. Every client sends the same three request headers, which the origin checks before the rounds start, and every client checks the status and body of every response. Leyline is commit `d6d58d15`, with no browser profile. These runs used Linux 7.0.0-38-generic, Rust 1.98.1 and Go 1.27.3, with HTTP/1.1 over TLS and the CPU pinning listed under Setup.

`leyline` is the async `Session` with tasks on the tokio multi-thread runtime. `leyline-blocking` is `blocking::Session`, one per thread. ureq has one `Agent` per thread. fasthttp runs one goroutine per request in flight. hyper is the hyper-util client on tokio.

Requests per second over loopback (round trip about 15 µs), with CPU time per request in µs:

| Client | Sequential | 8 in flight | 64 in flight |
| --- | ---: | ---: | ---: |
| Leyline blocking | 64,665 (8.27) | 392,900 (9.47) | 976,540 (14.12) |
| Leyline async | 63,806 (8.73) | 343,810 (10.12) | 846,651 (11.07) |
| fasthttp 1.77.0 | 66,980 (9.02) | 365,365 (11.04) | 950,530 (11.73) |
| ureq 3.5.0 | 62,493 (9.14) | 368,049 (10.83) | 992,121 (14.88) |
| hyper 1.12.0 | 60,391 (9.51) | 331,848 (11.28) | 744,428 (14.82) |

Requests per second over a network namespace link with 50 µs of netem delay each way (ping round trip 0.12 to 0.25 ms):

| Client | Sequential | 8 in flight | 64 in flight |
| --- | ---: | ---: | ---: |
| Leyline blocking | 7,863 (5.86) | 61,182 (7.09) | 389,497 (10.27) |
| Leyline async | 7,821 (6.23) | 57,344 (7.23) | 375,199 (8.87) |
| fasthttp 1.77.0 | 7,734 (9.42) | 57,286 (8.60) | 374,338 (11.19) |
| ureq 3.5.0 | 7,769 (6.80) | 61,307 (8.42) | 387,141 (11.90) |
| hyper 1.12.0 | 7,802 (6.88) | 57,134 (8.51) | 366,802 (11.00) |

The ratio of Leyline blocking to the fastest other client, paired by round, with its bootstrap 95% interval:

| Cell | Loopback | Delayed link |
| --- | --- | --- |
| Sequential | 0.966 (0.957 to 0.974) vs fasthttp: behind | 1.008 (1.005 to 1.011) vs hyper: ahead |
| 8 in flight | 1.063 (1.038 to 1.086) vs ureq: ahead | 0.999 (0.995 to 1.004) vs ureq: tie |
| 64 in flight | 1.002 (0.950 to 1.062) vs ureq: tie | 1.014 (1.003 to 1.026) vs ureq: ahead |

What the tables show:

- Leyline blocking uses the least CPU per request in every cell except 64 in flight, where Leyline async uses less.
- Over loopback, fasthttp is about 3.5% faster on sequential requests while using 9% more CPU per request. A profile shows Go's scheduler spinning briefly before it sleeps, which shortens the wait for each response. With 0.1 ms of added round trip that lead is gone, and fasthttp is last.
- At 64 in flight over loopback, the rounds varied by 16% to 22% for the three fastest clients, so the interval does not separate Leyline blocking from ureq.
- Leyline async is behind Leyline blocking and ureq with 8 or 64 requests in flight, by 4% to 15%.

## Setup

| Item | Value |
| --- | --- |
| CPU | AMD Ryzen 9 9950X3D, 16 cores, two CCDs, boost on, `performance` governor |
| OS | Linux 7.0.0-31-generic |
| Rust | 1.98.1 |
| Build | `cargo build --release` for the Rust clients with no profile overrides; `go build` for tls-client |
| Transport | HTTP/2 over TLS on loopback, 10-byte response body |
| Server | Hyper 1.x over tokio-rustls (`benches/examples/origin.rs`) on CPUs 0-6 and 16-22 |
| Clients | CPUs 8-15 and 24-31, one client at a time |
| Leyline | Chrome 149 profile against wreq and reqwest, Chrome 152 against tls-client |
| wreq | 0.16.1 with `wreq-util` 0.2.0, `Emulation::Chrome149` |
| reqwest | 0.13.5 with rustls, HTTP/2 prior knowledge, no browser profile |
| tls-client | 1.16.0, `profiles.Chrome_152`, built with Go 1.27.1 |

Every client trusts only the test CA and verifies the server certificate and
host name. Before the rounds start, an equivalence check confirms that both
clients receive the same status and body. Every response body is checked
during the rounds. Each client sends its own default request headers, so
Leyline, wreq, and tls-client send Chrome's header set and reqwest sends its
short default set.

Each result file records the Leyline revision, the kernel, the CPU, the
governor, the CPU pinning, the load average, the peer versions, and the
SHA-256 of each client binary.

The host was not dedicated. The pinning left CPUs 7 and 23 free of the
server and the clients.

## Limits

- One machine. The HTTP/2 tables ran over loopback only, which removes network latency, so they show client CPU cost. The HTTP/1.1 delayed-link run emulates the round trip with netem on a veth pair; it is not a physical network.
- One response size (10 bytes). Large bodies measure decompression and copy
  cost, which this table does not cover.
- HTTP/3 is not compared here.

## Reproduce

You need Linux, Rust 1.96 or later, Go 1.26 or later, Python 3, `taskset`,
and OpenSSL.

1. Create a test CA and a server certificate for `127.0.0.1`:

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

2. Run the matrix from `benches/comparison`:

   ```bash
   CMP_CA=$PWD/pki/ca.der CMP_CERT=$PWD/pki/server.der CMP_KEY=$PWD/pki/server-key.der \
     ./matrix.sh
   ```

3. Run the client-library comparison from `benches/comparison`. It also needs `perf`, Node.js 22.19 or later, Java 21, and a Python with `python-client/requirements.txt` installed:

   ```bash
   CMP_CA=$PWD/pki/ca.der CMP_CERT=$PWD/pki/server.der CMP_KEY=$PWD/pki/server-key.der \
     SERVER_CPUS=0-6,16-22 CLIENT_CPUS=8-15,24-31 ./libs.sh
   ```

`matrix.sh` builds the four clients and the Hyper server, runs the nine
cells, and writes one JSON file and one log per cell to
`results/<date>/`. `PEERS`, `ROUNDS`, `SERVER_CPUS`, and `CLIENT_CPUS` override
the defaults. Set the CPU governor to `performance` before the run for stable
results.

4. Run the HTTP/1.1 comparison with `PROTO=h1 ROUNDS=10`. For the delayed link, put the origin in a network namespace joined to the host by a veth pair with `tc qdisc add dev <veth> root netem delay 50us` on each end, issue a server certificate that also covers the namespace address, and set `SERVER_NETNS=<namespace>` and `SERVER_ADDR=<address>:0`.

`libs.sh` builds all thirteen clients, checks each one's response body, and writes `rows.txt`, `table.txt`, and `receipts.txt` (versions, binary hashes, and host state) to `results/<date>-libs/`. `LIBS`, `CELLS`, and `ROUNDS` override the defaults.
