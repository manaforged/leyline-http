# leyline-http

Every public item of the crate, generated from the compiler's view of the
code with `cargo-public-api 0.52.0`. Features: `full`. Target: `aarch64-apple-darwin`.
Items marked `#[doc(hidden)]` are internal and not listed.

Regenerate with `python3 scripts/generate-api.py`. The release check runs
`python3 scripts/generate-api.py --check` and fails when this page is stale.

<details>
<summary>Symbol index</summary>

`0` · `AfterAccept` · `AfterCchUa` · `AfterCchUaMobile` · `AfterCchUaPlatform` · `AfterContentType` · `AfterUserAgent` · `Android` · `AuditData` · `Auto` · `BeforeAcceptEncoding` · `Binary` · `Body` · `BodyStream` · `BrandOverlay` · `BrandOverlayError` · `Brave` · `Brave146` · `Brotli` · `Browser` · `BrowserProfile` · `Builder` · `Call` · `Cancel` · `Certificate` · `CfNetwork` · `CfnetworkIOS18` · `CfnetworkMacOS26` · `Chrome` · `Chrome145` · `Chrome146` · `Chrome147` · `Chrome148` · `Chrome149` · `Chrome150` · `Chrome151` · `Chrome152` · `ChromiumBrand` · `ClientIdentity` · `Close` · `CloseFrame` · `CompressionConfig` · `CompressionError` · `Config` · `Connect` · `ConnectError` · `Connection` · `ConnectionError` · `ContentEncoding` · `Cookie` · `CrossOrigin` · `Decode` · `Deflate` · `DigestAuth` · `Dns` · `DnsConfig` · `Done` · `Edge` · `Empty` · `EnhanceYourCalm` · `Error` · `ErrorCode` · `Family` · `Firefox` · `Firefox148` · `Firefox149` · `Firefox150` · `Firefox151` · `Firefox152` · `Firefox153` · `Firefox154` · `FlowControlError` · `Follow` · `Form` · `FormNavigate` · `FrameSizeError` · `FrameTooLarge` · `Future` · `Gzip` · `H2Error` · `H2Fingerprint` · `H2PlatformOverride` · `H2PriorityProfile` · `H2Profile` · `H3Config` · `Handshake` · `HandshakeIo` · `HappyEyeballsConfig` · `Head` · `HeaderAnchor` · `HeaderContext` · `HeaderList` · `HeaderPair` · `Host` · `Hostname` · `Hpack` · `Http1` · `Http11Required` · `Http1_1` · `Http2` · `Http3` · `HttpVersion` · `IOS` · `Identity` · `InadequateSecurity` · `InternalError` · `IntoFuture` · `IntoParamPair` · `Io` · `Item` · `Ja3Input` · `Ja4Input` · `Ja4hInput` · `Jar` · `Json` · `Kind` · `LINUX` · `Lax` · `LeylineService` · `Linux` · `Log` · `Logged` · `MACOS` · `MacOS` · `Native` · `Navigate` · `NoError` · `NoProxy` · `None` · `OkHttp` · `OkHttpAndroid10` · `Opera` · `Output` · `Parse` · `Part` · `Pending` · `Ping` · `Pinning` · `Platform` · `PlatformIdentity` · `Pong` · `PoolConfig` · `PoolStats` · `Preset` · `Profile` · `ProfileError` · `ProfileMeta` · `ProfileRegistry` · `ProtocolError` · `ProtocolPolicy` · `Proxy` · `ProxyConfig` · `ProxyRule` · `ProxyUrl` · `Race` · `Redirect` · `RedirectAction` · `RedirectAttempt` · `RedirectPolicy` · `RefusedStream` · `Reply` · `Request` · `RequestBuilder` · `ResolveFuture` · `Resolver` · `Response` · `ResponseTiming` · `Result` · `RetryPolicy` · `RetryTrigger` · `Safari` · `Safari18` · `Safari26` · `SafariIOS17` · `SafariIOS18` · `SafariIos` · `SameSite` · `Script` · `Sent` · `ServerError` · `Service` · `Session` · `SessionBuilder` · `SettingsTimeout` · `SocketConfig` · `SslConfig` · `SslConnect` · `Status` · `Stop` · `Stream` · `StreamClosed` · `Strict` · `SystemResolver` · `TcpConnect` · `TcpProfile` · `Text` · `Timeout` · `TimeoutConfig` · `Timing` · `Tls` · `Tls10` · `Tls12` · `Tls13` · `TlsContext` · `TlsError` · `TlsFingerprint` · `TlsMinVersion` · `TlsProfile` · `TlsTrustConfig` · `Trace` · `TracingTrace` · `Transport` · `TrustStore` · `Unverified` · `Url` · `Vivaldi` · `WINDOWS` · `WebSocketBuilder` · `WebSocketConfig` · `Windows` · `WsConnection` · `WsMessage` · `WsSink` · `WsStream` · `Xhr` · `Zstd` · `accept` · `accept_language` · `accept_unmasked_frames` · `active_connection_id_limit` · `add_ca_der` · `add_ca_file` · `add_pinned_leaf_sha256` · `add_root_certificate_der` · `add_root_certificate_file` · `addrs` · `akamai` · `all` · `all_cookies` · `allow_non_idempotent_retry` · `alpn` · `alps` · `alps_new_codepoint` · `anchor` · `anchor_name` · `anchored` · `android` · `append` · `append_anchored` · `append_header` · `append_headers` · `as_bytes` · `as_str` · `as_text` · `attempt_limit` · `audit` · `backoff_factor` · `bare` · `basic_auth` · `bearer_auth` · `body` · `body_prefix` · `boundary` · `brand` · `brave` · `brotli` · `browser` · `build` · `build_headers` · `builder` · `builtin` · `bytes` · `ca_der_count` · `ca_files` · `call` · `captured_against` · `cert_compression` · `certificate_chain_file` · `chrome` · `chromium_major` · `cipher` · `ciphers` · `clear` · `client_identity` · `client_identity_files` · `clone` · `close` · `cmp` · `code` · `compress` · `compression` · `compute_ja3` · `compute_ja4` · `compute_ja4h` · `compute_ja4t` · `config` · `connect` · `connect_ms` · `connect_timeout` · `contains_named` · `content_length` · `content_type` · `cookie` · `cookie_header` · `cookie_jar` · `cookies` · `copy_to` · `creation_time` · `curves` · `custom` · `danger_accept_invalid_certs` · `dcid_length` · `deep_clone` · `default` · `default_browser` · `default_firefox` · `default_priority` · `default_timeout` · `deflate` · `delegated_credentials` · `delete` · `deserialize` · `detect_host` · `df` · `digest_auth` · `disable_env_proxies` · `dns` · `domain` · `done` · `download_to` · `ech_grease` · `edge` · `elapsed` · `enable_push` · `entries` · `eq` · `error_for_status` · `evictions_dead` · `evictions_idle` · `evictions_lru` · `exclusive` · `execute` · `expected_h2_fingerprint` · `expected_h2_fingerprint_for` · `expected_ja4` · `expected_resumed_ja4` · `expires` · `export_cookies` · `extension_ids` · `extension_permutation` · `extra_headers` · `family` · `family_hellos` · `file` · `filename` · `fingerprint` · `firefox` · `fmt` · `for_family` · `for_platform` · `form` · `form_str` · `from` · `from_env` · `from_pairs` · `from_profile` · `from_string` · `from_toml` · `from_u32` · `get` · `get_browser` · `get_cookie` · `get_named` · `global` · `grease` · `gzip` · `h1_hits` · `h1_misses` · `h2` · `h2_fingerprint` · `h2_hits` · `h2_misses` · `h2_ping_after_idle` · `h2_ping_failures` · `h2_ping_timeout` · `h3_hits` · `h3_misses` · `happy_eyeballs` · `has_sni` · `hash` · `head` · `header` · `header_all` · `header_map` · `header_order` · `header_table_size` · `headers` · `headers_mut` · `hello_library` · `hello_rep` · `host` · `host_only` · `http` · `http1` · `http2` · `http3` · `http_identity` · `http_only` · `http_version` · `https` · `https_only` · `id` · `identity` · `identity_for` · `identity_key` · `idle_timeout` · `infer_anchor` · `initial_backoff` · `initial_connection_window_size` · `initial_max_data` · `initial_max_stream_data_bidi_local` · `initial_max_stream_data_bidi_remote` · `initial_max_stream_data_uni` · `initial_max_streams_bidi` · `initial_max_streams_uni` · `initial_stream_window_size` · `installs` · `interface` · `into_bytes` · `into_future` · `into_param_pair` · `into_stream` · `into_string` · `into_text` · `io` · `ios` · `is_before` · `is_body` · `is_client_error` · `is_connect` · `is_connection_closed` · `is_decode` · `is_empty` · `is_expired` · `is_firefox` · `is_http2` · `is_redirect` · `is_retryable` · `is_server_error` · `is_status` · `is_stream` · `is_success` · `is_timeout` · `iter` · `ja3` · `ja4` · `ja4h` · `ja4t` · `jitter` · `json` · `keepalive` · `kind` · `label` · `last_access` · `latest` · `layer` · `len` · `len_hint` · `leyline` · `limited` · `linux` · `load` · `load_cookies` · `load_warnings` · `local_address` · `local_ipv4` · `local_ipv6` · `location` · `locked` · `macos` · `matches` · `max` · `max_backoff` · `max_concurrent_streams` · `max_connections` · `max_field_section_size` · `max_frame_size` · `max_h1_conns_per_host` · `max_header_list_size` · `max_idle_timeout` · `max_message_size` · `max_redirects` · `max_response_body_bytes` · `max_retries` · `max_retry_after` · `max_tls_12` · `max_udp_payload_size` · `max_write_buffer_size` · `merge` · `message` · `meta` · `method` · `mime` · `min_tls_version` · `mobile_flag` · `mss` · `multipart` · `name` · `navigate_accept` · `navigate_accept_override` · `new` · `no_delay` · `no_proxy` · `none` · `ocsp_stapling` · `omit_settings` · `on` · `on_status` · `opera` · `origin` · `outcome` · `overlay` · `padding` · `parse` · `part` · `partial_cmp` · `pass` · `pass_library` · `patch` · `path` · `permute_extensions` · `pinned_leaf_sha256` · `platform` · `platforms` · `poll_next` · `poll_ready` · `pool_config` · `pool_stats` · `port` · `post` · `pre_shared_key` · `preconnect` · `preconnect_via` · `prefer_http2` · `preset` · `previous` · `private_key_file` · `profile` · `profile_key` · `protocol` · `protocol_policy` · `proxies` · `proxy` · `pseudo_order` · `put` · `qpack_blocked_streams` · `qpack_max_table_capacity` · `query` · `race` · `read` · `read_buffer_size` · `read_until` · `reason` · `record_size_limit` · `recv` · `recv_buffer_size` · `redirect_chain` · `redirect_policy` · `referer` · `remove_all` · `remove_all_named` · `remove_named` · `remove_named_for_host` · `request` · `request_header_order` · `request_headers` · `request_trust_anchors` · `resolve` · `resolve_delay` · `resolve_for_platform` · `resolve_host` · `resolve_host_to_addrs` · `resolver` · `response_header` · `response_header_timeout` · `resumed_ja4` · `retry` · `retry_on` · `reused` · `rotate_hello` · `rotate_tls` · `safari` · `same_site` · `sec_ch_platform` · `sec_ch_ua` · `sec_ch_ua_mobile` · `sec_ch_ua_platform` · `secure` · `send` · `send_binary` · `send_buffer_size` · `send_ms` · `send_raw` · `sent` · `serialize` · `session` · `session_tickets` · `set` · `set_cookie` · `set_named` · `set_named_on` · `settings_order` · `sigalgs` · `signed_cert_timestamps` · `size` · `snapshot` · `socket_config` · `socks5` · `socks5h` · `source` · `split` · `stale_probed` · `status` · `store_response_cookies` · `store_set_cookie` · `stream` · `stream_dependency` · `stream_id` · `stream_with_length` · `strict` · `tcp_keepalive` · `tcp_keepalive_interval` · `tcp_keepalive_retries` · `tcp_nodelay` · `tcp_profile` · `tcp_user_timeout` · `text` · `text_utf8` · `text_with_charset` · `timeout` · `timeouts` · `timing` · `tls` · `tls_alpn` · `tls_cipher` · `tls_peer_certificate` · `tls_record_version` · `tls_trust` · `tls_version` · `total` · `total_ms` · `trace` · `trailers` · `transient` · `try_from` · `ttl` · `unknown_setting8` · `unknown_setting9` · `uri` · `url` · `user_agent` · `uses_env` · `uses_env_roots` · `uses_system_roots` · `value` · `verified_against` · `version` · `version_for` · `vivaldi` · `websocket` · `websocket_config` · `weight` · `window_scale` · `window_size` · `windows` · `with_backoff` · `with_cookie_jar` · `with_max_retries` · `with_max_retry_after` · `with_message` · `with_proxy` · `with_redirect_policy` · `with_rule` · `with_source` · `without_env` · `without_env_roots` · `without_system_roots` · `write_buffer_size` · `zstd`

</details>

## `leyline`

```rust,ignore
pub mod leyline
pub fn &(K, V)::into_param_pair(self) -> (alloc::string::String, alloc::string::String)
pub fn (K, V)::into_param_pair(self) -> (alloc::string::String, alloc::string::String)
```

### `Body`

```rust,ignore
pub fn leyline::Body::from(form: leyline::multipart::Form) -> leyline::Body
pub struct leyline::Body(_)
impl leyline::Body
pub fn leyline::Body::is_empty(&self) -> bool
pub fn leyline::Body::is_stream(&self) -> bool
pub fn leyline::Body::len_hint(&self) -> core::option::Option<u64>
pub fn leyline::Body::stream<S>(stream: S) -> Self where S: futures_core::stream::Stream<Item = core::io::error::Result<bytes::bytes::Bytes>> + core::marker::Send + 'static
pub fn leyline::Body::stream_with_length<S>(stream: S, length: u64) -> Self where S: futures_core::stream::Stream<Item = core::io::error::Result<bytes::bytes::Bytes>> + core::marker::Send + 'static
impl core::convert::From<&'static [u8]> for leyline::Body
pub fn leyline::Body::from(v: &'static [u8]) -> Self
impl core::convert::From<&'static str> for leyline::Body
pub fn leyline::Body::from(s: &'static str) -> Self
impl core::convert::From<()> for leyline::Body
pub fn leyline::Body::from(_: ()) -> Self
impl core::convert::From<alloc::string::String> for leyline::Body
pub fn leyline::Body::from(s: alloc::string::String) -> Self
impl core::convert::From<alloc::vec::Vec<u8>> for leyline::Body
pub fn leyline::Body::from(v: alloc::vec::Vec<u8>) -> Self
impl core::convert::From<bytes::bytes::Bytes> for leyline::Body
pub fn leyline::Body::from(b: bytes::bytes::Bytes) -> Self
pub fn leyline::Body::from(form: leyline::multipart::Form) -> leyline::Body
impl core::default::Default for leyline::Body
pub fn leyline::Body::default() -> leyline::Body
impl core::fmt::Debug for leyline::Body
pub fn leyline::Body::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl futures_core::stream::Stream for leyline::Body
pub type leyline::Body::Item = core::result::Result<bytes::bytes::Bytes, core::io::error::Error>
pub fn leyline::Body::poll_next(self: core::pin::Pin<&mut Self>, cx: &mut core::task::wake::Context<'_>) -> core::task::poll::Poll<core::option::Option<Self::Item>>
impl !core::marker::Freeze for leyline::Body
impl core::marker::Send for leyline::Body
impl !core::marker::Sync for leyline::Body
impl core::marker::Unpin for leyline::Body
impl core::marker::UnsafeUnpin for leyline::Body
impl !core::panic::unwind_safe::RefUnwindSafe for leyline::Body
impl !core::panic::unwind_safe::UnwindSafe for leyline::Body
impl tower_service::Service<http::request::Request<leyline::Body>> for leyline::LeylineService
impl core::convert::From<http::request::Request<leyline::Body>> for leyline::Request
```

### `BodyStream`

```rust,ignore
pub struct leyline::BodyStream
impl core::fmt::Debug for leyline::BodyStream
pub fn leyline::BodyStream::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl futures_core::stream::Stream for leyline::BodyStream
pub type leyline::BodyStream::Item = core::result::Result<bytes::bytes::Bytes, core::io::error::Error>
pub fn leyline::BodyStream::poll_next(self: core::pin::Pin<&mut Self>, cx: &mut core::task::wake::Context<'_>) -> core::task::poll::Poll<core::option::Option<Self::Item>>
impl core::marker::Freeze for leyline::BodyStream
impl core::marker::Send for leyline::BodyStream
impl core::marker::Sync for leyline::BodyStream
impl core::marker::Unpin for leyline::BodyStream
impl core::marker::UnsafeUnpin for leyline::BodyStream
impl !core::panic::unwind_safe::RefUnwindSafe for leyline::BodyStream
impl !core::panic::unwind_safe::UnwindSafe for leyline::BodyStream
```

### `Browser`

```rust,ignore
impl leyline::Browser
pub fn leyline::Browser::all() -> &'static [leyline::Browser]
pub fn leyline::Browser::chromium_major(&self) -> core::option::Option<u32>
pub fn leyline::Browser::default_browser() -> Self
pub fn leyline::Browser::default_firefox() -> Self
pub fn leyline::Browser::family(&self) -> &'static str
pub fn leyline::Browser::family_hellos(self) -> &'static [Self]
pub fn leyline::Browser::for_platform(self, platform: leyline::Platform) -> Self
pub fn leyline::Browser::hello_rep(self) -> Self
pub fn leyline::Browser::is_firefox(&self) -> bool
pub fn leyline::Browser::latest(family: leyline::profile::Family) -> Self
pub fn leyline::Browser::max_tls_12(&self) -> bool
pub fn leyline::Browser::profile_key(&self) -> (&'static str, u32)
impl leyline::Browser
pub fn leyline::Browser::profile(self) -> &'static leyline::profile::BrowserProfile
impl core::clone::Clone for leyline::Browser
pub fn leyline::Browser::clone(&self) -> leyline::Browser
impl core::cmp::Eq for leyline::Browser
impl core::cmp::PartialEq for leyline::Browser
pub fn leyline::Browser::eq(&self, other: &leyline::Browser) -> bool
impl core::default::Default for leyline::Browser
pub fn leyline::Browser::default() -> Self
impl core::fmt::Debug for leyline::Browser
pub fn leyline::Browser::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::fmt::Display for leyline::Browser
pub fn leyline::Browser::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::hash::Hash for leyline::Browser
pub fn leyline::Browser::hash<__H: core::hash::Hasher>(&self, state: &mut __H)
impl core::marker::Copy for leyline::Browser
impl core::marker::StructuralPartialEq for leyline::Browser
impl core::marker::Freeze for leyline::Browser
impl core::marker::Send for leyline::Browser
impl core::marker::Sync for leyline::Browser
impl core::marker::Unpin for leyline::Browser
impl core::marker::UnsafeUnpin for leyline::Browser
impl core::panic::unwind_safe::RefUnwindSafe for leyline::Browser
impl core::panic::unwind_safe::UnwindSafe for leyline::Browser
#[non_exhaustive] pub enum leyline::Browser
pub leyline::Browser::Brave146
pub leyline::Browser::CfnetworkIOS18
pub leyline::Browser::CfnetworkMacOS26
pub leyline::Browser::Chrome145
pub leyline::Browser::Chrome146
pub leyline::Browser::Chrome147
pub leyline::Browser::Chrome148
pub leyline::Browser::Chrome149
pub leyline::Browser::Chrome150
pub leyline::Browser::Chrome151
pub leyline::Browser::Chrome152
pub leyline::Browser::Firefox148
pub leyline::Browser::Firefox149
pub leyline::Browser::Firefox150
pub leyline::Browser::Firefox151
pub leyline::Browser::Firefox152
pub leyline::Browser::Firefox153
pub leyline::Browser::Firefox154
pub leyline::Browser::OkHttpAndroid10
pub leyline::Browser::Safari18
pub leyline::Browser::Safari26
pub leyline::Browser::SafariIOS17
pub leyline::Browser::SafariIOS18
impl leyline::Browser
pub fn leyline::Browser::all() -> &'static [leyline::Browser]
pub fn leyline::Browser::chromium_major(&self) -> core::option::Option<u32>
pub fn leyline::Browser::default_browser() -> Self
pub fn leyline::Browser::default_firefox() -> Self
pub fn leyline::Browser::family(&self) -> &'static str
pub fn leyline::Browser::family_hellos(self) -> &'static [Self]
pub fn leyline::Browser::for_platform(self, platform: leyline::Platform) -> Self
pub fn leyline::Browser::hello_rep(self) -> Self
pub fn leyline::Browser::is_firefox(&self) -> bool
pub fn leyline::Browser::latest(family: leyline::profile::Family) -> Self
pub fn leyline::Browser::max_tls_12(&self) -> bool
pub fn leyline::Browser::profile_key(&self) -> (&'static str, u32)
impl leyline::Browser
pub fn leyline::Browser::profile(self) -> &'static leyline::profile::BrowserProfile
impl core::clone::Clone for leyline::Browser
pub fn leyline::Browser::clone(&self) -> leyline::Browser
impl core::cmp::Eq for leyline::Browser
impl core::cmp::PartialEq for leyline::Browser
pub fn leyline::Browser::eq(&self, other: &leyline::Browser) -> bool
impl core::default::Default for leyline::Browser
pub fn leyline::Browser::default() -> Self
impl core::fmt::Debug for leyline::Browser
pub fn leyline::Browser::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::fmt::Display for leyline::Browser
pub fn leyline::Browser::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::hash::Hash for leyline::Browser
pub fn leyline::Browser::hash<__H: core::hash::Hasher>(&self, state: &mut __H)
impl core::marker::Copy for leyline::Browser
impl core::marker::StructuralPartialEq for leyline::Browser
impl core::marker::Freeze for leyline::Browser
impl core::marker::Send for leyline::Browser
impl core::marker::Sync for leyline::Browser
impl core::marker::Unpin for leyline::Browser
impl core::marker::UnsafeUnpin for leyline::Browser
impl core::panic::unwind_safe::RefUnwindSafe for leyline::Browser
impl core::panic::unwind_safe::UnwindSafe for leyline::Browser
```

### `BrowserProfile`

```rust,ignore
#[non_exhaustive] pub struct leyline::BrowserProfile
pub leyline::BrowserProfile::h2: leyline::profile::H2Profile
pub leyline::BrowserProfile::identity: std::collections::hash::map::HashMap<alloc::string::String, leyline::profile::PlatformIdentity>
pub leyline::BrowserProfile::meta: leyline::profile::ProfileMeta
pub leyline::BrowserProfile::tls: leyline::profile::TlsProfile
```

### `ChromiumBrand`

```rust,ignore
impl leyline::ChromiumBrand
pub fn leyline::ChromiumBrand::label(self) -> &'static str
pub fn leyline::ChromiumBrand::overlay(self, chromium_major: u32, platform: leyline::Platform, profile_user_agent: &str, profile_sec_ch_ua: &str) -> core::result::Result<core::option::Option<leyline::profile::BrandOverlay>, leyline::profile::BrandOverlayError>
pub fn leyline::ChromiumBrand::version_for(self, chromium_major: u32) -> core::option::Option<u32>
impl core::clone::Clone for leyline::ChromiumBrand
pub fn leyline::ChromiumBrand::clone(&self) -> leyline::ChromiumBrand
impl core::cmp::Eq for leyline::ChromiumBrand
impl core::cmp::PartialEq for leyline::ChromiumBrand
pub fn leyline::ChromiumBrand::eq(&self, other: &leyline::ChromiumBrand) -> bool
impl core::default::Default for leyline::ChromiumBrand
pub fn leyline::ChromiumBrand::default() -> leyline::ChromiumBrand
impl core::fmt::Debug for leyline::ChromiumBrand
pub fn leyline::ChromiumBrand::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::hash::Hash for leyline::ChromiumBrand
pub fn leyline::ChromiumBrand::hash<__H: core::hash::Hasher>(&self, state: &mut __H)
impl core::marker::Copy for leyline::ChromiumBrand
impl core::marker::StructuralPartialEq for leyline::ChromiumBrand
impl core::marker::Freeze for leyline::ChromiumBrand
impl core::marker::Send for leyline::ChromiumBrand
impl core::marker::Sync for leyline::ChromiumBrand
impl core::marker::Unpin for leyline::ChromiumBrand
impl core::marker::UnsafeUnpin for leyline::ChromiumBrand
impl core::panic::unwind_safe::RefUnwindSafe for leyline::ChromiumBrand
impl core::panic::unwind_safe::UnwindSafe for leyline::ChromiumBrand
#[non_exhaustive] pub enum leyline::ChromiumBrand
pub leyline::ChromiumBrand::Chrome
pub leyline::ChromiumBrand::Edge
pub leyline::ChromiumBrand::Opera
pub leyline::ChromiumBrand::Vivaldi
impl leyline::ChromiumBrand
pub fn leyline::ChromiumBrand::label(self) -> &'static str
pub fn leyline::ChromiumBrand::overlay(self, chromium_major: u32, platform: leyline::Platform, profile_user_agent: &str, profile_sec_ch_ua: &str) -> core::result::Result<core::option::Option<leyline::profile::BrandOverlay>, leyline::profile::BrandOverlayError>
pub fn leyline::ChromiumBrand::version_for(self, chromium_major: u32) -> core::option::Option<u32>
impl core::clone::Clone for leyline::ChromiumBrand
pub fn leyline::ChromiumBrand::clone(&self) -> leyline::ChromiumBrand
impl core::cmp::Eq for leyline::ChromiumBrand
impl core::cmp::PartialEq for leyline::ChromiumBrand
pub fn leyline::ChromiumBrand::eq(&self, other: &leyline::ChromiumBrand) -> bool
impl core::default::Default for leyline::ChromiumBrand
pub fn leyline::ChromiumBrand::default() -> leyline::ChromiumBrand
impl core::fmt::Debug for leyline::ChromiumBrand
pub fn leyline::ChromiumBrand::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::hash::Hash for leyline::ChromiumBrand
pub fn leyline::ChromiumBrand::hash<__H: core::hash::Hasher>(&self, state: &mut __H)
impl core::marker::Copy for leyline::ChromiumBrand
impl core::marker::StructuralPartialEq for leyline::ChromiumBrand
impl core::marker::Freeze for leyline::ChromiumBrand
impl core::marker::Send for leyline::ChromiumBrand
impl core::marker::Sync for leyline::ChromiumBrand
impl core::marker::Unpin for leyline::ChromiumBrand
impl core::marker::UnsafeUnpin for leyline::ChromiumBrand
impl core::panic::unwind_safe::RefUnwindSafe for leyline::ChromiumBrand
impl core::panic::unwind_safe::UnwindSafe for leyline::ChromiumBrand
```

### `CloseFrame`

```rust,ignore
#[non_exhaustive] pub struct leyline::CloseFrame
pub leyline::CloseFrame::code: u16
pub leyline::CloseFrame::reason: alloc::string::String
impl leyline::CloseFrame
pub fn leyline::CloseFrame::new(code: u16, reason: impl core::convert::Into<alloc::string::String>) -> Self
impl core::clone::Clone for leyline::CloseFrame
pub fn leyline::CloseFrame::clone(&self) -> leyline::CloseFrame
impl core::cmp::Eq for leyline::CloseFrame
impl core::cmp::PartialEq for leyline::CloseFrame
pub fn leyline::CloseFrame::eq(&self, other: &leyline::CloseFrame) -> bool
impl core::fmt::Debug for leyline::CloseFrame
pub fn leyline::CloseFrame::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::marker::StructuralPartialEq for leyline::CloseFrame
impl core::marker::Freeze for leyline::CloseFrame
impl core::marker::Send for leyline::CloseFrame
impl core::marker::Sync for leyline::CloseFrame
impl core::marker::Unpin for leyline::CloseFrame
impl core::marker::UnsafeUnpin for leyline::CloseFrame
impl core::panic::unwind_safe::RefUnwindSafe for leyline::CloseFrame
impl core::panic::unwind_safe::UnwindSafe for leyline::CloseFrame
```

### `CompressionConfig`

```rust,ignore
#[non_exhaustive] pub struct leyline::CompressionConfig
pub leyline::CompressionConfig::brotli: bool
pub leyline::CompressionConfig::deflate: bool
pub leyline::CompressionConfig::gzip: bool
pub leyline::CompressionConfig::zstd: bool
impl leyline::CompressionConfig
pub fn leyline::CompressionConfig::brotli(self, on: bool) -> Self
pub fn leyline::CompressionConfig::deflate(self, on: bool) -> Self
pub fn leyline::CompressionConfig::gzip(self, on: bool) -> Self
pub fn leyline::CompressionConfig::new() -> Self
pub fn leyline::CompressionConfig::none() -> Self
pub fn leyline::CompressionConfig::zstd(self, on: bool) -> Self
impl core::clone::Clone for leyline::CompressionConfig
pub fn leyline::CompressionConfig::clone(&self) -> leyline::CompressionConfig
impl core::cmp::Eq for leyline::CompressionConfig
impl core::cmp::PartialEq for leyline::CompressionConfig
pub fn leyline::CompressionConfig::eq(&self, other: &leyline::CompressionConfig) -> bool
impl core::default::Default for leyline::CompressionConfig
pub fn leyline::CompressionConfig::default() -> Self
impl core::fmt::Debug for leyline::CompressionConfig
pub fn leyline::CompressionConfig::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::marker::Copy for leyline::CompressionConfig
impl core::marker::StructuralPartialEq for leyline::CompressionConfig
impl core::marker::Freeze for leyline::CompressionConfig
impl core::marker::Send for leyline::CompressionConfig
impl core::marker::Sync for leyline::CompressionConfig
impl core::marker::Unpin for leyline::CompressionConfig
impl core::marker::UnsafeUnpin for leyline::CompressionConfig
impl core::panic::unwind_safe::RefUnwindSafe for leyline::CompressionConfig
impl core::panic::unwind_safe::UnwindSafe for leyline::CompressionConfig
```

### `ContentEncoding`

```rust,ignore
#[non_exhaustive] pub enum leyline::ContentEncoding
pub leyline::ContentEncoding::Brotli
pub leyline::ContentEncoding::Deflate
pub leyline::ContentEncoding::Gzip
pub leyline::ContentEncoding::Zstd
impl core::clone::Clone for leyline::ContentEncoding
pub fn leyline::ContentEncoding::clone(&self) -> leyline::ContentEncoding
impl core::cmp::Eq for leyline::ContentEncoding
impl core::cmp::PartialEq for leyline::ContentEncoding
pub fn leyline::ContentEncoding::eq(&self, other: &leyline::ContentEncoding) -> bool
impl core::fmt::Debug for leyline::ContentEncoding
pub fn leyline::ContentEncoding::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::marker::Copy for leyline::ContentEncoding
impl core::marker::StructuralPartialEq for leyline::ContentEncoding
impl core::marker::Freeze for leyline::ContentEncoding
impl core::marker::Send for leyline::ContentEncoding
impl core::marker::Sync for leyline::ContentEncoding
impl core::marker::Unpin for leyline::ContentEncoding
impl core::marker::UnsafeUnpin for leyline::ContentEncoding
impl core::panic::unwind_safe::RefUnwindSafe for leyline::ContentEncoding
impl core::panic::unwind_safe::UnwindSafe for leyline::ContentEncoding
```

### `DigestAuth`

```rust,ignore
pub struct leyline::DigestAuth
impl leyline::DigestAuth
pub fn leyline::DigestAuth::new(username: impl core::convert::Into<alloc::string::String>, password: impl core::convert::Into<alloc::string::String>) -> Self
impl core::clone::Clone for leyline::DigestAuth
pub fn leyline::DigestAuth::clone(&self) -> leyline::DigestAuth
impl core::fmt::Debug for leyline::DigestAuth
pub fn leyline::DigestAuth::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::marker::Freeze for leyline::DigestAuth
impl core::marker::Send for leyline::DigestAuth
impl core::marker::Sync for leyline::DigestAuth
impl core::marker::Unpin for leyline::DigestAuth
impl core::marker::UnsafeUnpin for leyline::DigestAuth
impl core::panic::unwind_safe::RefUnwindSafe for leyline::DigestAuth
impl core::panic::unwind_safe::UnwindSafe for leyline::DigestAuth
```

### `DnsConfig`

```rust,ignore
pub struct leyline::DnsConfig
impl leyline::DnsConfig
pub fn leyline::DnsConfig::new() -> Self
pub fn leyline::DnsConfig::resolve_host(self, host: impl core::convert::AsRef<str>, addr: core::net::socket_addr::SocketAddr) -> Self
pub fn leyline::DnsConfig::resolve_host_to_addrs<I>(self, host: impl core::convert::AsRef<str>, addrs: I) -> Self where I: core::iter::traits::collect::IntoIterator<Item = core::net::socket_addr::SocketAddr>
pub fn leyline::DnsConfig::resolver(self, resolver: alloc::sync::Arc<dyn leyline::tls::Resolver>) -> Self
impl core::clone::Clone for leyline::DnsConfig
pub fn leyline::DnsConfig::clone(&self) -> leyline::DnsConfig
impl core::default::Default for leyline::DnsConfig
pub fn leyline::DnsConfig::default() -> Self
impl core::fmt::Debug for leyline::DnsConfig
pub fn leyline::DnsConfig::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::marker::Freeze for leyline::DnsConfig
impl core::marker::Send for leyline::DnsConfig
impl core::marker::Sync for leyline::DnsConfig
impl core::marker::Unpin for leyline::DnsConfig
impl core::marker::UnsafeUnpin for leyline::DnsConfig
impl !core::panic::unwind_safe::RefUnwindSafe for leyline::DnsConfig
impl !core::panic::unwind_safe::UnwindSafe for leyline::DnsConfig
```

### `Error`

```rust,ignore
pub fn leyline::Error::from(e: leyline::TlsError) -> Self
pub fn leyline::Error::from(e: leyline::H2Error) -> Self
pub fn leyline::Error::from(e: leyline::TlsError) -> Self
pub struct leyline::Error
impl leyline::Error
pub fn leyline::Error::body_prefix(&self) -> core::option::Option<&[u8]>
pub fn leyline::Error::h2(&self) -> core::option::Option<&leyline::H2Error>
pub fn leyline::Error::io(&self) -> core::option::Option<&core::io::error::Error>
pub fn leyline::Error::is_body(&self) -> bool
pub fn leyline::Error::is_connect(&self) -> bool
pub fn leyline::Error::is_connection_closed(&self) -> bool
pub fn leyline::Error::is_decode(&self) -> bool
pub fn leyline::Error::is_redirect(&self) -> bool
pub fn leyline::Error::is_status(&self) -> bool
pub fn leyline::Error::is_timeout(&self) -> bool
pub fn leyline::Error::kind(&self) -> leyline::Kind
pub fn leyline::Error::message(&self) -> core::option::Option<&str>
pub fn leyline::Error::new(kind: leyline::Kind) -> Self
pub fn leyline::Error::status(&self) -> core::option::Option<http::status::StatusCode>
pub fn leyline::Error::tls(&self) -> core::option::Option<&leyline::TlsError>
pub fn leyline::Error::url(&self) -> core::option::Option<&http::uri::Uri>
pub fn leyline::Error::with_message(self, message: impl core::convert::Into<alloc::borrow::Cow<'static, str>>) -> Self
pub fn leyline::Error::with_source(self, source: impl core::convert::Into<alloc::boxed::Box<(dyn core::error::Error + core::marker::Send + core::marker::Sync)>>) -> Self
impl core::convert::From<core::io::error::Error> for leyline::Error
pub fn leyline::Error::from(e: core::io::error::Error) -> Self
impl core::convert::From<http::error::Error> for leyline::Error
pub fn leyline::Error::from(e: http::error::Error) -> Self
impl core::convert::From<http::header::name::InvalidHeaderName> for leyline::Error
pub fn leyline::Error::from(e: http::header::name::InvalidHeaderName) -> Self
impl core::convert::From<http::header::value::InvalidHeaderValue> for leyline::Error
pub fn leyline::Error::from(e: http::header::value::InvalidHeaderValue) -> Self
impl core::convert::From<http::uri::InvalidUri> for leyline::Error
pub fn leyline::Error::from(e: http::uri::InvalidUri) -> Self
pub fn leyline::Error::from(e: leyline::H2Error) -> Self
pub fn leyline::Error::from(e: leyline::TlsError) -> Self
impl core::error::Error for leyline::Error
pub fn leyline::Error::source(&self) -> core::option::Option<&(dyn core::error::Error + 'static)>
impl core::fmt::Debug for leyline::Error
pub fn leyline::Error::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::fmt::Display for leyline::Error
pub fn leyline::Error::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::marker::Freeze for leyline::Error
impl core::marker::Send for leyline::Error
impl core::marker::Sync for leyline::Error
impl core::marker::Unpin for leyline::Error
impl core::marker::UnsafeUnpin for leyline::Error
impl !core::panic::unwind_safe::RefUnwindSafe for leyline::Error
impl !core::panic::unwind_safe::UnwindSafe for leyline::Error
```

### `ErrorCode`

```rust,ignore
#[non_exhaustive] #[repr(u32)] pub enum leyline::ErrorCode
pub leyline::ErrorCode::Cancel = 8
pub leyline::ErrorCode::CompressionError = 9
pub leyline::ErrorCode::ConnectError = 10
pub leyline::ErrorCode::EnhanceYourCalm = 11
pub leyline::ErrorCode::FlowControlError = 3
pub leyline::ErrorCode::FrameSizeError = 6
pub leyline::ErrorCode::Http11Required = 13
pub leyline::ErrorCode::InadequateSecurity = 12
pub leyline::ErrorCode::InternalError = 2
pub leyline::ErrorCode::NoError = 0
pub leyline::ErrorCode::ProtocolError = 1
pub leyline::ErrorCode::RefusedStream = 7
pub leyline::ErrorCode::SettingsTimeout = 4
pub leyline::ErrorCode::StreamClosed = 5
impl leyline::ErrorCode
pub fn leyline::ErrorCode::from_u32(val: u32) -> Self
impl core::clone::Clone for leyline::ErrorCode
pub fn leyline::ErrorCode::clone(&self) -> leyline::ErrorCode
impl core::cmp::Eq for leyline::ErrorCode
impl core::cmp::PartialEq for leyline::ErrorCode
pub fn leyline::ErrorCode::eq(&self, other: &leyline::ErrorCode) -> bool
impl core::fmt::Debug for leyline::ErrorCode
pub fn leyline::ErrorCode::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::marker::Copy for leyline::ErrorCode
impl core::marker::StructuralPartialEq for leyline::ErrorCode
impl core::marker::Freeze for leyline::ErrorCode
impl core::marker::Send for leyline::ErrorCode
impl core::marker::Sync for leyline::ErrorCode
impl core::marker::Unpin for leyline::ErrorCode
impl core::marker::UnsafeUnpin for leyline::ErrorCode
impl core::panic::unwind_safe::RefUnwindSafe for leyline::ErrorCode
impl core::panic::unwind_safe::UnwindSafe for leyline::ErrorCode
```

### `H2Error`

```rust,ignore
#[non_exhaustive] pub enum leyline::H2Error
pub leyline::H2Error::Connection
pub leyline::H2Error::Connection::code: leyline::ErrorCode
pub leyline::H2Error::Connection::reason: alloc::string::String
pub leyline::H2Error::FrameTooLarge
pub leyline::H2Error::FrameTooLarge::max: u32
pub leyline::H2Error::FrameTooLarge::size: u32
pub leyline::H2Error::Hpack(alloc::string::String)
pub leyline::H2Error::Io(core::io::error::Error)
pub leyline::H2Error::Stream
pub leyline::H2Error::Stream::code: leyline::ErrorCode
pub leyline::H2Error::Stream::stream_id: u32
impl core::convert::From<core::io::error::Error> for leyline::H2Error
pub fn leyline::H2Error::from(source: core::io::error::Error) -> Self
impl core::convert::From<leyline::H2Error> for leyline::Error
impl core::error::Error for leyline::H2Error
pub fn leyline::H2Error::source(&self) -> core::option::Option<&(dyn core::error::Error + 'static)>
impl core::fmt::Debug for leyline::H2Error
pub fn leyline::H2Error::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::fmt::Display for leyline::H2Error
pub fn leyline::H2Error::fmt(&self, __formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::marker::Freeze for leyline::H2Error
impl core::marker::Send for leyline::H2Error
impl core::marker::Sync for leyline::H2Error
impl core::marker::Unpin for leyline::H2Error
impl core::marker::UnsafeUnpin for leyline::H2Error
impl !core::panic::unwind_safe::RefUnwindSafe for leyline::H2Error
impl !core::panic::unwind_safe::UnwindSafe for leyline::H2Error
impl core::convert::From<leyline::H2Error> for leyline::Error
```

### `H3Config`

```rust,ignore
#[non_exhaustive] pub struct leyline::H3Config
pub leyline::H3Config::active_connection_id_limit: u64
pub leyline::H3Config::dcid_length: usize
pub leyline::H3Config::initial_max_data: u64
pub leyline::H3Config::initial_max_stream_data_bidi_local: u64
pub leyline::H3Config::initial_max_stream_data_bidi_remote: u64
pub leyline::H3Config::initial_max_stream_data_uni: u64
pub leyline::H3Config::initial_max_streams_bidi: u64
pub leyline::H3Config::initial_max_streams_uni: u64
pub leyline::H3Config::max_field_section_size: u64
pub leyline::H3Config::max_idle_timeout: core::time::Duration
pub leyline::H3Config::max_response_body_bytes: u64
pub leyline::H3Config::max_udp_payload_size: u16
pub leyline::H3Config::qpack_blocked_streams: u64
pub leyline::H3Config::qpack_max_table_capacity: u64
impl leyline::H3Config
pub fn leyline::H3Config::chrome() -> Self
pub fn leyline::H3Config::firefox() -> Self
pub fn leyline::H3Config::for_family(family: &str) -> core::result::Result<Self, leyline::Error>
pub fn leyline::H3Config::safari() -> Self
impl core::clone::Clone for leyline::H3Config
pub fn leyline::H3Config::clone(&self) -> leyline::H3Config
impl core::fmt::Debug for leyline::H3Config
pub fn leyline::H3Config::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::marker::Freeze for leyline::H3Config
impl core::marker::Send for leyline::H3Config
impl core::marker::Sync for leyline::H3Config
impl core::marker::Unpin for leyline::H3Config
impl core::marker::UnsafeUnpin for leyline::H3Config
impl core::panic::unwind_safe::RefUnwindSafe for leyline::H3Config
impl core::panic::unwind_safe::UnwindSafe for leyline::H3Config
```

### `HeaderList`

```rust,ignore
pub struct leyline::HeaderList
impl leyline::HeaderList
pub fn leyline::HeaderList::append(&mut self, n: impl core::convert::TryInto<http::header::name::HeaderName>, v: impl core::convert::TryInto<http::header::value::HeaderValue>) -> leyline::Result<()>
pub fn leyline::HeaderList::append_anchored(&mut self, anchor: leyline::profile::anchor::HeaderAnchor, n: impl core::convert::TryInto<http::header::name::HeaderName>, v: impl core::convert::TryInto<http::header::value::HeaderValue>) -> leyline::Result<()>
pub fn leyline::HeaderList::from_pairs<N, V>(headers: alloc::vec::Vec<(N, V)>) -> leyline::Result<Self> where N: core::convert::TryInto<http::header::name::HeaderName>, V: core::convert::TryInto<http::header::value::HeaderValue>
pub fn leyline::HeaderList::get(&self, name: &str) -> core::option::Option<&http::header::value::HeaderValue>
pub fn leyline::HeaderList::is_empty(&self) -> bool
pub fn leyline::HeaderList::iter(&self) -> impl core::iter::traits::iterator::Iterator<Item = (&http::header::name::HeaderName, &http::header::value::HeaderValue)>
pub fn leyline::HeaderList::new() -> Self
pub fn leyline::HeaderList::remove_all(&mut self, name: &str)
pub fn leyline::HeaderList::set(&mut self, n: impl core::convert::TryInto<http::header::name::HeaderName>, v: impl core::convert::TryInto<http::header::value::HeaderValue>) -> leyline::Result<()>
impl core::clone::Clone for leyline::HeaderList
pub fn leyline::HeaderList::clone(&self) -> leyline::HeaderList
impl core::cmp::Eq for leyline::HeaderList
impl core::cmp::PartialEq for leyline::HeaderList
pub fn leyline::HeaderList::eq(&self, other: &leyline::HeaderList) -> bool
impl core::default::Default for leyline::HeaderList
pub fn leyline::HeaderList::default() -> leyline::HeaderList
impl core::fmt::Debug for leyline::HeaderList
pub fn leyline::HeaderList::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::marker::StructuralPartialEq for leyline::HeaderList
impl<N, V> core::convert::TryFrom<alloc::vec::Vec<(N, V)>> for leyline::HeaderList where N: core::convert::TryInto<http::header::name::HeaderName>, V: core::convert::TryInto<http::header::value::HeaderValue>
pub type leyline::HeaderList::Error = leyline::Error
pub fn leyline::HeaderList::try_from(headers: alloc::vec::Vec<(N, V)>) -> leyline::Result<Self>
impl core::marker::Freeze for leyline::HeaderList
impl core::marker::Send for leyline::HeaderList
impl core::marker::Sync for leyline::HeaderList
impl core::marker::Unpin for leyline::HeaderList
impl core::marker::UnsafeUnpin for leyline::HeaderList
impl core::panic::unwind_safe::RefUnwindSafe for leyline::HeaderList
impl core::panic::unwind_safe::UnwindSafe for leyline::HeaderList
```

### `HttpVersion`

```rust,ignore
#[non_exhaustive] pub enum leyline::HttpVersion
pub leyline::HttpVersion::Http1_1
pub leyline::HttpVersion::Http2
pub leyline::HttpVersion::Http3
impl leyline::HttpVersion
pub fn leyline::HttpVersion::as_str(self) -> &'static str
impl core::clone::Clone for leyline::HttpVersion
pub fn leyline::HttpVersion::clone(&self) -> leyline::HttpVersion
impl core::cmp::Eq for leyline::HttpVersion
impl core::cmp::PartialEq for leyline::HttpVersion
pub fn leyline::HttpVersion::eq(&self, other: &leyline::HttpVersion) -> bool
impl core::fmt::Debug for leyline::HttpVersion
pub fn leyline::HttpVersion::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::marker::Copy for leyline::HttpVersion
impl core::marker::StructuralPartialEq for leyline::HttpVersion
impl core::marker::Freeze for leyline::HttpVersion
impl core::marker::Send for leyline::HttpVersion
impl core::marker::Sync for leyline::HttpVersion
impl core::marker::Unpin for leyline::HttpVersion
impl core::marker::UnsafeUnpin for leyline::HttpVersion
impl core::panic::unwind_safe::RefUnwindSafe for leyline::HttpVersion
impl core::panic::unwind_safe::UnwindSafe for leyline::HttpVersion
```

### `Identity`

```rust,ignore
pub struct leyline::Identity
impl leyline::Identity
pub fn leyline::Identity::hello_library(self) -> leyline::Result<alloc::vec::Vec<Self>>
pub fn leyline::Identity::http(self) -> leyline::Browser
pub fn leyline::Identity::locked(browser: leyline::Browser, platform: leyline::Platform) -> Self
pub fn leyline::Identity::pass(self, dest: leyline::Browser) -> leyline::Result<Self>
pub fn leyline::Identity::pass_library(self) -> alloc::vec::Vec<Self>
pub fn leyline::Identity::platform(self) -> leyline::Platform
pub fn leyline::Identity::rotate_hello(self) -> leyline::Result<Self>
pub fn leyline::Identity::rotate_tls(self, tls: leyline::Browser) -> leyline::Result<Self>
pub fn leyline::Identity::sec_ch_ua(self) -> leyline::Result<alloc::string::String>
pub fn leyline::Identity::tls(self) -> leyline::Browser
pub fn leyline::Identity::user_agent(self) -> leyline::Result<alloc::string::String>
impl core::clone::Clone for leyline::Identity
pub fn leyline::Identity::clone(&self) -> leyline::Identity
impl core::cmp::Eq for leyline::Identity
impl core::cmp::PartialEq for leyline::Identity
pub fn leyline::Identity::eq(&self, other: &leyline::Identity) -> bool
impl core::fmt::Debug for leyline::Identity
pub fn leyline::Identity::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::hash::Hash for leyline::Identity
pub fn leyline::Identity::hash<__H: core::hash::Hasher>(&self, state: &mut __H)
impl core::marker::Copy for leyline::Identity
impl core::marker::StructuralPartialEq for leyline::Identity
impl core::marker::Freeze for leyline::Identity
impl core::marker::Send for leyline::Identity
impl core::marker::Sync for leyline::Identity
impl core::marker::Unpin for leyline::Identity
impl core::marker::UnsafeUnpin for leyline::Identity
impl core::panic::unwind_safe::RefUnwindSafe for leyline::Identity
impl core::panic::unwind_safe::UnwindSafe for leyline::Identity
```

### `IntoParamPair`

```rust,ignore
pub trait leyline::IntoParamPair
pub fn leyline::IntoParamPair::into_param_pair(self) -> (alloc::string::String, alloc::string::String)
impl<K, V> leyline::IntoParamPair for &(K, V) where K: core::convert::AsRef<str>, V: core::convert::AsRef<str>
impl<K, V> leyline::IntoParamPair for (K, V) where K: core::convert::AsRef<str>, V: core::convert::AsRef<str>
```

### `Kind`

```rust,ignore
#[non_exhaustive] pub enum leyline::Kind
pub leyline::Kind::Body
pub leyline::Kind::Builder
pub leyline::Kind::Config
pub leyline::Kind::Connect
pub leyline::Kind::Decode
pub leyline::Kind::Http2
pub leyline::Kind::Http3
pub leyline::Kind::Io
pub leyline::Kind::Json
pub leyline::Kind::Proxy
pub leyline::Kind::Redirect
pub leyline::Kind::Request
pub leyline::Kind::Status
pub leyline::Kind::Timeout
pub leyline::Kind::Tls
pub leyline::Kind::Url
impl leyline::Kind
pub fn leyline::Kind::as_str(self) -> &'static str
impl core::clone::Clone for leyline::Kind
pub fn leyline::Kind::clone(&self) -> leyline::Kind
impl core::cmp::Eq for leyline::Kind
impl core::cmp::PartialEq for leyline::Kind
pub fn leyline::Kind::eq(&self, other: &leyline::Kind) -> bool
impl core::fmt::Debug for leyline::Kind
pub fn leyline::Kind::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::fmt::Display for leyline::Kind
pub fn leyline::Kind::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::hash::Hash for leyline::Kind
pub fn leyline::Kind::hash<__H: core::hash::Hasher>(&self, state: &mut __H)
impl core::marker::Copy for leyline::Kind
impl core::marker::StructuralPartialEq for leyline::Kind
impl core::marker::Freeze for leyline::Kind
impl core::marker::Send for leyline::Kind
impl core::marker::Sync for leyline::Kind
impl core::marker::Unpin for leyline::Kind
impl core::marker::UnsafeUnpin for leyline::Kind
impl core::panic::unwind_safe::RefUnwindSafe for leyline::Kind
impl core::panic::unwind_safe::UnwindSafe for leyline::Kind
```

### `LeylineService`

```rust,ignore
pub struct leyline::LeylineService
impl leyline::LeylineService
pub fn leyline::LeylineService::new(session: leyline::Session) -> Self
pub fn leyline::LeylineService::session(&self) -> &leyline::Session
impl core::clone::Clone for leyline::LeylineService
pub fn leyline::LeylineService::clone(&self) -> leyline::LeylineService
pub type leyline::LeylineService::Error = leyline::Error
pub type leyline::LeylineService::Future = core::pin::Pin<alloc::boxed::Box<(dyn core::future::future::Future<Output = core::result::Result<http::response::Response<leyline::Body>, leyline::Error>> + core::marker::Send)>>
pub type leyline::LeylineService::Response = http::response::Response<leyline::Body>
pub fn leyline::LeylineService::call(&mut self, req: http::request::Request<leyline::Body>) -> Self::Future
pub fn leyline::LeylineService::poll_ready(&mut self, _cx: &mut core::task::wake::Context<'_>) -> core::task::poll::Poll<leyline::Result<()>>
pub type leyline::LeylineService::Error = leyline::Error
pub type leyline::LeylineService::Future = core::pin::Pin<alloc::boxed::Box<(dyn core::future::future::Future<Output = core::result::Result<leyline::Response, leyline::Error>> + core::marker::Send)>>
pub type leyline::LeylineService::Response = leyline::Response
pub fn leyline::LeylineService::call(&mut self, req: leyline::Request) -> Self::Future
pub fn leyline::LeylineService::poll_ready(&mut self, _cx: &mut core::task::wake::Context<'_>) -> core::task::poll::Poll<leyline::Result<()>>
impl core::marker::Freeze for leyline::LeylineService
impl core::marker::Send for leyline::LeylineService
impl core::marker::Sync for leyline::LeylineService
impl core::marker::Unpin for leyline::LeylineService
impl core::marker::UnsafeUnpin for leyline::LeylineService
impl !core::panic::unwind_safe::RefUnwindSafe for leyline::LeylineService
impl !core::panic::unwind_safe::UnwindSafe for leyline::LeylineService
pub type leyline::LeylineService::Error = leyline::Error
pub type leyline::LeylineService::Future = core::pin::Pin<alloc::boxed::Box<(dyn core::future::future::Future<Output = core::result::Result<leyline::Response, leyline::Error>> + core::marker::Send)>>
pub type leyline::LeylineService::Response = leyline::Response
pub fn leyline::LeylineService::call(&mut self, req: leyline::Request) -> Self::Future
pub fn leyline::LeylineService::poll_ready(&mut self, _cx: &mut core::task::wake::Context<'_>) -> core::task::poll::Poll<leyline::Result<()>>
```

### `NoProxy`

```rust,ignore
pub struct leyline::NoProxy
impl leyline::NoProxy
pub fn leyline::NoProxy::from_env() -> core::option::Option<Self>
pub fn leyline::NoProxy::from_string(raw: &str) -> core::option::Option<Self>
pub fn leyline::NoProxy::matches(&self, host: &str) -> bool
pub fn leyline::NoProxy::new<I, S>(patterns: I) -> Self where I: core::iter::traits::collect::IntoIterator<Item = S>, S: core::convert::Into<alloc::string::String>
impl core::clone::Clone for leyline::NoProxy
pub fn leyline::NoProxy::clone(&self) -> leyline::NoProxy
impl core::cmp::Eq for leyline::NoProxy
impl core::cmp::PartialEq for leyline::NoProxy
pub fn leyline::NoProxy::eq(&self, other: &leyline::NoProxy) -> bool
impl core::default::Default for leyline::NoProxy
pub fn leyline::NoProxy::default() -> leyline::NoProxy
impl core::fmt::Debug for leyline::NoProxy
pub fn leyline::NoProxy::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::marker::StructuralPartialEq for leyline::NoProxy
impl core::marker::Freeze for leyline::NoProxy
impl core::marker::Send for leyline::NoProxy
impl core::marker::Sync for leyline::NoProxy
impl core::marker::Unpin for leyline::NoProxy
impl core::marker::UnsafeUnpin for leyline::NoProxy
impl core::panic::unwind_safe::RefUnwindSafe for leyline::NoProxy
impl core::panic::unwind_safe::UnwindSafe for leyline::NoProxy
```

### `Platform`

```rust,ignore
impl leyline::Platform
pub fn leyline::Platform::detect_host() -> Self
pub fn leyline::Platform::identity_key(&self) -> &'static str
pub fn leyline::Platform::mobile_flag(&self) -> &'static str
pub fn leyline::Platform::resolve(self) -> Self
pub fn leyline::Platform::sec_ch_platform(&self) -> &'static str
pub fn leyline::Platform::tcp_profile(&self) -> leyline::TcpProfile
impl core::clone::Clone for leyline::Platform
pub fn leyline::Platform::clone(&self) -> leyline::Platform
impl core::cmp::Eq for leyline::Platform
impl core::cmp::PartialEq for leyline::Platform
pub fn leyline::Platform::eq(&self, other: &leyline::Platform) -> bool
impl core::default::Default for leyline::Platform
pub fn leyline::Platform::default() -> leyline::Platform
impl core::fmt::Debug for leyline::Platform
pub fn leyline::Platform::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::fmt::Display for leyline::Platform
pub fn leyline::Platform::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::hash::Hash for leyline::Platform
pub fn leyline::Platform::hash<__H: core::hash::Hasher>(&self, state: &mut __H)
impl core::marker::Copy for leyline::Platform
impl core::marker::StructuralPartialEq for leyline::Platform
impl core::marker::Freeze for leyline::Platform
impl core::marker::Send for leyline::Platform
impl core::marker::Sync for leyline::Platform
impl core::marker::Unpin for leyline::Platform
impl core::marker::UnsafeUnpin for leyline::Platform
impl core::panic::unwind_safe::RefUnwindSafe for leyline::Platform
impl core::panic::unwind_safe::UnwindSafe for leyline::Platform
#[non_exhaustive] pub enum leyline::Platform
pub leyline::Platform::Android
pub leyline::Platform::Host
pub leyline::Platform::IOS
pub leyline::Platform::Linux
pub leyline::Platform::MacOS
pub leyline::Platform::Windows
impl leyline::Platform
pub fn leyline::Platform::detect_host() -> Self
pub fn leyline::Platform::identity_key(&self) -> &'static str
pub fn leyline::Platform::mobile_flag(&self) -> &'static str
pub fn leyline::Platform::resolve(self) -> Self
pub fn leyline::Platform::sec_ch_platform(&self) -> &'static str
pub fn leyline::Platform::tcp_profile(&self) -> leyline::TcpProfile
impl core::clone::Clone for leyline::Platform
pub fn leyline::Platform::clone(&self) -> leyline::Platform
impl core::cmp::Eq for leyline::Platform
impl core::cmp::PartialEq for leyline::Platform
pub fn leyline::Platform::eq(&self, other: &leyline::Platform) -> bool
impl core::default::Default for leyline::Platform
pub fn leyline::Platform::default() -> leyline::Platform
impl core::fmt::Debug for leyline::Platform
pub fn leyline::Platform::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::fmt::Display for leyline::Platform
pub fn leyline::Platform::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::hash::Hash for leyline::Platform
pub fn leyline::Platform::hash<__H: core::hash::Hasher>(&self, state: &mut __H)
impl core::marker::Copy for leyline::Platform
impl core::marker::StructuralPartialEq for leyline::Platform
impl core::marker::Freeze for leyline::Platform
impl core::marker::Send for leyline::Platform
impl core::marker::Sync for leyline::Platform
impl core::marker::Unpin for leyline::Platform
impl core::marker::UnsafeUnpin for leyline::Platform
impl core::panic::unwind_safe::RefUnwindSafe for leyline::Platform
impl core::panic::unwind_safe::UnwindSafe for leyline::Platform
```

### `PoolConfig`

```rust,ignore
#[non_exhaustive] pub struct leyline::PoolConfig
pub leyline::PoolConfig::h2_ping_after_idle: core::option::Option<core::time::Duration>
pub leyline::PoolConfig::h2_ping_timeout: core::time::Duration
pub leyline::PoolConfig::idle_timeout: core::time::Duration
pub leyline::PoolConfig::keepalive: bool
pub leyline::PoolConfig::max_connections: usize
pub leyline::PoolConfig::max_h1_conns_per_host: usize
impl leyline::PoolConfig
pub fn leyline::PoolConfig::h2_ping_after_idle(self, d: impl core::convert::Into<core::option::Option<core::time::Duration>>) -> Self
pub fn leyline::PoolConfig::h2_ping_timeout(self, d: core::time::Duration) -> Self
pub fn leyline::PoolConfig::idle_timeout(self, d: core::time::Duration) -> Self
pub fn leyline::PoolConfig::keepalive(self, on: bool) -> Self
pub fn leyline::PoolConfig::max_connections(self, n: usize) -> Self
pub fn leyline::PoolConfig::max_h1_conns_per_host(self, n: usize) -> Self
pub fn leyline::PoolConfig::new() -> Self
impl core::clone::Clone for leyline::PoolConfig
pub fn leyline::PoolConfig::clone(&self) -> leyline::PoolConfig
impl core::cmp::Eq for leyline::PoolConfig
impl core::cmp::PartialEq for leyline::PoolConfig
pub fn leyline::PoolConfig::eq(&self, other: &leyline::PoolConfig) -> bool
impl core::default::Default for leyline::PoolConfig
pub fn leyline::PoolConfig::default() -> Self
impl core::fmt::Debug for leyline::PoolConfig
pub fn leyline::PoolConfig::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::marker::Copy for leyline::PoolConfig
impl core::marker::StructuralPartialEq for leyline::PoolConfig
impl core::marker::Freeze for leyline::PoolConfig
impl core::marker::Send for leyline::PoolConfig
impl core::marker::Sync for leyline::PoolConfig
impl core::marker::Unpin for leyline::PoolConfig
impl core::marker::UnsafeUnpin for leyline::PoolConfig
impl core::panic::unwind_safe::RefUnwindSafe for leyline::PoolConfig
impl core::panic::unwind_safe::UnwindSafe for leyline::PoolConfig
```

### `PoolStats`

```rust,ignore
#[non_exhaustive] pub struct leyline::PoolStats
pub leyline::PoolStats::entries: usize
pub leyline::PoolStats::evictions_dead: u64
pub leyline::PoolStats::evictions_idle: u64
pub leyline::PoolStats::evictions_lru: u64
pub leyline::PoolStats::h1_hits: u64
pub leyline::PoolStats::h1_misses: u64
pub leyline::PoolStats::h2_hits: u64
pub leyline::PoolStats::h2_misses: u64
pub leyline::PoolStats::h2_ping_failures: u64
pub leyline::PoolStats::h3_hits: u64
pub leyline::PoolStats::h3_misses: u64
pub leyline::PoolStats::installs: u64
pub leyline::PoolStats::max_connections: usize
pub leyline::PoolStats::stale_probed: u64
impl core::clone::Clone for leyline::PoolStats
pub fn leyline::PoolStats::clone(&self) -> leyline::PoolStats
impl core::cmp::Eq for leyline::PoolStats
impl core::cmp::PartialEq for leyline::PoolStats
pub fn leyline::PoolStats::eq(&self, other: &leyline::PoolStats) -> bool
impl core::default::Default for leyline::PoolStats
pub fn leyline::PoolStats::default() -> leyline::PoolStats
impl core::fmt::Debug for leyline::PoolStats
pub fn leyline::PoolStats::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::marker::Copy for leyline::PoolStats
impl core::marker::StructuralPartialEq for leyline::PoolStats
impl core::marker::Freeze for leyline::PoolStats
impl core::marker::Send for leyline::PoolStats
impl core::marker::Sync for leyline::PoolStats
impl core::marker::Unpin for leyline::PoolStats
impl core::marker::UnsafeUnpin for leyline::PoolStats
impl core::panic::unwind_safe::RefUnwindSafe for leyline::PoolStats
impl core::panic::unwind_safe::UnwindSafe for leyline::PoolStats
```

### `Preset`

```rust,ignore
#[non_exhaustive] pub enum leyline::Preset
pub leyline::Preset::CrossOrigin
pub leyline::Preset::Form
pub leyline::Preset::FormNavigate
pub leyline::Preset::Native
pub leyline::Preset::Navigate
pub leyline::Preset::SameSite
pub leyline::Preset::Script
pub leyline::Preset::Xhr
```

### `ProtocolPolicy`

```rust,ignore
#[non_exhaustive] pub enum leyline::ProtocolPolicy
pub leyline::ProtocolPolicy::Auto
pub leyline::ProtocolPolicy::Http1
pub leyline::ProtocolPolicy::Http2
pub leyline::ProtocolPolicy::Http3
pub leyline::ProtocolPolicy::Race
impl core::clone::Clone for leyline::ProtocolPolicy
pub fn leyline::ProtocolPolicy::clone(&self) -> leyline::ProtocolPolicy
impl core::cmp::Eq for leyline::ProtocolPolicy
impl core::cmp::PartialEq for leyline::ProtocolPolicy
pub fn leyline::ProtocolPolicy::eq(&self, other: &leyline::ProtocolPolicy) -> bool
impl core::fmt::Debug for leyline::ProtocolPolicy
pub fn leyline::ProtocolPolicy::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::marker::Copy for leyline::ProtocolPolicy
impl core::marker::StructuralPartialEq for leyline::ProtocolPolicy
impl core::marker::Freeze for leyline::ProtocolPolicy
impl core::marker::Send for leyline::ProtocolPolicy
impl core::marker::Sync for leyline::ProtocolPolicy
impl core::marker::Unpin for leyline::ProtocolPolicy
impl core::marker::UnsafeUnpin for leyline::ProtocolPolicy
impl core::panic::unwind_safe::RefUnwindSafe for leyline::ProtocolPolicy
impl core::panic::unwind_safe::UnwindSafe for leyline::ProtocolPolicy
```

### `ProxyConfig`

```rust,ignore
pub struct leyline::ProxyConfig
impl leyline::ProxyConfig
pub fn leyline::ProxyConfig::all(self, proxy_url: impl core::convert::Into<alloc::string::String>) -> Self
pub fn leyline::ProxyConfig::new() -> Self
pub fn leyline::ProxyConfig::no_proxy(self, no_proxy: leyline::NoProxy) -> Self
pub fn leyline::ProxyConfig::uses_env(&self) -> bool
pub fn leyline::ProxyConfig::with_rule(self, rule: leyline::ProxyRule) -> Self
pub fn leyline::ProxyConfig::without_env(self) -> Self
impl core::clone::Clone for leyline::ProxyConfig
pub fn leyline::ProxyConfig::clone(&self) -> leyline::ProxyConfig
impl core::default::Default for leyline::ProxyConfig
pub fn leyline::ProxyConfig::default() -> Self
impl core::fmt::Debug for leyline::ProxyConfig
pub fn leyline::ProxyConfig::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::marker::Freeze for leyline::ProxyConfig
impl core::marker::Send for leyline::ProxyConfig
impl core::marker::Sync for leyline::ProxyConfig
impl core::marker::Unpin for leyline::ProxyConfig
impl core::marker::UnsafeUnpin for leyline::ProxyConfig
impl core::panic::unwind_safe::RefUnwindSafe for leyline::ProxyConfig
impl core::panic::unwind_safe::UnwindSafe for leyline::ProxyConfig
```

### `ProxyRule`

```rust,ignore
#[non_exhaustive] pub struct leyline::ProxyRule
impl leyline::ProxyRule
pub fn leyline::ProxyRule::all(proxy_url: impl core::convert::Into<alloc::string::String>) -> Self
pub fn leyline::ProxyRule::http(proxy_url: impl core::convert::Into<alloc::string::String>) -> Self
pub fn leyline::ProxyRule::https(proxy_url: impl core::convert::Into<alloc::string::String>) -> Self
pub fn leyline::ProxyRule::url(&self) -> &str
impl core::clone::Clone for leyline::ProxyRule
pub fn leyline::ProxyRule::clone(&self) -> leyline::ProxyRule
impl core::cmp::Eq for leyline::ProxyRule
impl core::cmp::PartialEq for leyline::ProxyRule
pub fn leyline::ProxyRule::eq(&self, other: &leyline::ProxyRule) -> bool
impl core::fmt::Debug for leyline::ProxyRule
pub fn leyline::ProxyRule::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::marker::StructuralPartialEq for leyline::ProxyRule
impl core::marker::Freeze for leyline::ProxyRule
impl core::marker::Send for leyline::ProxyRule
impl core::marker::Sync for leyline::ProxyRule
impl core::marker::Unpin for leyline::ProxyRule
impl core::marker::UnsafeUnpin for leyline::ProxyRule
impl core::panic::unwind_safe::RefUnwindSafe for leyline::ProxyRule
impl core::panic::unwind_safe::UnwindSafe for leyline::ProxyRule
```

### `ProxyUrl`

```rust,ignore
pub struct leyline::ProxyUrl(_)
impl leyline::ProxyUrl
pub fn leyline::ProxyUrl::as_str(&self) -> &str
pub fn leyline::ProxyUrl::http(raw: impl core::convert::AsRef<str>) -> leyline::Result<Self>
pub fn leyline::ProxyUrl::https(raw: impl core::convert::AsRef<str>) -> leyline::Result<Self>
pub fn leyline::ProxyUrl::into_string(self) -> alloc::string::String
pub fn leyline::ProxyUrl::parse(raw: impl core::convert::AsRef<str>) -> leyline::Result<Self>
pub fn leyline::ProxyUrl::socks5(raw: impl core::convert::AsRef<str>) -> leyline::Result<Self>
pub fn leyline::ProxyUrl::socks5h(raw: impl core::convert::AsRef<str>) -> leyline::Result<Self>
impl core::clone::Clone for leyline::ProxyUrl
pub fn leyline::ProxyUrl::clone(&self) -> leyline::ProxyUrl
impl core::cmp::Eq for leyline::ProxyUrl
impl core::cmp::PartialEq for leyline::ProxyUrl
pub fn leyline::ProxyUrl::eq(&self, other: &leyline::ProxyUrl) -> bool
impl core::convert::From<leyline::ProxyUrl> for alloc::string::String
pub fn alloc::string::String::from(value: leyline::ProxyUrl) -> Self
impl core::fmt::Debug for leyline::ProxyUrl
pub fn leyline::ProxyUrl::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::fmt::Display for leyline::ProxyUrl
pub fn leyline::ProxyUrl::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::hash::Hash for leyline::ProxyUrl
pub fn leyline::ProxyUrl::hash<__H: core::hash::Hasher>(&self, state: &mut __H)
impl core::marker::StructuralPartialEq for leyline::ProxyUrl
impl core::marker::Freeze for leyline::ProxyUrl
impl core::marker::Send for leyline::ProxyUrl
impl core::marker::Sync for leyline::ProxyUrl
impl core::marker::Unpin for leyline::ProxyUrl
impl core::marker::UnsafeUnpin for leyline::ProxyUrl
impl core::panic::unwind_safe::RefUnwindSafe for leyline::ProxyUrl
impl core::panic::unwind_safe::UnwindSafe for leyline::ProxyUrl
```

### `RedirectAction`

```rust,ignore
#[non_exhaustive] pub enum leyline::RedirectAction
pub leyline::RedirectAction::Follow
pub leyline::RedirectAction::Stop
impl core::clone::Clone for leyline::RedirectAction
pub fn leyline::RedirectAction::clone(&self) -> leyline::RedirectAction
impl core::cmp::Eq for leyline::RedirectAction
impl core::cmp::PartialEq for leyline::RedirectAction
pub fn leyline::RedirectAction::eq(&self, other: &leyline::RedirectAction) -> bool
impl core::fmt::Debug for leyline::RedirectAction
pub fn leyline::RedirectAction::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::marker::Copy for leyline::RedirectAction
impl core::marker::StructuralPartialEq for leyline::RedirectAction
impl core::marker::Freeze for leyline::RedirectAction
impl core::marker::Send for leyline::RedirectAction
impl core::marker::Sync for leyline::RedirectAction
impl core::marker::Unpin for leyline::RedirectAction
impl core::marker::UnsafeUnpin for leyline::RedirectAction
impl core::panic::unwind_safe::RefUnwindSafe for leyline::RedirectAction
impl core::panic::unwind_safe::UnwindSafe for leyline::RedirectAction
```

### `RedirectAttempt`

```rust,ignore
#[non_exhaustive] pub struct leyline::RedirectAttempt<'a>
pub leyline::RedirectAttempt::location: core::option::Option<&'a str>
pub leyline::RedirectAttempt::previous: &'a [alloc::string::String]
pub leyline::RedirectAttempt::status: u16
pub leyline::RedirectAttempt::url: &'a http::uri::Uri
impl<'a> core::clone::Clone for leyline::RedirectAttempt<'a>
pub fn leyline::RedirectAttempt<'a>::clone(&self) -> leyline::RedirectAttempt<'a>
impl<'a> core::fmt::Debug for leyline::RedirectAttempt<'a>
pub fn leyline::RedirectAttempt<'a>::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl<'a> core::marker::Copy for leyline::RedirectAttempt<'a>
impl<'a> core::marker::Freeze for leyline::RedirectAttempt<'a>
impl<'a> core::marker::Send for leyline::RedirectAttempt<'a>
impl<'a> core::marker::Sync for leyline::RedirectAttempt<'a>
impl<'a> core::marker::Unpin for leyline::RedirectAttempt<'a>
impl<'a> core::marker::UnsafeUnpin for leyline::RedirectAttempt<'a>
impl<'a> core::panic::unwind_safe::RefUnwindSafe for leyline::RedirectAttempt<'a>
impl<'a> core::panic::unwind_safe::UnwindSafe for leyline::RedirectAttempt<'a>
```

### `RedirectPolicy`

```rust,ignore
pub struct leyline::RedirectPolicy
impl leyline::RedirectPolicy
pub fn leyline::RedirectPolicy::custom<F>(f: F) -> Self where F: core::ops::function::Fn(leyline::RedirectAttempt<'_>) -> leyline::RedirectAction + core::marker::Send + core::marker::Sync + 'static
pub fn leyline::RedirectPolicy::limited(max: usize) -> Self
pub fn leyline::RedirectPolicy::none() -> Self
impl core::clone::Clone for leyline::RedirectPolicy
pub fn leyline::RedirectPolicy::clone(&self) -> leyline::RedirectPolicy
impl core::default::Default for leyline::RedirectPolicy
pub fn leyline::RedirectPolicy::default() -> Self
impl core::fmt::Debug for leyline::RedirectPolicy
pub fn leyline::RedirectPolicy::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::marker::Freeze for leyline::RedirectPolicy
impl core::marker::Send for leyline::RedirectPolicy
impl core::marker::Sync for leyline::RedirectPolicy
impl core::marker::Unpin for leyline::RedirectPolicy
impl core::marker::UnsafeUnpin for leyline::RedirectPolicy
impl !core::panic::unwind_safe::RefUnwindSafe for leyline::RedirectPolicy
impl !core::panic::unwind_safe::UnwindSafe for leyline::RedirectPolicy
```

### `Request`

```rust,ignore
impl tower_service::Service<leyline::Request> for leyline::LeylineService
pub struct leyline::Request
impl leyline::Request
pub fn leyline::Request::allow_non_idempotent_retry(self, v: bool) -> Self
pub fn leyline::Request::body(self, body: impl core::convert::Into<leyline::Body>) -> Self
pub fn leyline::Request::digest_auth(self, auth: leyline::DigestAuth) -> Self
pub fn leyline::Request::header(self, name: impl core::convert::TryInto<http::header::name::HeaderName>, value: impl core::convert::TryInto<http::header::value::HeaderValue>) -> Self
pub fn leyline::Request::headers(&self) -> &leyline::HeaderList
pub fn leyline::Request::headers_mut(&mut self) -> &mut leyline::HeaderList
pub fn leyline::Request::method(&self) -> &http::method::Method
pub fn leyline::Request::new(method: http::method::Method, url: impl core::convert::TryInto<http::uri::Uri>) -> Self
pub fn leyline::Request::preset(self, preset: leyline::profile::preset::Preset) -> Self
pub fn leyline::Request::retry(self, policy: leyline::RetryPolicy) -> Self
pub fn leyline::Request::stream(self) -> Self
pub fn leyline::Request::timeout(self, timeout: core::time::Duration) -> Self
pub fn leyline::Request::timeouts(self, timeouts: leyline::TimeoutConfig) -> Self
pub fn leyline::Request::url(&self) -> &http::uri::Uri
pub fn leyline::Request::from(req: http::request::Request<leyline::Body>) -> Self
impl core::fmt::Debug for leyline::Request
pub fn leyline::Request::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl tower_service::Service<leyline::Request> for leyline::LeylineService
impl !core::marker::Freeze for leyline::Request
impl core::marker::Send for leyline::Request
impl !core::marker::Sync for leyline::Request
impl core::marker::Unpin for leyline::Request
impl core::marker::UnsafeUnpin for leyline::Request
impl !core::panic::unwind_safe::RefUnwindSafe for leyline::Request
impl !core::panic::unwind_safe::UnwindSafe for leyline::Request
```

### `RequestBuilder`

```rust,ignore
pub struct leyline::RequestBuilder
impl leyline::RequestBuilder
pub fn leyline::RequestBuilder::accept(self, value: &str) -> Self
pub fn leyline::RequestBuilder::accept_language(self, value: &str) -> Self
pub fn leyline::RequestBuilder::allow_non_idempotent_retry(self, allow: bool) -> Self
pub fn leyline::RequestBuilder::anchored(self, anchor: leyline::profile::anchor::HeaderAnchor, name: impl core::convert::TryInto<http::header::name::HeaderName>, value: impl core::convert::TryInto<http::header::value::HeaderValue>) -> Self
pub fn leyline::RequestBuilder::append_header(self, name: impl core::convert::TryInto<http::header::name::HeaderName>, value: impl core::convert::TryInto<http::header::value::HeaderValue>) -> Self
pub fn leyline::RequestBuilder::append_headers<I, P>(self, headers: I) -> Self where I: core::iter::traits::collect::IntoIterator<Item = P>, P: leyline::IntoParamPair
pub fn leyline::RequestBuilder::basic_auth(self, username: &str, password: &str) -> Self
pub fn leyline::RequestBuilder::bearer_auth(self, token: &str) -> Self
pub fn leyline::RequestBuilder::body(self, body: impl core::convert::Into<leyline::Body>) -> Self
pub fn leyline::RequestBuilder::compress(self, encoding: leyline::ContentEncoding) -> Self
pub fn leyline::RequestBuilder::content_type(self, value: &str) -> Self
pub fn leyline::RequestBuilder::digest_auth(self, auth: leyline::DigestAuth) -> Self
pub fn leyline::RequestBuilder::form<I, P>(self, params: I) -> Self where I: core::iter::traits::collect::IntoIterator<Item = P>, P: leyline::IntoParamPair
pub fn leyline::RequestBuilder::form_str(self, encoded: &str) -> Self
pub fn leyline::RequestBuilder::header(self, name: impl core::convert::TryInto<http::header::name::HeaderName>, value: impl core::convert::TryInto<http::header::value::HeaderValue>) -> Self
pub fn leyline::RequestBuilder::header_order(self, order: &[&str]) -> Self
pub fn leyline::RequestBuilder::headers<I, P>(self, headers: I) -> Self where I: core::iter::traits::collect::IntoIterator<Item = P>, P: leyline::IntoParamPair
pub fn leyline::RequestBuilder::json(self, value: &impl serde_core::ser::Serialize) -> Self
pub fn leyline::RequestBuilder::multipart(self, form: leyline::multipart::Form) -> Self
pub fn leyline::RequestBuilder::origin(self, value: &str) -> Self
pub fn leyline::RequestBuilder::preset(self, preset: leyline::profile::preset::Preset) -> Self
pub fn leyline::RequestBuilder::proxy(self, proxy_url: &str) -> Self
pub fn leyline::RequestBuilder::query<I, P>(self, params: I) -> Self where I: core::iter::traits::collect::IntoIterator<Item = P>, P: leyline::IntoParamPair
pub fn leyline::RequestBuilder::referer(self, value: &str) -> Self
pub fn leyline::RequestBuilder::retry(self, policy: leyline::RetryPolicy) -> Self
pub fn leyline::RequestBuilder::stream(self) -> Self
pub fn leyline::RequestBuilder::timeout(self, timeout: core::time::Duration) -> Self
pub fn leyline::RequestBuilder::timeouts(self, timeouts: leyline::TimeoutConfig) -> Self
pub fn leyline::RequestBuilder::user_agent(self, value: &str) -> Self
impl leyline::RequestBuilder
pub async fn leyline::RequestBuilder::send(self) -> leyline::Result<leyline::Response>
impl core::future::into_future::IntoFuture for leyline::RequestBuilder
pub type leyline::RequestBuilder::IntoFuture = core::pin::Pin<alloc::boxed::Box<(dyn core::future::future::Future<Output = <leyline::RequestBuilder as core::future::into_future::IntoFuture>::Output> + core::marker::Send)>>
pub type leyline::RequestBuilder::Output = core::result::Result<leyline::Response, leyline::Error>
pub fn leyline::RequestBuilder::into_future(self) -> Self::IntoFuture
impl !core::marker::Freeze for leyline::RequestBuilder
impl core::marker::Send for leyline::RequestBuilder
impl !core::marker::Sync for leyline::RequestBuilder
impl core::marker::Unpin for leyline::RequestBuilder
impl core::marker::UnsafeUnpin for leyline::RequestBuilder
impl !core::panic::unwind_safe::RefUnwindSafe for leyline::RequestBuilder
impl !core::panic::unwind_safe::UnwindSafe for leyline::RequestBuilder
```

### `Response`

```rust,ignore
pub struct leyline::Response
impl leyline::Response
pub fn leyline::Response::as_bytes(&self) -> core::option::Option<&[u8]>
pub fn leyline::Response::as_text(&self) -> core::option::Option<leyline::Result<&str>>
pub fn leyline::Response::audit(&self) -> core::option::Option<&leyline::audit::AuditData>
pub async fn leyline::Response::bytes(&mut self) -> leyline::Result<&[u8]>
pub fn leyline::Response::content_length(&self) -> core::option::Option<u64>
pub fn leyline::Response::content_type(&self) -> core::option::Option<&str>
pub fn leyline::Response::cookie(&self, name: &str) -> core::option::Option<&str>
pub fn leyline::Response::cookies(&self) -> impl core::iter::traits::iterator::Iterator<Item = (&str, &str)>
pub async fn leyline::Response::copy_to<W>(self, writer: &mut W) -> leyline::Result<u64> where W: tokio::io::async_write::AsyncWrite + core::marker::Unpin
pub async fn leyline::Response::download_to(self, path: impl core::convert::AsRef<std::path::Path>) -> leyline::Result<u64>
pub fn leyline::Response::error_for_status(self) -> leyline::Result<Self>
pub fn leyline::Response::header(&self, name: &str) -> core::option::Option<&str>
pub fn leyline::Response::header_all<'a>(&'a self, name: &'a str) -> impl core::iter::traits::iterator::Iterator<Item = &'a str> + 'a
pub fn leyline::Response::header_map(&self) -> http::header::map::HeaderMap
pub fn leyline::Response::headers(&self) -> impl core::iter::traits::iterator::Iterator<Item = (&http::header::name::HeaderName, &http::header::value::HeaderValue)>
pub async fn leyline::Response::into_bytes(self) -> leyline::Result<alloc::vec::Vec<u8>>
pub fn leyline::Response::into_stream(self) -> leyline::Result<leyline::BodyStream>
pub async fn leyline::Response::into_text(self) -> leyline::Result<alloc::string::String>
pub fn leyline::Response::is_client_error(&self) -> bool
pub fn leyline::Response::is_redirect(&self) -> bool
pub fn leyline::Response::is_server_error(&self) -> bool
pub fn leyline::Response::is_success(&self) -> bool
pub async fn leyline::Response::json<T: serde_core::de::DeserializeOwned>(&mut self) -> leyline::Result<T>
pub fn leyline::Response::redirect_chain(&self) -> &[alloc::string::String]
pub fn leyline::Response::request_headers(&self) -> impl core::iter::traits::iterator::Iterator<Item = (&str, &str)>
pub fn leyline::Response::status(&self) -> http::status::StatusCode
pub async fn leyline::Response::text(&mut self) -> leyline::Result<alloc::string::String>
pub async fn leyline::Response::text_utf8(&mut self) -> leyline::Result<&str>
pub async fn leyline::Response::text_with_charset(&mut self, default_encoding: &str) -> leyline::Result<alloc::string::String>
pub fn leyline::Response::timing(&self) -> &leyline::ResponseTiming
pub fn leyline::Response::tls_alpn(&self) -> core::option::Option<&str>
pub fn leyline::Response::tls_cipher(&self) -> core::option::Option<&str>
pub fn leyline::Response::tls_peer_certificate(&self) -> core::option::Option<&[u8]>
pub fn leyline::Response::tls_version(&self) -> core::option::Option<&str>
pub fn leyline::Response::trailers(&self) -> impl core::iter::traits::iterator::Iterator<Item = (&http::header::name::HeaderName, &http::header::value::HeaderValue)>
pub fn leyline::Response::url(&self) -> &str
pub fn leyline::Response::version(&self) -> leyline::HttpVersion
impl leyline::Response
pub async fn leyline::Response::read_until<F>(self, limit: usize, done: F) -> leyline::Result<alloc::vec::Vec<u8>> where F: core::ops::function::FnMut(&[u8], usize) -> bool
impl core::fmt::Debug for leyline::Response
pub fn leyline::Response::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl !core::marker::Freeze for leyline::Response
impl core::marker::Send for leyline::Response
impl core::marker::Sync for leyline::Response
impl core::marker::Unpin for leyline::Response
impl core::marker::UnsafeUnpin for leyline::Response
impl !core::panic::unwind_safe::RefUnwindSafe for leyline::Response
impl !core::panic::unwind_safe::UnwindSafe for leyline::Response
```

### `ResponseTiming`

```rust,ignore
#[non_exhaustive] pub struct leyline::ResponseTiming
pub leyline::ResponseTiming::connect_ms: core::option::Option<u32>
pub leyline::ResponseTiming::reused: bool
pub leyline::ResponseTiming::send_ms: u32
pub leyline::ResponseTiming::total_ms: u32
impl core::clone::Clone for leyline::ResponseTiming
pub fn leyline::ResponseTiming::clone(&self) -> leyline::ResponseTiming
impl core::cmp::Eq for leyline::ResponseTiming
impl core::cmp::PartialEq for leyline::ResponseTiming
pub fn leyline::ResponseTiming::eq(&self, other: &leyline::ResponseTiming) -> bool
impl core::default::Default for leyline::ResponseTiming
pub fn leyline::ResponseTiming::default() -> leyline::ResponseTiming
impl core::fmt::Debug for leyline::ResponseTiming
pub fn leyline::ResponseTiming::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::marker::Copy for leyline::ResponseTiming
impl core::marker::StructuralPartialEq for leyline::ResponseTiming
impl core::marker::Freeze for leyline::ResponseTiming
impl core::marker::Send for leyline::ResponseTiming
impl core::marker::Sync for leyline::ResponseTiming
impl core::marker::Unpin for leyline::ResponseTiming
impl core::marker::UnsafeUnpin for leyline::ResponseTiming
impl core::panic::unwind_safe::RefUnwindSafe for leyline::ResponseTiming
impl core::panic::unwind_safe::UnwindSafe for leyline::ResponseTiming
```

### `Result`

```rust,ignore
pub type leyline::Result<T> = core::result::Result<T, leyline::Error>
```

### `RetryPolicy`

```rust,ignore
#[non_exhaustive] pub struct leyline::RetryPolicy
pub leyline::RetryPolicy::backoff_factor: f64
pub leyline::RetryPolicy::initial_backoff: core::time::Duration
pub leyline::RetryPolicy::jitter: bool
pub leyline::RetryPolicy::max_backoff: core::time::Duration
pub leyline::RetryPolicy::max_retries: u32
pub leyline::RetryPolicy::max_retry_after: core::time::Duration
pub leyline::RetryPolicy::retry_on: alloc::vec::Vec<leyline::RetryTrigger>
impl leyline::RetryPolicy
pub fn leyline::RetryPolicy::backoff_factor(self, factor: f64) -> Self
pub fn leyline::RetryPolicy::jitter(self, on: bool) -> Self
pub fn leyline::RetryPolicy::none() -> Self
pub fn leyline::RetryPolicy::on(self, trigger: leyline::RetryTrigger) -> Self
pub fn leyline::RetryPolicy::on_status(self, code: u16) -> Self
pub fn leyline::RetryPolicy::retry_on(self, triggers: impl core::iter::traits::collect::IntoIterator<Item = leyline::RetryTrigger>) -> Self
pub fn leyline::RetryPolicy::transient() -> Self
pub fn leyline::RetryPolicy::with_backoff(self, initial: core::time::Duration, max: core::time::Duration) -> Self
pub fn leyline::RetryPolicy::with_max_retries(self, n: u32) -> Self
pub fn leyline::RetryPolicy::with_max_retry_after(self, max: core::time::Duration) -> Self
impl core::clone::Clone for leyline::RetryPolicy
pub fn leyline::RetryPolicy::clone(&self) -> leyline::RetryPolicy
impl core::default::Default for leyline::RetryPolicy
pub fn leyline::RetryPolicy::default() -> Self
impl core::fmt::Debug for leyline::RetryPolicy
pub fn leyline::RetryPolicy::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::marker::Freeze for leyline::RetryPolicy
impl core::marker::Send for leyline::RetryPolicy
impl core::marker::Sync for leyline::RetryPolicy
impl core::marker::Unpin for leyline::RetryPolicy
impl core::marker::UnsafeUnpin for leyline::RetryPolicy
impl core::panic::unwind_safe::RefUnwindSafe for leyline::RetryPolicy
impl core::panic::unwind_safe::UnwindSafe for leyline::RetryPolicy
```

### `RetryTrigger`

```rust,ignore
#[non_exhaustive] pub enum leyline::RetryTrigger
pub leyline::RetryTrigger::ConnectionError
pub leyline::RetryTrigger::ServerError
pub leyline::RetryTrigger::Status(u16)
pub leyline::RetryTrigger::Timeout
impl core::clone::Clone for leyline::RetryTrigger
pub fn leyline::RetryTrigger::clone(&self) -> leyline::RetryTrigger
impl core::fmt::Debug for leyline::RetryTrigger
pub fn leyline::RetryTrigger::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::marker::Copy for leyline::RetryTrigger
impl core::marker::Freeze for leyline::RetryTrigger
impl core::marker::Send for leyline::RetryTrigger
impl core::marker::Sync for leyline::RetryTrigger
impl core::marker::Unpin for leyline::RetryTrigger
impl core::marker::UnsafeUnpin for leyline::RetryTrigger
impl core::panic::unwind_safe::RefUnwindSafe for leyline::RetryTrigger
impl core::panic::unwind_safe::UnwindSafe for leyline::RetryTrigger
```

### `Session`

```rust,ignore
pub struct leyline::Session
impl leyline::Session
pub fn leyline::Session::brand(&self) -> core::option::Option<leyline::ChromiumBrand>
pub fn leyline::Session::brave() -> Self
pub fn leyline::Session::browser(&self) -> core::option::Option<leyline::Browser>
pub fn leyline::Session::builder() -> leyline::SessionBuilder
pub fn leyline::Session::chrome() -> Self
pub fn leyline::Session::cookies(&self) -> &leyline::cookie::Jar
pub fn leyline::Session::default_timeout(&self) -> core::time::Duration
pub fn leyline::Session::delete(&self, url: &str) -> leyline::RequestBuilder
pub fn leyline::Session::edge() -> Self
pub async fn leyline::Session::execute(&self, req: leyline::Request) -> leyline::Result<leyline::Response>
pub fn leyline::Session::firefox() -> Self
pub fn leyline::Session::get(&self, url: &str) -> leyline::RequestBuilder
pub fn leyline::Session::head(&self, url: &str) -> leyline::RequestBuilder
pub fn leyline::Session::identity(&self) -> core::option::Option<leyline::Identity>
pub fn leyline::Session::new() -> Self
pub fn leyline::Session::opera() -> Self
pub fn leyline::Session::patch(&self, url: &str) -> leyline::RequestBuilder
pub fn leyline::Session::platform(&self) -> leyline::Platform
pub fn leyline::Session::pool_stats(&self) -> leyline::PoolStats
pub fn leyline::Session::post(&self, url: &str) -> leyline::RequestBuilder
pub async fn leyline::Session::preconnect(&self, url: &str) -> leyline::Result<()>
pub async fn leyline::Session::preconnect_via(&self, url: &str, proxy: core::option::Option<&str>) -> leyline::Result<()>
pub fn leyline::Session::profile(browser: leyline::Browser, platform: leyline::Platform) -> leyline::Result<Self>
pub fn leyline::Session::protocol_policy(&self) -> leyline::ProtocolPolicy
pub fn leyline::Session::put(&self, url: &str) -> leyline::RequestBuilder
pub fn leyline::Session::request(&self, method: http::method::Method, url: impl core::convert::TryInto<http::uri::Uri>) -> leyline::RequestBuilder
pub fn leyline::Session::response_header_timeout(&self) -> core::option::Option<core::time::Duration>
pub fn leyline::Session::safari() -> Self
pub fn leyline::Session::vivaldi() -> Self
pub fn leyline::Session::with_cookie_jar(&self, cookie_jar: leyline::cookie::Jar) -> Self
pub fn leyline::Session::with_proxy(&self, proxy_url: &str) -> leyline::Result<Self>
pub fn leyline::Session::with_redirect_policy(&self, policy: leyline::RedirectPolicy) -> Self
impl leyline::Session
pub fn leyline::Session::websocket(&self, url: &str) -> leyline::WebSocketBuilder
impl core::clone::Clone for leyline::Session
pub fn leyline::Session::clone(&self) -> leyline::Session
impl core::default::Default for leyline::Session
pub fn leyline::Session::default() -> Self
impl core::fmt::Debug for leyline::Session
pub fn leyline::Session::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::fmt::Display for leyline::Session
pub fn leyline::Session::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::marker::Freeze for leyline::Session
impl core::marker::Send for leyline::Session
impl core::marker::Sync for leyline::Session
impl core::marker::Unpin for leyline::Session
impl core::marker::UnsafeUnpin for leyline::Session
impl !core::panic::unwind_safe::RefUnwindSafe for leyline::Session
impl !core::panic::unwind_safe::UnwindSafe for leyline::Session
```

### `SessionBuilder`

```rust,ignore
pub struct leyline::SessionBuilder
impl leyline::SessionBuilder
pub fn leyline::SessionBuilder::accept_language(self, lang: impl core::convert::Into<alloc::string::String>) -> Self
pub fn leyline::SessionBuilder::add_pinned_leaf_sha256(self, sha256: [u8; 32]) -> Self
pub fn leyline::SessionBuilder::add_root_certificate_der(self, der: impl core::convert::Into<alloc::vec::Vec<u8>>) -> Self
pub fn leyline::SessionBuilder::add_root_certificate_file(self, path: impl core::convert::Into<std::path::PathBuf>) -> Self
pub fn leyline::SessionBuilder::android(self) -> Self
pub fn leyline::SessionBuilder::audit(self, enabled: bool) -> Self
pub fn leyline::SessionBuilder::brand(self, brand: leyline::ChromiumBrand) -> Self
pub fn leyline::SessionBuilder::brave(self) -> Self
pub fn leyline::SessionBuilder::browser(self, browser: leyline::Browser) -> Self
pub fn leyline::SessionBuilder::build(self) -> leyline::Result<leyline::Session>
pub fn leyline::SessionBuilder::chrome(self) -> Self
pub fn leyline::SessionBuilder::client_identity_files(self, certificate_chain_file: impl core::convert::Into<std::path::PathBuf>, private_key_file: impl core::convert::Into<std::path::PathBuf>) -> Self
pub fn leyline::SessionBuilder::compression(self, config: leyline::CompressionConfig) -> Self
pub fn leyline::SessionBuilder::connect_timeout(self, timeout: core::time::Duration) -> Self
pub fn leyline::SessionBuilder::cookie_jar(self, jar: leyline::cookie::Jar) -> Self
pub fn leyline::SessionBuilder::danger_accept_invalid_certs(self, accept: bool) -> Self
pub fn leyline::SessionBuilder::disable_env_proxies(self) -> Self
pub fn leyline::SessionBuilder::dns(self, config: leyline::DnsConfig) -> Self
pub fn leyline::SessionBuilder::edge(self) -> Self
pub fn leyline::SessionBuilder::extra_headers<I, P>(self, headers: I) -> Self where I: core::iter::traits::collect::IntoIterator<Item = P>, P: leyline::IntoParamPair
pub fn leyline::SessionBuilder::firefox(self) -> Self
pub fn leyline::SessionBuilder::happy_eyeballs(self, config: leyline::tls::HappyEyeballsConfig) -> Self
pub fn leyline::SessionBuilder::http1(self) -> Self
pub fn leyline::SessionBuilder::http2(self) -> Self
pub fn leyline::SessionBuilder::http3(self) -> Self
pub fn leyline::SessionBuilder::http_identity(self, browser: leyline::Browser) -> Self
pub fn leyline::SessionBuilder::https_only(self, enabled: bool) -> Self
pub fn leyline::SessionBuilder::identity(self, id: leyline::Identity) -> Self
pub fn leyline::SessionBuilder::ios(self) -> Self
pub fn leyline::SessionBuilder::layer<L>(self, layer: L) -> Self where L: tower_layer::Layer<leyline::layer::Transport>, <L as tower_layer::Layer>::Service: tower_service::Service<leyline::layer::Call, Response = leyline::layer::Reply, Error = leyline::Error> + core::clone::Clone + core::marker::Send + core::marker::Sync + 'static, <<L as tower_layer::Layer>::Service as tower_service::Service<leyline::layer::Call>>::Future: core::marker::Send + 'static
pub fn leyline::SessionBuilder::linux(self) -> Self
pub fn leyline::SessionBuilder::macos(self) -> Self
pub fn leyline::SessionBuilder::max_redirects(self, n: usize) -> Self
pub fn leyline::SessionBuilder::no_proxy(self, no_proxy: leyline::NoProxy) -> Self
pub fn leyline::SessionBuilder::opera(self) -> Self
pub fn leyline::SessionBuilder::platform(self, platform: leyline::Platform) -> Self
pub fn leyline::SessionBuilder::pool_config(self, config: leyline::PoolConfig) -> Self
pub fn leyline::SessionBuilder::profile(self, browser: leyline::Browser, platform: leyline::Platform) -> Self
pub fn leyline::SessionBuilder::protocol_policy(self, policy: leyline::ProtocolPolicy) -> Self
pub fn leyline::SessionBuilder::proxies(self, config: leyline::ProxyConfig) -> Self
pub fn leyline::SessionBuilder::proxy(self, proxy: impl core::convert::Into<alloc::string::String>) -> Self
pub fn leyline::SessionBuilder::race(self) -> Self
pub fn leyline::SessionBuilder::redirect_policy(self, policy: leyline::RedirectPolicy) -> Self
pub fn leyline::SessionBuilder::resolve_host(self, host: impl core::convert::AsRef<str>, addr: core::net::socket_addr::SocketAddr) -> Self
pub fn leyline::SessionBuilder::resolve_host_to_addrs<I>(self, host: impl core::convert::AsRef<str>, addrs: I) -> Self where I: core::iter::traits::collect::IntoIterator<Item = core::net::socket_addr::SocketAddr>
pub fn leyline::SessionBuilder::resolver(self, resolver: alloc::sync::Arc<dyn leyline::tls::Resolver>) -> Self
pub fn leyline::SessionBuilder::retry(self, policy: leyline::RetryPolicy) -> Self
pub fn leyline::SessionBuilder::safari(self) -> Self
pub fn leyline::SessionBuilder::socket_config(self, config: leyline::SocketConfig) -> Self
pub fn leyline::SessionBuilder::tcp_profile(self, profile: leyline::TcpProfile) -> Self
pub fn leyline::SessionBuilder::timeout(self, timeout: core::time::Duration) -> Self
pub fn leyline::SessionBuilder::timeouts(self, config: leyline::TimeoutConfig) -> Self
pub fn leyline::SessionBuilder::tls_trust(self, trust: leyline::TlsTrustConfig) -> Self
pub fn leyline::SessionBuilder::trace(self, hook: impl leyline::trace::Trace) -> Self
pub fn leyline::SessionBuilder::vivaldi(self) -> Self
pub fn leyline::SessionBuilder::websocket_config(self, config: leyline::WebSocketConfig) -> Self
pub fn leyline::SessionBuilder::windows(self) -> Self
pub fn leyline::SessionBuilder::without_env_roots(self) -> Self
pub fn leyline::SessionBuilder::without_system_roots(self) -> Self
impl core::marker::Freeze for leyline::SessionBuilder
impl core::marker::Send for leyline::SessionBuilder
impl core::marker::Sync for leyline::SessionBuilder
impl core::marker::Unpin for leyline::SessionBuilder
impl core::marker::UnsafeUnpin for leyline::SessionBuilder
impl !core::panic::unwind_safe::RefUnwindSafe for leyline::SessionBuilder
impl !core::panic::unwind_safe::UnwindSafe for leyline::SessionBuilder
```

### `SocketConfig`

```rust,ignore
#[non_exhaustive] pub struct leyline::SocketConfig
pub leyline::SocketConfig::interface: core::option::Option<alloc::string::String>
pub leyline::SocketConfig::local_address: core::option::Option<core::net::ip_addr::IpAddr>
pub leyline::SocketConfig::local_ipv4: core::option::Option<core::net::ip_addr::Ipv4Addr>
pub leyline::SocketConfig::local_ipv6: core::option::Option<core::net::ip_addr::Ipv6Addr>
pub leyline::SocketConfig::recv_buffer_size: core::option::Option<usize>
pub leyline::SocketConfig::send_buffer_size: core::option::Option<usize>
pub leyline::SocketConfig::strict: bool
pub leyline::SocketConfig::tcp_keepalive: core::option::Option<core::time::Duration>
pub leyline::SocketConfig::tcp_keepalive_interval: core::option::Option<core::time::Duration>
pub leyline::SocketConfig::tcp_keepalive_retries: core::option::Option<u32>
pub leyline::SocketConfig::tcp_nodelay: core::option::Option<bool>
pub leyline::SocketConfig::tcp_user_timeout: core::option::Option<core::time::Duration>
impl leyline::SocketConfig
pub fn leyline::SocketConfig::interface(self, name: impl core::convert::Into<alloc::string::String>) -> Self
pub fn leyline::SocketConfig::local_address(self, addr: impl core::convert::Into<core::option::Option<core::net::ip_addr::IpAddr>>) -> Self
pub fn leyline::SocketConfig::local_ipv4(self, addr: impl core::convert::Into<core::option::Option<core::net::ip_addr::Ipv4Addr>>) -> Self
pub fn leyline::SocketConfig::local_ipv6(self, addr: impl core::convert::Into<core::option::Option<core::net::ip_addr::Ipv6Addr>>) -> Self
pub fn leyline::SocketConfig::new() -> Self
pub fn leyline::SocketConfig::recv_buffer_size(self, n: impl core::convert::Into<core::option::Option<usize>>) -> Self
pub fn leyline::SocketConfig::send_buffer_size(self, n: impl core::convert::Into<core::option::Option<usize>>) -> Self
pub fn leyline::SocketConfig::strict(self, on: bool) -> Self
pub fn leyline::SocketConfig::tcp_keepalive(self, d: impl core::convert::Into<core::option::Option<core::time::Duration>>) -> Self
pub fn leyline::SocketConfig::tcp_keepalive_interval(self, d: impl core::convert::Into<core::option::Option<core::time::Duration>>) -> Self
pub fn leyline::SocketConfig::tcp_keepalive_retries(self, n: impl core::convert::Into<core::option::Option<u32>>) -> Self
pub fn leyline::SocketConfig::tcp_nodelay(self, on: impl core::convert::Into<core::option::Option<bool>>) -> Self
pub fn leyline::SocketConfig::tcp_user_timeout(self, d: impl core::convert::Into<core::option::Option<core::time::Duration>>) -> Self
impl core::clone::Clone for leyline::SocketConfig
pub fn leyline::SocketConfig::clone(&self) -> leyline::SocketConfig
impl core::cmp::Eq for leyline::SocketConfig
impl core::cmp::PartialEq for leyline::SocketConfig
pub fn leyline::SocketConfig::eq(&self, other: &leyline::SocketConfig) -> bool
impl core::default::Default for leyline::SocketConfig
pub fn leyline::SocketConfig::default() -> Self
impl core::fmt::Debug for leyline::SocketConfig
pub fn leyline::SocketConfig::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::marker::StructuralPartialEq for leyline::SocketConfig
impl core::marker::Freeze for leyline::SocketConfig
impl core::marker::Send for leyline::SocketConfig
impl core::marker::Sync for leyline::SocketConfig
impl core::marker::Unpin for leyline::SocketConfig
impl core::marker::UnsafeUnpin for leyline::SocketConfig
impl core::panic::unwind_safe::RefUnwindSafe for leyline::SocketConfig
impl core::panic::unwind_safe::UnwindSafe for leyline::SocketConfig
```

### `TcpProfile`

```rust,ignore
#[non_exhaustive] pub struct leyline::TcpProfile
pub leyline::TcpProfile::df: bool
pub leyline::TcpProfile::mss: u32
pub leyline::TcpProfile::no_delay: bool
pub leyline::TcpProfile::ttl: u32
pub leyline::TcpProfile::window_scale: u32
pub leyline::TcpProfile::window_size: u32
impl leyline::TcpProfile
pub const leyline::TcpProfile::IOS: Self
pub const leyline::TcpProfile::LINUX: Self
pub const leyline::TcpProfile::MACOS: Self
pub const leyline::TcpProfile::WINDOWS: Self
impl core::clone::Clone for leyline::TcpProfile
pub fn leyline::TcpProfile::clone(&self) -> leyline::TcpProfile
impl core::fmt::Debug for leyline::TcpProfile
pub fn leyline::TcpProfile::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::marker::Copy for leyline::TcpProfile
impl core::marker::Freeze for leyline::TcpProfile
impl core::marker::Send for leyline::TcpProfile
impl core::marker::Sync for leyline::TcpProfile
impl core::marker::Unpin for leyline::TcpProfile
impl core::marker::UnsafeUnpin for leyline::TcpProfile
impl core::panic::unwind_safe::RefUnwindSafe for leyline::TcpProfile
impl core::panic::unwind_safe::UnwindSafe for leyline::TcpProfile
```

### `TimeoutConfig`

```rust,ignore
#[non_exhaustive] pub struct leyline::TimeoutConfig
pub leyline::TimeoutConfig::connect: core::option::Option<core::time::Duration>
pub leyline::TimeoutConfig::read: core::option::Option<core::time::Duration>
pub leyline::TimeoutConfig::response_header: core::option::Option<core::time::Duration>
pub leyline::TimeoutConfig::total: core::time::Duration
impl leyline::TimeoutConfig
pub fn leyline::TimeoutConfig::connect(self, d: impl core::convert::Into<core::option::Option<core::time::Duration>>) -> Self
pub fn leyline::TimeoutConfig::new() -> Self
pub fn leyline::TimeoutConfig::read(self, d: impl core::convert::Into<core::option::Option<core::time::Duration>>) -> Self
pub fn leyline::TimeoutConfig::response_header(self, d: impl core::convert::Into<core::option::Option<core::time::Duration>>) -> Self
pub fn leyline::TimeoutConfig::total(self, d: core::time::Duration) -> Self
impl core::clone::Clone for leyline::TimeoutConfig
pub fn leyline::TimeoutConfig::clone(&self) -> leyline::TimeoutConfig
impl core::cmp::Eq for leyline::TimeoutConfig
impl core::cmp::PartialEq for leyline::TimeoutConfig
pub fn leyline::TimeoutConfig::eq(&self, other: &leyline::TimeoutConfig) -> bool
impl core::default::Default for leyline::TimeoutConfig
pub fn leyline::TimeoutConfig::default() -> Self
impl core::fmt::Debug for leyline::TimeoutConfig
pub fn leyline::TimeoutConfig::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::marker::Copy for leyline::TimeoutConfig
impl core::marker::StructuralPartialEq for leyline::TimeoutConfig
impl core::marker::Freeze for leyline::TimeoutConfig
impl core::marker::Send for leyline::TimeoutConfig
impl core::marker::Sync for leyline::TimeoutConfig
impl core::marker::Unpin for leyline::TimeoutConfig
impl core::marker::UnsafeUnpin for leyline::TimeoutConfig
impl core::panic::unwind_safe::RefUnwindSafe for leyline::TimeoutConfig
impl core::panic::unwind_safe::UnwindSafe for leyline::TimeoutConfig
```

### `TlsContext`

```rust,ignore
pub struct leyline::TlsContext(_)
```

### `TlsError`

```rust,ignore
impl leyline::TlsError
pub fn leyline::TlsError::is_retryable(&self) -> bool
impl core::convert::From<leyline::TlsError> for leyline::Error
impl core::error::Error for leyline::TlsError
pub fn leyline::TlsError::source(&self) -> core::option::Option<&(dyn core::error::Error + 'static)>
impl core::fmt::Debug for leyline::TlsError
pub fn leyline::TlsError::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::fmt::Display for leyline::TlsError
pub fn leyline::TlsError::fmt(&self, __formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::marker::Freeze for leyline::TlsError
impl core::marker::Send for leyline::TlsError
impl core::marker::Sync for leyline::TlsError
impl core::marker::Unpin for leyline::TlsError
impl core::marker::UnsafeUnpin for leyline::TlsError
impl !core::panic::unwind_safe::RefUnwindSafe for leyline::TlsError
impl !core::panic::unwind_safe::UnwindSafe for leyline::TlsError
#[non_exhaustive] pub enum leyline::TlsError
pub leyline::TlsError::Certificate(alloc::string::String)
pub leyline::TlsError::Dns(core::io::error::Error)
pub leyline::TlsError::Handshake(alloc::string::String)
pub leyline::TlsError::HandshakeIo(core::io::error::Error)
pub leyline::TlsError::Hostname(alloc::string::String)
pub leyline::TlsError::Pinning(alloc::string::String)
pub leyline::TlsError::Profile(alloc::string::String)
pub leyline::TlsError::SslConfig(alloc::string::String)
pub leyline::TlsError::SslConnect(alloc::string::String)
pub leyline::TlsError::TcpConnect(core::io::error::Error)
pub leyline::TlsError::TrustStore(alloc::string::String)
impl leyline::TlsError
pub fn leyline::TlsError::is_retryable(&self) -> bool
impl core::convert::From<leyline::TlsError> for leyline::Error
impl core::error::Error for leyline::TlsError
pub fn leyline::TlsError::source(&self) -> core::option::Option<&(dyn core::error::Error + 'static)>
impl core::fmt::Debug for leyline::TlsError
pub fn leyline::TlsError::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::fmt::Display for leyline::TlsError
pub fn leyline::TlsError::fmt(&self, __formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::marker::Freeze for leyline::TlsError
impl core::marker::Send for leyline::TlsError
impl core::marker::Sync for leyline::TlsError
impl core::marker::Unpin for leyline::TlsError
impl core::marker::UnsafeUnpin for leyline::TlsError
impl !core::panic::unwind_safe::RefUnwindSafe for leyline::TlsError
impl !core::panic::unwind_safe::UnwindSafe for leyline::TlsError
impl core::convert::From<leyline::TlsError> for leyline::Error
```

### `TlsMinVersion`

```rust,ignore
impl core::clone::Clone for leyline::TlsMinVersion
pub fn leyline::TlsMinVersion::clone(&self) -> leyline::TlsMinVersion
impl core::cmp::Eq for leyline::TlsMinVersion
impl core::cmp::Ord for leyline::TlsMinVersion
pub fn leyline::TlsMinVersion::cmp(&self, other: &leyline::TlsMinVersion) -> core::cmp::Ordering
impl core::cmp::PartialEq for leyline::TlsMinVersion
pub fn leyline::TlsMinVersion::eq(&self, other: &leyline::TlsMinVersion) -> bool
impl core::cmp::PartialOrd for leyline::TlsMinVersion
pub fn leyline::TlsMinVersion::partial_cmp(&self, other: &leyline::TlsMinVersion) -> core::option::Option<core::cmp::Ordering>
impl core::fmt::Debug for leyline::TlsMinVersion
pub fn leyline::TlsMinVersion::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::marker::Copy for leyline::TlsMinVersion
impl core::marker::StructuralPartialEq for leyline::TlsMinVersion
impl core::marker::Freeze for leyline::TlsMinVersion
impl core::marker::Send for leyline::TlsMinVersion
impl core::marker::Sync for leyline::TlsMinVersion
impl core::marker::Unpin for leyline::TlsMinVersion
impl core::marker::UnsafeUnpin for leyline::TlsMinVersion
impl core::panic::unwind_safe::RefUnwindSafe for leyline::TlsMinVersion
impl core::panic::unwind_safe::UnwindSafe for leyline::TlsMinVersion
#[non_exhaustive] pub enum leyline::TlsMinVersion
pub leyline::TlsMinVersion::Tls10
pub leyline::TlsMinVersion::Tls12
pub leyline::TlsMinVersion::Tls13
impl core::clone::Clone for leyline::TlsMinVersion
pub fn leyline::TlsMinVersion::clone(&self) -> leyline::TlsMinVersion
impl core::cmp::Eq for leyline::TlsMinVersion
impl core::cmp::Ord for leyline::TlsMinVersion
pub fn leyline::TlsMinVersion::cmp(&self, other: &leyline::TlsMinVersion) -> core::cmp::Ordering
impl core::cmp::PartialEq for leyline::TlsMinVersion
pub fn leyline::TlsMinVersion::eq(&self, other: &leyline::TlsMinVersion) -> bool
impl core::cmp::PartialOrd for leyline::TlsMinVersion
pub fn leyline::TlsMinVersion::partial_cmp(&self, other: &leyline::TlsMinVersion) -> core::option::Option<core::cmp::Ordering>
impl core::fmt::Debug for leyline::TlsMinVersion
pub fn leyline::TlsMinVersion::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::marker::Copy for leyline::TlsMinVersion
impl core::marker::StructuralPartialEq for leyline::TlsMinVersion
impl core::marker::Freeze for leyline::TlsMinVersion
impl core::marker::Send for leyline::TlsMinVersion
impl core::marker::Sync for leyline::TlsMinVersion
impl core::marker::Unpin for leyline::TlsMinVersion
impl core::marker::UnsafeUnpin for leyline::TlsMinVersion
impl core::panic::unwind_safe::RefUnwindSafe for leyline::TlsMinVersion
impl core::panic::unwind_safe::UnwindSafe for leyline::TlsMinVersion
```

### `TlsTrustConfig`

```rust,ignore
impl leyline::TlsTrustConfig
pub fn leyline::TlsTrustConfig::add_ca_der(self, der: impl core::convert::Into<alloc::vec::Vec<u8>>) -> Self
pub fn leyline::TlsTrustConfig::add_ca_file(self, path: impl core::convert::Into<std::path::PathBuf>) -> Self
pub fn leyline::TlsTrustConfig::add_pinned_leaf_sha256(self, sha256: [u8; 32]) -> Self
pub fn leyline::TlsTrustConfig::ca_der_count(&self) -> usize
pub fn leyline::TlsTrustConfig::ca_files(&self) -> &[std::path::PathBuf]
pub fn leyline::TlsTrustConfig::client_identity(&self) -> core::option::Option<&leyline::tls::ClientIdentity>
pub fn leyline::TlsTrustConfig::client_identity_files(self, certificate_chain_file: impl core::convert::Into<std::path::PathBuf>, private_key_file: impl core::convert::Into<std::path::PathBuf>) -> Self
pub fn leyline::TlsTrustConfig::new() -> Self
pub fn leyline::TlsTrustConfig::pinned_leaf_sha256(&self) -> &[[u8; 32]]
pub fn leyline::TlsTrustConfig::uses_env_roots(&self) -> bool
pub fn leyline::TlsTrustConfig::uses_system_roots(&self) -> bool
pub fn leyline::TlsTrustConfig::without_env_roots(self) -> Self
pub fn leyline::TlsTrustConfig::without_system_roots(self) -> Self
impl core::clone::Clone for leyline::TlsTrustConfig
pub fn leyline::TlsTrustConfig::clone(&self) -> leyline::TlsTrustConfig
impl core::default::Default for leyline::TlsTrustConfig
pub fn leyline::TlsTrustConfig::default() -> Self
impl core::fmt::Debug for leyline::TlsTrustConfig
pub fn leyline::TlsTrustConfig::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::marker::Freeze for leyline::TlsTrustConfig
impl core::marker::Send for leyline::TlsTrustConfig
impl core::marker::Sync for leyline::TlsTrustConfig
impl core::marker::Unpin for leyline::TlsTrustConfig
impl core::marker::UnsafeUnpin for leyline::TlsTrustConfig
impl core::panic::unwind_safe::RefUnwindSafe for leyline::TlsTrustConfig
impl core::panic::unwind_safe::UnwindSafe for leyline::TlsTrustConfig
pub struct leyline::TlsTrustConfig
impl leyline::TlsTrustConfig
pub fn leyline::TlsTrustConfig::add_ca_der(self, der: impl core::convert::Into<alloc::vec::Vec<u8>>) -> Self
pub fn leyline::TlsTrustConfig::add_ca_file(self, path: impl core::convert::Into<std::path::PathBuf>) -> Self
pub fn leyline::TlsTrustConfig::add_pinned_leaf_sha256(self, sha256: [u8; 32]) -> Self
pub fn leyline::TlsTrustConfig::ca_der_count(&self) -> usize
pub fn leyline::TlsTrustConfig::ca_files(&self) -> &[std::path::PathBuf]
pub fn leyline::TlsTrustConfig::client_identity(&self) -> core::option::Option<&leyline::tls::ClientIdentity>
pub fn leyline::TlsTrustConfig::client_identity_files(self, certificate_chain_file: impl core::convert::Into<std::path::PathBuf>, private_key_file: impl core::convert::Into<std::path::PathBuf>) -> Self
pub fn leyline::TlsTrustConfig::new() -> Self
pub fn leyline::TlsTrustConfig::pinned_leaf_sha256(&self) -> &[[u8; 32]]
pub fn leyline::TlsTrustConfig::uses_env_roots(&self) -> bool
pub fn leyline::TlsTrustConfig::uses_system_roots(&self) -> bool
pub fn leyline::TlsTrustConfig::without_env_roots(self) -> Self
pub fn leyline::TlsTrustConfig::without_system_roots(self) -> Self
impl core::clone::Clone for leyline::TlsTrustConfig
pub fn leyline::TlsTrustConfig::clone(&self) -> leyline::TlsTrustConfig
impl core::default::Default for leyline::TlsTrustConfig
pub fn leyline::TlsTrustConfig::default() -> Self
impl core::fmt::Debug for leyline::TlsTrustConfig
pub fn leyline::TlsTrustConfig::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::marker::Freeze for leyline::TlsTrustConfig
impl core::marker::Send for leyline::TlsTrustConfig
impl core::marker::Sync for leyline::TlsTrustConfig
impl core::marker::Unpin for leyline::TlsTrustConfig
impl core::marker::UnsafeUnpin for leyline::TlsTrustConfig
impl core::panic::unwind_safe::RefUnwindSafe for leyline::TlsTrustConfig
impl core::panic::unwind_safe::UnwindSafe for leyline::TlsTrustConfig
```

### `WebSocketBuilder`

```rust,ignore
pub struct leyline::WebSocketBuilder
impl leyline::WebSocketBuilder
pub fn leyline::WebSocketBuilder::config(self, config: leyline::WebSocketConfig) -> Self
pub async fn leyline::WebSocketBuilder::connect(self) -> leyline::Result<leyline::WsConnection>
pub fn leyline::WebSocketBuilder::header(self, name: &str, value: &str) -> Self
pub fn leyline::WebSocketBuilder::headers<I, P>(self, headers: I) -> Self where I: core::iter::traits::collect::IntoIterator<Item = P>, P: leyline::IntoParamPair
pub fn leyline::WebSocketBuilder::http1(self) -> Self
pub fn leyline::WebSocketBuilder::proxy(self, proxy_url: impl core::convert::Into<alloc::string::String>) -> Self
impl core::future::into_future::IntoFuture for leyline::WebSocketBuilder
pub type leyline::WebSocketBuilder::IntoFuture = core::pin::Pin<alloc::boxed::Box<(dyn core::future::future::Future<Output = <leyline::WebSocketBuilder as core::future::into_future::IntoFuture>::Output> + core::marker::Send)>>
pub type leyline::WebSocketBuilder::Output = core::result::Result<leyline::WsConnection, leyline::Error>
pub fn leyline::WebSocketBuilder::into_future(self) -> Self::IntoFuture
impl core::marker::Freeze for leyline::WebSocketBuilder
impl core::marker::Send for leyline::WebSocketBuilder
impl core::marker::Sync for leyline::WebSocketBuilder
impl core::marker::Unpin for leyline::WebSocketBuilder
impl core::marker::UnsafeUnpin for leyline::WebSocketBuilder
impl !core::panic::unwind_safe::RefUnwindSafe for leyline::WebSocketBuilder
impl !core::panic::unwind_safe::UnwindSafe for leyline::WebSocketBuilder
```

### `WebSocketConfig`

```rust,ignore
#[non_exhaustive] pub struct leyline::WebSocketConfig
pub leyline::WebSocketConfig::accept_unmasked_frames: bool
pub leyline::WebSocketConfig::max_frame_size: core::option::Option<usize>
pub leyline::WebSocketConfig::max_message_size: core::option::Option<usize>
pub leyline::WebSocketConfig::max_write_buffer_size: core::option::Option<usize>
pub leyline::WebSocketConfig::prefer_http2: bool
pub leyline::WebSocketConfig::read_buffer_size: core::option::Option<usize>
pub leyline::WebSocketConfig::write_buffer_size: core::option::Option<usize>
impl leyline::WebSocketConfig
pub fn leyline::WebSocketConfig::accept_unmasked_frames(self, on: bool) -> Self
pub fn leyline::WebSocketConfig::max_frame_size(self, n: impl core::convert::Into<core::option::Option<usize>>) -> Self
pub fn leyline::WebSocketConfig::max_message_size(self, n: impl core::convert::Into<core::option::Option<usize>>) -> Self
pub fn leyline::WebSocketConfig::max_write_buffer_size(self, n: impl core::convert::Into<core::option::Option<usize>>) -> Self
pub fn leyline::WebSocketConfig::new() -> Self
pub fn leyline::WebSocketConfig::prefer_http2(self, on: bool) -> Self
pub fn leyline::WebSocketConfig::read_buffer_size(self, n: impl core::convert::Into<core::option::Option<usize>>) -> Self
pub fn leyline::WebSocketConfig::write_buffer_size(self, n: impl core::convert::Into<core::option::Option<usize>>) -> Self
impl core::clone::Clone for leyline::WebSocketConfig
pub fn leyline::WebSocketConfig::clone(&self) -> leyline::WebSocketConfig
impl core::cmp::Eq for leyline::WebSocketConfig
impl core::cmp::PartialEq for leyline::WebSocketConfig
pub fn leyline::WebSocketConfig::eq(&self, other: &leyline::WebSocketConfig) -> bool
impl core::default::Default for leyline::WebSocketConfig
pub fn leyline::WebSocketConfig::default() -> Self
impl core::fmt::Debug for leyline::WebSocketConfig
pub fn leyline::WebSocketConfig::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::marker::Copy for leyline::WebSocketConfig
impl core::marker::StructuralPartialEq for leyline::WebSocketConfig
impl core::marker::Freeze for leyline::WebSocketConfig
impl core::marker::Send for leyline::WebSocketConfig
impl core::marker::Sync for leyline::WebSocketConfig
impl core::marker::Unpin for leyline::WebSocketConfig
impl core::marker::UnsafeUnpin for leyline::WebSocketConfig
impl core::panic::unwind_safe::RefUnwindSafe for leyline::WebSocketConfig
impl core::panic::unwind_safe::UnwindSafe for leyline::WebSocketConfig
```

### `WsConnection`

```rust,ignore
pub struct leyline::WsConnection
impl leyline::WsConnection
pub async fn leyline::WsConnection::close(&mut self) -> leyline::Result<()>
pub fn leyline::WsConnection::header(&self, name: &str) -> core::option::Option<&str>
pub fn leyline::WsConnection::is_http2(&self) -> bool
pub fn leyline::WsConnection::protocol(&self) -> core::option::Option<&str>
pub async fn leyline::WsConnection::recv(&mut self) -> leyline::Result<core::option::Option<leyline::WsMessage>>
pub async fn leyline::WsConnection::send(&mut self, msg: &str) -> leyline::Result<()>
pub async fn leyline::WsConnection::send_binary(&mut self, data: alloc::vec::Vec<u8>) -> leyline::Result<()>
pub async fn leyline::WsConnection::send_raw(&mut self, msg: leyline::WsMessage) -> leyline::Result<()>
pub fn leyline::WsConnection::split(self) -> (leyline::WsSink, leyline::WsStream)
impl !core::marker::Freeze for leyline::WsConnection
impl core::marker::Send for leyline::WsConnection
impl core::marker::Sync for leyline::WsConnection
impl core::marker::Unpin for leyline::WsConnection
impl core::marker::UnsafeUnpin for leyline::WsConnection
impl !core::panic::unwind_safe::RefUnwindSafe for leyline::WsConnection
impl !core::panic::unwind_safe::UnwindSafe for leyline::WsConnection
```

### `WsMessage`

```rust,ignore
#[non_exhaustive] pub enum leyline::WsMessage
pub leyline::WsMessage::Binary(alloc::vec::Vec<u8>)
pub leyline::WsMessage::Close(core::option::Option<leyline::CloseFrame>)
pub leyline::WsMessage::Ping
pub leyline::WsMessage::Pong
pub leyline::WsMessage::Text(alloc::string::String)
impl core::clone::Clone for leyline::WsMessage
pub fn leyline::WsMessage::clone(&self) -> leyline::WsMessage
impl core::cmp::Eq for leyline::WsMessage
impl core::cmp::PartialEq for leyline::WsMessage
pub fn leyline::WsMessage::eq(&self, other: &leyline::WsMessage) -> bool
impl core::fmt::Debug for leyline::WsMessage
pub fn leyline::WsMessage::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::marker::StructuralPartialEq for leyline::WsMessage
impl core::marker::Freeze for leyline::WsMessage
impl core::marker::Send for leyline::WsMessage
impl core::marker::Sync for leyline::WsMessage
impl core::marker::Unpin for leyline::WsMessage
impl core::marker::UnsafeUnpin for leyline::WsMessage
impl core::panic::unwind_safe::RefUnwindSafe for leyline::WsMessage
impl core::panic::unwind_safe::UnwindSafe for leyline::WsMessage
```

### `WsSink`

```rust,ignore
pub struct leyline::WsSink
impl leyline::WsSink
pub async fn leyline::WsSink::close(&mut self) -> leyline::Result<()>
pub async fn leyline::WsSink::send(&mut self, msg: &str) -> leyline::Result<()>
pub async fn leyline::WsSink::send_binary(&mut self, data: alloc::vec::Vec<u8>) -> leyline::Result<()>
pub async fn leyline::WsSink::send_raw(&mut self, msg: leyline::WsMessage) -> leyline::Result<()>
impl !core::marker::Freeze for leyline::WsSink
impl core::marker::Send for leyline::WsSink
impl core::marker::Sync for leyline::WsSink
impl core::marker::Unpin for leyline::WsSink
impl core::marker::UnsafeUnpin for leyline::WsSink
impl !core::panic::unwind_safe::RefUnwindSafe for leyline::WsSink
impl !core::panic::unwind_safe::UnwindSafe for leyline::WsSink
```

### `WsStream`

```rust,ignore
pub struct leyline::WsStream
impl leyline::WsStream
pub async fn leyline::WsStream::recv(&mut self) -> leyline::Result<core::option::Option<leyline::WsMessage>>
impl core::marker::Freeze for leyline::WsStream
impl core::marker::Send for leyline::WsStream
impl core::marker::Sync for leyline::WsStream
impl core::marker::Unpin for leyline::WsStream
impl core::marker::UnsafeUnpin for leyline::WsStream
impl !core::panic::unwind_safe::RefUnwindSafe for leyline::WsStream
impl !core::panic::unwind_safe::UnwindSafe for leyline::WsStream
```

### `http`

```rust,ignore
pub use leyline::http
```

## `leyline::audit`

```rust,ignore
pub mod leyline::audit
```

### `AuditData`

```rust,ignore
#[non_exhaustive] pub struct leyline::audit::AuditData
pub leyline::audit::AuditData::h2_fingerprint: alloc::string::String
pub leyline::audit::AuditData::ja3: alloc::string::String
pub leyline::audit::AuditData::ja4: alloc::string::String
pub leyline::audit::AuditData::ja4h: alloc::string::String
pub leyline::audit::AuditData::ja4t: alloc::string::String
impl core::clone::Clone for leyline::audit::AuditData
pub fn leyline::audit::AuditData::clone(&self) -> leyline::audit::AuditData
impl core::fmt::Debug for leyline::audit::AuditData
pub fn leyline::audit::AuditData::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::marker::Freeze for leyline::audit::AuditData
impl core::marker::Send for leyline::audit::AuditData
impl core::marker::Sync for leyline::audit::AuditData
impl core::marker::Unpin for leyline::audit::AuditData
impl core::marker::UnsafeUnpin for leyline::audit::AuditData
impl core::panic::unwind_safe::RefUnwindSafe for leyline::audit::AuditData
impl core::panic::unwind_safe::UnwindSafe for leyline::audit::AuditData
```

### `Ja3Input`

```rust,ignore
pub struct leyline::audit::Ja3Input<'a>
pub leyline::audit::Ja3Input::ciphers: &'a [alloc::string::String]
pub leyline::audit::Ja3Input::curves: &'a [alloc::string::String]
pub leyline::audit::Ja3Input::extension_ids: &'a [u16]
pub leyline::audit::Ja3Input::tls_record_version: u16
impl<'a> core::marker::Freeze for leyline::audit::Ja3Input<'a>
impl<'a> core::marker::Send for leyline::audit::Ja3Input<'a>
impl<'a> core::marker::Sync for leyline::audit::Ja3Input<'a>
impl<'a> core::marker::Unpin for leyline::audit::Ja3Input<'a>
impl<'a> core::marker::UnsafeUnpin for leyline::audit::Ja3Input<'a>
impl<'a> core::panic::unwind_safe::RefUnwindSafe for leyline::audit::Ja3Input<'a>
impl<'a> core::panic::unwind_safe::UnwindSafe for leyline::audit::Ja3Input<'a>
```

### `Ja4Input`

```rust,ignore
pub struct leyline::audit::Ja4Input<'a>
pub leyline::audit::Ja4Input::alpn: &'a str
pub leyline::audit::Ja4Input::ciphers: &'a [alloc::string::String]
pub leyline::audit::Ja4Input::curves: &'a [alloc::string::String]
pub leyline::audit::Ja4Input::extension_ids: &'a [u16]
pub leyline::audit::Ja4Input::has_sni: bool
pub leyline::audit::Ja4Input::sigalgs: &'a [alloc::string::String]
pub leyline::audit::Ja4Input::tls_version: &'a str
impl<'a> core::marker::Freeze for leyline::audit::Ja4Input<'a>
impl<'a> core::marker::Send for leyline::audit::Ja4Input<'a>
impl<'a> core::marker::Sync for leyline::audit::Ja4Input<'a>
impl<'a> core::marker::Unpin for leyline::audit::Ja4Input<'a>
impl<'a> core::marker::UnsafeUnpin for leyline::audit::Ja4Input<'a>
impl<'a> core::panic::unwind_safe::RefUnwindSafe for leyline::audit::Ja4Input<'a>
impl<'a> core::panic::unwind_safe::UnwindSafe for leyline::audit::Ja4Input<'a>
```

### `Ja4hInput`

```rust,ignore
pub struct leyline::audit::Ja4hInput<'a>
pub leyline::audit::Ja4hInput::headers: &'a [(alloc::string::String, alloc::string::String)]
pub leyline::audit::Ja4hInput::http_version: &'a str
pub leyline::audit::Ja4hInput::method: &'a str
impl<'a> core::marker::Freeze for leyline::audit::Ja4hInput<'a>
impl<'a> core::marker::Send for leyline::audit::Ja4hInput<'a>
impl<'a> core::marker::Sync for leyline::audit::Ja4hInput<'a>
impl<'a> core::marker::Unpin for leyline::audit::Ja4hInput<'a>
impl<'a> core::marker::UnsafeUnpin for leyline::audit::Ja4hInput<'a>
impl<'a> core::panic::unwind_safe::RefUnwindSafe for leyline::audit::Ja4hInput<'a>
impl<'a> core::panic::unwind_safe::UnwindSafe for leyline::audit::Ja4hInput<'a>
```

### `compute_ja3`

```rust,ignore
pub fn leyline::audit::compute_ja3(input: &leyline::audit::Ja3Input<'_>) -> alloc::string::String
```

### `compute_ja4`

```rust,ignore
pub fn leyline::audit::compute_ja4(input: &leyline::audit::Ja4Input<'_>) -> alloc::string::String
```

### `compute_ja4h`

```rust,ignore
pub fn leyline::audit::compute_ja4h(input: &leyline::audit::Ja4hInput<'_>) -> alloc::string::String
```

### `compute_ja4t`

```rust,ignore
pub fn leyline::audit::compute_ja4t(window_size: u32, mss: u16, window_scale: u8, is_windows: bool) -> alloc::string::String
```

### `extension_ids`

```rust,ignore
pub fn leyline::audit::extension_ids(tls: &leyline::profile::TlsProfile) -> alloc::vec::Vec<u16>
```

## `leyline::cookie`

```rust,ignore
pub mod leyline::cookie
```

### `Cookie`

```rust,ignore
pub struct leyline::cookie::Cookie
pub leyline::cookie::Cookie::creation_time: std::time::SystemTime
pub leyline::cookie::Cookie::domain: alloc::string::String
pub leyline::cookie::Cookie::expires: core::option::Option<std::time::SystemTime>
pub leyline::cookie::Cookie::host_only: bool
pub leyline::cookie::Cookie::http_only: bool
pub leyline::cookie::Cookie::last_access: std::time::SystemTime
pub leyline::cookie::Cookie::name: alloc::string::String
pub leyline::cookie::Cookie::path: alloc::string::String
pub leyline::cookie::Cookie::same_site: leyline::cookie::SameSite
pub leyline::cookie::Cookie::secure: bool
pub leyline::cookie::Cookie::value: alloc::string::String
impl leyline::cookie::Cookie
pub fn leyline::cookie::Cookie::is_expired(&self) -> bool
pub fn leyline::cookie::Cookie::matches(&self, url_domain: &str, url_path: &str, is_secure: bool) -> bool
impl core::clone::Clone for leyline::cookie::Cookie
pub fn leyline::cookie::Cookie::clone(&self) -> leyline::cookie::Cookie
impl core::fmt::Debug for leyline::cookie::Cookie
pub fn leyline::cookie::Cookie::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl serde_core::ser::Serialize for leyline::cookie::Cookie
pub fn leyline::cookie::Cookie::serialize<__S>(&self, __serializer: __S) -> core::result::Result<<__S as serde_core::ser::Serializer>::Ok, <__S as serde_core::ser::Serializer>::Error> where __S: serde_core::ser::Serializer
impl<'de> serde_core::de::Deserialize<'de> for leyline::cookie::Cookie
pub fn leyline::cookie::Cookie::deserialize<__D>(__deserializer: __D) -> core::result::Result<Self, <__D as serde_core::de::Deserializer>::Error> where __D: serde_core::de::Deserializer<'de>
impl core::marker::Freeze for leyline::cookie::Cookie
impl core::marker::Send for leyline::cookie::Cookie
impl core::marker::Sync for leyline::cookie::Cookie
impl core::marker::Unpin for leyline::cookie::Cookie
impl core::marker::UnsafeUnpin for leyline::cookie::Cookie
impl core::panic::unwind_safe::RefUnwindSafe for leyline::cookie::Cookie
impl core::panic::unwind_safe::UnwindSafe for leyline::cookie::Cookie
```

### `Jar`

```rust,ignore
pub struct leyline::cookie::Jar
impl leyline::cookie::Jar
pub fn leyline::cookie::Jar::all_cookies(&self) -> alloc::vec::Vec<leyline::cookie::Cookie>
pub fn leyline::cookie::Jar::clear(&self)
pub fn leyline::cookie::Jar::contains_named(&self, name: &str) -> bool
pub fn leyline::cookie::Jar::cookie_header(&self, url: &url::Url) -> core::option::Option<alloc::string::String>
pub fn leyline::cookie::Jar::deep_clone(&self) -> Self
pub fn leyline::cookie::Jar::export_cookies(&self, raw_url: &str) -> alloc::string::String
pub fn leyline::cookie::Jar::get_cookie(&self, url: &str, name: &str) -> core::option::Option<alloc::string::String>
pub fn leyline::cookie::Jar::get_named(&self, name: &str) -> core::option::Option<alloc::string::String>
pub fn leyline::cookie::Jar::is_empty(&self) -> bool
pub fn leyline::cookie::Jar::len(&self) -> usize
pub fn leyline::cookie::Jar::load_cookies(&self, cookie_str: &str, raw_url: &str)
pub fn leyline::cookie::Jar::merge(&self, other: &leyline::cookie::Jar)
pub fn leyline::cookie::Jar::new() -> Self
pub fn leyline::cookie::Jar::remove_all_named(&self, name: &str) -> usize
pub fn leyline::cookie::Jar::remove_named(&self, name: &str) -> bool
pub fn leyline::cookie::Jar::remove_named_for_host(&self, host: &str, name: &str) -> usize
pub fn leyline::cookie::Jar::set_cookie(&self, url: &str, name: &str, value: &str)
pub fn leyline::cookie::Jar::set_named(&self, name: &str, value: &str) -> bool
pub fn leyline::cookie::Jar::set_named_on(&self, domain: &str, name: &str, value: &str)
pub fn leyline::cookie::Jar::store_response_cookies(&self, headers: &[&str], url: &url::Url)
pub fn leyline::cookie::Jar::store_set_cookie(&self, header: &str, url: &url::Url)
impl core::clone::Clone for leyline::cookie::Jar
pub fn leyline::cookie::Jar::clone(&self) -> leyline::cookie::Jar
impl core::default::Default for leyline::cookie::Jar
pub fn leyline::cookie::Jar::default() -> Self
impl core::fmt::Debug for leyline::cookie::Jar
pub fn leyline::cookie::Jar::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl serde_core::ser::Serialize for leyline::cookie::Jar
pub fn leyline::cookie::Jar::serialize<S: serde_core::ser::Serializer>(&self, ser: S) -> core::result::Result<<S as serde_core::ser::Serializer>::Ok, <S as serde_core::ser::Serializer>::Error>
impl<'de> serde_core::de::Deserialize<'de> for leyline::cookie::Jar
pub fn leyline::cookie::Jar::deserialize<D: serde_core::de::Deserializer<'de>>(de: D) -> core::result::Result<Self, <D as serde_core::de::Deserializer>::Error>
impl core::marker::Freeze for leyline::cookie::Jar
impl core::marker::Send for leyline::cookie::Jar
impl core::marker::Sync for leyline::cookie::Jar
impl core::marker::Unpin for leyline::cookie::Jar
impl core::marker::UnsafeUnpin for leyline::cookie::Jar
impl core::panic::unwind_safe::RefUnwindSafe for leyline::cookie::Jar
impl core::panic::unwind_safe::UnwindSafe for leyline::cookie::Jar
```

### `SameSite`

```rust,ignore
pub enum leyline::cookie::SameSite
pub leyline::cookie::SameSite::Lax
pub leyline::cookie::SameSite::None
pub leyline::cookie::SameSite::Strict
impl core::clone::Clone for leyline::cookie::SameSite
pub fn leyline::cookie::SameSite::clone(&self) -> leyline::cookie::SameSite
impl core::cmp::Eq for leyline::cookie::SameSite
impl core::cmp::PartialEq for leyline::cookie::SameSite
pub fn leyline::cookie::SameSite::eq(&self, other: &leyline::cookie::SameSite) -> bool
impl core::fmt::Debug for leyline::cookie::SameSite
pub fn leyline::cookie::SameSite::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::marker::Copy for leyline::cookie::SameSite
impl core::marker::StructuralPartialEq for leyline::cookie::SameSite
impl serde_core::ser::Serialize for leyline::cookie::SameSite
pub fn leyline::cookie::SameSite::serialize<__S>(&self, __serializer: __S) -> core::result::Result<<__S as serde_core::ser::Serializer>::Ok, <__S as serde_core::ser::Serializer>::Error> where __S: serde_core::ser::Serializer
impl<'de> serde_core::de::Deserialize<'de> for leyline::cookie::SameSite
pub fn leyline::cookie::SameSite::deserialize<__D>(__deserializer: __D) -> core::result::Result<Self, <__D as serde_core::de::Deserializer>::Error> where __D: serde_core::de::Deserializer<'de>
impl core::marker::Freeze for leyline::cookie::SameSite
impl core::marker::Send for leyline::cookie::SameSite
impl core::marker::Sync for leyline::cookie::SameSite
impl core::marker::Unpin for leyline::cookie::SameSite
impl core::marker::UnsafeUnpin for leyline::cookie::SameSite
impl core::panic::unwind_safe::RefUnwindSafe for leyline::cookie::SameSite
impl core::panic::unwind_safe::UnwindSafe for leyline::cookie::SameSite
```

## `leyline::layer`

```rust,ignore
pub mod leyline::layer
```

### `Call`

```rust,ignore
pub struct leyline::layer::Call
impl leyline::layer::Call
pub fn leyline::layer::Call::body(&self) -> &leyline::Body
pub fn leyline::layer::Call::headers(&self) -> &leyline::HeaderList
pub fn leyline::layer::Call::headers_mut(&mut self) -> &mut leyline::HeaderList
pub fn leyline::layer::Call::method(&self) -> &http::method::Method
pub fn leyline::layer::Call::proxy(&self) -> core::option::Option<&str>
pub fn leyline::layer::Call::stream(&self) -> bool
pub fn leyline::layer::Call::uri(&self) -> &http::uri::Uri
impl tower_service::Service<leyline::layer::Call> for leyline::layer::Transport
impl<S> tower_service::Service<leyline::layer::Call> for leyline::layer::Logged<S> where S: tower_service::Service<leyline::layer::Call, Response = leyline::layer::Reply, Error = leyline::Error>, <S as tower_service::Service>::Future: core::marker::Send + 'static
impl !core::marker::Freeze for leyline::layer::Call
impl core::marker::Send for leyline::layer::Call
impl !core::marker::Sync for leyline::layer::Call
impl core::marker::Unpin for leyline::layer::Call
impl core::marker::UnsafeUnpin for leyline::layer::Call
impl !core::panic::unwind_safe::RefUnwindSafe for leyline::layer::Call
impl !core::panic::unwind_safe::UnwindSafe for leyline::layer::Call
impl<S> tower_service::Service<leyline::layer::Call> for leyline::layer::Logged<S> where S: tower_service::Service<leyline::layer::Call, Response = leyline::layer::Reply, Error = leyline::Error>, <S as tower_service::Service>::Future: core::marker::Send + 'static
impl tower_service::Service<leyline::layer::Call> for leyline::layer::Transport
```

### `Log`

```rust,ignore
pub struct leyline::layer::Log
impl core::clone::Clone for leyline::layer::Log
pub fn leyline::layer::Log::clone(&self) -> leyline::layer::Log
impl core::default::Default for leyline::layer::Log
pub fn leyline::layer::Log::default() -> leyline::layer::Log
impl core::fmt::Debug for leyline::layer::Log
pub fn leyline::layer::Log::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::marker::Copy for leyline::layer::Log
impl<S> tower_layer::Layer<S> for leyline::layer::Log
pub type leyline::layer::Log::Service = leyline::layer::Logged<S>
pub fn leyline::layer::Log::layer(&self, inner: S) -> leyline::layer::Logged<S>
impl core::marker::Freeze for leyline::layer::Log
impl core::marker::Send for leyline::layer::Log
impl core::marker::Sync for leyline::layer::Log
impl core::marker::Unpin for leyline::layer::Log
impl core::marker::UnsafeUnpin for leyline::layer::Log
impl core::panic::unwind_safe::RefUnwindSafe for leyline::layer::Log
impl core::panic::unwind_safe::UnwindSafe for leyline::layer::Log
```

### `Logged`

```rust,ignore
pub type leyline::layer::Logged<S>::Error = leyline::Error
pub type leyline::layer::Logged<S>::Future = core::pin::Pin<alloc::boxed::Box<(dyn core::future::future::Future<Output = core::result::Result<leyline::layer::Reply, leyline::Error>> + core::marker::Send)>>
pub type leyline::layer::Logged<S>::Response = leyline::layer::Reply
pub fn leyline::layer::Logged<S>::call(&mut self, call: leyline::layer::Call) -> leyline::layer::Pending
pub fn leyline::layer::Logged<S>::poll_ready(&mut self, cx: &mut core::task::wake::Context<'_>) -> core::task::poll::Poll<leyline::Result<()>>
pub struct leyline::layer::Logged<S>
impl<S: core::clone::Clone> core::clone::Clone for leyline::layer::Logged<S>
pub fn leyline::layer::Logged<S>::clone(&self) -> leyline::layer::Logged<S>
impl<S: core::fmt::Debug> core::fmt::Debug for leyline::layer::Logged<S>
pub fn leyline::layer::Logged<S>::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl<S: core::marker::Copy> core::marker::Copy for leyline::layer::Logged<S>
pub type leyline::layer::Logged<S>::Error = leyline::Error
pub type leyline::layer::Logged<S>::Future = core::pin::Pin<alloc::boxed::Box<(dyn core::future::future::Future<Output = core::result::Result<leyline::layer::Reply, leyline::Error>> + core::marker::Send)>>
pub type leyline::layer::Logged<S>::Response = leyline::layer::Reply
pub fn leyline::layer::Logged<S>::call(&mut self, call: leyline::layer::Call) -> leyline::layer::Pending
pub fn leyline::layer::Logged<S>::poll_ready(&mut self, cx: &mut core::task::wake::Context<'_>) -> core::task::poll::Poll<leyline::Result<()>>
impl<S> core::marker::Freeze for leyline::layer::Logged<S> where S: core::marker::Freeze
impl<S> core::marker::Send for leyline::layer::Logged<S> where S: core::marker::Send
impl<S> core::marker::Sync for leyline::layer::Logged<S> where S: core::marker::Sync
impl<S> core::marker::Unpin for leyline::layer::Logged<S> where S: core::marker::Unpin
impl<S> core::marker::UnsafeUnpin for leyline::layer::Logged<S> where S: core::marker::UnsafeUnpin
impl<S> core::panic::unwind_safe::RefUnwindSafe for leyline::layer::Logged<S> where S: core::panic::unwind_safe::RefUnwindSafe
impl<S> core::panic::unwind_safe::UnwindSafe for leyline::layer::Logged<S> where S: core::panic::unwind_safe::UnwindSafe
```

### `Pending`

```rust,ignore
pub type leyline::layer::Pending = core::pin::Pin<alloc::boxed::Box<(dyn core::future::future::Future<Output = leyline::Result<leyline::layer::Reply>> + core::marker::Send)>>
```

### `Reply`

```rust,ignore
pub struct leyline::layer::Reply
impl leyline::layer::Reply
pub fn leyline::layer::Reply::body(self, body: impl core::convert::Into<alloc::vec::Vec<u8>>) -> Self
pub fn leyline::layer::Reply::get(&self, name: &str) -> core::option::Option<&str>
pub fn leyline::layer::Reply::header(self, name: impl core::convert::TryInto<http::header::name::HeaderName>, value: impl core::convert::TryInto<http::header::value::HeaderValue>) -> leyline::Result<Self>
pub fn leyline::layer::Reply::new(status: http::status::StatusCode) -> Self
pub fn leyline::layer::Reply::status(&self) -> http::status::StatusCode
impl !core::marker::Freeze for leyline::layer::Reply
impl core::marker::Send for leyline::layer::Reply
impl core::marker::Sync for leyline::layer::Reply
impl core::marker::Unpin for leyline::layer::Reply
impl core::marker::UnsafeUnpin for leyline::layer::Reply
impl !core::panic::unwind_safe::RefUnwindSafe for leyline::layer::Reply
impl !core::panic::unwind_safe::UnwindSafe for leyline::layer::Reply
```

### `Transport`

```rust,ignore
pub type leyline::layer::Transport::Error = leyline::Error
pub type leyline::layer::Transport::Future = core::pin::Pin<alloc::boxed::Box<(dyn core::future::future::Future<Output = core::result::Result<leyline::layer::Reply, leyline::Error>> + core::marker::Send)>>
pub type leyline::layer::Transport::Response = leyline::layer::Reply
pub fn leyline::layer::Transport::call(&mut self, call: leyline::layer::Call) -> leyline::layer::Pending
pub fn leyline::layer::Transport::poll_ready(&mut self, _cx: &mut core::task::wake::Context<'_>) -> core::task::poll::Poll<leyline::Result<()>>
pub struct leyline::layer::Transport
impl core::clone::Clone for leyline::layer::Transport
pub fn leyline::layer::Transport::clone(&self) -> leyline::layer::Transport
impl core::default::Default for leyline::layer::Transport
pub fn leyline::layer::Transport::default() -> leyline::layer::Transport
impl core::fmt::Debug for leyline::layer::Transport
pub fn leyline::layer::Transport::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::marker::Copy for leyline::layer::Transport
pub type leyline::layer::Transport::Error = leyline::Error
pub type leyline::layer::Transport::Future = core::pin::Pin<alloc::boxed::Box<(dyn core::future::future::Future<Output = core::result::Result<leyline::layer::Reply, leyline::Error>> + core::marker::Send)>>
pub type leyline::layer::Transport::Response = leyline::layer::Reply
pub fn leyline::layer::Transport::call(&mut self, call: leyline::layer::Call) -> leyline::layer::Pending
pub fn leyline::layer::Transport::poll_ready(&mut self, _cx: &mut core::task::wake::Context<'_>) -> core::task::poll::Poll<leyline::Result<()>>
impl core::marker::Freeze for leyline::layer::Transport
impl core::marker::Send for leyline::layer::Transport
impl core::marker::Sync for leyline::layer::Transport
impl core::marker::Unpin for leyline::layer::Transport
impl core::marker::UnsafeUnpin for leyline::layer::Transport
impl core::panic::unwind_safe::RefUnwindSafe for leyline::layer::Transport
impl core::panic::unwind_safe::UnwindSafe for leyline::layer::Transport
```

## `leyline::multipart`

```rust,ignore
pub mod leyline::multipart
```

### `Form`

```rust,ignore
pub struct leyline::multipart::Form
impl leyline::multipart::Form
pub fn leyline::multipart::Form::boundary(&self) -> &str
pub fn leyline::multipart::Form::content_type(&self) -> alloc::string::String
pub fn leyline::multipart::Form::file(self, name: impl core::convert::Into<alloc::string::String>, path: impl core::convert::AsRef<std::path::Path>) -> core::io::error::Result<Self>
pub fn leyline::multipart::Form::new() -> Self
pub fn leyline::multipart::Form::part(self, name: impl core::convert::Into<alloc::string::String>, part: leyline::multipart::Part) -> Self
pub fn leyline::multipart::Form::text(self, name: impl core::convert::Into<alloc::string::String>, value: impl core::convert::Into<alloc::string::String>) -> Self
impl core::convert::From<leyline::multipart::Form> for leyline::Body
impl core::default::Default for leyline::multipart::Form
pub fn leyline::multipart::Form::default() -> Self
impl core::marker::Freeze for leyline::multipart::Form
impl core::marker::Send for leyline::multipart::Form
impl !core::marker::Sync for leyline::multipart::Form
impl core::marker::Unpin for leyline::multipart::Form
impl core::marker::UnsafeUnpin for leyline::multipart::Form
impl !core::panic::unwind_safe::RefUnwindSafe for leyline::multipart::Form
impl !core::panic::unwind_safe::UnwindSafe for leyline::multipart::Form
impl core::convert::From<leyline::multipart::Form> for leyline::Body
```

### `Part`

```rust,ignore
pub struct leyline::multipart::Part
impl leyline::multipart::Part
pub fn leyline::multipart::Part::bytes(bytes: impl core::convert::Into<bytes::bytes::Bytes>) -> Self
pub fn leyline::multipart::Part::filename(self, name: impl core::convert::Into<alloc::string::String>) -> Self
pub fn leyline::multipart::Part::header(self, name: impl core::convert::Into<alloc::string::String>, value: impl core::convert::Into<alloc::string::String>) -> Self
pub fn leyline::multipart::Part::mime(self, mime: impl core::convert::Into<alloc::string::String>) -> Self
pub fn leyline::multipart::Part::stream<S>(stream: S) -> Self where S: futures_core::stream::Stream<Item = core::io::error::Result<bytes::bytes::Bytes>> + core::marker::Send + 'static
pub fn leyline::multipart::Part::text(value: impl core::convert::Into<alloc::string::String>) -> Self
impl !core::marker::Freeze for leyline::multipart::Part
impl core::marker::Send for leyline::multipart::Part
impl !core::marker::Sync for leyline::multipart::Part
impl core::marker::Unpin for leyline::multipart::Part
impl core::marker::UnsafeUnpin for leyline::multipart::Part
impl !core::panic::unwind_safe::RefUnwindSafe for leyline::multipart::Part
impl !core::panic::unwind_safe::UnwindSafe for leyline::multipart::Part
```

## `leyline::profile`

```rust,ignore
pub mod leyline::profile
```

### `BrandOverlay`

```rust,ignore
#[non_exhaustive] pub struct leyline::profile::BrandOverlay
pub leyline::profile::BrandOverlay::extra_headers: alloc::vec::Vec<(alloc::string::String, alloc::string::String)>
pub leyline::profile::BrandOverlay::navigate_accept: core::option::Option<alloc::string::String>
pub leyline::profile::BrandOverlay::sec_ch_ua: alloc::string::String
pub leyline::profile::BrandOverlay::user_agent: alloc::string::String
impl core::clone::Clone for leyline::profile::BrandOverlay
pub fn leyline::profile::BrandOverlay::clone(&self) -> leyline::profile::BrandOverlay
impl core::fmt::Debug for leyline::profile::BrandOverlay
pub fn leyline::profile::BrandOverlay::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::marker::Freeze for leyline::profile::BrandOverlay
impl core::marker::Send for leyline::profile::BrandOverlay
impl core::marker::Sync for leyline::profile::BrandOverlay
impl core::marker::Unpin for leyline::profile::BrandOverlay
impl core::marker::UnsafeUnpin for leyline::profile::BrandOverlay
impl core::panic::unwind_safe::RefUnwindSafe for leyline::profile::BrandOverlay
impl core::panic::unwind_safe::UnwindSafe for leyline::profile::BrandOverlay
```

### `BrandOverlayError`

```rust,ignore
#[non_exhaustive] pub enum leyline::profile::BrandOverlayError
pub leyline::profile::BrandOverlayError::Unverified
pub leyline::profile::BrandOverlayError::Unverified::brand: leyline::ChromiumBrand
pub leyline::profile::BrandOverlayError::Unverified::chromium_major: u32
pub leyline::profile::BrandOverlayError::Unverified::platform: leyline::Platform
impl core::clone::Clone for leyline::profile::BrandOverlayError
pub fn leyline::profile::BrandOverlayError::clone(&self) -> leyline::profile::BrandOverlayError
impl core::cmp::Eq for leyline::profile::BrandOverlayError
impl core::cmp::PartialEq for leyline::profile::BrandOverlayError
pub fn leyline::profile::BrandOverlayError::eq(&self, other: &leyline::profile::BrandOverlayError) -> bool
impl core::error::Error for leyline::profile::BrandOverlayError
impl core::fmt::Debug for leyline::profile::BrandOverlayError
pub fn leyline::profile::BrandOverlayError::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::fmt::Display for leyline::profile::BrandOverlayError
pub fn leyline::profile::BrandOverlayError::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::marker::StructuralPartialEq for leyline::profile::BrandOverlayError
impl core::marker::Freeze for leyline::profile::BrandOverlayError
impl core::marker::Send for leyline::profile::BrandOverlayError
impl core::marker::Sync for leyline::profile::BrandOverlayError
impl core::marker::Unpin for leyline::profile::BrandOverlayError
impl core::marker::UnsafeUnpin for leyline::profile::BrandOverlayError
impl core::panic::unwind_safe::RefUnwindSafe for leyline::profile::BrandOverlayError
impl core::panic::unwind_safe::UnwindSafe for leyline::profile::BrandOverlayError
```

### `Browser`

```rust,ignore
#[non_exhaustive] pub enum leyline::profile::Browser
pub leyline::profile::Browser::Brave146
pub leyline::profile::Browser::CfnetworkIOS18
pub leyline::profile::Browser::CfnetworkMacOS26
pub leyline::profile::Browser::Chrome145
pub leyline::profile::Browser::Chrome146
pub leyline::profile::Browser::Chrome147
pub leyline::profile::Browser::Chrome148
pub leyline::profile::Browser::Chrome149
pub leyline::profile::Browser::Chrome150
pub leyline::profile::Browser::Chrome151
pub leyline::profile::Browser::Chrome152
pub leyline::profile::Browser::Firefox148
pub leyline::profile::Browser::Firefox149
pub leyline::profile::Browser::Firefox150
pub leyline::profile::Browser::Firefox151
pub leyline::profile::Browser::Firefox152
pub leyline::profile::Browser::Firefox153
pub leyline::profile::Browser::Firefox154
pub leyline::profile::Browser::OkHttpAndroid10
pub leyline::profile::Browser::Safari18
pub leyline::profile::Browser::Safari26
pub leyline::profile::Browser::SafariIOS17
pub leyline::profile::Browser::SafariIOS18
```

### `BrowserProfile`

```rust,ignore
#[non_exhaustive] pub struct leyline::profile::BrowserProfile
pub leyline::profile::BrowserProfile::h2: leyline::profile::H2Profile
pub leyline::profile::BrowserProfile::identity: std::collections::hash::map::HashMap<alloc::string::String, leyline::profile::PlatformIdentity>
pub leyline::profile::BrowserProfile::meta: leyline::profile::ProfileMeta
pub leyline::profile::BrowserProfile::tls: leyline::profile::TlsProfile
impl leyline::profile::BrowserProfile
pub fn leyline::profile::BrowserProfile::bare() -> Self
impl leyline::profile::BrowserProfile
pub fn leyline::profile::BrowserProfile::expected_h2_fingerprint(&self) -> core::option::Option<&str>
pub fn leyline::profile::BrowserProfile::expected_h2_fingerprint_for(&self, platform: leyline::Platform) -> core::option::Option<&str>
pub fn leyline::profile::BrowserProfile::expected_ja4(&self) -> core::option::Option<&str>
pub fn leyline::profile::BrowserProfile::expected_resumed_ja4(&self) -> core::option::Option<&str>
pub fn leyline::profile::BrowserProfile::from_toml(toml_str: &str) -> core::result::Result<Self, leyline::profile::ProfileError>
pub fn leyline::profile::BrowserProfile::identity_for(&self, platform: leyline::Platform) -> core::option::Option<&leyline::profile::PlatformIdentity>
pub fn leyline::profile::BrowserProfile::load_warnings(&self) -> alloc::vec::Vec<alloc::string::String>
impl core::clone::Clone for leyline::profile::BrowserProfile
pub fn leyline::profile::BrowserProfile::clone(&self) -> leyline::profile::BrowserProfile
impl core::fmt::Debug for leyline::profile::BrowserProfile
pub fn leyline::profile::BrowserProfile::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl<'de> serde_core::de::Deserialize<'de> for leyline::profile::BrowserProfile
pub fn leyline::profile::BrowserProfile::deserialize<__D>(__deserializer: __D) -> core::result::Result<Self, <__D as serde_core::de::Deserializer>::Error> where __D: serde_core::de::Deserializer<'de>
impl core::marker::Freeze for leyline::profile::BrowserProfile
impl core::marker::Send for leyline::profile::BrowserProfile
impl core::marker::Sync for leyline::profile::BrowserProfile
impl core::marker::Unpin for leyline::profile::BrowserProfile
impl core::marker::UnsafeUnpin for leyline::profile::BrowserProfile
impl core::panic::unwind_safe::RefUnwindSafe for leyline::profile::BrowserProfile
impl core::panic::unwind_safe::UnwindSafe for leyline::profile::BrowserProfile
impl leyline::profile::BrowserProfile
pub fn leyline::profile::BrowserProfile::bare() -> Self
impl leyline::profile::BrowserProfile
pub fn leyline::profile::BrowserProfile::expected_h2_fingerprint(&self) -> core::option::Option<&str>
pub fn leyline::profile::BrowserProfile::expected_h2_fingerprint_for(&self, platform: leyline::Platform) -> core::option::Option<&str>
pub fn leyline::profile::BrowserProfile::expected_ja4(&self) -> core::option::Option<&str>
pub fn leyline::profile::BrowserProfile::expected_resumed_ja4(&self) -> core::option::Option<&str>
pub fn leyline::profile::BrowserProfile::from_toml(toml_str: &str) -> core::result::Result<Self, leyline::profile::ProfileError>
pub fn leyline::profile::BrowserProfile::identity_for(&self, platform: leyline::Platform) -> core::option::Option<&leyline::profile::PlatformIdentity>
pub fn leyline::profile::BrowserProfile::load_warnings(&self) -> alloc::vec::Vec<alloc::string::String>
impl core::clone::Clone for leyline::profile::BrowserProfile
pub fn leyline::profile::BrowserProfile::clone(&self) -> leyline::profile::BrowserProfile
impl core::fmt::Debug for leyline::profile::BrowserProfile
pub fn leyline::profile::BrowserProfile::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl<'de> serde_core::de::Deserialize<'de> for leyline::profile::BrowserProfile
pub fn leyline::profile::BrowserProfile::deserialize<__D>(__deserializer: __D) -> core::result::Result<Self, <__D as serde_core::de::Deserializer>::Error> where __D: serde_core::de::Deserializer<'de>
impl core::marker::Freeze for leyline::profile::BrowserProfile
impl core::marker::Send for leyline::profile::BrowserProfile
impl core::marker::Sync for leyline::profile::BrowserProfile
impl core::marker::Unpin for leyline::profile::BrowserProfile
impl core::marker::UnsafeUnpin for leyline::profile::BrowserProfile
impl core::panic::unwind_safe::RefUnwindSafe for leyline::profile::BrowserProfile
impl core::panic::unwind_safe::UnwindSafe for leyline::profile::BrowserProfile
```

### `ChromiumBrand`

```rust,ignore
#[non_exhaustive] pub enum leyline::profile::ChromiumBrand
pub leyline::profile::ChromiumBrand::Chrome
pub leyline::profile::ChromiumBrand::Edge
pub leyline::profile::ChromiumBrand::Opera
pub leyline::profile::ChromiumBrand::Vivaldi
```

### `Family`

```rust,ignore
#[non_exhaustive] pub enum leyline::profile::Family
pub leyline::profile::Family::Brave
pub leyline::profile::Family::CfNetwork
pub leyline::profile::Family::Chrome
pub leyline::profile::Family::Firefox
pub leyline::profile::Family::OkHttp
pub leyline::profile::Family::Safari
pub leyline::profile::Family::SafariIos
impl core::clone::Clone for leyline::profile::Family
pub fn leyline::profile::Family::clone(&self) -> leyline::profile::Family
impl core::cmp::Eq for leyline::profile::Family
impl core::cmp::PartialEq for leyline::profile::Family
pub fn leyline::profile::Family::eq(&self, other: &leyline::profile::Family) -> bool
impl core::fmt::Debug for leyline::profile::Family
pub fn leyline::profile::Family::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::fmt::Display for leyline::profile::Family
pub fn leyline::profile::Family::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::hash::Hash for leyline::profile::Family
pub fn leyline::profile::Family::hash<__H: core::hash::Hasher>(&self, state: &mut __H)
impl core::marker::Copy for leyline::profile::Family
impl core::marker::StructuralPartialEq for leyline::profile::Family
impl core::marker::Freeze for leyline::profile::Family
impl core::marker::Send for leyline::profile::Family
impl core::marker::Sync for leyline::profile::Family
impl core::marker::Unpin for leyline::profile::Family
impl core::marker::UnsafeUnpin for leyline::profile::Family
impl core::panic::unwind_safe::RefUnwindSafe for leyline::profile::Family
impl core::panic::unwind_safe::UnwindSafe for leyline::profile::Family
```

### `H2Fingerprint`

```rust,ignore
#[non_exhaustive] pub struct leyline::profile::H2Fingerprint
pub leyline::profile::H2Fingerprint::akamai: core::option::Option<alloc::string::String>
impl core::clone::Clone for leyline::profile::H2Fingerprint
pub fn leyline::profile::H2Fingerprint::clone(&self) -> leyline::profile::H2Fingerprint
impl core::default::Default for leyline::profile::H2Fingerprint
pub fn leyline::profile::H2Fingerprint::default() -> leyline::profile::H2Fingerprint
impl core::fmt::Debug for leyline::profile::H2Fingerprint
pub fn leyline::profile::H2Fingerprint::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl<'de> serde_core::de::Deserialize<'de> for leyline::profile::H2Fingerprint
pub fn leyline::profile::H2Fingerprint::deserialize<__D>(__deserializer: __D) -> core::result::Result<Self, <__D as serde_core::de::Deserializer>::Error> where __D: serde_core::de::Deserializer<'de>
impl core::marker::Freeze for leyline::profile::H2Fingerprint
impl core::marker::Send for leyline::profile::H2Fingerprint
impl core::marker::Sync for leyline::profile::H2Fingerprint
impl core::marker::Unpin for leyline::profile::H2Fingerprint
impl core::marker::UnsafeUnpin for leyline::profile::H2Fingerprint
impl core::panic::unwind_safe::RefUnwindSafe for leyline::profile::H2Fingerprint
impl core::panic::unwind_safe::UnwindSafe for leyline::profile::H2Fingerprint
```

### `H2PlatformOverride`

```rust,ignore
#[non_exhaustive] pub struct leyline::profile::H2PlatformOverride
pub leyline::profile::H2PlatformOverride::enable_push: core::option::Option<bool>
pub leyline::profile::H2PlatformOverride::fingerprint: core::option::Option<leyline::profile::H2Fingerprint>
pub leyline::profile::H2PlatformOverride::header_table_size: core::option::Option<u32>
pub leyline::profile::H2PlatformOverride::initial_connection_window_size: core::option::Option<u32>
pub leyline::profile::H2PlatformOverride::initial_stream_window_size: core::option::Option<u32>
pub leyline::profile::H2PlatformOverride::max_concurrent_streams: core::option::Option<u32>
pub leyline::profile::H2PlatformOverride::max_frame_size: core::option::Option<u32>
pub leyline::profile::H2PlatformOverride::max_header_list_size: core::option::Option<u32>
pub leyline::profile::H2PlatformOverride::omit_settings: alloc::vec::Vec<alloc::string::String>
pub leyline::profile::H2PlatformOverride::pseudo_order: core::option::Option<alloc::vec::Vec<alloc::string::String>>
pub leyline::profile::H2PlatformOverride::settings_order: core::option::Option<alloc::vec::Vec<alloc::string::String>>
pub leyline::profile::H2PlatformOverride::unknown_setting8: core::option::Option<u32>
pub leyline::profile::H2PlatformOverride::unknown_setting9: core::option::Option<u32>
impl core::clone::Clone for leyline::profile::H2PlatformOverride
pub fn leyline::profile::H2PlatformOverride::clone(&self) -> leyline::profile::H2PlatformOverride
impl core::default::Default for leyline::profile::H2PlatformOverride
pub fn leyline::profile::H2PlatformOverride::default() -> leyline::profile::H2PlatformOverride
impl core::fmt::Debug for leyline::profile::H2PlatformOverride
pub fn leyline::profile::H2PlatformOverride::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl<'de> serde_core::de::Deserialize<'de> for leyline::profile::H2PlatformOverride
pub fn leyline::profile::H2PlatformOverride::deserialize<__D>(__deserializer: __D) -> core::result::Result<Self, <__D as serde_core::de::Deserializer>::Error> where __D: serde_core::de::Deserializer<'de>
impl core::marker::Freeze for leyline::profile::H2PlatformOverride
impl core::marker::Send for leyline::profile::H2PlatformOverride
impl core::marker::Sync for leyline::profile::H2PlatformOverride
impl core::marker::Unpin for leyline::profile::H2PlatformOverride
impl core::marker::UnsafeUnpin for leyline::profile::H2PlatformOverride
impl core::panic::unwind_safe::RefUnwindSafe for leyline::profile::H2PlatformOverride
impl core::panic::unwind_safe::UnwindSafe for leyline::profile::H2PlatformOverride
```

### `H2PriorityProfile`

```rust,ignore
#[non_exhaustive] pub struct leyline::profile::H2PriorityProfile
pub leyline::profile::H2PriorityProfile::exclusive: bool
pub leyline::profile::H2PriorityProfile::stream_dependency: u32
pub leyline::profile::H2PriorityProfile::weight: u8
impl core::clone::Clone for leyline::profile::H2PriorityProfile
pub fn leyline::profile::H2PriorityProfile::clone(&self) -> leyline::profile::H2PriorityProfile
impl core::cmp::Eq for leyline::profile::H2PriorityProfile
impl core::cmp::PartialEq for leyline::profile::H2PriorityProfile
pub fn leyline::profile::H2PriorityProfile::eq(&self, other: &leyline::profile::H2PriorityProfile) -> bool
impl core::fmt::Debug for leyline::profile::H2PriorityProfile
pub fn leyline::profile::H2PriorityProfile::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::marker::Copy for leyline::profile::H2PriorityProfile
impl core::marker::StructuralPartialEq for leyline::profile::H2PriorityProfile
impl<'de> serde_core::de::Deserialize<'de> for leyline::profile::H2PriorityProfile
pub fn leyline::profile::H2PriorityProfile::deserialize<__D>(__deserializer: __D) -> core::result::Result<Self, <__D as serde_core::de::Deserializer>::Error> where __D: serde_core::de::Deserializer<'de>
impl core::marker::Freeze for leyline::profile::H2PriorityProfile
impl core::marker::Send for leyline::profile::H2PriorityProfile
impl core::marker::Sync for leyline::profile::H2PriorityProfile
impl core::marker::Unpin for leyline::profile::H2PriorityProfile
impl core::marker::UnsafeUnpin for leyline::profile::H2PriorityProfile
impl core::panic::unwind_safe::RefUnwindSafe for leyline::profile::H2PriorityProfile
impl core::panic::unwind_safe::UnwindSafe for leyline::profile::H2PriorityProfile
```

### `H2Profile`

```rust,ignore
#[non_exhaustive] pub struct leyline::profile::H2Profile
pub leyline::profile::H2Profile::default_priority: core::option::Option<leyline::profile::H2PriorityProfile>
pub leyline::profile::H2Profile::enable_push: core::option::Option<bool>
pub leyline::profile::H2Profile::fingerprint: core::option::Option<leyline::profile::H2Fingerprint>
pub leyline::profile::H2Profile::header_table_size: core::option::Option<u32>
pub leyline::profile::H2Profile::initial_connection_window_size: core::option::Option<u32>
pub leyline::profile::H2Profile::initial_stream_window_size: core::option::Option<u32>
pub leyline::profile::H2Profile::max_concurrent_streams: core::option::Option<u32>
pub leyline::profile::H2Profile::max_frame_size: core::option::Option<u32>
pub leyline::profile::H2Profile::max_header_list_size: core::option::Option<u32>
pub leyline::profile::H2Profile::platforms: std::collections::hash::map::HashMap<alloc::string::String, leyline::profile::H2PlatformOverride>
pub leyline::profile::H2Profile::pseudo_order: alloc::vec::Vec<alloc::string::String>
pub leyline::profile::H2Profile::settings_order: alloc::vec::Vec<alloc::string::String>
pub leyline::profile::H2Profile::unknown_setting8: core::option::Option<u32>
pub leyline::profile::H2Profile::unknown_setting9: core::option::Option<u32>
impl leyline::profile::H2Profile
pub fn leyline::profile::H2Profile::resolve_for_platform(&self, platform: leyline::Platform) -> core::result::Result<leyline::profile::H2Profile, leyline::Error>
impl core::clone::Clone for leyline::profile::H2Profile
pub fn leyline::profile::H2Profile::clone(&self) -> leyline::profile::H2Profile
impl core::fmt::Debug for leyline::profile::H2Profile
pub fn leyline::profile::H2Profile::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl<'de> serde_core::de::Deserialize<'de> for leyline::profile::H2Profile
pub fn leyline::profile::H2Profile::deserialize<__D>(__deserializer: __D) -> core::result::Result<Self, <__D as serde_core::de::Deserializer>::Error> where __D: serde_core::de::Deserializer<'de>
impl core::marker::Freeze for leyline::profile::H2Profile
impl core::marker::Send for leyline::profile::H2Profile
impl core::marker::Sync for leyline::profile::H2Profile
impl core::marker::Unpin for leyline::profile::H2Profile
impl core::marker::UnsafeUnpin for leyline::profile::H2Profile
impl core::panic::unwind_safe::RefUnwindSafe for leyline::profile::H2Profile
impl core::panic::unwind_safe::UnwindSafe for leyline::profile::H2Profile
```

### `HeaderAnchor`

```rust,ignore
#[non_exhaustive] pub enum leyline::profile::HeaderAnchor
pub leyline::profile::HeaderAnchor::AfterAccept
pub leyline::profile::HeaderAnchor::AfterCchUa
pub leyline::profile::HeaderAnchor::AfterCchUaMobile
pub leyline::profile::HeaderAnchor::AfterCchUaPlatform
pub leyline::profile::HeaderAnchor::AfterContentType
pub leyline::profile::HeaderAnchor::AfterUserAgent
pub leyline::profile::HeaderAnchor::BeforeAcceptEncoding
```

### `Platform`

```rust,ignore
#[non_exhaustive] pub enum leyline::profile::Platform
pub leyline::profile::Platform::Android
pub leyline::profile::Platform::Host
pub leyline::profile::Platform::IOS
pub leyline::profile::Platform::Linux
pub leyline::profile::Platform::MacOS
pub leyline::profile::Platform::Windows
```

### `PlatformIdentity`

```rust,ignore
#[non_exhaustive] pub struct leyline::profile::PlatformIdentity
pub leyline::profile::PlatformIdentity::accept_language: core::option::Option<alloc::string::String>
pub leyline::profile::PlatformIdentity::extra_headers: alloc::vec::Vec<(alloc::string::String, alloc::string::String)>
pub leyline::profile::PlatformIdentity::navigate_accept_override: core::option::Option<alloc::string::String>
pub leyline::profile::PlatformIdentity::request_header_order: core::option::Option<alloc::vec::Vec<alloc::string::String>>
pub leyline::profile::PlatformIdentity::sec_ch_ua: alloc::string::String
pub leyline::profile::PlatformIdentity::user_agent: alloc::string::String
impl core::clone::Clone for leyline::profile::PlatformIdentity
pub fn leyline::profile::PlatformIdentity::clone(&self) -> leyline::profile::PlatformIdentity
impl core::fmt::Debug for leyline::profile::PlatformIdentity
pub fn leyline::profile::PlatformIdentity::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl<'de> serde_core::de::Deserialize<'de> for leyline::profile::PlatformIdentity
pub fn leyline::profile::PlatformIdentity::deserialize<__D>(__deserializer: __D) -> core::result::Result<Self, <__D as serde_core::de::Deserializer>::Error> where __D: serde_core::de::Deserializer<'de>
impl core::marker::Freeze for leyline::profile::PlatformIdentity
impl core::marker::Send for leyline::profile::PlatformIdentity
impl core::marker::Sync for leyline::profile::PlatformIdentity
impl core::marker::Unpin for leyline::profile::PlatformIdentity
impl core::marker::UnsafeUnpin for leyline::profile::PlatformIdentity
impl core::panic::unwind_safe::RefUnwindSafe for leyline::profile::PlatformIdentity
impl core::panic::unwind_safe::UnwindSafe for leyline::profile::PlatformIdentity
```

### `Preset`

```rust,ignore
#[non_exhaustive] pub enum leyline::profile::Preset
pub leyline::profile::Preset::CrossOrigin
pub leyline::profile::Preset::Form
pub leyline::profile::Preset::FormNavigate
pub leyline::profile::Preset::Native
pub leyline::profile::Preset::Navigate
pub leyline::profile::Preset::SameSite
pub leyline::profile::Preset::Script
pub leyline::profile::Preset::Xhr
```

### `ProfileError`

```rust,ignore
#[non_exhaustive] pub enum leyline::profile::ProfileError
pub leyline::profile::ProfileError::Empty
pub leyline::profile::ProfileError::Empty::path: std::path::PathBuf
pub leyline::profile::ProfileError::Io
pub leyline::profile::ProfileError::Io::path: std::path::PathBuf
pub leyline::profile::ProfileError::Io::source: core::io::error::Error
pub leyline::profile::ProfileError::Parse
pub leyline::profile::ProfileError::Parse::path: core::option::Option<std::path::PathBuf>
pub leyline::profile::ProfileError::Parse::source: alloc::boxed::Box<(dyn core::error::Error + core::marker::Send + core::marker::Sync)>
impl core::error::Error for leyline::profile::ProfileError
pub fn leyline::profile::ProfileError::source(&self) -> core::option::Option<&(dyn core::error::Error + 'static)>
impl core::fmt::Debug for leyline::profile::ProfileError
pub fn leyline::profile::ProfileError::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::fmt::Display for leyline::profile::ProfileError
pub fn leyline::profile::ProfileError::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::marker::Freeze for leyline::profile::ProfileError
impl core::marker::Send for leyline::profile::ProfileError
impl core::marker::Sync for leyline::profile::ProfileError
impl core::marker::Unpin for leyline::profile::ProfileError
impl core::marker::UnsafeUnpin for leyline::profile::ProfileError
impl !core::panic::unwind_safe::RefUnwindSafe for leyline::profile::ProfileError
impl !core::panic::unwind_safe::UnwindSafe for leyline::profile::ProfileError
```

### `ProfileMeta`

```rust,ignore
#[non_exhaustive] pub struct leyline::profile::ProfileMeta
pub leyline::profile::ProfileMeta::browser: alloc::string::String
pub leyline::profile::ProfileMeta::captured_against: core::option::Option<alloc::string::String>
pub leyline::profile::ProfileMeta::family: alloc::string::String
pub leyline::profile::ProfileMeta::name: alloc::string::String
pub leyline::profile::ProfileMeta::verified_against: alloc::string::String
pub leyline::profile::ProfileMeta::version: u32
impl core::clone::Clone for leyline::profile::ProfileMeta
pub fn leyline::profile::ProfileMeta::clone(&self) -> leyline::profile::ProfileMeta
impl core::fmt::Debug for leyline::profile::ProfileMeta
pub fn leyline::profile::ProfileMeta::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl<'de> serde_core::de::Deserialize<'de> for leyline::profile::ProfileMeta
pub fn leyline::profile::ProfileMeta::deserialize<__D>(__deserializer: __D) -> core::result::Result<Self, <__D as serde_core::de::Deserializer>::Error> where __D: serde_core::de::Deserializer<'de>
impl core::marker::Freeze for leyline::profile::ProfileMeta
impl core::marker::Send for leyline::profile::ProfileMeta
impl core::marker::Sync for leyline::profile::ProfileMeta
impl core::marker::Unpin for leyline::profile::ProfileMeta
impl core::marker::UnsafeUnpin for leyline::profile::ProfileMeta
impl core::panic::unwind_safe::RefUnwindSafe for leyline::profile::ProfileMeta
impl core::panic::unwind_safe::UnwindSafe for leyline::profile::ProfileMeta
```

### `ProfileRegistry`

```rust,ignore
pub struct leyline::profile::ProfileRegistry
impl leyline::profile::ProfileRegistry
pub fn leyline::profile::ProfileRegistry::builtin() -> Self
pub fn leyline::profile::ProfileRegistry::get(&self, browser: &str, version: u32) -> core::option::Option<&leyline::profile::BrowserProfile>
pub fn leyline::profile::ProfileRegistry::get_browser(&self, browser: leyline::Browser) -> core::option::Option<&leyline::profile::BrowserProfile>
pub fn leyline::profile::ProfileRegistry::global() -> &'static Self
pub fn leyline::profile::ProfileRegistry::is_empty(&self) -> bool
pub fn leyline::profile::ProfileRegistry::len(&self) -> usize
pub fn leyline::profile::ProfileRegistry::load(dir: &std::path::Path) -> core::result::Result<Self, leyline::profile::ProfileError>
pub fn leyline::profile::ProfileRegistry::new() -> Self
impl core::default::Default for leyline::profile::ProfileRegistry
pub fn leyline::profile::ProfileRegistry::default() -> Self
impl core::marker::Freeze for leyline::profile::ProfileRegistry
impl core::marker::Send for leyline::profile::ProfileRegistry
impl core::marker::Sync for leyline::profile::ProfileRegistry
impl core::marker::Unpin for leyline::profile::ProfileRegistry
impl core::marker::UnsafeUnpin for leyline::profile::ProfileRegistry
impl core::panic::unwind_safe::RefUnwindSafe for leyline::profile::ProfileRegistry
impl core::panic::unwind_safe::UnwindSafe for leyline::profile::ProfileRegistry
```

### `TlsFingerprint`

```rust,ignore
#[non_exhaustive] pub struct leyline::profile::TlsFingerprint
pub leyline::profile::TlsFingerprint::ja4: core::option::Option<alloc::string::String>
pub leyline::profile::TlsFingerprint::platforms: std::collections::hash::map::HashMap<alloc::string::String, leyline::profile::TlsFingerprint>
pub leyline::profile::TlsFingerprint::resumed_ja4: core::option::Option<alloc::string::String>
impl core::clone::Clone for leyline::profile::TlsFingerprint
pub fn leyline::profile::TlsFingerprint::clone(&self) -> leyline::profile::TlsFingerprint
impl core::default::Default for leyline::profile::TlsFingerprint
pub fn leyline::profile::TlsFingerprint::default() -> leyline::profile::TlsFingerprint
impl core::fmt::Debug for leyline::profile::TlsFingerprint
pub fn leyline::profile::TlsFingerprint::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl<'de> serde_core::de::Deserialize<'de> for leyline::profile::TlsFingerprint
pub fn leyline::profile::TlsFingerprint::deserialize<__D>(__deserializer: __D) -> core::result::Result<Self, <__D as serde_core::de::Deserializer>::Error> where __D: serde_core::de::Deserializer<'de>
impl core::marker::Freeze for leyline::profile::TlsFingerprint
impl core::marker::Send for leyline::profile::TlsFingerprint
impl core::marker::Sync for leyline::profile::TlsFingerprint
impl core::marker::Unpin for leyline::profile::TlsFingerprint
impl core::marker::UnsafeUnpin for leyline::profile::TlsFingerprint
impl core::panic::unwind_safe::RefUnwindSafe for leyline::profile::TlsFingerprint
impl core::panic::unwind_safe::UnwindSafe for leyline::profile::TlsFingerprint
```

### `TlsProfile`

```rust,ignore
#[non_exhaustive] pub struct leyline::profile::TlsProfile
pub leyline::profile::TlsProfile::alps: core::option::Option<alloc::string::String>
pub leyline::profile::TlsProfile::alps_new_codepoint: bool
pub leyline::profile::TlsProfile::cert_compression: alloc::vec::Vec<alloc::string::String>
pub leyline::profile::TlsProfile::ciphers: alloc::vec::Vec<alloc::string::String>
pub leyline::profile::TlsProfile::curves: alloc::vec::Vec<alloc::string::String>
pub leyline::profile::TlsProfile::delegated_credentials: core::option::Option<alloc::string::String>
pub leyline::profile::TlsProfile::ech_grease: bool
pub leyline::profile::TlsProfile::extension_permutation: core::option::Option<alloc::vec::Vec<u16>>
pub leyline::profile::TlsProfile::fingerprint: core::option::Option<leyline::profile::TlsFingerprint>
pub leyline::profile::TlsProfile::grease: bool
pub leyline::profile::TlsProfile::min_tls_version: core::option::Option<alloc::string::String>
pub leyline::profile::TlsProfile::ocsp_stapling: bool
pub leyline::profile::TlsProfile::padding: bool
pub leyline::profile::TlsProfile::permute_extensions: bool
pub leyline::profile::TlsProfile::pre_shared_key: bool
pub leyline::profile::TlsProfile::record_size_limit: core::option::Option<u16>
pub leyline::profile::TlsProfile::request_trust_anchors: bool
pub leyline::profile::TlsProfile::session_tickets: bool
pub leyline::profile::TlsProfile::sigalgs: alloc::vec::Vec<alloc::string::String>
pub leyline::profile::TlsProfile::signed_cert_timestamps: bool
impl core::clone::Clone for leyline::profile::TlsProfile
pub fn leyline::profile::TlsProfile::clone(&self) -> leyline::profile::TlsProfile
impl core::fmt::Debug for leyline::profile::TlsProfile
pub fn leyline::profile::TlsProfile::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl<'de> serde_core::de::Deserialize<'de> for leyline::profile::TlsProfile
pub fn leyline::profile::TlsProfile::deserialize<__D>(__deserializer: __D) -> core::result::Result<Self, <__D as serde_core::de::Deserializer>::Error> where __D: serde_core::de::Deserializer<'de>
impl core::marker::Freeze for leyline::profile::TlsProfile
impl core::marker::Send for leyline::profile::TlsProfile
impl core::marker::Sync for leyline::profile::TlsProfile
impl core::marker::Unpin for leyline::profile::TlsProfile
impl core::marker::UnsafeUnpin for leyline::profile::TlsProfile
impl core::panic::unwind_safe::RefUnwindSafe for leyline::profile::TlsProfile
impl core::panic::unwind_safe::UnwindSafe for leyline::profile::TlsProfile
```

### `infer_anchor`

```rust,ignore
pub fn leyline::profile::infer_anchor(name: &str) -> core::option::Option<leyline::profile::anchor::HeaderAnchor>
```

## `leyline::profile::anchor`

```rust,ignore
pub mod leyline::profile::anchor
```

### `HeaderAnchor`

```rust,ignore
#[non_exhaustive] pub enum leyline::profile::anchor::HeaderAnchor
pub leyline::profile::anchor::HeaderAnchor::AfterAccept
pub leyline::profile::anchor::HeaderAnchor::AfterCchUa
pub leyline::profile::anchor::HeaderAnchor::AfterCchUaMobile
pub leyline::profile::anchor::HeaderAnchor::AfterCchUaPlatform
pub leyline::profile::anchor::HeaderAnchor::AfterContentType
pub leyline::profile::anchor::HeaderAnchor::AfterUserAgent
pub leyline::profile::anchor::HeaderAnchor::BeforeAcceptEncoding
impl leyline::profile::anchor::HeaderAnchor
pub fn leyline::profile::anchor::HeaderAnchor::anchor_name(&self) -> &'static str
pub fn leyline::profile::anchor::HeaderAnchor::is_before(&self) -> bool
impl core::clone::Clone for leyline::profile::anchor::HeaderAnchor
pub fn leyline::profile::anchor::HeaderAnchor::clone(&self) -> leyline::profile::anchor::HeaderAnchor
impl core::cmp::Eq for leyline::profile::anchor::HeaderAnchor
impl core::cmp::PartialEq for leyline::profile::anchor::HeaderAnchor
pub fn leyline::profile::anchor::HeaderAnchor::eq(&self, other: &leyline::profile::anchor::HeaderAnchor) -> bool
impl core::fmt::Debug for leyline::profile::anchor::HeaderAnchor
pub fn leyline::profile::anchor::HeaderAnchor::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::hash::Hash for leyline::profile::anchor::HeaderAnchor
pub fn leyline::profile::anchor::HeaderAnchor::hash<__H: core::hash::Hasher>(&self, state: &mut __H)
impl core::marker::Copy for leyline::profile::anchor::HeaderAnchor
impl core::marker::StructuralPartialEq for leyline::profile::anchor::HeaderAnchor
impl core::marker::Freeze for leyline::profile::anchor::HeaderAnchor
impl core::marker::Send for leyline::profile::anchor::HeaderAnchor
impl core::marker::Sync for leyline::profile::anchor::HeaderAnchor
impl core::marker::Unpin for leyline::profile::anchor::HeaderAnchor
impl core::marker::UnsafeUnpin for leyline::profile::anchor::HeaderAnchor
impl core::panic::unwind_safe::RefUnwindSafe for leyline::profile::anchor::HeaderAnchor
impl core::panic::unwind_safe::UnwindSafe for leyline::profile::anchor::HeaderAnchor
impl leyline::profile::anchor::HeaderAnchor
pub fn leyline::profile::anchor::HeaderAnchor::anchor_name(&self) -> &'static str
pub fn leyline::profile::anchor::HeaderAnchor::is_before(&self) -> bool
impl core::clone::Clone for leyline::profile::anchor::HeaderAnchor
pub fn leyline::profile::anchor::HeaderAnchor::clone(&self) -> leyline::profile::anchor::HeaderAnchor
impl core::cmp::Eq for leyline::profile::anchor::HeaderAnchor
impl core::cmp::PartialEq for leyline::profile::anchor::HeaderAnchor
pub fn leyline::profile::anchor::HeaderAnchor::eq(&self, other: &leyline::profile::anchor::HeaderAnchor) -> bool
impl core::fmt::Debug for leyline::profile::anchor::HeaderAnchor
pub fn leyline::profile::anchor::HeaderAnchor::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::hash::Hash for leyline::profile::anchor::HeaderAnchor
pub fn leyline::profile::anchor::HeaderAnchor::hash<__H: core::hash::Hasher>(&self, state: &mut __H)
impl core::marker::Copy for leyline::profile::anchor::HeaderAnchor
impl core::marker::StructuralPartialEq for leyline::profile::anchor::HeaderAnchor
impl core::marker::Freeze for leyline::profile::anchor::HeaderAnchor
impl core::marker::Send for leyline::profile::anchor::HeaderAnchor
impl core::marker::Sync for leyline::profile::anchor::HeaderAnchor
impl core::marker::Unpin for leyline::profile::anchor::HeaderAnchor
impl core::marker::UnsafeUnpin for leyline::profile::anchor::HeaderAnchor
impl core::panic::unwind_safe::RefUnwindSafe for leyline::profile::anchor::HeaderAnchor
impl core::panic::unwind_safe::UnwindSafe for leyline::profile::anchor::HeaderAnchor
```

### `infer_anchor`

```rust,ignore
pub fn leyline::profile::anchor::infer_anchor(name: &str) -> core::option::Option<leyline::profile::anchor::HeaderAnchor>
```

## `leyline::profile::preset`

```rust,ignore
pub mod leyline::profile::preset
```

### `HeaderContext`

```rust,ignore
pub struct leyline::profile::preset::HeaderContext<'a>
pub leyline::profile::preset::HeaderContext::accept_language: &'a str
pub leyline::profile::preset::HeaderContext::firefox: bool
pub leyline::profile::preset::HeaderContext::origin: &'a str
pub leyline::profile::preset::HeaderContext::referer: &'a str
pub leyline::profile::preset::HeaderContext::sec_ch_ua: &'a str
pub leyline::profile::preset::HeaderContext::sec_ch_ua_mobile: &'a str
pub leyline::profile::preset::HeaderContext::sec_ch_ua_platform: &'a str
pub leyline::profile::preset::HeaderContext::user_agent: &'a str
impl<'a> core::marker::Freeze for leyline::profile::preset::HeaderContext<'a>
impl<'a> core::marker::Send for leyline::profile::preset::HeaderContext<'a>
impl<'a> core::marker::Sync for leyline::profile::preset::HeaderContext<'a>
impl<'a> core::marker::Unpin for leyline::profile::preset::HeaderContext<'a>
impl<'a> core::marker::UnsafeUnpin for leyline::profile::preset::HeaderContext<'a>
impl<'a> core::panic::unwind_safe::RefUnwindSafe for leyline::profile::preset::HeaderContext<'a>
impl<'a> core::panic::unwind_safe::UnwindSafe for leyline::profile::preset::HeaderContext<'a>
```

### `HeaderPair`

```rust,ignore
pub type leyline::profile::preset::HeaderPair = (alloc::borrow::Cow<'static, str>, alloc::borrow::Cow<'static, str>)
```

### `Preset`

```rust,ignore
#[non_exhaustive] pub enum leyline::profile::preset::Preset
pub leyline::profile::preset::Preset::CrossOrigin
pub leyline::profile::preset::Preset::Form
pub leyline::profile::preset::Preset::FormNavigate
pub leyline::profile::preset::Preset::Native
pub leyline::profile::preset::Preset::Navigate
pub leyline::profile::preset::Preset::SameSite
pub leyline::profile::preset::Preset::Script
pub leyline::profile::preset::Preset::Xhr
impl leyline::profile::preset::Preset
pub fn leyline::profile::preset::Preset::build_headers(&self, ctx: &leyline::profile::preset::HeaderContext<'_>) -> alloc::vec::Vec<leyline::profile::preset::HeaderPair>
impl core::clone::Clone for leyline::profile::preset::Preset
pub fn leyline::profile::preset::Preset::clone(&self) -> leyline::profile::preset::Preset
impl core::cmp::Eq for leyline::profile::preset::Preset
impl core::cmp::PartialEq for leyline::profile::preset::Preset
pub fn leyline::profile::preset::Preset::eq(&self, other: &leyline::profile::preset::Preset) -> bool
impl core::fmt::Debug for leyline::profile::preset::Preset
pub fn leyline::profile::preset::Preset::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::hash::Hash for leyline::profile::preset::Preset
pub fn leyline::profile::preset::Preset::hash<__H: core::hash::Hasher>(&self, state: &mut __H)
impl core::marker::Copy for leyline::profile::preset::Preset
impl core::marker::StructuralPartialEq for leyline::profile::preset::Preset
impl core::marker::Freeze for leyline::profile::preset::Preset
impl core::marker::Send for leyline::profile::preset::Preset
impl core::marker::Sync for leyline::profile::preset::Preset
impl core::marker::Unpin for leyline::profile::preset::Preset
impl core::marker::UnsafeUnpin for leyline::profile::preset::Preset
impl core::panic::unwind_safe::RefUnwindSafe for leyline::profile::preset::Preset
impl core::panic::unwind_safe::UnwindSafe for leyline::profile::preset::Preset
impl leyline::profile::preset::Preset
pub fn leyline::profile::preset::Preset::build_headers(&self, ctx: &leyline::profile::preset::HeaderContext<'_>) -> alloc::vec::Vec<leyline::profile::preset::HeaderPair>
impl core::clone::Clone for leyline::profile::preset::Preset
pub fn leyline::profile::preset::Preset::clone(&self) -> leyline::profile::preset::Preset
impl core::cmp::Eq for leyline::profile::preset::Preset
impl core::cmp::PartialEq for leyline::profile::preset::Preset
pub fn leyline::profile::preset::Preset::eq(&self, other: &leyline::profile::preset::Preset) -> bool
impl core::fmt::Debug for leyline::profile::preset::Preset
pub fn leyline::profile::preset::Preset::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::hash::Hash for leyline::profile::preset::Preset
pub fn leyline::profile::preset::Preset::hash<__H: core::hash::Hasher>(&self, state: &mut __H)
impl core::marker::Copy for leyline::profile::preset::Preset
impl core::marker::StructuralPartialEq for leyline::profile::preset::Preset
impl core::marker::Freeze for leyline::profile::preset::Preset
impl core::marker::Send for leyline::profile::preset::Preset
impl core::marker::Sync for leyline::profile::preset::Preset
impl core::marker::Unpin for leyline::profile::preset::Preset
impl core::marker::UnsafeUnpin for leyline::profile::preset::Preset
impl core::panic::unwind_safe::RefUnwindSafe for leyline::profile::preset::Preset
impl core::panic::unwind_safe::UnwindSafe for leyline::profile::preset::Preset
impl leyline::profile::preset::Preset
pub fn leyline::profile::preset::Preset::build_headers(&self, ctx: &leyline::profile::preset::HeaderContext<'_>) -> alloc::vec::Vec<leyline::profile::preset::HeaderPair>
impl core::clone::Clone for leyline::profile::preset::Preset
pub fn leyline::profile::preset::Preset::clone(&self) -> leyline::profile::preset::Preset
impl core::cmp::Eq for leyline::profile::preset::Preset
impl core::cmp::PartialEq for leyline::profile::preset::Preset
pub fn leyline::profile::preset::Preset::eq(&self, other: &leyline::profile::preset::Preset) -> bool
impl core::fmt::Debug for leyline::profile::preset::Preset
pub fn leyline::profile::preset::Preset::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::hash::Hash for leyline::profile::preset::Preset
pub fn leyline::profile::preset::Preset::hash<__H: core::hash::Hasher>(&self, state: &mut __H)
impl core::marker::Copy for leyline::profile::preset::Preset
impl core::marker::StructuralPartialEq for leyline::profile::preset::Preset
impl core::marker::Freeze for leyline::profile::preset::Preset
impl core::marker::Send for leyline::profile::preset::Preset
impl core::marker::Sync for leyline::profile::preset::Preset
impl core::marker::Unpin for leyline::profile::preset::Preset
impl core::marker::UnsafeUnpin for leyline::profile::preset::Preset
impl core::panic::unwind_safe::RefUnwindSafe for leyline::profile::preset::Preset
impl core::panic::unwind_safe::UnwindSafe for leyline::profile::preset::Preset
```

## `leyline::tls`

```rust,ignore
pub mod leyline::tls
```

### `ClientIdentity`

```rust,ignore
#[non_exhaustive] pub struct leyline::tls::ClientIdentity
pub leyline::tls::ClientIdentity::certificate_chain_file: std::path::PathBuf
pub leyline::tls::ClientIdentity::private_key_file: std::path::PathBuf
impl core::clone::Clone for leyline::tls::ClientIdentity
pub fn leyline::tls::ClientIdentity::clone(&self) -> leyline::tls::ClientIdentity
impl core::fmt::Debug for leyline::tls::ClientIdentity
pub fn leyline::tls::ClientIdentity::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::marker::Freeze for leyline::tls::ClientIdentity
impl core::marker::Send for leyline::tls::ClientIdentity
impl core::marker::Sync for leyline::tls::ClientIdentity
impl core::marker::Unpin for leyline::tls::ClientIdentity
impl core::marker::UnsafeUnpin for leyline::tls::ClientIdentity
impl core::panic::unwind_safe::RefUnwindSafe for leyline::tls::ClientIdentity
impl core::panic::unwind_safe::UnwindSafe for leyline::tls::ClientIdentity
```

### `HappyEyeballsConfig`

```rust,ignore
#[non_exhaustive] pub struct leyline::tls::HappyEyeballsConfig
pub leyline::tls::HappyEyeballsConfig::attempt_limit: usize
pub leyline::tls::HappyEyeballsConfig::resolve_delay: core::time::Duration
impl leyline::tls::HappyEyeballsConfig
pub fn leyline::tls::HappyEyeballsConfig::attempt_limit(self, n: usize) -> Self
pub fn leyline::tls::HappyEyeballsConfig::new() -> Self
pub fn leyline::tls::HappyEyeballsConfig::resolve_delay(self, d: core::time::Duration) -> Self
impl core::clone::Clone for leyline::tls::HappyEyeballsConfig
pub fn leyline::tls::HappyEyeballsConfig::clone(&self) -> leyline::tls::HappyEyeballsConfig
impl core::default::Default for leyline::tls::HappyEyeballsConfig
pub fn leyline::tls::HappyEyeballsConfig::default() -> Self
impl core::fmt::Debug for leyline::tls::HappyEyeballsConfig
pub fn leyline::tls::HappyEyeballsConfig::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::marker::Copy for leyline::tls::HappyEyeballsConfig
impl core::marker::Freeze for leyline::tls::HappyEyeballsConfig
impl core::marker::Send for leyline::tls::HappyEyeballsConfig
impl core::marker::Sync for leyline::tls::HappyEyeballsConfig
impl core::marker::Unpin for leyline::tls::HappyEyeballsConfig
impl core::marker::UnsafeUnpin for leyline::tls::HappyEyeballsConfig
impl core::panic::unwind_safe::RefUnwindSafe for leyline::tls::HappyEyeballsConfig
impl core::panic::unwind_safe::UnwindSafe for leyline::tls::HappyEyeballsConfig
```

### `ResolveFuture`

```rust,ignore
pub type leyline::tls::ResolveFuture<'a> = core::pin::Pin<alloc::boxed::Box<(dyn core::future::future::Future<Output = core::result::Result<alloc::vec::Vec<core::net::socket_addr::SocketAddr>, core::io::error::Error>> + core::marker::Send + 'a)>>
```

### `Resolver`

```rust,ignore
impl leyline::tls::Resolver for leyline::tls::SystemResolver
pub trait leyline::tls::Resolver: core::marker::Send + core::marker::Sync + 'static
pub fn leyline::tls::Resolver::resolve<'a>(&'a self, host: &'a str, port: u16) -> leyline::tls::ResolveFuture<'a>
impl leyline::tls::Resolver for leyline::tls::SystemResolver
```

### `SystemResolver`

```rust,ignore
pub struct leyline::tls::SystemResolver
impl core::clone::Clone for leyline::tls::SystemResolver
pub fn leyline::tls::SystemResolver::clone(&self) -> leyline::tls::SystemResolver
impl core::default::Default for leyline::tls::SystemResolver
pub fn leyline::tls::SystemResolver::default() -> leyline::tls::SystemResolver
impl core::fmt::Debug for leyline::tls::SystemResolver
pub fn leyline::tls::SystemResolver::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::marker::Copy for leyline::tls::SystemResolver
pub fn leyline::tls::SystemResolver::resolve<'a>(&'a self, host: &'a str, port: u16) -> leyline::tls::ResolveFuture<'a>
impl core::marker::Freeze for leyline::tls::SystemResolver
impl core::marker::Send for leyline::tls::SystemResolver
impl core::marker::Sync for leyline::tls::SystemResolver
impl core::marker::Unpin for leyline::tls::SystemResolver
impl core::marker::UnsafeUnpin for leyline::tls::SystemResolver
impl core::panic::unwind_safe::RefUnwindSafe for leyline::tls::SystemResolver
impl core::panic::unwind_safe::UnwindSafe for leyline::tls::SystemResolver
pub fn leyline::tls::SystemResolver::resolve<'a>(&'a self, host: &'a str, port: u16) -> leyline::tls::ResolveFuture<'a>
```

### `TlsContext`

```rust,ignore
pub struct leyline::tls::TlsContext(_)
impl leyline::tls::TlsContext
pub fn leyline::tls::TlsContext::from_profile(profile: &leyline::profile::BrowserProfile, min_version: leyline::TlsMinVersion) -> core::result::Result<Self, leyline::TlsError>
impl core::marker::Freeze for leyline::tls::TlsContext
impl core::marker::Send for leyline::tls::TlsContext
impl core::marker::Sync for leyline::tls::TlsContext
impl core::marker::Unpin for leyline::tls::TlsContext
impl core::marker::UnsafeUnpin for leyline::tls::TlsContext
impl core::panic::unwind_safe::RefUnwindSafe for leyline::tls::TlsContext
impl core::panic::unwind_safe::UnwindSafe for leyline::tls::TlsContext
impl leyline::tls::TlsContext
pub fn leyline::tls::TlsContext::from_profile(profile: &leyline::profile::BrowserProfile, min_version: leyline::TlsMinVersion) -> core::result::Result<Self, leyline::TlsError>
impl core::marker::Freeze for leyline::tls::TlsContext
impl core::marker::Send for leyline::tls::TlsContext
impl core::marker::Sync for leyline::tls::TlsContext
impl core::marker::Unpin for leyline::tls::TlsContext
impl core::marker::UnsafeUnpin for leyline::tls::TlsContext
impl core::panic::unwind_safe::RefUnwindSafe for leyline::tls::TlsContext
impl core::panic::unwind_safe::UnwindSafe for leyline::tls::TlsContext
```

### `TlsError`

```rust,ignore
#[non_exhaustive] pub enum leyline::tls::TlsError
pub leyline::tls::TlsError::Certificate(alloc::string::String)
pub leyline::tls::TlsError::Dns(core::io::error::Error)
pub leyline::tls::TlsError::Handshake(alloc::string::String)
pub leyline::tls::TlsError::HandshakeIo(core::io::error::Error)
pub leyline::tls::TlsError::Hostname(alloc::string::String)
pub leyline::tls::TlsError::Pinning(alloc::string::String)
pub leyline::tls::TlsError::Profile(alloc::string::String)
pub leyline::tls::TlsError::SslConfig(alloc::string::String)
pub leyline::tls::TlsError::SslConnect(alloc::string::String)
pub leyline::tls::TlsError::TcpConnect(core::io::error::Error)
pub leyline::tls::TlsError::TrustStore(alloc::string::String)
```

### `TlsMinVersion`

```rust,ignore
#[non_exhaustive] pub enum leyline::tls::TlsMinVersion
pub leyline::tls::TlsMinVersion::Tls10
pub leyline::tls::TlsMinVersion::Tls12
pub leyline::tls::TlsMinVersion::Tls13
```

### `TlsTrustConfig`

```rust,ignore
pub struct leyline::tls::TlsTrustConfig
```

## `leyline::trace`

```rust,ignore
pub mod leyline::trace
```

### `Connect`

```rust,ignore
#[non_exhaustive] pub struct leyline::trace::Connect<'a>
pub leyline::trace::Connect::elapsed: core::time::Duration
pub leyline::trace::Connect::host: &'a str
pub leyline::trace::Connect::id: u64
pub leyline::trace::Connect::port: u16
pub leyline::trace::Connect::reused: bool
impl<'a> core::marker::Freeze for leyline::trace::Connect<'a>
impl<'a> core::marker::Send for leyline::trace::Connect<'a>
impl<'a> core::marker::Sync for leyline::trace::Connect<'a>
impl<'a> core::marker::Unpin for leyline::trace::Connect<'a>
impl<'a> core::marker::UnsafeUnpin for leyline::trace::Connect<'a>
impl<'a> core::panic::unwind_safe::RefUnwindSafe for leyline::trace::Connect<'a>
impl<'a> core::panic::unwind_safe::UnwindSafe for leyline::trace::Connect<'a>
pub fn alloc::sync::Arc<T>::connect(&self, ev: &leyline::trace::Connect<'_>)
```

### `Dns`

```rust,ignore
#[non_exhaustive] pub struct leyline::trace::Dns<'a>
pub leyline::trace::Dns::addrs: usize
pub leyline::trace::Dns::elapsed: core::time::Duration
pub leyline::trace::Dns::host: &'a str
pub leyline::trace::Dns::id: u64
pub leyline::trace::Dns::port: u16
impl<'a> core::marker::Freeze for leyline::trace::Dns<'a>
impl<'a> core::marker::Send for leyline::trace::Dns<'a>
impl<'a> core::marker::Sync for leyline::trace::Dns<'a>
impl<'a> core::marker::Unpin for leyline::trace::Dns<'a>
impl<'a> core::marker::UnsafeUnpin for leyline::trace::Dns<'a>
impl<'a> core::panic::unwind_safe::RefUnwindSafe for leyline::trace::Dns<'a>
impl<'a> core::panic::unwind_safe::UnwindSafe for leyline::trace::Dns<'a>
pub fn alloc::sync::Arc<T>::dns(&self, ev: &leyline::trace::Dns<'_>)
```

### `Done`

```rust,ignore
#[non_exhaustive] pub struct leyline::trace::Done<'a>
pub leyline::trace::Done::elapsed: core::time::Duration
pub leyline::trace::Done::id: u64
pub leyline::trace::Done::outcome: core::result::Result<(), &'a leyline::Error>
impl<'a> core::marker::Freeze for leyline::trace::Done<'a>
impl<'a> core::marker::Send for leyline::trace::Done<'a>
impl<'a> core::marker::Sync for leyline::trace::Done<'a>
impl<'a> core::marker::Unpin for leyline::trace::Done<'a>
impl<'a> core::marker::UnsafeUnpin for leyline::trace::Done<'a>
impl<'a> !core::panic::unwind_safe::RefUnwindSafe for leyline::trace::Done<'a>
impl<'a> !core::panic::unwind_safe::UnwindSafe for leyline::trace::Done<'a>
pub fn alloc::sync::Arc<T>::done(&self, ev: &leyline::trace::Done<'_>)
```

### `Head`

```rust,ignore
#[non_exhaustive] pub struct leyline::trace::Head<'a>
pub leyline::trace::Head::elapsed: core::time::Duration
pub leyline::trace::Head::host: &'a str
pub leyline::trace::Head::id: u64
pub leyline::trace::Head::protocol: leyline::HttpVersion
pub leyline::trace::Head::status: u16
impl<'a> core::marker::Freeze for leyline::trace::Head<'a>
impl<'a> core::marker::Send for leyline::trace::Head<'a>
impl<'a> core::marker::Sync for leyline::trace::Head<'a>
impl<'a> core::marker::Unpin for leyline::trace::Head<'a>
impl<'a> core::marker::UnsafeUnpin for leyline::trace::Head<'a>
impl<'a> core::panic::unwind_safe::RefUnwindSafe for leyline::trace::Head<'a>
impl<'a> core::panic::unwind_safe::UnwindSafe for leyline::trace::Head<'a>
pub fn alloc::sync::Arc<T>::head(&self, ev: &leyline::trace::Head<'_>)
```

### `Sent`

```rust,ignore
#[non_exhaustive] pub struct leyline::trace::Sent<'a>
pub leyline::trace::Sent::elapsed: core::time::Duration
pub leyline::trace::Sent::host: &'a str
pub leyline::trace::Sent::id: u64
pub leyline::trace::Sent::protocol: leyline::HttpVersion
impl<'a> core::marker::Freeze for leyline::trace::Sent<'a>
impl<'a> core::marker::Send for leyline::trace::Sent<'a>
impl<'a> core::marker::Sync for leyline::trace::Sent<'a>
impl<'a> core::marker::Unpin for leyline::trace::Sent<'a>
impl<'a> core::marker::UnsafeUnpin for leyline::trace::Sent<'a>
impl<'a> core::panic::unwind_safe::RefUnwindSafe for leyline::trace::Sent<'a>
impl<'a> core::panic::unwind_safe::UnwindSafe for leyline::trace::Sent<'a>
pub fn alloc::sync::Arc<T>::sent(&self, ev: &leyline::trace::Sent<'_>)
```

### `Timing`

```rust,ignore
pub struct leyline::trace::Timing
impl leyline::trace::Timing
pub fn leyline::trace::Timing::new() -> Self
pub fn leyline::trace::Timing::snapshot(&self) -> leyline::ResponseTiming
impl core::default::Default for leyline::trace::Timing
pub fn leyline::trace::Timing::default() -> leyline::trace::Timing
impl core::fmt::Debug for leyline::trace::Timing
pub fn leyline::trace::Timing::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
pub fn leyline::trace::Timing::connect(&self, ev: &leyline::trace::Connect<'_>)
pub fn leyline::trace::Timing::dns(&self, ev: &leyline::trace::Dns<'_>)
pub fn leyline::trace::Timing::done(&self, ev: &leyline::trace::Done<'_>)
pub fn leyline::trace::Timing::head(&self, ev: &leyline::trace::Head<'_>)
pub fn leyline::trace::Timing::sent(&self, ev: &leyline::trace::Sent<'_>)
pub fn leyline::trace::Timing::tls(&self, ev: &leyline::trace::Tls<'_>)
impl !core::marker::Freeze for leyline::trace::Timing
impl core::marker::Send for leyline::trace::Timing
impl core::marker::Sync for leyline::trace::Timing
impl core::marker::Unpin for leyline::trace::Timing
impl core::marker::UnsafeUnpin for leyline::trace::Timing
impl core::panic::unwind_safe::RefUnwindSafe for leyline::trace::Timing
impl core::panic::unwind_safe::UnwindSafe for leyline::trace::Timing
pub fn leyline::trace::Timing::connect(&self, ev: &leyline::trace::Connect<'_>)
pub fn leyline::trace::Timing::dns(&self, ev: &leyline::trace::Dns<'_>)
pub fn leyline::trace::Timing::done(&self, ev: &leyline::trace::Done<'_>)
pub fn leyline::trace::Timing::head(&self, ev: &leyline::trace::Head<'_>)
pub fn leyline::trace::Timing::sent(&self, ev: &leyline::trace::Sent<'_>)
pub fn leyline::trace::Timing::tls(&self, ev: &leyline::trace::Tls<'_>)
```

### `Tls`

```rust,ignore
#[non_exhaustive] pub struct leyline::trace::Tls<'a>
pub leyline::trace::Tls::alpn: core::option::Option<&'a str>
pub leyline::trace::Tls::cipher: core::option::Option<&'a str>
pub leyline::trace::Tls::elapsed: core::time::Duration
pub leyline::trace::Tls::host: &'a str
pub leyline::trace::Tls::id: u64
pub leyline::trace::Tls::version: core::option::Option<&'a str>
impl<'a> core::marker::Freeze for leyline::trace::Tls<'a>
impl<'a> core::marker::Send for leyline::trace::Tls<'a>
impl<'a> core::marker::Sync for leyline::trace::Tls<'a>
impl<'a> core::marker::Unpin for leyline::trace::Tls<'a>
impl<'a> core::marker::UnsafeUnpin for leyline::trace::Tls<'a>
impl<'a> core::panic::unwind_safe::RefUnwindSafe for leyline::trace::Tls<'a>
impl<'a> core::panic::unwind_safe::UnwindSafe for leyline::trace::Tls<'a>
pub fn alloc::sync::Arc<T>::tls(&self, ev: &leyline::trace::Tls<'_>)
```

### `Trace`

```rust,ignore
impl leyline::trace::Trace for leyline::trace::Timing
impl leyline::trace::Trace for leyline::trace::TracingTrace
pub trait leyline::trace::Trace: core::marker::Send + core::marker::Sync + 'static
pub fn leyline::trace::Trace::connect(&self, ev: &leyline::trace::Connect<'_>)
pub fn leyline::trace::Trace::dns(&self, ev: &leyline::trace::Dns<'_>)
pub fn leyline::trace::Trace::done(&self, ev: &leyline::trace::Done<'_>)
pub fn leyline::trace::Trace::head(&self, ev: &leyline::trace::Head<'_>)
pub fn leyline::trace::Trace::sent(&self, ev: &leyline::trace::Sent<'_>)
pub fn leyline::trace::Trace::tls(&self, ev: &leyline::trace::Tls<'_>)
impl leyline::trace::Trace for leyline::trace::Timing
impl leyline::trace::Trace for leyline::trace::TracingTrace
impl<T: leyline::trace::Trace> leyline::trace::Trace for alloc::sync::Arc<T>
```

### `TracingTrace`

```rust,ignore
pub struct leyline::trace::TracingTrace
impl core::clone::Clone for leyline::trace::TracingTrace
pub fn leyline::trace::TracingTrace::clone(&self) -> leyline::trace::TracingTrace
impl core::default::Default for leyline::trace::TracingTrace
pub fn leyline::trace::TracingTrace::default() -> leyline::trace::TracingTrace
impl core::fmt::Debug for leyline::trace::TracingTrace
pub fn leyline::trace::TracingTrace::fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result
impl core::marker::Copy for leyline::trace::TracingTrace
pub fn leyline::trace::TracingTrace::connect(&self, ev: &leyline::trace::Connect<'_>)
pub fn leyline::trace::TracingTrace::dns(&self, ev: &leyline::trace::Dns<'_>)
pub fn leyline::trace::TracingTrace::done(&self, ev: &leyline::trace::Done<'_>)
pub fn leyline::trace::TracingTrace::head(&self, ev: &leyline::trace::Head<'_>)
pub fn leyline::trace::TracingTrace::sent(&self, ev: &leyline::trace::Sent<'_>)
pub fn leyline::trace::TracingTrace::tls(&self, ev: &leyline::trace::Tls<'_>)
impl core::marker::Freeze for leyline::trace::TracingTrace
impl core::marker::Send for leyline::trace::TracingTrace
impl core::marker::Sync for leyline::trace::TracingTrace
impl core::marker::Unpin for leyline::trace::TracingTrace
impl core::marker::UnsafeUnpin for leyline::trace::TracingTrace
impl core::panic::unwind_safe::RefUnwindSafe for leyline::trace::TracingTrace
impl core::panic::unwind_safe::UnwindSafe for leyline::trace::TracingTrace
pub fn leyline::trace::TracingTrace::connect(&self, ev: &leyline::trace::Connect<'_>)
pub fn leyline::trace::TracingTrace::dns(&self, ev: &leyline::trace::Dns<'_>)
pub fn leyline::trace::TracingTrace::done(&self, ev: &leyline::trace::Done<'_>)
pub fn leyline::trace::TracingTrace::head(&self, ev: &leyline::trace::Head<'_>)
pub fn leyline::trace::TracingTrace::sent(&self, ev: &leyline::trace::Sent<'_>)
pub fn leyline::trace::TracingTrace::tls(&self, ev: &leyline::trace::Tls<'_>)
```
