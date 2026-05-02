# Wire Evidence

Leyline's public claim is not "we have many profiles." The claim is that a
selected profile produces a coherent wire identity: TLS ClientHello, ALPN, H2
SETTINGS, pseudo-header order, TCP shape, identity headers, cookie behavior,
redirect behavior, proxy behavior, and response metadata all agree.

This document is the evidence ledger for that claim.

## Local Gates

Run before release:

```powershell
cargo check -p leyline
cargo check -p leyline --no-default-features
cargo test -p leyline --lib
cargo test --workspace --exclude leyline-quiche
```

## Live Gates

Run when network access and public endpoints are available:

```powershell
cargo test -p leyline --test tls_peet -- --ignored --nocapture
cargo test -p leyline --test smoke -- --ignored --nocapture
```

Record the result with:

- date
- commit SHA
- OS and architecture
- Rust version
- profile under test
- observed JA3
- observed JA4
- observed JA4T
- observed JA4H
- observed Akamai H2 fingerprint
- relevant TLS extensions
- ALPN
- proxy mode, if any
- pass/fail notes

## Release Evidence Template

```text
Date:
Commit:
Host:
Rust:

Profile:
Expected JA4:
Observed JA4:
Expected H2:
Observed H2:
Observed JA4T:
Observed JA4H:
ALPN:
TLS version:
Cipher:

Result:
Notes:
```

## Current Required Properties

- TLS ClientHello matches the selected profile's expected JA4.
- H2 SETTINGS and pseudo-header order match the selected profile's expected
  Akamai fingerprint.
- Request headers preserve browser order after caller overrides.
- Cross-origin redirects strip sensitive headers.
- Cookies round-trip with browser-compatible path ordering and domain rules.
- HTTP and SOCKS proxy wire bytes match their RFC shapes.
- H1 fallback occurs only on ALPN mismatch or explicit HTTP/1 policy.
- H3 is explicit, feature-gated, and rejected when paired with a proxy.
- Response audit data is attached to every buffered response.

## Known Non-Goals

- Profile count is not a quality metric.
- Very old browser profiles are not maintained for completeness alone.
- H3 proxy tunneling is not currently implemented.
- H3 request/response streaming is not currently implemented.
