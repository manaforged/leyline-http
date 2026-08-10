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

## CFNetwork Capture Ledger (first entries, 2026-08-09)

The cfnetwork family is anchored to **our own captures** (tls.peet.ws
observations of the `tools/cfnetwork-capture` probe + the capture server's
raw ClientHello log) — no public browser reference applies to an app-stack
identity. Recapture rig: `tools/cfnetwork-capture/` (permanent; also the
freshness rig for future iOS releases).

| Profile | captured_against | ja4 | Akamai H2 | Verified |
| --- | --- | --- | --- | --- |
| `cfnetwork/macos26.toml` | CFNetwork/3860.600.21 Darwin/25.5.0 (macOS 26.6) | `t13d2013h2_a09f3c656075_7f0f34a4126d` | `2:0;4:4194304;3:100;9:1|10485760|0|m,s,p,a` | 3 runs stable; live gate `live_cfnetwork_macos26_matches_capture` |
| `cfnetwork/ios18.toml` | CFNetwork/3826.600.41 iOS 18.6 (22G86) sim | `t13d2014h2_a09f3c656075_7f0f34a4126d` | `2:0;4:2097152;3:100|10485760|0|m,s,p,a` | 3 runs stable; live gate `live_cfnetwork_ios18_matches_capture` |

Key captured facts baked into the profiles (all first-party, 3-run stable):

- GREASE cipher at the front of the cipher list; GREASE extensions first/last.
- Duplicate sigalg `rsa_pss_rsae_sha384` (0x0805) — required the carried
  BoringSSL patch (leyline-fingerprint.patch) allowing duplicate sigalgs.
- `compress_certificate = zlib` (Apple; browsers advertise brotli).
- Pseudo-header order `m,s,p,a` (distinct from Safari's `m,s,a,p` and
  Chrome's `m,a,s,p`).
- iOS 18.6: no ML-KEM, no `unknown9` H2 setting, TLS 1.3 order AES_128 first,
  `supported_versions` advertises TLS 1.0/1.1, RFC 7685 padding extension
  (394 bytes at SNI tls.peet.ws — BoringSSL's built-in 512-target padding).
- macOS 26: ML-KEM768 group + keyshare, `unknown9 = 1` H2 setting, TLS 1.3
  order AES_256 first, no padding extension.

