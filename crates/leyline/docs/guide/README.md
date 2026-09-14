# Leyline user guide

Leyline is an HTTP client that mimics browsers on the wire. This guide shows you
how to use it, in reading order.

1. [Quick start](quick-start.md): add the dependency, send your first request.
2. [Sessions](sessions.md): pick a browser, a platform, and a brand.
3. [Requests](requests.md): methods, headers, query strings, bodies, presets.
4. [Responses](responses.md): status, headers, cookies, bodies, timing, audit.
5. [Streaming](streaming.md): stream a request body or a response body.
6. [Retries and timeouts](retries-and-timeouts.md): retry policies and the four timeouts.
7. [Proxies](proxies.md): proxy rules, bypass lists, environment discovery.
8. [Cookies](cookies.md): the jar, sharing it, and what it stores.
9. [WebSocket](websocket.md): connect over HTTP/2 or HTTP/1.1.
10. [HTTP/3](http3.md): the `http3` feature and the protocol policy.
11. [Fingerprints](fingerprints.md): what a profile pins and how to audit it.
12. [Features and targets](features-and-targets.md): cargo features, targets, MSRV.

See [Responses](responses.md) for status handling and body errors.

Reference pages beside this guide: [profiles](../PROFILES.md) and
[MSRV](../MSRV.md).

Every Rust block in this guide is compiled as a doctest of the
`leyline-http` crate; the Tower blocks compile only under
`--features tower`, and the `toml` blocks are not compiled. Blocks that would
open a socket are marked `no_run`, so they compile but do not send traffic.
