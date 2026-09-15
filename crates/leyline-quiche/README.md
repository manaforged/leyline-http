# leyline-quiche

A vendored fork of [`cloudflare/quiche`](https://github.com/cloudflare/quiche)
v0.23.7 that links against [`leyline-bssl`](../leyline-bssl) instead
of `boring`, so [`leyline-http`](https://crates.io/crates/leyline-http)'s HTTP/2 and
HTTP/3 ClientHellos share the same patched BoringSSL build.

**Most users do not want this crate directly.** Depend on `leyline-http`;
import `leyline`. It pulls `leyline-quiche` in when the `http3` feature is on.


## License

BSD-2-Clause, inherited from upstream `cloudflare/quiche`. Local changes
by Manaforge Technologies, LLC are released under the same license. Full attribution
in the workspace [`NOTICE`](https://github.com/manaforged/leyline-http/blob/main/NOTICE).
