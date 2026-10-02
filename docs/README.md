# Leyline user guide

Leyline is an HTTP client that mimics browsers on the wire. This guide shows you
how to use it, in reading order.

Leyline has two kinds of session:

| You want | Call | What the server sees |
| --- | --- | --- |
| Plain HTTP | `leyline::get(url)` or `Session::new()` | A plain client with a `leyline/<version>` user agent. |
| Look like a browser | `Session::browser(Browser::default())` or `Session::builder().browser(Browser::Chrome154)` | The TLS, HTTP/2, and headers of a captured browser. |

`Session::browser(Browser::default())` impersonates the newest bundled Chrome
on Windows. `Session::new()` and a builder with no browser build a plain client.

1. [Quick start](guide/quick-start.md): add the dependency, send your first request.
2. [Sessions](guide/sessions.md): pick a browser, a platform, and a brand.
3. [Choosing a profile](guide/choosing-a-profile.md): which browser and platform to claim.
4. [Requests](guide/requests.md): methods, headers, bodies, presets, tabs, and downloads.
5. [Responses](guide/responses.md): status, headers, `Link` pagination, block detection, bodies, timing, audit.
6. [Streaming](guide/streaming.md): stream a body in either direction, download a file, or stop a read early.
7. [Redirects](guide/redirects.md): the redirect limit and custom redirect policies.
8. [Retries and timeouts](guide/retries-and-timeouts.md): retry policies and the four timeouts.
9. [Errors](guide/errors.md): group errors by category, map them to statuses, and find out which errors a retry covers.
10. [Proxies](guide/proxies.md): proxy rules, bypass lists, environment discovery.
11. [Cookies](guide/cookies.md): the jar, saving it, devices, and connection state.
12. [WebSocket](guide/websocket.md): connect over HTTP/2 or HTTP/1.1.
13. [HTTP/3](guide/http3.md): the `http3` feature and the protocol policy.
14. [TLS trust](guide/tls-trust.md): roots, certificate pins, client certificates, the TLS version floor.
15. [Network](guide/network.md): DNS overrides, Happy Eyeballs, socket options.
16. [Logging and tracing](guide/logging.md): `tracing` targets and request phases.
17. [Fingerprints](guide/fingerprints.md): what a profile pins and how to audit it.
18. [Features and targets](guide/features-and-targets.md): cargo features, targets, MSRV.
19. [Supported platforms](guide/platforms.md): the six build targets and the tools each one needs.
20. [Cancellation](guide/cancellation.md): what happens when you drop a request or a body.
21. [Testing](guide/testing.md): local plain and TLS servers, and asserting what was sent.
22. [Service integration](guide/service-integration.md): Tower, shared sessions, and error mapping in a server.
23. [Crawling](guide/crawling.md): host limits, proxy pools, block detection, and tags.
24. [Accounts](guide/accounts.md): tabs, saved devices, and the state of a returning browser.

Reference pages beside this guide: [profiles](guide/profiles.md) and
[MSRV](guide/msrv.md).

The Rust blocks in these pages are doctests of the `leyline-http` crate. The
Tower block in [Requests](guide/requests.md) compiles only with the `tower`
feature. A plain `cargo test` does not enable it. The pull-request check in CI
runs `cargo test` with
`--features leyline-http/full,leyline-http/bench-internals`, which does. The
`toml`, `sh`, and `js` blocks are not compiled. Blocks that open a socket are
marked `no_run`, so they compile but do not send traffic.
