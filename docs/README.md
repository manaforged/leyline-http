# Leyline user guide

Leyline (`leyline-http`, imported as `leyline`) is an async HTTP client for
Rust built on Tokio. It plays the role of Python's `requests.Session` or
JavaScript's `fetch`.
Use it for REST APIs, resumable file downloads, reverse-proxy relays, and
crawling, with optional browser impersonation when a server checks TLS, HTTP/2,
or HTTP/3 fingerprints.

## Pick a session kind

For APIs and services, use a plain session: `Session::new()`. To look like a
real browser, use a browser session: `Session::from(Browser::default())`.
[Plain and browser sessions](guide/sessions.md#plain-and-browser-sessions)
shows what each one sends. Build one `Session` and clone it across tasks; see
[Share a session](guide/sessions.md#share-a-session).

## Mental model: `Session`, `Tab`, and `Crawl`

| Type | Think of it as | Use it for |
| --- | --- | --- |
| `Session` | A browser profile or `requests.Session` | Sending requests, sharing connections, cookies, proxies, retries, and rate limits |
| `Tab` | One browser tab (`session.tab()`) | Tracking the current page URL (`Referer`, `Origin`, `sec-fetch-*`) and submitting HTML forms (no JS) |
| `Crawl` | A turnkey site spider (`session.crawl(seed)`) | Following links in scope with `robots.txt`, sitemaps, deduplication, and checkpoints |

> If you use a coding agent, point it at [`llms.txt`](llms.txt) (machine-readable index with module paths and pitfalls) or [`AGENTS.md`](https://github.com/manaforged/leyline-http/blob/main/AGENTS.md) in the repository root.

## Core

- [Quick start](guide/quick-start.md): install, send GET/POST/JSON requests, handle 404s, and download a file.
- [API clients](guide/api-clients.md): base URLs, bearer auth, retries and rate limits, pagination, caching, and token refresh.
- [Sessions](guide/sessions.md): plain vs browser sessions, auth scoping, `before_send`, blocking mode, `to_builder`, and shutdown.
- [Requests](guide/requests.md): methods, headers, `Body::file`, multipart uploads, browser request kinds, `Tab`, `protocol`, and `http::Request` interop.
- [Responses](guide/responses.md): status, headers, `Link` pagination, body size caps, `from_cache`, `ech_accepted`, timing, and `into_http`.
- [Errors](guide/errors.md): the 19 `Kind` variants, 6 predicates, `Error::request`, `gateway_status()`, and `Error::find`.
- [Retries and timeouts](guide/retries-and-timeouts.md): retry policies, retry budgets, backoff, and the six timeouts.
- [Streaming](guide/streaming.md): resumable `download_to`, streaming request/response bodies, `read_until`, and server-sent events (SSE).
- [Cancellation](guide/cancellation.md): cancelling requests with timeouts or `tokio::select!`, and what happens on the wire.

## Routing and proxies

- [Redirects](guide/redirects.md): redirect limits, credential stripping, and custom policies.
- [Proxies](guide/proxies.md): HTTP/HTTPS/SOCKS5 rules, custom headers, port-aware `no_proxy`, env discovery, proxy pools, and `Proxy::custom`.

## Browsers, accounts, and crawling

- [Choosing a profile](guide/choosing-a-profile.md): pick a browser family, platform, and stability strategy.
- [Cookies](guide/cookies.md): `Jar`, `Cookie::parse`, `CookieBuilder`, `cookies(None)`, and serialization.
- [Accounts](guide/accounts.md): `Tab` navigation, HTML form login, `Session::save`/`load`, `autosave`, and saved connection state.
- [Crawling](guide/crawling.md): `Session::crawl`, `robots.txt`, `HostLimits`, block detection, and checkpoints.

## Protocols and operations

- [WebSocket](guide/websocket.md): HTTP/2 extended `CONNECT` and HTTP/1.1 upgrades, handshake inspection, and tuning.
- [HTTP/3](guide/http3.md): the `http3` feature, `Protocol::Race` vs `Auto`, and per-request `.protocol(...)`.
- [TLS trust](guide/tls-trust.md): CA roots (`ca_file`, `ca_der`, `ca_pem`), certificate pins, PEM/PKCS#12 mTLS, DoH/ECH (hides SNI), and TLS version floor.
- [Network](guide/network.md): DNS overrides, DoH, Happy Eyeballs, Unix sockets, interface binding, TCP profiles, pool tuning, `HostLimits`, and HSTS.
- [Logging and tracing](guide/logging.md): `tracing` targets, `trace::Log`, `trace::Metrics`, and custom `Trace` listeners.
- [Service integration](guide/service-integration.md): Axum/hyper gateways, Tower, `http` interop, shared sessions, and gateway status mapping.
- [Throughput](guide/throughput.md): default async vs `run_per_core` vs `build_blocking`, Tokio tuning, and allocators.
- [Testing](guide/testing.md): local plain and TLS `testing::Server`, blocking servers, and asserting wire requests.

## Reference

- [Fingerprints](guide/fingerprints.md): TLS, HTTP/2, HTTP/3, TCP (`TcpProfile::from_ja4t`), and live session auditing.
- [Cargo features and linking](guide/features-and-targets.md): default and optional Cargo features, minimal builds, and native linking.
- [Supported platforms](guide/platforms.md): build targets, Docker/CI prerequisites, and `LEYLINE_BSSL_*` variables.
- [Profile reference](guide/profiles.md): bundled profiles, latest-build helpers, stability guarantees, and custom TOML profiles.
- [MSRV](guide/msrv.md): minimum supported Rust version and edition policy.

The [API reference](https://docs.rs/leyline-http) on docs.rs documents every public item.
