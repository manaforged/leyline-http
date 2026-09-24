# leyline-bssl

A safe Rust wrapper over
[`leyline-bssl-sys`](https://crates.io/crates/leyline-bssl-sys). It is a
trimmed fork of Cloudflare's [`boring`](https://github.com/cloudflare/boring)
v5.2.0, with wrappers for the carried BoringSSL patches. It exposes the `SslConnector`, `SslContextBuilder`, `Ssl`, and `X509` types that
[`leyline-http`](https://crates.io/crates/leyline-http) needs. Async streams
are in [`leyline-bssl-tokio`](https://crates.io/crates/leyline-bssl-tokio).

Depend on `leyline-http` instead of this crate. Its API is outside the
`leyline-http` semver promise.

Upstream attribution is in the repository
[NOTICE](https://github.com/manaforged/leyline-http/blob/main/NOTICE).
