# Client comparison harness

Head-to-head HTTP performance: leyline vs the other libraries in its space, all
racing one shared local HTTPS/2 server. This is the data behind the
"TLS-impersonation libraries" section of [`../../BENCHMARKS.md`](../../BENCHMARKS.md).

## What it measures

Each client runs two phases against the same server and prints one `RESULT`
line (`warm_rps` / `cold_rps`):

- **warm** — `WARM` sequential GETs on one reused, pooled client. The connection
  is established once, so this is the per-request stack cost (H2 framing +
  request build + response parse); the TLS handshake is amortized to ~0.
- **cold** — `COLD` GETs, each on a fresh client (full TLS handshake +
  ClientHello fingerprint generation per request). The handshake/impersonation
  cost.

`req/s = requests / wall-seconds`.

## Clients

| Client | Lang | TLS stack | Impersonates |
|---|---|---|---|
| leyline | Rust | leyline-bssl (BoringSSL fork) | Chrome 150 |
| wreq | Rust | boring (BoringSSL) | Chrome 137 |
| bogdanfinn/tls-client | Go | utls | Chrome 146 |
| azuretls | Go | utls | Chrome (latest) |

Each library impersonates its own newest Chrome — there is no Chrome major all
four support (leyline starts at 145, wreq tops out at 137) — so the fingerprints
are Chrome-class but not byte-identical. `verify-fingerprints.sh` hits
tls.peet.ws once per client and prints each JA4 as evidence they all emit a
Chrome ClientHello, so the perf numbers compare equal work.

## Why this is a harness, not one `cargo bench`

leyline and wreq each vendor their own BoringSSL. Cargo forbids two crates
declaring the same `links` native library in one graph, and even otherwise two
BoringSSL archives export the same C symbols and collide at link time — so they
cannot share a binary. tls-client and azuretls are Go. Hence every client is its
own process and the comparison is an external runner, not an in-process
criterion bench.

## Run

```bash
./run.sh                      # warm=2000 cold=200, server on 127.0.0.1:8443
WARM=5000 COLD=500 ./run.sh   # heavier
./verify-fingerprints.sh      # JA4 per client (needs network)
```

Needs a Go toolchain and a Rust toolchain. The wreq client builds BoringSSL from
source on first run (cmake + clang + go + perl); set `WREQ_TARGET` to a shared
target dir to reuse an existing BoringSSL build and skip that.
