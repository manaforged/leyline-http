# Leyline user guide

Leyline is an async HTTP/1.1, HTTP/2, and HTTP/3 client for Rust with sessions, crawling, encrypted DNS (DoH) and TLS 1.3 ECH, proxies, and optional browser TLS profiles. This guide shows you how to use it, in reading order.

Leyline has two kinds of session:

| You want | Call | What the server sees |
| --- | --- | --- |
| Plain HTTP | `Session::new()` | A plain client with a `leyline/<version>` user agent. |
| Browser profile | `Session::from(Browser::default())` or `Session::builder().browser(Browser::Chrome158).build()?` | The TLS, HTTP/2, HTTP/3, and headers of a captured browser. |

`Session::from(Browser::default())` sends the newest bundled Chrome profile on Windows. `Session::new()` and a builder with no browser build a plain client.

1. [Quick start](guide/quick-start.md): add the dependency, send your first request.
2. [API clients](guide/api-clients.md): base URLs, auth, JSON, retries, pagination, and shared sessions.
3. [Sessions](guide/sessions.md): pick a browser, a platform, and a brand.
4. [Choosing a profile](guide/choosing-a-profile.md): which browser and platform to claim.
5. [Requests](guide/requests.md): methods, headers, bodies, presets, tabs, and downloads.
6. [Responses](guide/responses.md): status, headers, `Link` pagination, block detection, bodies, timing, audit.
7. [Streaming](guide/streaming.md): stream a body in either direction, download a file, or stop a read early.
8. [Cancellation](guide/cancellation.md): what happens when you drop a request or a body.
9. [Redirects](guide/redirects.md): the redirect limit and custom redirect policies.
10. [Retries and timeouts](guide/retries-and-timeouts.md): retry policies and the four timeouts.
11. [Errors](guide/errors.md): group errors by category, map them to statuses, and find out which errors a retry covers.
12. [Proxies](guide/proxies.md): proxy rules, custom headers, bypass lists, environment discovery.
13. [Crawling](guide/crawling.md): `robots.txt`, host limits, proxy pools, block detection, and checkpoints.
14. [Cookies](guide/cookies.md): the jar, saving it, devices, and connection state.
15. [Accounts](guide/accounts.md): tabs, saved devices, and the state of a returning browser.
16. [WebSocket](guide/websocket.md): connect over HTTP/2 or HTTP/1.1.
17. [HTTP/3](guide/http3.md): the `http3` feature and the protocol policy.
18. [TLS trust](guide/tls-trust.md): roots, certificate pins, mTLS (PEM and PKCS#12), DoH/ECH, the TLS version floor.
19. [Network](guide/network.md): DNS overrides, Happy Eyeballs, Unix sockets, interface binding, socket options.
20. [Logging and tracing](guide/logging.md): `tracing` targets and request phases.
21. [Service integration](guide/service-integration.md): Tower, `http` crate interop, shared sessions, and error mapping.
22. [Throughput](guide/throughput.md): `run_per_core`, Tokio tuning, and high-concurrency layouts.
23. [Testing](guide/testing.md): local plain and TLS servers, and asserting what was sent.

Reference pages beside this guide: [fingerprints](guide/fingerprints.md), [features and targets](guide/features-and-targets.md), [supported platforms](guide/platforms.md), [profiles](guide/profiles.md), and [MSRV](guide/msrv.md).

The [API reference](https://docs.rs/leyline-http) on docs.rs lists every public item.
