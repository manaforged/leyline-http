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
  requests on one connection, where tls-client is 7.7% faster. Leyline's p99
  latency is lower in every concurrent scenario.
- reqwest sends no browser profile, so it does less work per request and per
  new connection than Leyline or wreq. The comparison with reqwest includes
  that difference.

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

They were mostly idle, and CPUs 7 and 23 were left
free for them.

## Limits

- One machine, one day, loopback only. Loopback removes network latency, so
  the results show client CPU cost. On a real network the round trip
  dominates, and the clients get closer together.
- One response size (10 bytes). Large bodies measure decompression and copy
  cost, which this table does not cover.
- HTTP/2 only. HTTP/1.1 and HTTP/3 are not compared here.

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

`matrix.sh` builds the four clients and the Hyper server, runs the nine
cells, and writes one JSON file and one log per cell to
`results/<date>/`. `PEERS`, `ROUNDS`, `SERVER_CPUS`, and `CLIENT_CPUS` override
the defaults. Set the CPU governor to `performance` before the run for stable
results.
