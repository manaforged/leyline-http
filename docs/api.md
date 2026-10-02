# Leyline API

This page and the [API reference](reference/leyline-http/index.md) are the
public contract of `leyline-http` (import name `leyline`). The API reference
lists every public item and is generated from the compiler; `cargo truesight
check` fails when it no longer matches the code. This page maps each job to
its call and to the guide that explains it, and holds the error kinds and the
rules the API follows.

The `bench-internals` feature and the `leyline_unstable_bssl` compiler flag
expose internals for Leyline's own tests and the BoringSSL context builder.
Neither is part of the contract, and the API reference leaves both out.

## Overview

```text
one-off     leyline::get(url)               one GET from a plain session
session     Session::new()                  plain client, cannot fail
            Session::browser(browser)       browser session, cannot fail
            Session::builder() → SessionBuilder → build() → Session   (no browser: plain client)
            session.with_proxy(config)      clone that shares the pool, other proxy
            session.fresh_pool()            clone with a new pool and TLS session cache
            session.with_cookie_jar(jar)    clone that shares the pool, other jar
            session.with_redirect(policy)   clone that shares the pool, other redirect policy
            session.with_identity(id)       clone with another identity, own pool partition
            session.with_base_url(url)      clone that shares the pool, other base URL
            session.tab() → Tab              current page, initiator for each request
            session.shutdown() · is_shut_down()   stop every clone and derived session
            session.state() → SessionState   TLS tickets, Alt-Svc, HSTS
            Device::capture / open / save_to / load_from   one saved account device
            device.autosave(&session, path, debounce) → DeviceAutosave · update / track / device / flush / shutdown
            device.tab(&session) → Tab        tab on the saved page
crawl       SessionBuilder::host_limits(HostLimits) · proxy_pool(ProxyPool) · RequestBuilder::tag
            ProxyPool::identified · rotate_on_block · Session::host_stats · BlockRules::statuses
request     session.get / post / put / patch / delete / head / request(Method, url) → RequestBuilder
send        RequestBuilder.send() or .await → Response
            RequestBuilder.download(path, limit) → bytes written
            RequestBuilder.pages() → Pages   follows Link rel="next"
            session.execute(http::Request<Body>) for prebuilt requests and tower
read        Response: status · headers · header · text · bytes · json · read_until
            raw: into_stream · copy_to      decoded and capped: into_decoded_stream · copy_decoded_to
            Response: url · redirect_chain · version · timing · attempts · proxy
            Response: link · links · relay_headers(RelayBody) · block · download_to
fail        Error · err.kind() → Kind · err.category() → ErrorCategory · gateway_status()
            RequestBuilder::error_for_status · err.status / body / body_text / headers / header / retry_after / retries_exhausted / attempts / proxy
            Error::find(&dyn std::error::Error)   leyline error under other layers
policy      TimeoutConfig · RetryPolicy · RedirectPolicy   (session default, request override)
identity    Browser · Platform · ChromiumBrand · Identity
observe     Trace hooks · trace::Metrics · trace::Fanout · Response::timing · Response::tls · Response::audit
tower       LeylineService (feature `tower`) · relay_headers(&HeaderMap, RelayBody) · redact_url
html        html::forms · Form::find · html::meta · html::links → Anchor (feature `html`)
test        testing::TestServer · TestResponse::delay / chunks (feature `test-util`)
```

One function per job. A setter that takes a config type accepts
`impl Into<Config>` where a shorthand exists, so the common case stays one
call:

```rust,no_run
use std::time::Duration;

use leyline::{Browser, Family, Platform, ProtocolPolicy, Session, TimeoutConfig};

# async fn run() -> leyline::Result<()> {
let session = Session::browser(Browser::default());
let body = session.get("https://example.com/").await?.text().await?;

let session = Session::builder()
    .browser(Browser::latest(Family::Firefox))
    .platform(Platform::MacOS)
    .protocol(ProtocolPolicy::Http2)
    .timeout(Duration::from_secs(15))
    .proxy("http://user:pass@host:8080")
    .build()?;

let resp = session
    .post("https://example.com/api")
    .header("x-trace", "1")
    .json(&serde_json::json!({ "q": 1 }))
    .timeout(TimeoutConfig::new().total(Duration::from_secs(5)))
    .await?;
# drop((body, resp));
# Ok(())
# }
```

## Where each job is documented

### Sessions and identities

| Job | Call | Guide |
| --- | --- | --- |
| Plain or browser session | `Session::new`, `Session::browser`, `Session::builder` | [Sessions](guide/sessions.md) |
| Newest or pinned browser | `Browser::latest`, `Browser::get`, `Browser::Chrome148` | [Choose a browser](guide/sessions.md#choose-a-browser) |
| Platform and brand | `SessionBuilder::platform`, `brand` | [Choose a platform](guide/sessions.md#choose-a-platform) |
| Token and base URL | `bearer_auth`, `base_url`, `with_base_url` | [Sessions](guide/sessions.md#set-a-token-and-a-base-url) |
| Languages and user agent | `languages`, `user_agent` | [Set the languages](guide/sessions.md#set-the-languages) |
| Derived sessions | `with_proxy`, `with_cookie_jar`, `with_redirect`, `with_identity`, `fresh_pool` | [Derive a session](guide/sessions.md#derive-a-session) |
| Stop every clone | `Session::shutdown`, `Error::is_shut_down` | [Stop a session](guide/sessions.md#stop-a-session) |
| Save TLS tickets, Alt-Svc, HSTS | `Session::state`, `SessionState::restore_into` | [Sessions](guide/sessions.md#save-and-restore-an-identity) |
| Read what a session sends | `Session::identity`, `Browser::identity`, `SessionIdentity::profile_id` | [Sessions](guide/sessions.md) |
| Mix and rotate identities | `Identity::locked`, `rotate_tls`, `rotate_hello`, `switch_family` | [Fingerprints](guide/fingerprints.md#mix-and-rotate-identities) |
| Detect a profile change | `expect_profile_id`, `Error::is_profile_changed` | [Choosing a profile](guide/choosing-a-profile.md) |
| Load or build a profile | `ProfileRegistry::load`, `BrowserProfile::from_toml`, `from_fingerprint` | [Profiles](guide/profiles.md) |
| Saved account device | `Device`, `DeviceAutosave` | [Accounts](guide/accounts.md) |

### Requests and responses

| Job | Call | Guide |
| --- | --- | --- |
| Methods, headers, query, bodies | `RequestBuilder` | [Requests](guide/requests.md) |
| Header order and merge | `SessionBuilder::headers`, `RequestBuilder::header` | [Requests](guide/requests.md#headers) |
| Page initiator and tabs | `initiator`, `Session::tab`, `Tab` | [Requests](guide/requests.md#keep-the-page-with-a-tab) |
| Prebuilt `http::Request` | `Session::execute` | [Requests](guide/requests.md#send-an-httprequest) |
| Status, headers, bodies | `Response` | [Responses](guide/responses.md) |
| Status as an error | `error_for_status`, `error_for_status_ref` | [Responses](guide/responses.md#turn-a-status-into-an-error) |
| `Link` pagination | `RequestBuilder::pages`, `Response::links` | [Responses](guide/responses.md#follow-link-pagination) |
| Stream, download, stop at a marker | `.stream()`, `into_decoded_stream`, `download`, `read_until` | [Streaming](guide/streaming.md) |
| Redirect policy | `RedirectPolicy` | [Redirects](guide/redirects.md) |
| Cookies | `cookie::Jar`, `Jar::autosave` | [Cookies](guide/cookies.md) |
| WebSocket | `Session::websocket` | [WebSocket](guide/websocket.md) |
| HTML forms, meta, links | `html::forms`, `html::meta`, `html::links` | [Accounts](guide/accounts.md) |
| Cancel a request | drop, `Session::shutdown` | [Cancellation](guide/cancellation.md) |

### Policy, transport, and observation

| Job | Call | Guide |
| --- | --- | --- |
| Timeouts and retries | `TimeoutConfig`, `RetryPolicy` | [Retries and timeouts](guide/retries-and-timeouts.md) |
| Proxies and proxy pools | `ProxyConfig`, `ProxyUrl`, `ProxyPool` | [Proxies](guide/proxies.md) |
| Host limits, blocks, crawl metrics | `HostLimits`, `BlockRules`, `host_stats`, `trace::Metrics` | [Crawling](guide/crawling.md) |
| DNS, sockets, pool, Happy Eyeballs | `DnsConfig`, `SocketConfig`, `PoolConfig`, `HappyEyeballsConfig` | [Network](guide/network.md) |
| Trust roots, pins, TLS floor | `TlsTrustConfig` | [TLS trust](guide/tls-trust.md) |
| HTTP/3 | `ProtocolPolicy` | [HTTP/3](guide/http3.md) |
| Logs and trace hooks | `Trace`, `TracingTrace`, `trace::Fanout` | [Logging and tracing](guide/logging.md) |
| Fingerprint audit | `audit(true)`, `Response::audit`, `AuditData::compare` | [Fingerprints](guide/fingerprints.md) |
| Errors | `Error`, `Kind`, `ErrorCategory` | [Errors](guide/errors.md) |
| Services, Tower, axum | `LeylineService`, `relay_headers`, `redact_url` | [Service integration](guide/service-integration.md) |
| Tests | `testing::TestServer` | [Testing](guide/testing.md) |

## Errors

One error type: `leyline::Error`, `Send + Sync + 'static`. Read
`err.kind()` for the `Kind` below, or `err.category()` for the
`ErrorCategory` that groups it for logs, metrics, and gateway statuses.
[Errors](guide/errors.md) covers categories, predicates, attempts, source
errors, and status mapping.

| Kind | Meaning |
|---|---|
| `Request` | HTTP-level failure (bad header, malformed request) |
| `Redirect` | A redirect could not be followed: a custom policy passed 32 hops, a `Location` scheme other than http or https, or a redirect that must resend a streamed request body. A policy that stops returns the 3xx response and is not an error |
| `Status` | `error_for_status` rejected the status |
| `Body` | A body cannot be used: a response body over `max_body_size` or a caller limit, a response body already taken, a request body that cannot be compressed or resent, or a WebSocket protocol violation or oversized message |
| `Decode` | Decompression failed: bad data, a truncated stream, or more than 4 codings |
| `Timeout` | `total` or `response_header` expired, or `read` or `body` expired in `bytes`, `text`, or `json` |
| `Connect` | DNS resolution or TCP connect failed, or the `connect` limit expired during DNS, TCP, a proxy handshake, or a TLS or HTTP/3 handshake. An expired limit also makes `is_timeout()` true |
| `Tls` | TLS handshake or certificate verification failed |
| `Http2` | HTTP/2 protocol failure |
| `Http3` | HTTP/3 protocol or handshake failure |
| `Proxy` | Proxy dial, handshake, authentication, or `CONNECT` failed, the proxy could not reach the origin, or a request of a session that rejected an invalid environment proxy |
| `Io` | Socket I/O, or an HTTP/1.1 response that breaks the wire format; `err.io()` then has kind `InvalidData` |
| `Config` | Invalid session, request, TLS profile, trust store, or proxy configuration, including an invalid environment proxy at `build()` |
| `Url` | URL parse failure |
| `Json` | JSON serialize or deserialize failure |

The profile loaders return `profile::ProfileError`, and
`multipart::Form::file` and `Part::file` return `std::io::Result`; neither
has a `Kind`.

## Features

Default: `charset`, `compression-gzip`, `compression-brotli`,
`compression-deflate`, `compression-zstd`, `multipart`, `websocket`,
`http3`, `html`. Opt-in: `socks`, `tower`, `test-util`, `bench-internals`.
`full` enables every feature except `test-util` and `bench-internals`. See
[Features and targets](guide/features-and-targets.md).

## API shape

- One function per job, and one owner each for timing, retries, redirects,
  cookies, and body decoding.
- Config types (`TimeoutConfig`, `RetryPolicy`, `RedirectPolicy`,
  `ProxyConfig`, `DnsConfig`, `TlsTrustConfig`, `PoolConfig`,
  `SocketConfig`, `HappyEyeballsConfig`, `CompressionConfig`,
  `WebSocketConfig`) have private fields and consuming setters named after
  the field, with no `with_` prefix. A setter for an optional value takes
  `impl Into<Option<T>>`, and `None` turns the setting off;
  `RetryPolicy::max_retry_after` takes a `Duration`, and its cap is off
  until set. A setter that adds to a list starts with `add_` or appends one
  item (`rule`, `on_status`).
- Every `SessionBuilder` config setter replaces the whole value. Start from
  `::new()`, which carries the defaults. A request setter merges over the
  session value field by field.
- Public enums, and public structs with public fields, are
  `#[non_exhaustive]`, `Cookie` and `SameSite` included. The exceptions are
  `audit::Ja3Input`, `Ja4Input`, and `Ja4hInput`, which callers fill with a
  struct literal.
- Builder input errors surface at `build()`, `send()`, or `connect()`,
  never eagerly. `Identity` is a value type, so its checks run at the call.
- `IntoUrl` is sealed. `Session::new()` and `Session::browser(..)` never
  fail: a trust-store problem fails later requests with `Kind::Tls`, and an
  invalid environment proxy fails them with `Kind::Proxy`.
- A patch release never removes a `Browser` variant. A retired profile keeps
  its variant, marked `#[deprecated]` through its `[meta] deprecated` note,
  until a minor release removes it. `Browser::latest` and
  `Browser::for_platform` never return a deprecated profile.
- `Browser`, `Family`, `Platform`, and `ChromiumBrand` serialize as the
  stable lowercase id that `id()` returns, and `FromStr` reads it back.

## Profile schema

`BrowserProfile` and the `leyline::profile` types mirror the bundled
`profiles/<family>/<version>.toml` files; field names are the TOML keys.
`profiles/bare.toml` holds the bare profile and `profiles/platforms.toml`
the per-platform TCP values. The `[meta]` keys `capture`, `variant`,
`hello`, `platform_browser`, `deprecated`, and `verified_at` are valid and
ignored at runtime; they are not fields of `ProfileMeta`, and the build reads
the first five to generate `Browser`. The schema is part of the API: a patch
release can add fields, and removing or renaming one needs a minor release.
See [Profiles](guide/profiles.md).

## Do not add

- A second constructor for a browser, brand, or platform
  (`Session::chrome()`, `SessionBuilder::macos()`). Use `browser`,
  `platform`, `brand`.
- A per-field copy of a config type on `SessionBuilder` or
  `RequestBuilder` (`connect_timeout`, `resolve_host`,
  `add_root_certificate_der`, `max_redirects`). Add the field to the config
  type.
- A second setter with other header semantics (`append_header`,
  `set_header`) or a typed shortcut for one header (`referer`).
  `SessionBuilder::user_agent` is the one session shortcut, because it
  replaces a profile default.
- A second request type. `Session::execute` takes `http::Request<Body>`.
- A second middleware model inside the send path. Wrap `LeylineService`
  with tower layers.
- A second body reader (`text_utf8`, `into_text`, `as_bytes`) or a second
  `Content-Encoding` decoder.
- A second cookie store, such as a per-response cookie map.
- A getter that copies session configuration back out. The exceptions
  report the state of a built session: `Session::identity`,
  `Session::proxy_url`, `Session::cookies`, `Session::pool_stats`, and
  `Session::host_stats`.
- A second timing, retry, or redirect owner.
- A browser, version, or brand table in Rust. Profile data owns it.
