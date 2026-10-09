# Module `leyline`

| Item | Kind | Description |
| --- | --- | --- |
| [`Body`](#body) | struct |  |
| [`Error`](#error) | struct |  |
| [`BlockRules`](#blockrules) | struct |  |
| [`BlockSignal`](#blocksignal) | struct |  |
| [`BodyStream`](#bodystream) | struct |  |
| [`BrowserProfile`](#browserprofile) | struct |  |
| [`CloseFrame`](#closeframe) | struct | Requires feature `websocket`. |
| [`CompressionConfig`](#compressionconfig) | struct |  |
| [`Device`](#device) | struct |  |
| [`DeviceAutosave`](#deviceautosave) | struct |  |
| [`DeviceAutosaveOptions`](#deviceautosaveoptions) | struct |  |
| [`DigestAuth`](#digestauth) | struct |  |
| [`DnsConfig`](#dnsconfig) | struct |  |
| [`HostLimits`](#hostlimits) | struct |  |
| [`HostStats`](#hoststats) | struct |  |
| [`Identity`](#identity) | struct |  |
| [`LeylineService`](#leylineservice) | struct | Requires feature `tower`. |
| [`Link`](#link) | struct |  |
| [`NoProxy`](#noproxy) | struct |  |
| [`Pages`](#pages) | struct |  |
| [`PoolConfig`](#poolconfig) | struct |  |
| [`PoolStats`](#poolstats) | struct |  |
| [`PrefixRead`](#prefixread) | struct |  |
| [`ProxyConfig`](#proxyconfig) | struct |  |
| [`ProxyHealth`](#proxyhealth) | struct |  |
| [`ProxyPool`](#proxypool) | struct |  |
| [`ProxyRule`](#proxyrule) | struct |  |
| [`ProxyUrl`](#proxyurl) | struct |  |
| [`RedirectAttempt`](#redirectattempt) | struct |  |
| [`RedirectPolicy`](#redirectpolicy) | struct |  |
| [`RequestBuilder`](#requestbuilder) | struct |  |
| [`Response`](#response) | struct |  |
| [`ResponseTiming`](#responsetiming) | struct |  |
| [`RetryPolicy`](#retrypolicy) | struct |  |
| [`Session`](#session) | struct |  |
| [`SessionBuilder`](#sessionbuilder) | struct |  |
| [`SessionIdentity`](#sessionidentity) | struct |  |
| [`SessionState`](#sessionstate) | struct |  |
| [`SocketConfig`](#socketconfig) | struct |  |
| [`Tab`](#tab) | struct |  |
| [`TcpProfile`](#tcpprofile) | struct |  |
| [`TimeoutConfig`](#timeoutconfig) | struct |  |
| [`TlsInfo`](#tlsinfo) | struct |  |
| [`TlsTrustConfig`](#tlstrustconfig) | struct |  |
| [`WebSocketBuilder`](#websocketbuilder) | struct | Requires feature `websocket`. |
| [`WebSocketConfig`](#websocketconfig) | struct |  |
| [`WsConnection`](#wsconnection) | struct | Requires feature `websocket`. |
| [`WsSink`](#wssink) | struct | Requires feature `websocket`. |
| [`WsStream`](#wsstream) | struct | Requires feature `websocket`. |
| [`BlockKind`](#blockkind) | enum |  |
| [`Browser`](#browser) | enum |  |
| [`ChromiumBrand`](#chromiumbrand) | enum |  |
| [`ContentEncoding`](#contentencoding) | enum |  |
| [`ErrorCategory`](#errorcategory) | enum |  |
| [`ErrorCode`](#errorcode) | enum |  |
| [`Family`](#family) | enum |  |
| [`FetchSite`](#fetchsite) | enum |  |
| [`H2Error`](#h2error) | enum |  |
| [`HeaderAnchor`](#headeranchor) | enum |  |
| [`HttpVersion`](#httpversion) | enum |  |
| [`Kind`](#kind) | enum |  |
| [`Platform`](#platform) | enum |  |
| [`Preset`](#preset) | enum |  |
| [`ProtocolPolicy`](#protocolpolicy) | enum |  |
| [`ProxyReply`](#proxyreply) | enum |  |
| [`RedirectAction`](#redirectaction) | enum |  |
| [`RelayBody`](#relaybody) | enum |  |
| [`RetryTrigger`](#retrytrigger) | enum |  |
| [`StopReason`](#stopreason) | enum |  |
| [`TlsError`](#tlserror) | enum |  |
| [`TlsMinVersion`](#tlsminversion) | enum |  |
| [`WaitFormat`](#waitformat) | enum |  |
| [`WsMessage`](#wsmessage) | enum | Requires feature `websocket`. |
| [`IntoParamPair`](#intoparampair) | trait |  |
| [`IntoUrl`](#intourl) | trait | URL input. |
| [`get`](#get) | fn | Send one GET. |
| [`redact_url`](#redact_url) | fn |  |
| [`relay_headers`](#relay_headers) | fn | Relay a header map. |
| [`Result`](#result) | type |  |
| [`Url`](#url) | use |  |
| [`http`](#http) | use |  |
| [`alloc::sync::Arc`](#allocsyncarc) | impl |  |
| [`alloc::string::String`](#allocstringstring) | impl |  |
| [`(K, V)`](#k-v) | impl |  |
| [`str`](#str) | impl |  |
| [`url::Url`](#urlurl) | impl |  |

## Structs

### `Body`

**Methods**

| Method | Description |
| --- | --- |
| <code>len_hint(&amp;self) -&gt; Option&lt;u64&gt;</code> |  |
| <code>stream&lt;S&gt;(stream: S, length: Option&lt;u64&gt;) -&gt; Self where S: Stream&lt;Item = Result&lt;Bytes&gt;&gt; + Send + 'static</code> |  |

**Trait implementations:** <code>From&lt;<a href="leyline-multipart.html#form">Form</a>&gt;</code>, <code>From&lt;&amp;'static [u8]&gt;</code>, <code>From&lt;&amp;'static str&gt;</code>, <code>From&lt;()&gt;</code>, <code>From&lt;String&gt;</code>, <code>From&lt;Vec&lt;u8&gt;&gt;</code>, <code>From&lt;Bytes&gt;</code>, <code>Default</code>, <code>Debug</code>, <code>Stream</code>


### `Error`

**Methods**

| Method | Description |
| --- | --- |
| <code>attempts(&amp;self) -&gt; u32</code> |  |
| <code>body(&amp;self) -&gt; Option&lt;&amp;[u8]&gt;</code> |  |
| <code>body_text(&amp;self) -&gt; Option&lt;Cow&lt;'_, str&gt;&gt;</code> |  |
| <code>find&lt;'a&gt;(error: &amp;'a (dyn Error + 'static)) -&gt; Option&lt;&amp;'a <a href="#error">Error</a>&gt;</code> | Find a leyline error in a chain. |
| <code>h2(&amp;self) -&gt; Option&lt;&amp;<a href="#h2error">H2Error</a>&gt;</code> |  |
| <code>header(&amp;self, name: &amp;str) -&gt; Option&lt;&amp;str&gt;</code> |  |
| <code>headers(&amp;self) -&gt; Option&lt;&amp;HeaderMap&gt;</code> |  |
| <code>io(&amp;self) -&gt; Option&lt;&amp;Error&gt;</code> |  |
| <code>kind(&amp;self) -&gt; <a href="#kind">Kind</a></code> | Errors. |
| <code>proxy(&amp;self) -&gt; Option&lt;&amp;str&gt;</code> |  |
| <code>retries_exhausted(&amp;self) -&gt; bool</code> |  |
| <code>retry_after(&amp;self) -&gt; Option&lt;Duration&gt;</code> |  |
| <code>status(&amp;self) -&gt; Option&lt;StatusCode&gt;</code> |  |
| <code>tls(&amp;self) -&gt; Option&lt;&amp;<a href="#tlserror">TlsError</a>&gt;</code> |  |
| <code>url(&amp;self) -&gt; Option&lt;&amp;Url&gt;</code> |  |
| <code>category(&amp;self) -&gt; <a href="#errorcategory">ErrorCategory</a></code> | Error category. |
| <code>is_dns(&amp;self) -&gt; bool</code> | DNS failure. |
| <code>is_body_limit(&amp;self) -&gt; bool</code> | Body limit. |
| <code>is_connect(&amp;self) -&gt; bool</code> | Connect failure. |
| <code>is_proxy(&amp;self) -&gt; bool</code> | Proxy failure. |
| <code>is_retryable(&amp;self) -&gt; bool</code> | Retryable error. |
| <code>is_status(&amp;self) -&gt; bool</code> | Status error. |
| <code>is_timeout(&amp;self) -&gt; bool</code> | Timeout. |
| <code>is_profile_changed(&amp;self) -&gt; bool</code> |  |
| <code>is_shut_down(&amp;self) -&gt; bool</code> | Detect a stopped session. |

**Trait implementations:** <code>From&lt;<a href="#h2error">H2Error</a>&gt;</code>, <code>From&lt;<a href="#tlserror">TlsError</a>&gt;</code>, <code>From&lt;Error&gt;</code>, <code>From&lt;InvalidHeaderName&gt;</code>, <code>From&lt;InvalidHeaderValue&gt;</code>, <code>From&lt;InvalidUri&gt;</code>, <code>From&lt;ParseError&gt;</code>, <code>Error</code>, <code>Debug</code>, <code>Display</code>


### `BlockRules`

**Methods**

| Method | Description |
| --- | --- |
| <code>builtin() -&gt; &amp;'static <a href="#blockrules">BlockRules</a></code> |  |
| <code>check(&amp;self, response: &amp;<a href="#response">Response</a>) -&gt; Option&lt;<a href="#blocksignal">BlockSignal</a>&gt;</code> |  |
| <code>extend(&amp;mut self, other: <a href="#blockrules">BlockRules</a>)</code> |  |
| <code>from_toml(source: &amp;str) -&gt; <a href="#result">Result</a>&lt;Self&gt;</code> | Load block rules from TOML. |
| <code>statuses(statuses: impl IntoIterator&lt;Item = u16&gt;) -&gt; Self</code> | Block rules. |

**Trait implementations:** <code>Clone</code>, <code>Default</code>, <code>Debug</code>


### `BlockSignal`

**Fields**

| Field | Description |
| --- | --- |
| <code>kind: <a href="#blockkind">BlockKind</a></code> |  |
| <code>vendor: String</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Debug</code>, <code>StructuralPartialEq</code>


### `BodyStream`

**Trait implementations:** <code>Debug</code>, <code>Stream</code>


### `BrowserProfile`

**Methods**

| Method | Description |
| --- | --- |
| <code>expected_h2_fingerprint(&amp;self) -&gt; Option&lt;&amp;str&gt;</code> |  |
| <code>expected_ja4(&amp;self) -&gt; Option&lt;&amp;str&gt;</code> |  |
| <code>from_toml(toml_str: &amp;str) -&gt; Result&lt;Self, <a href="leyline-profile.html#profileerror">ProfileError</a>&gt;</code> | Parse a profile from TOML. |
| <code>from_fingerprint(spec: <a href="leyline-profile.html#fingerprintspec">FingerprintSpec</a>) -&gt; <a href="#result">Result</a>&lt;Self&gt;</code> |  |

**Fields**

| Field | Description |
| --- | --- |
| <code>h2: <a href="leyline-profile.html#h2profile">H2Profile</a></code> |  |
| <code>h3: Option&lt;<a href="leyline-profile.html#h3profile">H3Profile</a>&gt;</code> |  |
| <code>identity: HashMap&lt;String, <a href="leyline-profile.html#platformidentity">PlatformIdentity</a>&gt;</code> |  |
| <code>meta: <a href="leyline-profile.html#profilemeta">ProfileMeta</a></code> |  |
| <code>tls: <a href="leyline-profile.html#tlsprofile">TlsProfile</a></code> |  |

**Trait implementations:** <code>Clone</code>, <code>Debug</code>, <code>Deserialize&lt;'de&gt;</code>


### `CloseFrame`

Requires feature `websocket`.

**Methods**

| Method | Description |
| --- | --- |
| <code>new(code: u16, reason: impl Into&lt;String&gt;) -&gt; Self</code> | Requires feature `websocket`. |

**Fields**

| Field | Description |
| --- | --- |
| <code>code: u16</code> | Requires feature `websocket`. |
| <code>reason: String</code> | Requires feature `websocket`. |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Debug</code>, <code>StructuralPartialEq</code>


### `CompressionConfig`

**Methods**

| Method | Description |
| --- | --- |
| <code>brotli(self, on: bool) -&gt; Self</code> |  |
| <code>deflate(self, on: bool) -&gt; Self</code> |  |
| <code>gzip(self, on: bool) -&gt; Self</code> |  |
| <code>max_body_size(self, bytes: usize) -&gt; Self</code> | Response body cap. |
| <code>max_error_body(self, bytes: usize) -&gt; Self</code> | Status-error body cap. |
| <code>new() -&gt; Self</code> |  |
| <code>none() -&gt; Self</code> |  |
| <code>zstd(self, on: bool) -&gt; Self</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Default</code>, <code>Debug</code>, <code>Copy</code>, <code>StructuralPartialEq</code>


### `Device`

**Methods**

| Method | Description |
| --- | --- |
| <code>autosave(&amp;self, session: &amp;<a href="#session">Session</a>, path: impl Into&lt;PathBuf&gt;, options: impl Into&lt;<a href="#deviceautosaveoptions">DeviceAutosaveOptions</a>&gt;) -&gt; <a href="#deviceautosave">DeviceAutosave</a></code> | Save a device as it changes. |
| <code>capture(session: &amp;<a href="#session">Session</a>, proxy: Option&lt;<a href="#proxyurl">ProxyUrl</a>&gt;) -&gt; <a href="#device">Device</a></code> | Capture a device. |
| <code>load_from(path: impl AsRef&lt;Path&gt;) -&gt; <a href="#result">Result</a>&lt;<a href="#device">Device</a>&gt;</code> |  |
| <code>open(&amp;self) -&gt; <a href="#result">Result</a>&lt;<a href="#session">Session</a>&gt;</code> | Open a saved device. |
| <code>pin_profile(&amp;mut self, session: &amp;<a href="#session">Session</a>) -&gt; <a href="#result">Result</a>&lt;()&gt;</code> |  |
| <code>save_to(&amp;self, path: impl AsRef&lt;Path&gt;) -&gt; <a href="#result">Result</a>&lt;()&gt;</code> |  |
| <code>session_builder(&amp;self) -&gt; <a href="#result">Result</a>&lt;<a href="#sessionbuilder">SessionBuilder</a>&gt;</code> |  |
| <code>tab(&amp;self, session: &amp;<a href="#session">Session</a>) -&gt; <a href="#tab">Tab</a></code> |  |
| <code>check(&amp;self, session: &amp;<a href="#session">Session</a>) -&gt; <a href="#result">Result</a>&lt;()&gt;</code> | Check a session against a device. |

**Fields**

| Field | Description |
| --- | --- |
| <code>app: BTreeMap&lt;String, Value&gt;</code> |  |
| <code>brand: Option&lt;<a href="#chromiumbrand">ChromiumBrand</a>&gt;</code> |  |
| <code>env_proxy: bool</code> |  |
| <code>identity: Option&lt;<a href="#identity">Identity</a>&gt;</code> |  |
| <code>jar: Option&lt;<a href="leyline-cookie.html#jar">Jar</a>&gt;</code> |  |
| <code>jar_path: Option&lt;PathBuf&gt;</code> |  |
| <code>languages: Option&lt;Vec&lt;String&gt;&gt;</code> |  |
| <code>page: Option&lt;Url&gt;</code> |  |
| <code>platform: <a href="#platform">Platform</a></code> |  |
| <code>profile_id: Option&lt;String&gt;</code> |  |
| <code>profile_toml: Option&lt;String&gt;</code> |  |
| <code>proxy: Option&lt;<a href="#proxyurl">ProxyUrl</a>&gt;</code> |  |
| <code>proxy_password_env: Option&lt;String&gt;</code> |  |
| <code>state: <a href="#sessionstate">SessionState</a></code> |  |
| <code>strict: bool</code> |  |
| <code>user_agent: Option&lt;String&gt;</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Debug</code>, <code>Serialize</code>, <code>Deserialize&lt;'de&gt;</code>


### `DeviceAutosave`

**Methods**

| Method | Description |
| --- | --- |
| <code>device(&amp;self) -&gt; <a href="#device">Device</a></code> |  |
| <code>async flush(&amp;self) -&gt; <a href="#result">Result</a>&lt;()&gt;</code> |  |
| <code>async shutdown(self) -&gt; <a href="#result">Result</a>&lt;()&gt;</code> |  |
| <code>track(&amp;self, tab: &amp;<a href="#tab">Tab</a>)</code> |  |
| <code>update(&amp;self, change: impl FnOnce(&amp;mut <a href="#device">Device</a>))</code> |  |

**Trait implementations:** <code>Debug</code>


### `DeviceAutosaveOptions`

**Methods**

| Method | Description |
| --- | --- |
| <code>const new(interval: Duration) -&gt; Self</code> | Device save intervals. |
| <code>const state_interval(self, state_interval: Duration) -&gt; Self</code> |  |

**Fields**

| Field | Description |
| --- | --- |
| <code>interval: Duration</code> |  |
| <code>state_interval: Duration</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>From&lt;Duration&gt;</code>, <code>Debug</code>, <code>Copy</code>, <code>StructuralPartialEq</code>


### `DigestAuth`

**Methods**

| Method | Description |
| --- | --- |
| <code>new(username: impl Into&lt;String&gt;, password: impl Into&lt;String&gt;) -&gt; Self</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Debug</code>


### `DnsConfig`

**Methods**

| Method | Description |
| --- | --- |
| <code>new() -&gt; Self</code> |  |
| <code>resolve_host(self, host: impl AsRef&lt;str&gt;, addrs: impl IntoIterator&lt;Item = SocketAddr&gt;) -&gt; Self</code> |  |
| <code>resolver(self, resolver: Arc&lt;dyn <a href="leyline-tls.html#resolver">Resolver</a>&gt;) -&gt; Self</code> |  |

**Trait implementations:** <code>Clone</code>, <code>From&lt;Arc&lt;dyn <a href="leyline-tls.html#resolver">Resolver</a>&gt;&gt;</code>, <code>Default</code>, <code>Debug</code>


### `HostLimits`

**Methods**

| Method | Description |
| --- | --- |
| <code>host(self, host: &amp;str, limits: <a href="#hostlimits">HostLimits</a>) -&gt; Self</code> | Limits for one host. |
| <code>max_in_flight(self, n: usize) -&gt; Self</code> | Requests in flight per host. |
| <code>max_pause(self, max: Duration) -&gt; Self</code> | Cap a server-requested pause. |
| <code>max_total_in_flight(self, n: usize) -&gt; Self</code> | Requests in flight in total. |
| <code>new() -&gt; Self</code> |  |
| <code>pause_for(self, default: Duration) -&gt; Self</code> | Default pause length. |
| <code>pause_on(self, statuses: impl IntoIterator&lt;Item = u16&gt;) -&gt; Self</code> | Pause a host on a status. |
| <code>per_second(self, rate: f64) -&gt; Self</code> | Requests per second per host. |

**Trait implementations:** <code>Clone</code>, <code>Default</code>, <code>Debug</code>


### `HostStats`

**Methods**

| Method | Description |
| --- | --- |
| <code>in_flight(&amp;self) -&gt; usize</code> |  |
| <code>origin(&amp;self) -&gt; &amp;str</code> |  |
| <code>waiting(&amp;self) -&gt; usize</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Debug</code>, <code>StructuralPartialEq</code>


### `Identity`

**Methods**

| Method | Description |
| --- | --- |
| <code>brand(self) -&gt; Option&lt;<a href="#chromiumbrand">ChromiumBrand</a>&gt;</code> |  |
| <code>http(self) -&gt; <a href="#browser">Browser</a></code> |  |
| <code>locked(browser: <a href="#browser">Browser</a>, platform: <a href="#platform">Platform</a>) -&gt; Self</code> |  |
| <code>platform(self) -&gt; <a href="#platform">Platform</a></code> |  |
| <code>rotate_hello(self) -&gt; <a href="#result">Result</a>&lt;Self&gt;</code> |  |
| <code>rotate_tls(self, tls: <a href="#browser">Browser</a>) -&gt; <a href="#result">Result</a>&lt;Self&gt;</code> | Rotate the TLS side of an identity. |
| <code>switch_family(self, dest: <a href="#browser">Browser</a>) -&gt; <a href="#result">Result</a>&lt;Self&gt;</code> |  |
| <code>tls(self) -&gt; <a href="#browser">Browser</a></code> |  |
| <code>with_brand(self, brand: <a href="#chromiumbrand">ChromiumBrand</a>) -&gt; Self</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Debug</code>, <code>Hash</code>, <code>Copy</code>, <code>StructuralPartialEq</code>, <code>Serialize</code>, <code>Deserialize&lt;'de&gt;</code>


### `LeylineService`

Requires feature `tower`.

**Methods**

| Method | Description |
| --- | --- |
| <code>new(session: <a href="#session">Session</a>) -&gt; Self</code> | tower. Requires feature `tower`. |

**Trait implementations:** <code>Clone</code>, <code>Debug</code>, <code>Service&lt;Request&lt;<a href="#body">Body</a>&gt;&gt;</code>


### `Link`

**Methods**

| Method | Description |
| --- | --- |
| <code>has_rel(&amp;self, rel: &amp;str) -&gt; bool</code> |  |

**Fields**

| Field | Description |
| --- | --- |
| <code>params: Vec&lt;(String, String)&gt;</code> |  |
| <code>rel: Vec&lt;String&gt;</code> |  |
| <code>url: Url</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Debug</code>, <code>StructuralPartialEq</code>


### `NoProxy`

**Methods**

| Method | Description |
| --- | --- |
| <code>new&lt;I, S&gt;(patterns: I) -&gt; Self where I: IntoIterator&lt;Item = S&gt;, S: Into&lt;String&gt;</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Default</code>, <code>Debug</code>, <code>StructuralPartialEq</code>


### `Pages`

**Methods**

| Method | Description |
| --- | --- |
| <code>limit(self, max: usize) -&gt; Self</code> | Limit pagination. |
| <code>async next(&amp;mut self) -&gt; Option&lt;<a href="#result">Result</a>&lt;<a href="#response">Response</a>&gt;&gt;</code> |  |

**Trait implementations:** <code>Debug</code>, <code>Stream</code>


### `PoolConfig`

**Methods**

| Method | Description |
| --- | --- |
| <code>h2_ping_after_idle(self, d: impl Into&lt;Option&lt;Duration&gt;&gt;) -&gt; Self</code> |  |
| <code>h2_ping_timeout(self, d: Duration) -&gt; Self</code> |  |
| <code>idle_timeout(self, d: Duration) -&gt; Self</code> |  |
| <code>keepalive(self, on: bool) -&gt; Self</code> |  |
| <code>max_connections(self, n: usize) -&gt; Self</code> |  |
| <code>max_h1_conns_per_host(self, n: usize) -&gt; Self</code> |  |
| <code>new() -&gt; Self</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Default</code>, <code>Debug</code>, <code>Copy</code>, <code>StructuralPartialEq</code>


### `PoolStats`

**Fields**

| Field | Description |
| --- | --- |
| <code>busy: usize</code> |  |
| <code>entries: usize</code> |  |
| <code>evictions_dead: u64</code> |  |
| <code>evictions_idle: u64</code> |  |
| <code>evictions_lru: u64</code> |  |
| <code>h1_hits: u64</code> |  |
| <code>h1_misses: u64</code> |  |
| <code>h2_hits: u64</code> |  |
| <code>h2_misses: u64</code> |  |
| <code>h2_ping_failures: u64</code> |  |
| <code>h3_hits: u64</code> |  |
| <code>h3_misses: u64</code> |  |
| <code>idle: usize</code> |  |
| <code>installs: u64</code> |  |
| <code>max_connections: usize</code> |  |
| <code>stale_probed: u64</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Default</code>, <code>Debug</code>, <code>Copy</code>, <code>StructuralPartialEq</code>


### `PrefixRead`

**Fields**

| Field | Description |
| --- | --- |
| <code>bytes: Vec&lt;u8&gt;</code> |  |
| <code>stopped_by: <a href="#stopreason">StopReason</a></code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Debug</code>, <code>StructuralPartialEq</code>


### `ProxyConfig`

**Methods**

| Method | Description |
| --- | --- |
| <code>env(self, on: bool) -&gt; Self</code> |  |
| <code>new() -&gt; Self</code> |  |
| <code>no_proxy(self, no_proxy: <a href="#noproxy">NoProxy</a>) -&gt; Self</code> |  |
| <code>rule(self, rule: <a href="#proxyrule">ProxyRule</a>) -&gt; Self</code> |  |

**Trait implementations:** <code>Clone</code>, <code>From&lt;&amp;String&gt;</code>, <code>From&lt;&amp;str&gt;</code>, <code>From&lt;String&gt;</code>, <code>From&lt;<a href="#proxyurl">ProxyUrl</a>&gt;</code>, <code>Default</code>, <code>Debug</code>


### `ProxyHealth`

**Fields**

| Field | Description |
| --- | --- |
| <code>banned_until: Option&lt;Instant&gt;</code> |  |
| <code>failures: u32</code> |  |
| <code>in_use: u32</code> |  |
| <code>proxy: String</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Debug</code>


### `ProxyPool`

**Methods**

| Method | Description |
| --- | --- |
| <code>ban_after(self, failures: u32) -&gt; Self</code> | Ban a failing proxy. |
| <code>ban_for(self, duration: Duration) -&gt; Self</code> |  |
| <code>identified&lt;P: Into&lt;<a href="#proxyconfig">ProxyConfig</a>&gt;&gt;(proxies: impl IntoIterator&lt;Item = (P, <a href="#identity">Identity</a>)&gt;) -&gt; Self</code> | Pin an identity to each proxy. |
| <code>new&lt;P: Into&lt;<a href="#proxyconfig">ProxyConfig</a>&gt;&gt;(proxies: impl IntoIterator&lt;Item = P&gt;) -&gt; Self</code> | Build a proxy pool. |
| <code>rotate_on_block(self, rules: <a href="#blockrules">BlockRules</a>) -&gt; Self</code> | Rotate the proxy on a block. |
| <code>stats(&amp;self) -&gt; Vec&lt;<a href="#proxyhealth">ProxyHealth</a>&gt;</code> | Proxy health. |
| <code>sticky_for(self, duration: Duration) -&gt; Self</code> | Keep a proxy for a time. |

**Trait implementations:** <code>Clone</code>, <code>Debug</code>


### `ProxyRule`

**Methods**

| Method | Description |
| --- | --- |
| <code>all(proxy_url: impl Into&lt;String&gt;) -&gt; Self</code> |  |
| <code>http(proxy_url: impl Into&lt;String&gt;) -&gt; Self</code> |  |
| <code>https(proxy_url: impl Into&lt;String&gt;) -&gt; Self</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Debug</code>, <code>StructuralPartialEq</code>


### `ProxyUrl`

**Methods**

| Method | Description |
| --- | --- |
| <code>parse(raw: impl AsRef&lt;str&gt;) -&gt; <a href="#result">Result</a>&lt;Self&gt;</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>TryFrom&lt;String&gt;</code>, <code>Debug</code>, <code>Display</code>, <code>Hash</code>, <code>StructuralPartialEq</code>, <code>Serialize</code>, <code>Deserialize&lt;'de&gt;</code>


### `RedirectAttempt`

**Fields**

| Field | Description |
| --- | --- |
| <code>location: Option&lt;&amp;'a str&gt;</code> |  |
| <code>previous: &amp;'a [Url]</code> |  |
| <code>status: u16</code> |  |
| <code>url: &amp;'a Url</code> |  |

**Trait implementations:** <code>Debug</code>, <code>Clone</code>, <code>Copy</code>


### `RedirectPolicy`

**Methods**

| Method | Description |
| --- | --- |
| <code>custom&lt;F&gt;(f: F) -&gt; Self where F: Fn(<a href="#redirectattempt">RedirectAttempt</a>&lt;'_&gt;) -&gt; <a href="#redirectaction">RedirectAction</a> + Send + Sync + 'static</code> |  |
| <code>limited(max: usize) -&gt; Self</code> |  |
| <code>none() -&gt; Self</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Default</code>, <code>Debug</code>


### `RequestBuilder`

**Methods**

| Method | Description |
| --- | --- |
| <code>anchored(self, anchor: <a href="#headeranchor">HeaderAnchor</a>, name: impl TryInto&lt;HeaderName&gt;, value: impl TryInto&lt;HeaderValue&gt;) -&gt; Self</code> |  |
| <code>basic_auth(self, username: &amp;str, password: &amp;str) -&gt; Self</code> | Basic auth. |
| <code>bearer_auth(self, token: &amp;str) -&gt; Self</code> | Bearer auth. |
| <code>body(self, body: impl Into&lt;<a href="#body">Body</a>&gt;) -&gt; Self</code> | Raw or streamed body. |
| <code>compress(self, encoding: <a href="#contentencoding">ContentEncoding</a>) -&gt; Self</code> | Compress a request body. |
| <code>digest_auth(self, auth: <a href="#digestauth">DigestAuth</a>) -&gt; Self</code> | Digest auth. |
| <code>form&lt;I, P&gt;(self, params: I) -&gt; Self where I: IntoIterator&lt;Item = P&gt;, P: <a href="#intoparampair">IntoParamPair</a></code> | Form body. |
| <code>header(self, name: impl TryInto&lt;HeaderName&gt;, value: impl TryInto&lt;HeaderValue&gt;) -&gt; Self</code> | Headers. |
| <code>header_order(self, order: &amp;[&amp;str]) -&gt; Self</code> | Header order. |
| <code>headers&lt;I, P&gt;(self, headers: I) -&gt; Self where I: IntoIterator&lt;Item = P&gt;, P: <a href="#intoparampair">IntoParamPair</a></code> |  |
| <code>json(self, value: &amp;impl Serialize) -&gt; Self</code> | JSON body. |
| <code>multipart(self, form: <a href="leyline-multipart.html#form">Form</a>) -&gt; Self</code> | Multipart body. Requires feature `multipart`. |
| <code>preset(self, preset: <a href="#preset">Preset</a>) -&gt; Self</code> | Set the fetch context. |
| <code>proxy(self, config: impl Into&lt;<a href="#proxyconfig">ProxyConfig</a>&gt;) -&gt; Self</code> | Proxy one request. |
| <code>query&lt;I, P&gt;(self, params: I) -&gt; Self where I: IntoIterator&lt;Item = P&gt;, P: <a href="#intoparampair">IntoParamPair</a></code> | Query. |
| <code>redirect(self, policy: <a href="#redirectpolicy">RedirectPolicy</a>) -&gt; Self</code> |  |
| <code>retry(self, policy: <a href="#retrypolicy">RetryPolicy</a>) -&gt; Self</code> |  |
| <code>stream(self) -&gt; Self</code> | Stream a response. |
| <code>timeout(self, config: impl Into&lt;<a href="#timeoutconfig">TimeoutConfig</a>&gt;) -&gt; Self</code> | Request timeout. |
| <code>cookie_jar(self, jar: <a href="leyline-cookie.html#jar">Jar</a>) -&gt; Self</code> |  |
| <code>async download(self, path: impl AsRef&lt;Path&gt;, limit: Option&lt;u64&gt;) -&gt; <a href="#result">Result</a>&lt;u64&gt;</code> | Download to a file. |
| <code>error_for_status(self) -&gt; Self</code> | Status check after retries, with the body. |
| <code>initiator(self, page: impl <a href="#intourl">IntoUrl</a>) -&gt; Self</code> |  |
| <code>async read_until&lt;F&gt;(self, limit: usize, done: F) -&gt; <a href="#result">Result</a>&lt;<a href="#prefixread">PrefixRead</a>&gt; where F: FnMut(&amp;[u8], usize) -&gt; bool</code> |  |
| <code>tag(self, tag: impl Into&lt;String&gt;) -&gt; Self</code> |  |
| <code>pages(self) -&gt; <a href="#pages">Pages</a></code> | Follow Link pagination. |
| <code>async send(self) -&gt; <a href="#result">Result</a>&lt;<a href="#response">Response</a>&gt;</code> | Send. |

**Trait implementations:** <code>Debug</code>, <code>IntoFuture</code>


### `Response`

**Methods**

| Method | Description |
| --- | --- |
| <code>attempts(&amp;self) -&gt; u32</code> |  |
| <code>audit(&amp;self) -&gt; Option&lt;&amp;<a href="leyline-audit.html#auditdata">AuditData</a>&gt;</code> |  |
| <code>block(&amp;self) -&gt; Option&lt;<a href="#blocksignal">BlockSignal</a>&gt;</code> | Detect a block page. |
| <code>content_length(&amp;self) -&gt; Option&lt;u64&gt;</code> |  |
| <code>cookies(&amp;self) -&gt; impl Iterator&lt;Item = <a href="leyline-cookie.html#cookie">Cookie</a>&gt; + '_</code> | Response cookies. |
| <code>error_for_status(self) -&gt; <a href="#result">Result</a>&lt;Self&gt;</code> | Status check. |
| <code>error_for_status_ref(&amp;self) -&gt; <a href="#result">Result</a>&lt;&amp;Self&gt;</code> |  |
| <code>header(&amp;self, name: &amp;str) -&gt; Option&lt;&amp;str&gt;</code> |  |
| <code>headers(&amp;self) -&gt; &amp;HeaderMap</code> | Response headers. |
| <code>proxy(&amp;self) -&gt; Option&lt;&amp;str&gt;</code> |  |
| <code>redirect_chain(&amp;self) -&gt; &amp;[Url]</code> |  |
| <code>request_headers(&amp;self) -&gt; impl Iterator&lt;Item = (&amp;str, &amp;str)&gt;</code> |  |
| <code>status(&amp;self) -&gt; StatusCode</code> |  |
| <code>timing(&amp;self) -&gt; &amp;<a href="#responsetiming">ResponseTiming</a></code> | Timing. |
| <code>tls(&amp;self) -&gt; Option&lt;&amp;<a href="#tlsinfo">TlsInfo</a>&gt;</code> | TLS details. |
| <code>trailers(&amp;self) -&gt; impl Iterator&lt;Item = (&amp;HeaderName, &amp;HeaderValue)&gt;</code> |  |
| <code>url(&amp;self) -&gt; &amp;Url</code> |  |
| <code>version(&amp;self) -&gt; <a href="#httpversion">HttpVersion</a></code> |  |
| <code>async bytes(self) -&gt; <a href="#result">Result</a>&lt;Bytes&gt;</code> | Read body. |
| <code>async copy_decoded_to&lt;W&gt;(self, writer: &amp;mut W, limit: Option&lt;u64&gt;) -&gt; <a href="#result">Result</a>&lt;u64&gt; where W: AsyncWrite + Unpin</code> | Copy the decoded body to a writer. |
| <code>async copy_to&lt;W&gt;(self, writer: &amp;mut W) -&gt; <a href="#result">Result</a>&lt;u64&gt; where W: AsyncWrite + Unpin</code> | Save a body to a file. |
| <code>into_decoded_stream(self, limit: Option&lt;u64&gt;) -&gt; <a href="#result">Result</a>&lt;<a href="#bodystream">BodyStream</a>&gt;</code> | Stream the decoded body. |
| <code>into_stream(self) -&gt; <a href="#result">Result</a>&lt;<a href="#bodystream">BodyStream</a>&gt;</code> | Stream body. |
| <code>async json&lt;T: DeserializeOwned&gt;(self) -&gt; <a href="#result">Result</a>&lt;T&gt;</code> | Read the body as JSON. |
| <code>async read_until&lt;F&gt;(self, limit: usize, done: F) -&gt; <a href="#result">Result</a>&lt;Vec&lt;u8&gt;&gt; where F: FnMut(&amp;[u8], usize) -&gt; bool</code> | Read a prefix of a body. |
| <code>async text(self) -&gt; <a href="#result">Result</a>&lt;String&gt;</code> | Read the body as text. |
| <code>async text_with_charset(self, default_encoding: &amp;str) -&gt; <a href="#result">Result</a>&lt;String&gt;</code> | Requires feature `charset`. |
| <code>async text_with_charset(self, _default_encoding: &amp;str) -&gt; <a href="#result">Result</a>&lt;String&gt;</code> | Requires feature `charset`. |
| <code>async download_to(self, path: impl AsRef&lt;Path&gt;, limit: Option&lt;u64&gt;) -&gt; <a href="#result">Result</a>&lt;u64&gt;</code> | Download a received response. |
| <code>link(&amp;self, rel: &amp;str) -&gt; Option&lt;Url&gt;</code> | Read one Link relation. |
| <code>links(&amp;self) -&gt; Vec&lt;<a href="#link">Link</a>&gt;</code> | Read Link headers. |
| <code>relay_headers(&amp;self, body: <a href="#relaybody">RelayBody</a>) -&gt; HeaderMap</code> | Relay response headers. |

**Trait implementations:** <code>Debug</code>


### `ResponseTiming`

**Fields**

| Field | Description |
| --- | --- |
| <code>connect_ms: Option&lt;u32&gt;</code> |  |
| <code>reused: bool</code> |  |
| <code>send_ms: u32</code> |  |
| <code>total_ms: u32</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Default</code>, <code>Debug</code>, <code>Copy</code>, <code>StructuralPartialEq</code>


### `RetryPolicy`

**Methods**

| Method | Description |
| --- | --- |
| <code>allow_non_idempotent(self, allow: bool) -&gt; Self</code> |  |
| <code>backoff(&amp;self, attempt: u32) -&gt; Duration</code> |  |
| <code>backoff_factor(self, factor: f64) -&gt; Self</code> |  |
| <code>initial_backoff(self, d: Duration) -&gt; Self</code> |  |
| <code>jitter(self, on: bool) -&gt; Self</code> |  |
| <code>max_backoff(self, d: Duration) -&gt; Self</code> |  |
| <code>max_retries(self, n: u32) -&gt; Self</code> |  |
| <code>max_retry_after(self, d: Duration) -&gt; Self</code> | Cap a server-requested wait. |
| <code>none() -&gt; Self</code> |  |
| <code>on_status(self, code: u16) -&gt; Self</code> |  |
| <code>retry_if&lt;F&gt;(self, f: F) -&gt; Self where F: Fn(&amp;<a href="#response">Response</a>) -&gt; bool + Send + Sync + RefUnwindSafe + 'static</code> | Retry on a condition. |
| <code>retry_on(self, triggers: impl IntoIterator&lt;Item = <a href="#retrytrigger">RetryTrigger</a>&gt;) -&gt; Self</code> | Retry triggers. |
| <code>retry_unsent(self, enabled: bool) -&gt; Self</code> | Retry a request that was not sent. |
| <code>rotate_proxies&lt;P: Into&lt;<a href="#proxyconfig">ProxyConfig</a>&gt;&gt;(self, proxies: impl IntoIterator&lt;Item = P&gt;) -&gt; Self</code> | Rotate proxies on retry. |
| <code>skip_blocks(self, rules: <a href="#blockrules">BlockRules</a>) -&gt; Self</code> | Return a block without a retry. |
| <code>transient() -&gt; Self</code> |  |
| <code>wait_header(self, name: impl Into&lt;String&gt;, format: <a href="#waitformat">WaitFormat</a>) -&gt; Self</code> | Wait on a rate-limit header. |

**Trait implementations:** <code>Clone</code>, <code>Default</code>, <code>Debug</code>


### `Session`

**Methods**

| Method | Description |
| --- | --- |
| <code>browser(browser: <a href="#browser">Browser</a>) -&gt; Self</code> | One-line browser session. |
| <code>builder() -&gt; <a href="#sessionbuilder">SessionBuilder</a></code> |  |
| <code>cookies(&amp;self) -&gt; &amp;<a href="leyline-cookie.html#jar">Jar</a></code> |  |
| <code>delete(&amp;self, url: impl <a href="#intourl">IntoUrl</a>) -&gt; <a href="#requestbuilder">RequestBuilder</a></code> |  |
| <code>async execute(&amp;self, req: Request&lt;<a href="#body">Body</a>&gt;) -&gt; <a href="#result">Result</a>&lt;<a href="#response">Response</a>&gt;</code> | Prebuilt request. |
| <code>fresh_pool(&amp;self) -&gt; Self</code> | Open new connections through the same proxy. |
| <code>get(&amp;self, url: impl <a href="#intourl">IntoUrl</a>) -&gt; <a href="#requestbuilder">RequestBuilder</a></code> |  |
| <code>head(&amp;self, url: impl <a href="#intourl">IntoUrl</a>) -&gt; <a href="#requestbuilder">RequestBuilder</a></code> |  |
| <code>host_stats(&amp;self) -&gt; Vec&lt;<a href="#hoststats">HostStats</a>&gt;</code> | Read per-host queues. |
| <code>new() -&gt; Self</code> | Default session. |
| <code>patch(&amp;self, url: impl <a href="#intourl">IntoUrl</a>) -&gt; <a href="#requestbuilder">RequestBuilder</a></code> |  |
| <code>pool_stats(&amp;self) -&gt; <a href="#poolstats">PoolStats</a></code> |  |
| <code>post(&amp;self, url: impl <a href="#intourl">IntoUrl</a>) -&gt; <a href="#requestbuilder">RequestBuilder</a></code> |  |
| <code>async preconnect(&amp;self, url: impl <a href="#intourl">IntoUrl</a>) -&gt; <a href="#result">Result</a>&lt;()&gt;</code> | Warm a connection. |
| <code>proxy_url(&amp;self) -&gt; Option&lt;<a href="#proxyurl">ProxyUrl</a>&gt;</code> |  |
| <code>put(&amp;self, url: impl <a href="#intourl">IntoUrl</a>) -&gt; <a href="#requestbuilder">RequestBuilder</a></code> |  |
| <code>request(&amp;self, method: Method, url: impl <a href="#intourl">IntoUrl</a>) -&gt; <a href="#requestbuilder">RequestBuilder</a></code> | Build a request. |
| <code>tab(&amp;self) -&gt; <a href="#tab">Tab</a></code> | Keep the current page. |
| <code>with_base_url(&amp;self, url: impl <a href="#intourl">IntoUrl</a>) -&gt; <a href="#result">Result</a>&lt;Self&gt;</code> | Derive a session with another base URL. |
| <code>with_cookie_jar(&amp;self, jar: <a href="leyline-cookie.html#jar">Jar</a>) -&gt; Self</code> | Derive a session with another jar. |
| <code>with_proxy(&amp;self, config: impl Into&lt;<a href="#proxyconfig">ProxyConfig</a>&gt;) -&gt; Self</code> | Derive a session with another proxy. |
| <code>with_redirect(&amp;self, policy: <a href="#redirectpolicy">RedirectPolicy</a>) -&gt; Self</code> |  |
| <code>identity(&amp;self) -&gt; <a href="#sessionidentity">SessionIdentity</a></code> | What a session sends. |
| <code>with_identity(&amp;self, identity: <a href="#identity">Identity</a>) -&gt; <a href="#result">Result</a>&lt;Self&gt;</code> | Derive a session with another identity. |
| <code>is_shut_down(&amp;self) -&gt; bool</code> |  |
| <code>shutdown(&amp;self)</code> | Stop every clone of a session. |
| <code>state(&amp;self) -&gt; <a href="#sessionstate">SessionState</a></code> | Save TLS tickets, Alt-Svc, and HSTS. |
| <code>websocket(&amp;self, url: impl <a href="#intourl">IntoUrl</a>) -&gt; <a href="#websocketbuilder">WebSocketBuilder</a></code> | WebSocket. Requires feature `websocket`. |

**Trait implementations:** <code>Clone</code>, <code>Default</code>, <code>Debug</code>, <code>Display</code>


### `SessionBuilder`

**Methods**

| Method | Description |
| --- | --- |
| <code>audit(self, enabled: bool) -&gt; Self</code> | Fingerprint audit. |
| <code>base_url(self, url: impl <a href="#intourl">IntoUrl</a>) -&gt; Self</code> | Session base URL. |
| <code>brand(self, brand: <a href="#chromiumbrand">ChromiumBrand</a>) -&gt; Self</code> | Brand overlay. |
| <code>browser(self, browser: <a href="#browser">Browser</a>) -&gt; Self</code> | Pick a browser. |
| <code>build(self) -&gt; <a href="#result">Result</a>&lt;<a href="#session">Session</a>&gt;</code> |  |
| <code>compression(self, config: <a href="#compressionconfig">CompressionConfig</a>) -&gt; Self</code> | Decompress. |
| <code>cookie_jar(self, jar: <a href="leyline-cookie.html#jar">Jar</a>) -&gt; Self</code> | Cookies. |
| <code>dns(self, config: impl Into&lt;<a href="#dnsconfig">DnsConfig</a>&gt;) -&gt; Self</code> | DNS. |
| <code>expect_profile_id(self, id: &amp;str) -&gt; Self</code> | Expect a profile id. |
| <code>headers&lt;I, P&gt;(self, headers: I) -&gt; Self where I: IntoIterator&lt;Item = P&gt;, P: <a href="#intoparampair">IntoParamPair</a></code> | Session default headers. |
| <code>host_limits(self, limits: <a href="#hostlimits">HostLimits</a>) -&gt; Self</code> | Limit each host. |
| <code>https_only(self, enabled: bool) -&gt; Self</code> | Refuse plain HTTP. |
| <code>identity(self, id: <a href="#identity">Identity</a>) -&gt; Self</code> | Mix TLS and HTTP identities. |
| <code>languages&lt;I, S&gt;(self, langs: I) -&gt; Self where I: IntoIterator&lt;Item = S&gt;, S: AsRef&lt;str&gt;</code> | Session languages. |
| <code>platform(self, platform: <a href="#platform">Platform</a>) -&gt; Self</code> | Pick a platform. |
| <code>pool(self, config: <a href="#poolconfig">PoolConfig</a>) -&gt; Self</code> | Pool. |
| <code>profile(self, profile: <a href="#browserprofile">BrowserProfile</a>) -&gt; Self</code> | Send a loaded profile. |
| <code>protocol(self, policy: <a href="#protocolpolicy">ProtocolPolicy</a>) -&gt; Self</code> | Protocol. |
| <code>proxy(self, config: impl Into&lt;<a href="#proxyconfig">ProxyConfig</a>&gt;) -&gt; Self</code> | Proxy. |
| <code>proxy_pool(self, pool: <a href="#proxypool">ProxyPool</a>) -&gt; Self</code> | Proxy pool. |
| <code>redirect(self, policy: <a href="#redirectpolicy">RedirectPolicy</a>) -&gt; Self</code> | Redirect. |
| <code>retry(self, policy: <a href="#retrypolicy">RetryPolicy</a>) -&gt; Self</code> | Retry. |
| <code>socket(self, config: <a href="#socketconfig">SocketConfig</a>) -&gt; Self</code> | Socket and connect tuning. |
| <code>tcp_profile(self, profile: <a href="#tcpprofile">TcpProfile</a>) -&gt; Self</code> | TCP fingerprint. |
| <code>timeout(self, config: impl Into&lt;<a href="#timeoutconfig">TimeoutConfig</a>&gt;) -&gt; Self</code> | Session timeout. |
| <code>tls_trust(self, trust: <a href="#tlstrustconfig">TlsTrustConfig</a>) -&gt; Self</code> | TLS trust. |
| <code>trace(self, hook: impl <a href="leyline-trace.html#trace">Trace</a>) -&gt; Self</code> | Trace. |
| <code>user_agent(self, value: &amp;str) -&gt; Self</code> | Session user agent. |
| <code>websocket_config(self, config: <a href="#websocketconfig">WebSocketConfig</a>) -&gt; Self</code> | Set WebSocket limits. |
| <code>bearer_auth(self, token: &amp;str) -&gt; Self</code> | Session bearer token. |

**Trait implementations:** <code>Debug</code>


### `SessionIdentity`

**Methods**

| Method | Description |
| --- | --- |
| <code>brand(&amp;self) -&gt; Option&lt;<a href="#chromiumbrand">ChromiumBrand</a>&gt;</code> |  |
| <code>browser(&amp;self) -&gt; Option&lt;<a href="#browser">Browser</a>&gt;</code> |  |
| <code>export_profile(&amp;self) -&gt; Option&lt;String&gt;</code> |  |
| <code>identity(&amp;self) -&gt; Option&lt;<a href="#identity">Identity</a>&gt;</code> |  |
| <code>languages(&amp;self) -&gt; Option&lt;&amp;[String]&gt;</code> |  |
| <code>platform(&amp;self) -&gt; <a href="#platform">Platform</a></code> |  |
| <code>profile_id(&amp;self) -&gt; Option&lt;&amp;str&gt;</code> |  |
| <code>to_identity(&amp;self) -&gt; Option&lt;<a href="#identity">Identity</a>&gt;</code> |  |
| <code>user_agent(&amp;self) -&gt; &amp;str</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Debug</code>, <code>StructuralPartialEq</code>


### `SessionState`

**Methods**

| Method | Description |
| --- | --- |
| <code>is_empty(&amp;self) -&gt; bool</code> |  |
| <code>restore_into(&amp;self, session: &amp;<a href="#session">Session</a>)</code> | Restore saved session state. |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Default</code>, <code>Debug</code>, <code>StructuralPartialEq</code>, <code>Serialize</code>, <code>Deserialize&lt;'de&gt;</code>


### `SocketConfig`

**Methods**

| Method | Description |
| --- | --- |
| <code>happy_eyeballs(self, config: impl Into&lt;Option&lt;<a href="leyline-tls.html#happyeyeballsconfig">HappyEyeballsConfig</a>&gt;&gt;) -&gt; Self</code> |  |
| <code>local_address(self, addr: impl Into&lt;Option&lt;IpAddr&gt;&gt;) -&gt; Self</code> |  |
| <code>local_ipv4(self, addr: impl Into&lt;Option&lt;Ipv4Addr&gt;&gt;) -&gt; Self</code> |  |
| <code>local_ipv6(self, addr: impl Into&lt;Option&lt;Ipv6Addr&gt;&gt;) -&gt; Self</code> |  |
| <code>new() -&gt; Self</code> |  |
| <code>recv_buffer_size(self, n: impl Into&lt;Option&lt;usize&gt;&gt;) -&gt; Self</code> |  |
| <code>send_buffer_size(self, n: impl Into&lt;Option&lt;usize&gt;&gt;) -&gt; Self</code> |  |
| <code>strict(self, on: bool) -&gt; Self</code> |  |
| <code>tcp_keepalive(self, d: impl Into&lt;Option&lt;Duration&gt;&gt;) -&gt; Self</code> |  |
| <code>tcp_keepalive_interval(self, d: impl Into&lt;Option&lt;Duration&gt;&gt;) -&gt; Self</code> |  |
| <code>tcp_keepalive_retries(self, n: impl Into&lt;Option&lt;u32&gt;&gt;) -&gt; Self</code> |  |
| <code>tcp_nodelay(self, on: impl Into&lt;Option&lt;bool&gt;&gt;) -&gt; Self</code> |  |
| <code>tcp_user_timeout(self, d: impl Into&lt;Option&lt;Duration&gt;&gt;) -&gt; Self</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Default</code>, <code>Debug</code>, <code>StructuralPartialEq</code>


### `Tab`

**Methods**

| Method | Description |
| --- | --- |
| <code>current(&amp;self) -&gt; Option&lt;Url&gt;</code> |  |
| <code>fetch(&amp;self, method: Method, url: impl <a href="#intourl">IntoUrl</a>) -&gt; <a href="#requestbuilder">RequestBuilder</a></code> | Send a script fetch from a tab. |
| <code>async follow(&amp;self, url: impl <a href="#intourl">IntoUrl</a>) -&gt; <a href="#result">Result</a>&lt;<a href="#response">Response</a>&gt;</code> |  |
| <code>async open(&amp;self, url: impl <a href="#intourl">IntoUrl</a>) -&gt; <a href="#result">Result</a>&lt;<a href="#response">Response</a>&gt;</code> | Browse with a tab. |
| <code>post_json(&amp;self, url: impl <a href="#intourl">IntoUrl</a>, body: &amp;impl Serialize) -&gt; <a href="#requestbuilder">RequestBuilder</a></code> | Post JSON from a tab. |
| <code>session(&amp;self) -&gt; &amp;<a href="#session">Session</a></code> |  |
| <code>set_current(&amp;self, page: Option&lt;Url&gt;)</code> |  |
| <code>async submit&lt;I, P&gt;(&amp;self, url: impl <a href="#intourl">IntoUrl</a>, fields: I) -&gt; <a href="#result">Result</a>&lt;<a href="#response">Response</a>&gt; where I: IntoIterator&lt;Item = P&gt;, P: <a href="#intoparampair">IntoParamPair</a></code> |  |
| <code>async submit_form(&amp;self, form: &amp;<a href="leyline-html.html#form">Form</a>) -&gt; <a href="#result">Result</a>&lt;<a href="#response">Response</a>&gt;</code> | Submit a page form. Requires feature `html`. |
| <code>subresource(&amp;self, url: impl <a href="#intourl">IntoUrl</a>, preset: <a href="#preset">Preset</a>) -&gt; <a href="#requestbuilder">RequestBuilder</a></code> | Load a subresource from a tab. |
| <code>xhr(&amp;self, url: impl <a href="#intourl">IntoUrl</a>) -&gt; <a href="#requestbuilder">RequestBuilder</a></code> | Send an XHR from a tab. |

**Trait implementations:** <code>Clone</code>, <code>Debug</code>


### `TcpProfile`

**Fields**

| Field | Description |
| --- | --- |
| <code>df: bool</code> |  |
| <code>mss: u32</code> |  |
| <code>no_delay: bool</code> |  |
| <code>options: Vec&lt;u8&gt;</code> |  |
| <code>ttl: u32</code> |  |
| <code>window_scale: u32</code> |  |
| <code>window_size: u32</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Default</code>, <code>Debug</code>, <code>StructuralPartialEq</code>, <code>Deserialize&lt;'de&gt;</code>


### `TimeoutConfig`

**Methods**

| Method | Description |
| --- | --- |
| <code>body(self, d: impl Into&lt;Option&lt;Duration&gt;&gt;) -&gt; Self</code> | Body read timeout. |
| <code>connect(self, d: impl Into&lt;Option&lt;Duration&gt;&gt;) -&gt; Self</code> |  |
| <code>error_body(self, d: impl Into&lt;Option&lt;Duration&gt;&gt;) -&gt; Self</code> | Status-error body read timeout. |
| <code>new() -&gt; Self</code> |  |
| <code>read(self, d: impl Into&lt;Option&lt;Duration&gt;&gt;) -&gt; Self</code> |  |
| <code>response_header(self, d: impl Into&lt;Option&lt;Duration&gt;&gt;) -&gt; Self</code> |  |
| <code>total(self, d: impl Into&lt;Option&lt;Duration&gt;&gt;) -&gt; Self</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>From&lt;Duration&gt;</code>, <code>Default</code>, <code>Debug</code>, <code>Copy</code>, <code>StructuralPartialEq</code>


### `TlsInfo`

**Fields**

| Field | Description |
| --- | --- |
| <code>cipher: Option&lt;String&gt;</code> |  |
| <code>peer_cert_der: Option&lt;Vec&lt;u8&gt;&gt;</code> |  |
| <code>version: Option&lt;String&gt;</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Default</code>, <code>Debug</code>


### `TlsTrustConfig`

**Methods**

| Method | Description |
| --- | --- |
| <code>add_ca_der(self, der: impl Into&lt;Vec&lt;u8&gt;&gt;) -&gt; Self</code> |  |
| <code>add_ca_file(self, path: impl Into&lt;PathBuf&gt;) -&gt; Self</code> |  |
| <code>add_pinned_leaf_sha256(self, sha256: [u8; 32]) -&gt; Self</code> |  |
| <code>client_identity(self, certificate_chain_file: impl Into&lt;PathBuf&gt;, private_key_file: impl Into&lt;PathBuf&gt;) -&gt; Self</code> |  |
| <code>danger_accept_invalid_certs(self, accept: bool) -&gt; Self</code> |  |
| <code>env_roots(self, on: bool) -&gt; Self</code> |  |
| <code>min_tls_version(self, version: <a href="#tlsminversion">TlsMinVersion</a>) -&gt; Self</code> | Set a TLS version floor. |
| <code>new() -&gt; Self</code> |  |
| <code>system_roots(self, on: bool) -&gt; Self</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Default</code>, <code>Debug</code>


### `WebSocketBuilder`

Requires feature `websocket`.

**Methods**

| Method | Description |
| --- | --- |
| <code>config(self, config: <a href="#websocketconfig">WebSocketConfig</a>) -&gt; Self</code> | Requires feature `websocket`. |
| <code>async connect(self) -&gt; <a href="#result">Result</a>&lt;<a href="#wsconnection">WsConnection</a>&gt;</code> | Requires feature `websocket`. |
| <code>header(self, name: impl TryInto&lt;HeaderName&gt;, value: impl TryInto&lt;HeaderValue&gt;) -&gt; Self</code> | Requires feature `websocket`. |
| <code>headers&lt;I, P&gt;(self, headers: I) -&gt; Self where I: IntoIterator&lt;Item = P&gt;, P: <a href="#intoparampair">IntoParamPair</a></code> | Requires feature `websocket`. |
| <code>proxy(self, config: impl Into&lt;<a href="#proxyconfig">ProxyConfig</a>&gt;) -&gt; Self</code> | Requires feature `websocket`. |

**Trait implementations:** <code>Debug</code>, <code>IntoFuture</code>


### `WebSocketConfig`

**Methods**

| Method | Description |
| --- | --- |
| <code>accept_unmasked_frames(self, on: bool) -&gt; Self</code> |  |
| <code>max_frame_size(self, n: impl Into&lt;Option&lt;usize&gt;&gt;) -&gt; Self</code> |  |
| <code>max_message_size(self, n: impl Into&lt;Option&lt;usize&gt;&gt;) -&gt; Self</code> |  |
| <code>max_write_buffer_size(self, n: impl Into&lt;Option&lt;usize&gt;&gt;) -&gt; Self</code> |  |
| <code>new() -&gt; Self</code> |  |
| <code>prefer_http2(self, on: bool) -&gt; Self</code> |  |
| <code>read_buffer_size(self, n: impl Into&lt;Option&lt;usize&gt;&gt;) -&gt; Self</code> |  |
| <code>write_buffer_size(self, n: impl Into&lt;Option&lt;usize&gt;&gt;) -&gt; Self</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Default</code>, <code>Debug</code>, <code>Copy</code>, <code>StructuralPartialEq</code>


### `WsConnection`

Requires feature `websocket`.

**Methods**

| Method | Description |
| --- | --- |
| <code>async close(&amp;mut self) -&gt; <a href="#result">Result</a>&lt;()&gt;</code> | Requires feature `websocket`. |
| <code>header(&amp;self, name: &amp;str) -&gt; Option&lt;&amp;str&gt;</code> | Requires feature `websocket`. |
| <code>protocol(&amp;self) -&gt; Option&lt;&amp;str&gt;</code> | Requires feature `websocket`. |
| <code>async recv(&amp;mut self) -&gt; <a href="#result">Result</a>&lt;Option&lt;<a href="#wsmessage">WsMessage</a>&gt;&gt;</code> | Requires feature `websocket`. |
| <code>async send(&amp;mut self, msg: <a href="#wsmessage">WsMessage</a>) -&gt; <a href="#result">Result</a>&lt;()&gt;</code> | Requires feature `websocket`. |
| <code>split(self) -&gt; (<a href="#wssink">WsSink</a>, <a href="#wsstream">WsStream</a>)</code> | Requires feature `websocket`. |

**Trait implementations:** <code>Debug</code>


### `WsSink`

Requires feature `websocket`.

**Methods**

| Method | Description |
| --- | --- |
| <code>async close(&amp;mut self) -&gt; <a href="#result">Result</a>&lt;()&gt;</code> | Requires feature `websocket`. |
| <code>async send(&amp;mut self, msg: <a href="#wsmessage">WsMessage</a>) -&gt; <a href="#result">Result</a>&lt;()&gt;</code> | Requires feature `websocket`. |

**Trait implementations:** <code>Debug</code>


### `WsStream`

Requires feature `websocket`.

**Methods**

| Method | Description |
| --- | --- |
| <code>async recv(&amp;mut self) -&gt; <a href="#result">Result</a>&lt;Option&lt;<a href="#wsmessage">WsMessage</a>&gt;&gt;</code> | Requires feature `websocket`. |

**Trait implementations:** <code>Debug</code>


## Enums

### `BlockKind`

**Variants**

| Variant | Description |
| --- | --- |
| <code>Block</code> |  |
| <code>Captcha</code> |  |
| <code>Challenge</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Debug</code>, <code>Hash</code>, <code>Copy</code>, <code>StructuralPartialEq</code>, <code>Deserialize&lt;'de&gt;</code>


### `Browser`

**Methods**

| Method | Description |
| --- | --- |
| <code>all() -&gt; &amp;'static [<a href="#browser">Browser</a>]</code> |  |
| <code>family(&amp;self) -&gt; <a href="#family">Family</a></code> |  |
| <code>for_platform(self, platform: <a href="#platform">Platform</a>) -&gt; Self</code> |  |
| <code>get(family: <a href="#family">Family</a>, version: u32) -&gt; Option&lt;Self&gt;</code> | Pin a browser version. |
| <code>id(&amp;self) -&gt; &amp;'static str</code> |  |
| <code>identity(self, platform: <a href="#platform">Platform</a>, brand: Option&lt;<a href="#chromiumbrand">ChromiumBrand</a>&gt;) -&gt; Option&lt;<a href="leyline-profile.html#platformidentity">PlatformIdentity</a>&gt;</code> | Identity values. |
| <code>latest(family: <a href="#family">Family</a>) -&gt; Self</code> |  |
| <code>version(&amp;self) -&gt; u32</code> |  |
| <code>profile(self) -&gt; &amp;'static <a href="#browserprofile">BrowserProfile</a></code> |  |

**Variants**

| Variant | Description |
| --- | --- |
| <code>Brave146</code> |  |
| <code>Brave154</code> |  |
| <code>Brave155</code> |  |
| <code>CfnetworkIOS18</code> |  |
| <code>CfnetworkIOS27</code> |  |
| <code>CfnetworkMacOS26</code> |  |
| <code>Chrome145</code> |  |
| <code>Chrome146</code> |  |
| <code>Chrome147</code> |  |
| <code>Chrome148</code> |  |
| <code>Chrome149</code> |  |
| <code>Chrome150</code> |  |
| <code>Chrome151</code> |  |
| <code>Chrome152</code> |  |
| <code>Chrome153</code> |  |
| <code>Chrome154</code> |  |
| <code>Chrome155</code> |  |
| <code>Firefox148</code> |  |
| <code>Firefox149</code> |  |
| <code>Firefox150</code> |  |
| <code>Firefox151</code> |  |
| <code>Firefox152</code> |  |
| <code>Firefox153</code> |  |
| <code>Firefox154</code> |  |
| <code>Firefox155</code> |  |
| <code>Firefox156</code> |  |
| <code>Firefox157</code> |  |
| <code>OkHttpAndroid10</code> |  |
| <code>Safari18</code> |  |
| <code>Safari26</code> |  |
| <code>Safari27</code> |  |
| <code>SafariIOS17</code> |  |
| <code>SafariIOS18</code> |  |
| <code>SafariIOS27</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Default</code>, <code>Debug</code>, <code>Display</code>, <code>Hash</code>, <code>Copy</code>, <code>StructuralPartialEq</code>, <code>FromStr</code>, <code>Serialize</code>, <code>Deserialize&lt;'de&gt;</code>


### `ChromiumBrand`

**Methods**

| Method | Description |
| --- | --- |
| <code>all() -&gt; &amp;'static [<a href="#chromiumbrand">ChromiumBrand</a>]</code> |  |
| <code>id(&amp;self) -&gt; &amp;'static str</code> |  |

**Variants**

| Variant | Description |
| --- | --- |
| <code>Chrome</code> |  |
| <code>Edge</code> |  |
| <code>Opera</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Default</code>, <code>Debug</code>, <code>Display</code>, <code>Hash</code>, <code>Copy</code>, <code>StructuralPartialEq</code>, <code>FromStr</code>, <code>Serialize</code>, <code>Deserialize&lt;'de&gt;</code>


### `ContentEncoding`

**Variants**

| Variant | Description |
| --- | --- |
| <code>Brotli</code> |  |
| <code>Deflate</code> |  |
| <code>Gzip</code> |  |
| <code>Zstd</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Debug</code>, <code>Copy</code>, <code>StructuralPartialEq</code>


### `ErrorCategory`

**Methods**

| Method | Description |
| --- | --- |
| <code>as_str(self) -&gt; &amp;'static str</code> | Name an error category. |
| <code>gateway_status(self) -&gt; Option&lt;StatusCode&gt;</code> |  |

**Variants**

| Variant | Description |
| --- | --- |
| <code>Body</code> |  |
| <code>BodyLimit</code> |  |
| <code>Config</code> |  |
| <code>Connect</code> |  |
| <code>Decode</code> |  |
| <code>Dns</code> |  |
| <code>Other</code> |  |
| <code>Protocol</code> |  |
| <code>Proxy</code> |  |
| <code>ProxyTarget</code> |  |
| <code>Redirect</code> |  |
| <code>Request</code> |  |
| <code>Status</code> |  |
| <code>Timeout</code> |  |
| <code>Tls</code> |  |
| <code>Url</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Debug</code>, <code>Display</code>, <code>Hash</code>, <code>Copy</code>, <code>StructuralPartialEq</code>


### `ErrorCode`

**Methods**

| Method | Description |
| --- | --- |
| <code>from_u32(val: u32) -&gt; Self</code> |  |

**Variants**

| Variant | Description |
| --- | --- |
| <code>Cancel = 8</code> |  |
| <code>CompressionError = 9</code> |  |
| <code>ConnectError = 10</code> |  |
| <code>EnhanceYourCalm = 11</code> |  |
| <code>FlowControlError = 3</code> |  |
| <code>FrameSizeError = 6</code> |  |
| <code>Http11Required = 13</code> |  |
| <code>InadequateSecurity = 12</code> |  |
| <code>InternalError = 2</code> |  |
| <code>NoError = 0</code> |  |
| <code>ProtocolError = 1</code> |  |
| <code>RefusedStream = 7</code> |  |
| <code>SettingsTimeout = 4</code> |  |
| <code>StreamClosed = 5</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Debug</code>, <code>Copy</code>, <code>StructuralPartialEq</code>


### `Family`

**Methods**

| Method | Description |
| --- | --- |
| <code>all() -&gt; &amp;'static [<a href="#family">Family</a>]</code> |  |
| <code>id(&amp;self) -&gt; &amp;'static str</code> |  |

**Variants**

| Variant | Description |
| --- | --- |
| <code>Brave</code> |  |
| <code>CfNetwork</code> |  |
| <code>Chrome</code> |  |
| <code>Firefox</code> |  |
| <code>OkHttp</code> |  |
| <code>Safari</code> |  |
| <code>SafariIos</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Debug</code>, <code>Display</code>, <code>Hash</code>, <code>Copy</code>, <code>StructuralPartialEq</code>, <code>FromStr</code>, <code>Serialize</code>, <code>Deserialize&lt;'de&gt;</code>


### `FetchSite`

**Methods**

| Method | Description |
| --- | --- |
| <code>as_str(self) -&gt; &amp;'static str</code> |  |
| <code>of(context: &amp;Url, request: &amp;Url) -&gt; Self</code> | sec-fetch-site value. |

**Variants**

| Variant | Description |
| --- | --- |
| <code>CrossSite</code> |  |
| <code>SameOrigin</code> |  |
| <code>SameSite</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Debug</code>, <code>Display</code>, <code>Hash</code>, <code>Copy</code>, <code>StructuralPartialEq</code>


### `H2Error`

**Fields**

| Field | Description |
| --- | --- |
| <code>Connection::code: <a href="#errorcode">ErrorCode</a></code> |  |
| <code>Connection::reason: String</code> |  |
| <code>FrameTooLarge::max: u32</code> |  |
| <code>FrameTooLarge::size: u32</code> |  |
| <code>Stream::code: <a href="#errorcode">ErrorCode</a></code> |  |
| <code>Stream::stream_id: u32</code> |  |

**Variants**

| Variant | Description |
| --- | --- |
| <code>Connection</code> |  |
| <code>FrameTooLarge</code> |  |
| <code>Hpack(String)</code> |  |
| <code>Io(Error)</code> |  |
| <code>RequestBody(Error)</code> |  |
| <code>Stream</code> |  |

**Trait implementations:** <code>From&lt;Error&gt;</code>, <code>Error</code>, <code>Debug</code>, <code>Display</code>


### `HeaderAnchor`

**Variants**

| Variant | Description |
| --- | --- |
| <code>AfterAccept</code> |  |
| <code>AfterCchUa</code> |  |
| <code>AfterCchUaMobile</code> |  |
| <code>AfterCchUaPlatform</code> |  |
| <code>AfterContentType</code> |  |
| <code>AfterFetchDest</code> |  |
| <code>AfterUserAgent</code> |  |
| <code>BeforeAcceptEncoding</code> |  |
| <code>BeforeCchUa</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Debug</code>, <code>Hash</code>, <code>Copy</code>, <code>StructuralPartialEq</code>


### `HttpVersion`

**Methods**

| Method | Description |
| --- | --- |
| <code>as_str(self) -&gt; &amp;'static str</code> |  |

**Variants**

| Variant | Description |
| --- | --- |
| <code>Http1_1</code> |  |
| <code>Http2</code> |  |
| <code>Http3</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Debug</code>, <code>Copy</code>, <code>StructuralPartialEq</code>


### `Kind`

**Methods**

| Method | Description |
| --- | --- |
| <code>as_str(self) -&gt; &amp;'static str</code> |  |

**Variants**

| Variant | Description |
| --- | --- |
| <code>Body</code> |  |
| <code>Config</code> |  |
| <code>Connect</code> |  |
| <code>Decode</code> |  |
| <code>Http2</code> |  |
| <code>Http3</code> |  |
| <code>Io</code> |  |
| <code>Json</code> |  |
| <code>Proxy</code> |  |
| <code>Redirect</code> |  |
| <code>Request</code> |  |
| <code>Status</code> |  |
| <code>Timeout</code> |  |
| <code>Tls</code> |  |
| <code>Url</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Debug</code>, <code>Display</code>, <code>Hash</code>, <code>Copy</code>, <code>StructuralPartialEq</code>


### `Platform`

**Methods**

| Method | Description |
| --- | --- |
| <code>all() -&gt; &amp;'static [<a href="#platform">Platform</a>]</code> |  |
| <code>id(&amp;self) -&gt; &amp;'static str</code> |  |
| <code>tcp_profile(&amp;self) -&gt; <a href="#tcpprofile">TcpProfile</a></code> |  |

**Variants**

| Variant | Description |
| --- | --- |
| <code>Android</code> |  |
| <code>Host</code> |  |
| <code>IOS</code> |  |
| <code>Linux</code> |  |
| <code>MacOS</code> |  |
| <code>Windows</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Default</code>, <code>Debug</code>, <code>Display</code>, <code>Hash</code>, <code>Copy</code>, <code>StructuralPartialEq</code>, <code>FromStr</code>, <code>Serialize</code>, <code>Deserialize&lt;'de&gt;</code>


### `Preset`

**Variants**

| Variant | Description |
| --- | --- |
| <code>CrossOrigin</code> |  |
| <code>Form</code> |  |
| <code>FormNavigate</code> |  |
| <code>FrameNavigate</code> |  |
| <code>Image</code> |  |
| <code>Native</code> |  |
| <code>Navigate</code> |  |
| <code>Reload</code> |  |
| <code>SameSite</code> |  |
| <code>Script</code> |  |
| <code>Xhr</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Debug</code>, <code>Hash</code>, <code>Copy</code>, <code>StructuralPartialEq</code>, <code>Deserialize&lt;'de&gt;</code>


### `ProtocolPolicy`

**Variants**

| Variant | Description |
| --- | --- |
| <code>Auto</code> |  |
| <code>Http1</code> |  |
| <code>Http2</code> |  |
| <code>Http3</code> | Requires feature `http3`. |
| <code>Race</code> | Requires feature `http3`. |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Debug</code>, <code>Copy</code>, <code>StructuralPartialEq</code>


### `ProxyReply`

**Variants**

| Variant | Description |
| --- | --- |
| <code>HttpStatus(u16)</code> |  |
| <code>Socks5(u8)</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Debug</code>, <code>Copy</code>, <code>StructuralPartialEq</code>


### `RedirectAction`

**Variants**

| Variant | Description |
| --- | --- |
| <code>Follow</code> |  |
| <code>Stop</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Debug</code>, <code>Copy</code>, <code>StructuralPartialEq</code>


### `RelayBody`

**Variants**

| Variant | Description |
| --- | --- |
| <code>AsReceived</code> |  |
| <code>Decoded</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Debug</code>, <code>Copy</code>, <code>StructuralPartialEq</code>


### `RetryTrigger`

**Variants**

| Variant | Description |
| --- | --- |
| <code>ConnectionError</code> |  |
| <code>ServerError</code> |  |
| <code>Status(u16)</code> |  |
| <code>Timeout</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Debug</code>, <code>Copy</code>


### `StopReason`

**Variants**

| Variant | Description |
| --- | --- |
| <code>EndOfBody</code> |  |
| <code>LimitReached</code> |  |
| <code>PredicateMatched</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Debug</code>, <code>Hash</code>, <code>Copy</code>, <code>StructuralPartialEq</code>


### `TlsError`

**Fields**

| Field | Description |
| --- | --- |
| <code>Certificate::detail: String</code> |  |
| <code>Certificate::reason: Option&lt;&amp;'static str&gt;</code> |  |
| <code>Certificate::verify_code: Option&lt;i32&gt;</code> |  |
| <code>Proxy::detail: String</code> |  |
| <code>Proxy::source: Option&lt;Error&gt;</code> |  |
| <code>Proxy::status: Option&lt;u16&gt;</code> |  |
| <code>ProxyTargetUnreachable::detail: String</code> |  |
| <code>ProxyTargetUnreachable::reply: <a href="#proxyreply">ProxyReply</a></code> |  |

**Variants**

| Variant | Description |
| --- | --- |
| <code>Certificate</code> |  |
| <code>Dns(Error)</code> |  |
| <code>Handshake(String)</code> |  |
| <code>HandshakeIo(Error)</code> |  |
| <code>Hostname(String)</code> |  |
| <code>Pinning(String)</code> |  |
| <code>Profile(String)</code> |  |
| <code>Proxy</code> |  |
| <code>ProxyTargetUnreachable</code> |  |
| <code>Rejected(Error)</code> |  |
| <code>SslConfig(String)</code> |  |
| <code>TcpConnect(Error)</code> |  |
| <code>TrustStore(String)</code> |  |

**Trait implementations:** <code>Error</code>, <code>Debug</code>, <code>Display</code>


### `TlsMinVersion`

**Variants**

| Variant | Description |
| --- | --- |
| <code>Tls10</code> |  |
| <code>Tls12</code> |  |
| <code>Tls13</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>Ord</code>, <code>PartialEq</code>, <code>PartialOrd</code>, <code>Debug</code>, <code>Copy</code>, <code>StructuralPartialEq</code>


### `WaitFormat`

**Variants**

| Variant | Description |
| --- | --- |
| <code>HttpDate</code> |  |
| <code>Seconds</code> |  |
| <code>UnixSeconds</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Debug</code>, <code>Hash</code>, <code>Copy</code>, <code>StructuralPartialEq</code>


### `WsMessage`

Requires feature `websocket`.

**Variants**

| Variant | Description |
| --- | --- |
| <code>Binary(Vec&lt;u8&gt;)</code> | Requires feature `websocket`. |
| <code>Close(Option&lt;<a href="#closeframe">CloseFrame</a>&gt;)</code> | Requires feature `websocket`. |
| <code>Ping</code> | Requires feature `websocket`. |
| <code>Pong</code> | Requires feature `websocket`. |
| <code>Text(String)</code> | Requires feature `websocket`. |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Debug</code>, <code>StructuralPartialEq</code>


## Traits

### `IntoParamPair`

**Methods**

| Method | Description |
| --- | --- |
| <code>into_param_pair(self) -&gt; (String, String)</code> |  |


### `IntoUrl`

URL input.

**Methods**

| Method | Description |
| --- | --- |
| <code>into_url(self) -&gt; <a href="#result">Result</a>&lt;Url&gt;</code> |  |


## Functions

### `get`

<pre>pub async fn <a href="#get">get</a>(url: impl <a href="#intourl">IntoUrl</a>) -&gt; <a href="#result">Result</a>&lt;<a href="#response">Response</a>&gt;</pre>

Send one GET.


### `redact_url`

<pre>pub fn <a href="#redact_url">redact_url</a>(raw: &amp;str) -&gt; String</pre>


### `relay_headers`

<pre>pub fn <a href="#relay_headers">relay_headers</a>(headers: &amp;HeaderMap, body: <a href="#relaybody">RelayBody</a>) -&gt; HeaderMap</pre>

Relay a header map.


## Type aliases

### `Result`

<pre>pub type <a href="#result">Result</a>&lt;T&gt; = Result&lt;T, <a href="#error">Error</a>&gt;</pre>


## Re-exports

### `Url`


### `http`


## Implementations

### `alloc::sync::Arc`

**Trait implementations:** <code><a href="leyline-trace.html#trace">Trace</a></code>


### `alloc::string::String`

**Trait implementations:** <code>From&lt;<a href="#proxyurl">ProxyUrl</a>&gt;</code>, <code><a href="#intourl">IntoUrl</a></code>


### `(K, V)`

**Trait implementations:** <code><a href="#intoparampair">IntoParamPair</a></code>


### `str`

**Trait implementations:** <code><a href="#intourl">IntoUrl</a></code>


### `url::Url`

**Trait implementations:** <code><a href="#intourl">IntoUrl</a></code>


---

Generated by truesight from `leyline-http` 0.1.3. See the [API overview](index.md).
