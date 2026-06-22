# leyline-bssl

Leyline's owned safe BoringSSL wrapper. Forked from [`btls`](https://crates.io/crates/btls)
(itself a fork of `0x676e67/boring2`), it exposes the `SslConnector` /
`SslContextBuilder` / `Ssl` / `X509` surface leyline's TLS fingerprinting needs,
over [`leyline-bssl-sys`](../leyline-bssl-sys) — the crate that pins the exact
BoringSSL revision and carries the Firefox C patches.

Detached from the leyline workspace (own `[workspace]`) so it builds only as a
path dependency, exactly as the upstream crate did. Async streams live in
[`leyline-bssl-tokio`](../leyline-bssl-tokio).

Upstream attribution is in the repo `/NOTICE`.
