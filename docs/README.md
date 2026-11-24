# Leyline user guide

Leyline (`leyline-http`, imported as `leyline`) is an async HTTP/1.1, HTTP/2, and HTTP/3 client for Rust with sessions, crawling, encrypted DNS (DoH) and TLS 1.3 ECH, proxies, and optional browser TLS profiles.

## Pick a session kind

| You want | Call | What the server sees |
| --- | --- | --- |
| Plain HTTP client | `Session::new()` | `user-agent: leyline/<version>`, `accept: */*`, no `sec-*` headers |
| Browser profile | `Session::from(Browser::default())` or `Session::builder().browser(Browser::Chrome158).build()?` | The TLS, HTTP/2, HTTP/3, and headers of a captured browser |

`Session::from(Browser::default())` uses the newest bundled Chrome profile on Windows. Build one `Session` and clone it across tasks; clones share connections, cookies, cache, and limits.

> **Using a coding agent?** Point it at [`llms.txt`](llms.txt) (machine-readable index of this guide) or [`AGENTS.md`](https://github.com/manaforged/leyline-http/blob/main/AGENTS.md) in the repository root.

## Core

- [Quick start](guide/quick-start.md): add the dependency, send a request, handle status errors, and download a file.
- [API clients](guide/api-clients.md): base URLs, auth, JSON, rate-limit retries, pagination, HTTP cache, and token refresh.
- [Sessions](guide/sessions.md): plain vs browser sessions, auth scoping, `before_send`, `build_blocking`, `to_builder`, and shutdown.
- [Choosing a profile](guide/choosing-a-profile.md): pick a browser family, platform, and stability strategy.
- [Requests](guide/requests.md): methods, headers, `Body::file`, multipart uploads, request kinds, tabs, `protocol`, and `http::Request` interop.
- [Responses](guide/responses.md): status, headers, `Link` pagination, body size caps, `from_cache`, `ech_accepted`, timing, and `into_http`.
- [Streaming](guide/streaming.md): stream request/response bodies, resumable `download_to`, `read_until`, and server-sent events (SSE).
- [Cancellation](guide/cancellation.md): what happens on the wire when you drop a request, body, or download.

## Reliability and routing

- [Redirects](guide/redirects.md): redirect limits, credential stripping, and custom policies.
- [Retries and timeouts](guide/retries-and-timeouts.md): retry policies, budgets, backoff, and the six timeouts.
- [Errors](guide/errors.md): the 19 `Kind` variants, `gateway_status()`, predicates, `Error::request`, and `Error::find`.
- [Proxies](guide/proxies.md): scheme rules, custom headers, port-aware `no_proxy`, env discovery, proxy pools, and `Proxy::custom`.

## Sessions and crawling

- [Crawling](guide/crawling.md): `Session::crawl`, `robots.txt`, `HostLimits`, block detection, proxy pools, and checkpoints.
- [Cookies](guide/cookies.md): `Jar`, `Cookie::parse`, `CookieBuilder`, `cookies(None)`, and serialization.
- [Accounts](guide/accounts.md): `Tab` navigation, `Session::save`/`load`, `autosave`, and saved connection state.

## Protocols and operations

- [WebSocket](guide/websocket.md): HTTP/2 extended `CONNECT` and HTTP/1.1 upgrades, handshake inspection, and tuning.
- [HTTP/3](guide/http3.md): the `http3` feature, `Protocol::Race` vs `Auto`, and per-request `.protocol(...)`.
- [TLS trust](guide/tls-trust.md): CA roots (`ca_file`, `ca_der`, `ca_pem`), certificate pins, PEM/PKCS#12 mTLS, DoH/ECH, and TLS version floor.
- [Network](guide/network.md): DNS overrides, DoH, Happy Eyeballs, Unix sockets, interface binding, TCP profiles, pool tuning, and HSTS.
- [Logging and tracing](guide/logging.md): `tracing` targets, `trace::Log`, `trace::Metrics`, and custom `Trace` listeners.
- [Service integration](guide/service-integration.md): Tower, `http` crate interop, shared sessions, and gateway status mapping.
- [Throughput](guide/throughput.md): default async vs `run_per_core` vs `build_blocking`, Tokio tuning, and allocators.
- [Testing](guide/testing.md): local plain and TLS `testing::Server`, blocking servers, and asserting wire requests.

## Reference

- [Fingerprints](guide/fingerprints.md): TLS, HTTP/2, HTTP/3, TCP (`TcpProfile::from_ja4t`), and live session auditing.
- [Cargo features and linking](guide/features-and-targets.md): default and optional Cargo features, feature implications, and native linking.
- [Supported platforms](guide/platforms.md): build targets, toolchains, and `LEYLINE_BSSL_*` environment variables.
- [Profile reference](guide/profiles.md): bundled profiles, capture status, stability guarantees, and custom TOML profiles.
- [MSRV](guide/msrv.md): minimum supported Rust version and edition policy.

The [API reference](https://docs.rs/leyline-http) on docs.rs documents every public item.
