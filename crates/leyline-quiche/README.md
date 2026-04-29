# leyline-quiche

A vendored fork of [`cloudflare/quiche`](https://github.com/cloudflare/quiche)
v0.23.7 that links against [`btls`](https://crates.io/crates/btls) instead
of `boring`, so [`leyline`](https://crates.io/crates/leyline)'s HTTP/2 and
HTTP/3 ClientHellos share the same patched BoringSSL build.

**Most users do not want this crate directly.** Depend on `leyline`; it
pulls `leyline-quiche` in transparently when you call `.http3()` on a
`SessionBuilder`.

## License

BSD-2-Clause, inherited from upstream `cloudflare/quiche`. Local changes
by Thomas Gardiner are released under the same license. Full attribution
in the workspace [`NOTICE`](https://github.com/manaforged/leyline-http/blob/main/NOTICE).
