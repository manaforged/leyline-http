# Module `leyline::profile`

| Item | Kind | Description |
| --- | --- | --- |
| [`FingerprintSpec`](#fingerprintspec) | struct |  |
| [`H2Fingerprint`](#h2fingerprint) | struct |  |
| [`H2PlatformOverride`](#h2platformoverride) | struct |  |
| [`H2PriorityProfile`](#h2priorityprofile) | struct |  |
| [`H2Profile`](#h2profile) | struct |  |
| [`H3Grease`](#h3grease) | struct |  |
| [`H3Profile`](#h3profile) | struct |  |
| [`H3Setting`](#h3setting) | struct |  |
| [`H3TransportParam`](#h3transportparam) | struct |  |
| [`H3VersionInformation`](#h3versioninformation) | struct |  |
| [`PlatformIdentity`](#platformidentity) | struct |  |
| [`ProfileMeta`](#profilemeta) | struct |  |
| [`ProfileRegistry`](#profileregistry) | struct |  |
| [`TlsFingerprint`](#tlsfingerprint) | struct |  |
| [`TlsProfile`](#tlsprofile) | struct |  |
| [`H3ConnectionIdLength`](#h3connectionidlength) | enum |  |
| [`H3CryptoReorder`](#h3cryptoreorder) | enum |  |
| [`H3CryptoSplit`](#h3cryptosplit) | enum |  |
| [`H3Order`](#h3order) | enum |  |
| [`H3VersionGrease`](#h3versiongrease) | enum |  |
| [`HeaderStyle`](#headerstyle) | enum |  |
| [`ProfileError`](#profileerror) | enum |  |

## Structs

### `FingerprintSpec`

**Methods**

| Method | Description |
| --- | --- |
| <code>akamai(self, raw: impl Into&lt;String&gt;) -&gt; Self</code> |  |
| <code>base(self, profile: <a href="leyline.html#browserprofile">BrowserProfile</a>) -&gt; Self</code> |  |
| <code>header_order&lt;I, S&gt;(self, order: I) -&gt; Self where I: IntoIterator&lt;Item = S&gt;, S: Into&lt;String&gt;</code> |  |
| <code>ja3(self, raw: impl Into&lt;String&gt;) -&gt; Self</code> |  |
| <code>ja4_r(self, raw: impl Into&lt;String&gt;) -&gt; Self</code> |  |
| <code>name(self, name: impl Into&lt;String&gt;) -&gt; Self</code> |  |
| <code>new() -&gt; Self</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Default</code>, <code>Debug</code>


### `H2Fingerprint`

**Fields**

| Field | Description |
| --- | --- |
| <code>akamai: Option&lt;String&gt;</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Default</code>, <code>Debug</code>, <code>Deserialize&lt;'de&gt;</code>


### `H2PlatformOverride`

**Fields**

| Field | Description |
| --- | --- |
| <code>enable_push: Option&lt;bool&gt;</code> |  |
| <code>fingerprint: Option&lt;<a href="#h2fingerprint">H2Fingerprint</a>&gt;</code> |  |
| <code>header_table_size: Option&lt;u32&gt;</code> |  |
| <code>initial_connection_window_size: Option&lt;u32&gt;</code> |  |
| <code>initial_stream_window_size: Option&lt;u32&gt;</code> |  |
| <code>max_concurrent_streams: Option&lt;u32&gt;</code> |  |
| <code>max_frame_size: Option&lt;u32&gt;</code> |  |
| <code>max_header_list_size: Option&lt;u32&gt;</code> |  |
| <code>omit_settings: Vec&lt;String&gt;</code> |  |
| <code>pseudo_order: Option&lt;Vec&lt;String&gt;&gt;</code> |  |
| <code>settings_order: Option&lt;Vec&lt;String&gt;&gt;</code> |  |
| <code>unknown_setting8: Option&lt;u32&gt;</code> |  |
| <code>unknown_setting9: Option&lt;u32&gt;</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Default</code>, <code>Debug</code>, <code>Deserialize&lt;'de&gt;</code>


### `H2PriorityProfile`

**Fields**

| Field | Description |
| --- | --- |
| <code>exclusive: bool</code> |  |
| <code>stream_dependency: u32</code> |  |
| <code>weight: u8</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Debug</code>, <code>Copy</code>, <code>StructuralPartialEq</code>, <code>Deserialize&lt;'de&gt;</code>


### `H2Profile`

**Fields**

| Field | Description |
| --- | --- |
| <code>default_priority: Option&lt;<a href="#h2priorityprofile">H2PriorityProfile</a>&gt;</code> |  |
| <code>enable_push: Option&lt;bool&gt;</code> |  |
| <code>fingerprint: Option&lt;<a href="#h2fingerprint">H2Fingerprint</a>&gt;</code> |  |
| <code>header_table_size: Option&lt;u32&gt;</code> |  |
| <code>initial_connection_window_size: Option&lt;u32&gt;</code> |  |
| <code>initial_stream_window_size: Option&lt;u32&gt;</code> |  |
| <code>max_concurrent_streams: Option&lt;u32&gt;</code> |  |
| <code>max_frame_size: Option&lt;u32&gt;</code> |  |
| <code>max_header_list_size: Option&lt;u32&gt;</code> |  |
| <code>platforms: HashMap&lt;String, <a href="#h2platformoverride">H2PlatformOverride</a>&gt;</code> |  |
| <code>pseudo_order: Vec&lt;String&gt;</code> |  |
| <code>settings_order: Vec&lt;String&gt;</code> |  |
| <code>unknown_setting8: Option&lt;u32&gt;</code> |  |
| <code>unknown_setting9: Option&lt;u32&gt;</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Debug</code>, <code>Deserialize&lt;'de&gt;</code>


### `H3Grease`

**Fields**

| Field | Description |
| --- | --- |
| <code>id_bits: u32</code> |  |
| <code>max_len: usize</code> |  |
| <code>value_bits: u32</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Debug</code>, <code>Copy</code>, <code>StructuralPartialEq</code>, <code>Deserialize&lt;'de&gt;</code>


### `H3Profile`

**Fields**

| Field | Description |
| --- | --- |
| <code>active_connection_id_limit: u64</code> |  |
| <code>control_grease_frame: Option&lt;<a href="#h3grease">H3Grease</a>&gt;</code> |  |
| <code>dcid_length: <a href="#h3connectionidlength">H3ConnectionIdLength</a></code> |  |
| <code>initial_crypto_reorder: <a href="#h3cryptoreorder">H3CryptoReorder</a></code> |  |
| <code>initial_crypto_split: <a href="#h3cryptosplit">H3CryptoSplit</a></code> |  |
| <code>initial_datagram_size: Option&lt;u16&gt;</code> |  |
| <code>initial_max_data: u64</code> |  |
| <code>initial_max_stream_data_bidi_local: u64</code> |  |
| <code>initial_max_stream_data_bidi_remote: u64</code> |  |
| <code>initial_max_stream_data_uni: u64</code> |  |
| <code>initial_max_streams_bidi: u64</code> |  |
| <code>initial_max_streams_uni: u64</code> |  |
| <code>max_ack_delay_ms: Option&lt;u64&gt;</code> |  |
| <code>max_field_section_size: Option&lt;u64&gt;</code> |  |
| <code>max_idle_timeout_secs: u64</code> |  |
| <code>max_udp_payload_size: u16</code> |  |
| <code>priority_update: bool</code> |  |
| <code>pseudo_order: Option&lt;Vec&lt;String&gt;&gt;</code> |  |
| <code>qpack_blocked_streams: Option&lt;u64&gt;</code> |  |
| <code>qpack_max_table_capacity: Option&lt;u64&gt;</code> |  |
| <code>race: bool</code> |  |
| <code>scid_length: Option&lt;usize&gt;</code> |  |
| <code>settings: Option&lt;Vec&lt;<a href="#h3setting">H3Setting</a>&gt;&gt;</code> |  |
| <code>tls: Option&lt;<a href="#tlsprofile">TlsProfile</a>&gt;</code> |  |
| <code>transport_order: <a href="#h3order">H3Order</a></code> |  |
| <code>transport_parameters: Option&lt;Vec&lt;<a href="#h3transportparam">H3TransportParam</a>&gt;&gt;</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Debug</code>, <code>Deserialize&lt;'de&gt;</code>


### `H3Setting`

**Fields**

| Field | Description |
| --- | --- |
| <code>grease: Option&lt;<a href="#h3grease">H3Grease</a>&gt;</code> |  |
| <code>id: Option&lt;u64&gt;</code> |  |
| <code>value: Option&lt;u64&gt;</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Debug</code>, <code>Copy</code>, <code>StructuralPartialEq</code>, <code>Deserialize&lt;'de&gt;</code>


### `H3TransportParam`

**Fields**

| Field | Description |
| --- | --- |
| <code>grease: Option&lt;<a href="#h3grease">H3Grease</a>&gt;</code> |  |
| <code>hex: Option&lt;String&gt;</code> |  |
| <code>id: Option&lt;u64&gt;</code> |  |
| <code>pinned: bool</code> |  |
| <code>varint: Option&lt;u64&gt;</code> |  |
| <code>versions: Option&lt;<a href="#h3versioninformation">H3VersionInformation</a>&gt;</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Debug</code>, <code>StructuralPartialEq</code>, <code>Deserialize&lt;'de&gt;</code>


### `H3VersionInformation`

**Fields**

| Field | Description |
| --- | --- |
| <code>available: Vec&lt;u32&gt;</code> |  |
| <code>chosen: u32</code> |  |
| <code>grease: Option&lt;<a href="#h3versiongrease">H3VersionGrease</a>&gt;</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Debug</code>, <code>StructuralPartialEq</code>, <code>Deserialize&lt;'de&gt;</code>


### `PlatformIdentity`

**Fields**

| Field | Description |
| --- | --- |
| <code>accept_language: Option&lt;String&gt;</code> |  |
| <code>sec_ch_ua: String</code> |  |
| <code>user_agent: String</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Debug</code>, <code>Deserialize&lt;'de&gt;</code>


### `ProfileMeta`

**Fields**

| Field | Description |
| --- | --- |
| <code>browser: String</code> |  |
| <code>captured_against: Option&lt;String&gt;</code> |  |
| <code>ch_ua_brand: Option&lt;String&gt;</code> |  |
| <code>chromium_major: Option&lt;u32&gt;</code> |  |
| <code>family: String</code> |  |
| <code>header_style: <a href="#headerstyle">HeaderStyle</a></code> |  |
| <code>name: String</code> |  |
| <code>verified_against: String</code> |  |
| <code>version: u32</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Debug</code>, <code>Deserialize&lt;'de&gt;</code>


### `ProfileRegistry`

**Methods**

| Method | Description |
| --- | --- |
| <code>get(&amp;self, browser: &amp;str, version: u32) -&gt; Option&lt;&amp;<a href="leyline.html#browserprofile">BrowserProfile</a>&gt;</code> |  |
| <code>global() -&gt; &amp;'static Self</code> |  |
| <code>load(dir: &amp;Path) -&gt; Result&lt;Self, <a href="#profileerror">ProfileError</a>&gt;</code> | Load profiles from a directory. |

**Trait implementations:** <code>Default</code>, <code>Debug</code>


### `TlsFingerprint`

**Fields**

| Field | Description |
| --- | --- |
| <code>ja4: Option&lt;String&gt;</code> |  |
| <code>platforms: HashMap&lt;String, <a href="#tlsfingerprint">TlsFingerprint</a>&gt;</code> |  |
| <code>resumed_ja4: Option&lt;String&gt;</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Default</code>, <code>Debug</code>, <code>Deserialize&lt;'de&gt;</code>


### `TlsProfile`

**Fields**

| Field | Description |
| --- | --- |
| <code>alps: Option&lt;String&gt;</code> |  |
| <code>alps_new_codepoint: bool</code> |  |
| <code>cert_compression: Vec&lt;String&gt;</code> |  |
| <code>ciphers: Vec&lt;String&gt;</code> |  |
| <code>curves: Vec&lt;String&gt;</code> |  |
| <code>delegated_credentials: Option&lt;String&gt;</code> |  |
| <code>ech_grease: bool</code> |  |
| <code>extension_permutation: Option&lt;Vec&lt;u16&gt;&gt;</code> |  |
| <code>extension_tail: Vec&lt;u16&gt;</code> |  |
| <code>fingerprint: Option&lt;<a href="#tlsfingerprint">TlsFingerprint</a>&gt;</code> |  |
| <code>grease: bool</code> |  |
| <code>key_shares: Option&lt;Vec&lt;String&gt;&gt;</code> |  |
| <code>min_tls_version: Option&lt;String&gt;</code> |  |
| <code>ocsp_stapling: bool</code> |  |
| <code>padding: bool</code> |  |
| <code>permute_extensions: bool</code> |  |
| <code>pre_shared_key: bool</code> |  |
| <code>record_size_limit: Option&lt;u16&gt;</code> |  |
| <code>request_trust_anchors: bool</code> |  |
| <code>session_tickets: bool</code> |  |
| <code>sigalg_grease: bool</code> |  |
| <code>sigalgs: Vec&lt;String&gt;</code> |  |
| <code>signed_cert_timestamps: bool</code> |  |
| <code>tls12_extensions: bool</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Debug</code>, <code>Deserialize&lt;'de&gt;</code>


## Enums

### `H3ConnectionIdLength`

**Fields**

| Field | Description |
| --- | --- |
| <code>Weighted::weights: Vec&lt;(usize, u32)&gt;</code> |  |

**Variants**

| Variant | Description |
| --- | --- |
| <code>Fixed(usize)</code> |  |
| <code>Weighted</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Debug</code>, <code>StructuralPartialEq</code>, <code>Deserialize&lt;'de&gt;</code>


### `H3CryptoReorder`

**Variants**

| Variant | Description |
| --- | --- |
| <code>None</code> |  |
| <code>SniMidpoint</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Default</code>, <code>Debug</code>, <code>Copy</code>, <code>StructuralPartialEq</code>, <code>Deserialize&lt;'de&gt;</code>


### `H3CryptoSplit`

**Variants**

| Variant | Description |
| --- | --- |
| <code>Even</code> |  |
| <code>Fill</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Default</code>, <code>Debug</code>, <code>Copy</code>, <code>StructuralPartialEq</code>, <code>Deserialize&lt;'de&gt;</code>


### `H3Order`

**Variants**

| Variant | Description |
| --- | --- |
| <code>Fixed</code> |  |
| <code>Rotate</code> |  |
| <code>Shuffle</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Default</code>, <code>Debug</code>, <code>Copy</code>, <code>StructuralPartialEq</code>, <code>Deserialize&lt;'de&gt;</code>


### `H3VersionGrease`

**Variants**

| Variant | Description |
| --- | --- |
| <code>First</code> |  |
| <code>Random</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Debug</code>, <code>Copy</code>, <code>StructuralPartialEq</code>, <code>Deserialize&lt;'de&gt;</code>


### `HeaderStyle`

**Variants**

| Variant | Description |
| --- | --- |
| <code>Bare = 10</code> |  |
| <code>Brave = 0</code> |  |
| <code>Brave154 = 1</code> |  |
| <code>CfNetwork = 2</code> |  |
| <code>CfNetwork26 = 3</code> |  |
| <code>Chromium = 4</code> |  |
| <code>Gecko = 5</code> |  |
| <code>OkHttp = 6</code> |  |
| <code>WebKit = 7</code> |  |
| <code>WebKit17 = 8</code> |  |
| <code>WebKit26 = 9</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Default</code>, <code>Debug</code>, <code>Hash</code>, <code>Copy</code>, <code>StructuralPartialEq</code>, <code>Deserialize&lt;'de&gt;</code>


### `ProfileError`

**Fields**

| Field | Description |
| --- | --- |
| <code>Empty::path: PathBuf</code> |  |
| <code>Io::path: PathBuf</code> |  |
| <code>Io::source: Error</code> |  |
| <code>Parse::path: Option&lt;PathBuf&gt;</code> |  |
| <code>Parse::source: Box&lt;(dyn Error + Send + Sync)&gt;</code> |  |

**Variants**

| Variant | Description |
| --- | --- |
| <code>Empty</code> |  |
| <code>Io</code> |  |
| <code>Parse</code> |  |

**Trait implementations:** <code>Error</code>, <code>Debug</code>, <code>Display</code>


---

Generated by truesight from `leyline-http` 0.1.4. See the [API overview](index.md).
