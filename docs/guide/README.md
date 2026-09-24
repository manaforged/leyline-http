# Leyline user guide

Leyline is an HTTP client that mimics browsers on the wire. This guide shows you
how to use it, in reading order.

1. [Quick start](quick-start.md): add the dependency, send your first request.
2. [Sessions](sessions.md): pick a browser, a platform, and a brand.
3. [Choosing a profile](choosing-a-profile.md): which browser and platform to claim.
4. [Requests](requests.md): methods, headers, query strings, bodies, presets.
5. [Responses](responses.md): status, headers, cookies, bodies, timing, audit.
6. [Streaming](streaming.md): stream a body in either direction, or stop a read early.
7. [Redirects](redirects.md): the redirect limit and custom redirect policies.
8. [Retries and timeouts](retries-and-timeouts.md): retry policies and the four timeouts.
9. [Errors](errors.md): match on `Kind` and find out which errors a retry covers.
10. [Proxies](proxies.md): proxy rules, bypass lists, environment discovery.
11. [Cookies](cookies.md): the jar, sharing it, and what it stores.
12. [WebSocket](websocket.md): connect over HTTP/2 or HTTP/1.1.
13. [HTTP/3](http3.md): the `http3` feature and the protocol policy.
14. [TLS trust](tls-trust.md): roots, certificate pins, client certificates.
15. [Network](network.md): DNS overrides, Happy Eyeballs, socket options.
16. [Logging and tracing](logging.md): `tracing` targets and request phases.
17. [Fingerprints](fingerprints.md): what a profile pins and how to audit it.
18. [Features and targets](features-and-targets.md): cargo features, targets, MSRV.
19. [Supported platforms](platforms.md): build targets and the minimum glibc.

Reference pages beside this guide: [profiles](profiles.md) and
[MSRV](msrv.md).

The Rust blocks in these pages are doctests of the `leyline-http` crate. The
Tower blocks in [Requests](requests.md) compile only under `--features tower`,
and the default test run does not enable it. The `toml`, `sh`, and `js` blocks
are not compiled. Blocks that open a socket are marked `no_run`, so they
compile but do not send traffic.
