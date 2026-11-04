# Upgrading to Leyline 0.2

This guide maps the 0.1.6 Rust API to 0.2.0. The API changes are breaking. There is no automatic migration command.

## Migration steps

1. Change the `leyline-http` dependency to `"0.2"` and use Rust 1.96 or later.
2. Find each old name in the table below and update its imports and calls.
3. Replace typed browser profiles with profile TOML where required.
4. Check error handling, retries, streaming, and saved-session behavior against the [release notes](CHANGELOG.md#020---2026-11-02).
5. Run `cargo check` and your application's tests with the features you use.

For new code, start with the [five main types](AGENTS.md#the-five-types). The [user guide](https://manaforged.github.io/leyline-http/) covers current workflows.

## Breaking changes in 0.2.0

Search for the old type or method name. A row marked removed has no direct replacement; follow the alternative in that row.

| 0.1.6 | 0.2.0 |
| --- | --- |
| `IntoParamPair` bounds and implementations | removed; `query`, `form`, and `Tab::submit` accept iterators whose items implement `Borrow<(K, V)>`, with string-like keys and values; owned and borrowed tuple calls stay the same |
| `Browser::latest(Family::Chrome)` | `Browser::chrome()`, and one function per product: `edge()`, `opera()`, `brave()`, `firefox()`, `safari()`, `safari_ios()`, `cfnetwork()`, `okhttp()` |
| `Browser::get(Family::Chrome, 154)` | `Browser::Chrome154`, or `"chrome-154".parse::<Browser>()` |
| `Session::browser(b)` | `Session::from(b)` |
| `browser.family()`, `Family`, `ChromiumBrand` | removed; the `Browser` variant names the product, and `chromium_major()` gives its Chromium base |
| `SessionBuilder::brand(ChromiumBrand::Edge)`, `Identity::with_brand(ChromiumBrand::Edge)` | a branded variant: `.browser(Browser::Edge154)` |
| a brand on a given Chromium major | the `Browser` in `Browser::all()` whose `id()` names the product and whose `chromium_major()` matches |
| `browser.identity(platform, brand)` returning `PlatformIdentity` | `browser.headers(platform)` returning `(name, value)` pairs, `user-agent`, `sec-ch-ua`, and `accept-language` among them |
| `.identity(Identity::from(b))`, `.identity(Identity::locked(b, p))` | `.browser(b)`, `.browser(b).platform(p)` |
| `Identity::rotate_tls(b)` | `.hello(b)` |
| `Identity::rotate_hello()` | `.hello(b.next_hello()?)` |
| `Identity::switch_family(dest)` | `.browser(dest).platform(p)`; a browser with no identity for the platform fails at `build()` |
| `session.identity()` with `Identity::http()`, `tls()`, `platform()`, `brand()` | `session.browser()`, `hello()`, `platform()`; the brand is part of the `Browser` variant |
| `SessionIdentity` getters, `export_profile()`, `to_identity()` | `Session::user_agent()`, `profile_id()`, `profile()`, `platform()`, `languages()`, and `to_builder()` |
| `Browser::profile()`, `BrowserProfile`, and the profile schema types (`TlsProfile`, `H2Profile`, `H3Profile`, `ProfileMeta`, `HeaderStyle`, and the rest) | the profile TOML from `Session::profile()`; load one with `.profile(toml)` |
| `.profile(BrowserProfile::from_toml(&toml)?)` | `.profile(&toml)`; bad TOML fails at `build()` |
| `BrowserProfile::from_fingerprint(FingerprintSpec::new().base(b).ja3(s))` | `.browser(b).ja3(s)`, with `ja4_r`, `akamai`, and `header_order`; call `browser` first, or `build()` fails with `Kind::Config` |
| `ProfileRegistry`, `ProfileError` | removed; load each profile file with `.profile(toml)` |
| `Preset`, `HeaderAnchor` | `RequestKind`, `HeaderPosition` |
| `RequestBuilder::preset`, `anchored`, `initiator`, `tag`, `compress` | `kind`, `header_at`, `from_page`, `label`, `compress_body` |
| `FetchSite` | removed; `from_page(url)` sets the site and referrer context, `Sec-Fetch-Site` included |
| `SessionBuilder::user_agent(ua)` | `.headers([("user-agent", ua)])` |
| `SessionBuilder::expect_profile_id(id)` | `Session::load(path)`, whose `build()` fails with `is_profile_changed()`, or a check of `session.profile_id()` |
| `.headers(&[("a", "b")])` | `.headers([("a", "b")])`, `.headers(vec)`, or `.headers(&header_map)`; each pair converts like `header(name, value)` |
| `BlockSignal { vendor, kind }`, `BlockKind` | `Block::Challenge(vendor)`, `Block::Captcha(vendor)`, `Block::Denied(vendor)`; `block.vendor()` |
| `BlockRules::statuses([403])` | `.block_statuses([403])` |
| `BlockRules::from_toml(s)?`, `BlockRules::extend` | `.block_rules(s)`; each call adds rules, and bad TOML fails at `build()` |
| `BlockRules::check(&resp)`, `BlockRules::builtin()` | `resp.block()`, which applies the built-in rules and the session's own |
| `ProxyPool::rotate_on_block(rules)`, `RetryPolicy::skip_blocks(rules)` | removed; every session applies its block rules, and `Retry::when` sees `attempt.is_block()` |
| `ProxyConfig::new(url)`, `ProxyConfig::rule(ProxyRule)`, `NoProxy` | `Proxy::new(url)`, `Proxy::all(url)`, `http(url)`, `https(url)`, `no_proxy(patterns)`, `env(on)` |
| `ProxyUrl::parse(url)` | a URL string; `build()` checks it |
| `ProxyPool::new(urls)`, `.proxy_pool(pool)` | `Proxy::list(urls)`, `.proxy(list)` |
| `ProxyPool::identified([(proxy, identity)])` | `Proxy::identified([(proxy, browser, platform)])` |
| `ProxyPool::stats()`, `ProxyHealth` | `session.stats()`: `proxies()`, `proxy_in_use(p)`, `proxy_failures(p)`, `proxy_banned_until(p)` |
| `RetryPolicy::rotate_proxies(urls)` | `.proxy(Proxy::list(urls))` |
| `session.proxy_url()` | `session.proxy()`, a `&str` with the password replaced by `***` |
| `TlsTrustConfig`, `.tls_trust(config)` | `Tls`, `.tls(tls)` |
| `add_ca_file`, `add_ca_der`, `add_pinned_leaf_sha256`, `client_identity` | `ca_file`, `ca_der`, `pin_sha256`, `client_cert` |
| `min_tls_version(TlsMinVersion::Tls12)`, `(TlsMinVersion::Tls13)` | `tls12_min()`, `tls13_only()` |
| `resp.tls()` returning `TlsInfo { version, cipher, peer_cert_der }` | `resp.tls_version()`, `tls_cipher()`, `peer_certificate()` |
| `.socket(SocketConfig::new().tcp_nodelay(true))` | `.tcp_nodelay(true)` on `SessionBuilder`, one setter per option; `strict` is `strict_socket_options` |
| `SocketConfig::local_address(ip)`, `local_ipv4(ip)`, `local_ipv6(ip)` | `.local_address(ip)`, once per address family; an IPv4 address no longer binds IPv6 connections |
| `SocketConfig::happy_eyeballs(HappyEyeballsConfig::new().resolve_delay(d).attempt_limit(n))` | `.happy_eyeballs_delay(d).happy_eyeballs_attempts(n)` |
| `.pool(PoolConfig::new())` with `max_connections`, `max_h1_conns_per_host`, `idle_timeout`, `keepalive`, `h2_ping_after_idle`, `h2_ping_timeout` | `SessionBuilder` setters: `max_connections`, `max_connections_per_host`, `pool_idle_timeout`, `keep_alive`, `http2_ping_after_idle`, `http2_ping_timeout` |
| `.dns(DnsConfig::new().resolve_host(host, addrs).resolver(r))` | `.resolve(host, addrs)` and `.dns_resolver(r)`; several `resolve` calls add up |
| `leyline::tls::Resolver` | `leyline::Resolver` |
| `ResolveFuture`, `SystemResolver` | `Resolver::resolve` returns `Pin<Box<dyn Future<Output = io::Result<Vec<SocketAddr>>> + Send + 'a>>`; the system resolver is the default |
| `.compression(CompressionConfig)` | `.accept_encoding([..])`, `.max_body_size(n)`, `.max_error_body_size(n)`; `accept_encoding([])` offers no coding |
| `ContentEncoding` | `Encoding` |
| `HostLimits::host(host, HostLimits::new().max_in_flight(n).per_second(r))` | `HostLimits::new().host_max_in_flight(host, n).host_per_second(host, r)` |
| `TimeoutConfig`, `RedirectPolicy`, `ProtocolPolicy` | `Timeout`, `Redirect`, `Protocol`; `RedirectPolicy::limited(n)` is `Redirect::limit(n)` |
| `session.pool_stats()`, `PoolStats` fields | `session.stats()`, with the same counters as methods; the six `h1_hits` to `h3_misses` fields are `hits(version)` and `misses(version)` with an `http::Version` |
| `session.host_stats()`, `HostStats` | `session.stats()`: `hosts()`, `in_flight(host)`, `waiting(host)` |
| `session.with_proxy(p)`, `with_cookie_jar(jar)`, `with_redirect(r)`, `with_base_url(url)`, `with_identity(id)` | `session.to_builder()`, the setter, then `build()` |
| `session.fresh_pool()` | `session.to_builder().share_connections(false).build()` |
| `SessionBuilder::cookie_jar(jar)`, `RequestBuilder::cookie_jar(jar)` | `.cookies(jar)` on `SessionBuilder` or on `session.to_builder()` |
| `Device::open`, `capture`, `save_to`, `load_from`, `session_builder` | `session.save(path)` and `Session::load(path)`, which returns a `SessionBuilder` |
| `Device::autosave`, `DeviceAutosave`, `DeviceAutosaveOptions`, `Jar::autosave`, `JarAutosave` | `session.autosave(path, every)` |
| `Session::state()`, `SessionState::restore_into(&session)` | `session.save(path)` and `Session::load(path)`, which carry the TLS tickets, Alt-Svc entries, and HSTS store |
| `Device::tab()` | `session.tab()` |
| `Jar::save_to`, `Jar::load_from` | `session.save(path)`, or serde: `Jar` is `Serialize` and `Deserialize` |
| `Jar::get_cookie`, `set_cookie`, `store_set_cookie`, `remove_named`, `all_cookies`, `cookie_header`, `extend_from` | `get`, `set`, `store`, `remove_everywhere`, `all`, `header_for`, `merge` |
| `Jar::load_cookies(pairs)` | `Jar::set` for each pair |
| `RetryPolicy`, `.retry(policy)` | `Retry`, `.retry(retry)`; `.retry(n)` retries `n` times on connect errors, timeouts, 429, 502, 503, and 504 |
| `retry_on`, `on_status`, `retry_if`, `RetryTrigger` | `Retry::new(n).when(\|attempt\| ..)`, with `attempt.is_transient()` and `attempt.is_block()` |
| `initial_backoff`, `max_backoff`, `backoff_factor`, `jitter` | `.backoff(min, max)` |
| `RetryPolicy::backoff(attempt)` | `Retry::delay(attempt)` |
| `max_retry_after`, `allow_non_idempotent` | `max_wait`, `any_method` |
| `retry_unsent` | removed; a request that never reached the server is retried for any method |
| `.wait_header(name, WaitFormat::Seconds)` | `.wait_header(name)`, which reads delay-seconds, an HTTP date, or a Unix timestamp; `.wait_header_ms(name)` for milliseconds |
| `RedirectAttempt` | `Attempt`, with `status()`, `url()`, `location()`, and `previous()` |
| `RedirectPolicy::custom(\|a\| RedirectAction::Follow)` | `Redirect::custom(\|a\| true)` |
| `HttpVersion`, `HttpVersion::as_str()` | `http::Version` from `Response::version()`; its `Debug` prints `HTTP/2.0` where `as_str()` printed `HTTP/2` |
| `resp.timing()`, `ResponseTiming` | `resp.reused()`, `connect_time()`, `send_time()`, `elapsed()`, as `Duration` values |
| `into_stream()`, `into_decoded_stream(limit)`, `BodyStream` | `bytes_stream()`, a `Body` that yields the decoded body as `leyline::Result<Bytes>`; `.decode(false)` on the request gives the raw bytes |
| `copy_to(writer)`, `copy_decoded_to(writer, limit)` | `write_to(writer)`; `.max_body_size(n)` on the request sets a limit |
| `download_to(path, limit)`, `RequestBuilder::download(path)` | `.error_for_status().stream()` on the request, then `Response::download_to(path)` |
| `RequestBuilder::read_until(n, f)`, `PrefixRead { bytes, stopped_by }`, `StopReason` | `.stream()`, then `Response::read_until(n, f)` returning `(bytes, matched)` |
| `error_for_status_ref()` | `error_for_status()`, which borrows the response |
| `resp.link(rel)` | `resp.links()` and `Link::has_rel(rel)` |
| `relay_headers(RelayBody)`, `RelayBody` | `relay_headers()`, which drops `content-encoding` and `content-length` when Leyline decoded the body |
| `resp.audit()`, `AuditData` fields | `resp.fingerprint()`, a `Fingerprint` with `ja3()`, `ja4()`, `ja4h()`, `ja4t()`, `akamai()`, `headers()` |
| `Observed`, `audit.compare(&observed)`, `FingerprintReport`, `FieldOutcome`, `HeaderOutcome` | `Fingerprint::from_json(text)`, `sent.diff(&observed)` returning `(field, expected, observed)` tuples |
| `audit::compute_ja3`, `compute_ja4`, `compute_ja4h`, `compute_ja4t`, `Ja3Input`, `Ja4Input`, `Ja4hInput` | removed; `Fingerprint` carries the values |
| `ErrorCategory`, `Error::category()` | `Kind`, `Error::kind()` |
| `is_timeout()`, `is_status()`, `is_body_limit()`, `is_dns()` | `err.kind()` compared with `Kind::Timeout`, `Kind::Status`, `Kind::BodyLimit`, or `Kind::Dns` |
| `Error::io()`, `tls()`, `h2()`, and downcasts to `TlsError`, `H2Error`, `ErrorCode`, or `ProxyReply` | `Error::kind()`, `proxy_status()`, and `h2_code()` |
| `Error::header(name)`, `body_text()` | `headers()?.get(name)`, `body().map(String::from_utf8_lossy)` |
| `session.websocket(url).header(k, v).connect()`, `WebSocketBuilder` | `session.get(url).header(k, v).websocket()` |
| `WsConnection`, `WsMessage`, `CloseFrame` | `WebSocket`, `Message`, `Message::Close(Some((code, reason)))` |
| `WsSink`, `WsStream` from `split()` | a `Sink<Message>` and a `Stream<Item = Result<Message>>`; send `Message::Close` to close |
| `.websocket_config(WebSocketConfig::new().prefer_http2(true))` | `.websocket_http2(true)`, one `websocket_*` setter per option |
| `impl Trace { fn head(&self, ev: &Head) .. fn summary(&self, ev: &Summary) }` | `impl Trace { fn event(&self, event: &Event) }`, matching `Event::Head { .. }`, `Event::Summary { .. }` |
| `trace::Dns`, `Connect`, `Tls`, `Sent`, `Head`, `Done`, `Summary`, `BodyEnd` | variants of `trace::Event` with the same fields |
| `BodyOutcome::Complete`, `Failed(e)`, `Dropped` | `Ok(())`, `Err(Some(e))`, `Err(None)` |
| `trace::TracingTrace` | `trace::Log`; the `tag` key is `label` |
| `trace::Fanout` | one `.trace(t)` call per hook |
| `metrics.snapshot()`, `MetricsSnapshot` | the same getters on `Metrics`; `{metrics}` prints the summary |
| `CrawlPage` | `Page` |
| `crawl.stats()`, `CrawlStats` | `crawl.requests()`, `pages()`, `errors()`, `skipped(reason)` |
| `Crawl::ignore_robots()` | `Crawl::respect(None)` |
| `req.pages()` returning `Pages` | `req.pages()` returning a `Stream`; `StreamExt::take(n)` limits it |
| `form.method()` returning `FormMethod`, `form.enctype()` returning `FormEnctype` | `http::Method`, and the MIME type as `&str` |
| `multipart::Form::text(name, value)`, `Form::file(name, path)` | `Form::part(name, Part::text(value))`, `Form::part(name, Part::file(path)?)` |
| `From<multipart::Form> for Body`, `Form::boundary` | `RequestBuilder::multipart(form)`, which sets the boundary header |
| `Tab::xhr(url)`, `post_json(url, body)`, `subresource(url, kind)` | `Tab::fetch(Method::GET, url)`, `.json(body)`, `.kind(kind)` |
| `Purpose::as_str()` | `Purpose::id()` |
| `DigestAuth` | `.digest_auth(user, pass)` |
| `LeylineService::new(session)` | `session`, which implements `tower::Service`; the response carries no timing extension, so use `Trace` |
| `leyline::get(url)` | `Session::new().get(url).send()` |
| `.await` on a `RequestBuilder` or `WebSocketBuilder` | `.send().await` |
| `Body::len_hint()` | removed |
| `testing::TestServer`, `TestResponse`, `RecordedRequest` | `testing::Server`, `testing::Response`, `testing::Request` |
| `TestServer::trust()` | `testing::Server::tls()` |
| `TestResponse { status, headers, body }` fields | `testing::Response::new(status).header(..).body(..)` |
| `RecordedRequest::header_count(name)` | `header_values(name).len()` |

## Direct FFI consumers

`leyline-bssl-sys` follows the bundled BoringSSL headers. In 0.2.0, the
`DIST_POINT_NAME_st::dpname` field and the `SSL_ERROR_HANDOFF` and
`SSL_ERROR_HANDBACK` constants are removed upstream. Remove access to that
field and obsolete handoff error handling. Do not reuse bindings generated
for the previous BoringSSL revision; use the bindings shipped with the crate.

`SSL_CREDENTIAL_add1_trust_anchor_group_inclusion` is replaced by
`SSL_CREDENTIAL_add1_trust_anchor_group`, which accepts an encoded trust-anchor
ID pattern instead of a base and numeric range. Follow the bundled header's
pattern format; this is not a method-name-only replacement.
