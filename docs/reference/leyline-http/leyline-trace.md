# Module `leyline::trace`

| Item | Kind | Description |
| --- | --- | --- |
| [`BodyEnd`](#bodyend) | struct |  |
| [`Connect`](#connect) | struct |  |
| [`Dns`](#dns) | struct |  |
| [`Done`](#done) | struct |  |
| [`Fanout`](#fanout) | struct |  |
| [`Head`](#head) | struct |  |
| [`Metrics`](#metrics) | struct |  |
| [`MetricsSnapshot`](#metricssnapshot) | struct |  |
| [`Sent`](#sent) | struct |  |
| [`Summary`](#summary) | struct |  |
| [`Tls`](#tls) | struct |  |
| [`TracingTrace`](#tracingtrace) | struct |  |
| [`BodyOutcome`](#bodyoutcome) | enum |  |
| [`Trace`](#trace) | trait |  |

## Structs

### `BodyEnd`

**Fields**

| Field | Description |
| --- | --- |
| <code>bytes: u64</code> |  |
| <code>elapsed: Duration</code> |  |
| <code>id: u64</code> |  |
| <code>outcome: <a href="#bodyoutcome">BodyOutcome</a>&lt;'a&gt;</code> |  |

**Trait implementations:** <code>Debug</code>


### `Connect`

**Fields**

| Field | Description |
| --- | --- |
| <code>elapsed: Duration</code> |  |
| <code>host: &amp;'a str</code> |  |
| <code>id: u64</code> |  |
| <code>port: u16</code> |  |
| <code>reused: bool</code> |  |

**Trait implementations:** <code>Debug</code>


### `Dns`

**Fields**

| Field | Description |
| --- | --- |
| <code>addrs: usize</code> |  |
| <code>elapsed: Duration</code> |  |
| <code>host: &amp;'a str</code> |  |
| <code>id: u64</code> |  |
| <code>port: u16</code> |  |

**Trait implementations:** <code>Debug</code>


### `Done`

**Fields**

| Field | Description |
| --- | --- |
| <code>elapsed: Duration</code> |  |
| <code>id: u64</code> |  |
| <code>outcome: Result&lt;(), &amp;'a <a href="leyline.html#error">Error</a>&gt;</code> |  |

**Trait implementations:** <code>Debug</code>


### `Fanout`

**Methods**

| Method | Description |
| --- | --- |
| <code>new() -&gt; Self</code> |  |
| <code>with(self, hook: impl <a href="#trace">Trace</a>) -&gt; Self</code> | Send events to several traces. |

**Trait implementations:** <code>Clone</code>, <code>Default</code>, <code>Debug</code>, <code><a href="#trace">Trace</a></code>


### `Head`

**Fields**

| Field | Description |
| --- | --- |
| <code>elapsed: Duration</code> |  |
| <code>headers: &amp;'a HeaderMap</code> |  |
| <code>host: &amp;'a str</code> |  |
| <code>id: u64</code> |  |
| <code>protocol: <a href="leyline.html#httpversion">HttpVersion</a></code> |  |
| <code>status: u16</code> |  |

**Trait implementations:** <code>Debug</code>


### `Metrics`

**Methods**

| Method | Description |
| --- | --- |
| <code>new() -&gt; Arc&lt;Self&gt;</code> | Count requests. |
| <code>snapshot(&amp;self) -&gt; <a href="#metricssnapshot">MetricsSnapshot</a></code> |  |

**Trait implementations:** <code>Default</code>, <code>Debug</code>, <code><a href="#trace">Trace</a></code>


### `MetricsSnapshot`

**Methods**

| Method | Description |
| --- | --- |
| <code>attempts(&amp;self) -&gt; u64</code> |  |
| <code>bodies_complete(&amp;self) -&gt; u64</code> |  |
| <code>bodies_dropped(&amp;self) -&gt; u64</code> |  |
| <code>bodies_failed(&amp;self) -&gt; u64</code> |  |
| <code>errors(&amp;self, category: <a href="leyline.html#errorcategory">ErrorCategory</a>) -&gt; u64</code> |  |
| <code>errors_total(&amp;self) -&gt; u64</code> |  |
| <code>latency(&amp;self) -&gt; Vec&lt;(Option&lt;Duration&gt;, u64)&gt;</code> |  |
| <code>requests(&amp;self) -&gt; u64</code> |  |
| <code>status_class(&amp;self, class: u8) -&gt; u64</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Default</code>, <code>Debug</code>, <code>Display</code>, <code>StructuralPartialEq</code>


### `Sent`

**Fields**

| Field | Description |
| --- | --- |
| <code>elapsed: Duration</code> |  |
| <code>host: &amp;'a str</code> |  |
| <code>id: u64</code> |  |
| <code>method: &amp;'a str</code> |  |
| <code>path: &amp;'a str</code> |  |
| <code>protocol: <a href="leyline.html#httpversion">HttpVersion</a></code> |  |

**Trait implementations:** <code>Debug</code>


### `Summary`

**Fields**

| Field | Description |
| --- | --- |
| <code>attempts: u32</code> |  |
| <code>browser: Option&lt;<a href="leyline.html#browser">Browser</a>&gt;</code> |  |
| <code>elapsed: Duration</code> |  |
| <code>id: u64</code> |  |
| <code>method: &amp;'a str</code> |  |
| <code>original_url: &amp;'a str</code> |  |
| <code>outcome: Result&lt;(), &amp;'a <a href="leyline.html#error">Error</a>&gt;</code> |  |
| <code>proxy: Option&lt;String&gt;</code> |  |
| <code>redirects: usize</code> |  |
| <code>status: Option&lt;StatusCode&gt;</code> |  |
| <code>streamed: bool</code> |  |
| <code>tag: Option&lt;&amp;'a str&gt;</code> |  |
| <code>url: Option&lt;&amp;'a Url&gt;</code> |  |
| <code>version: Option&lt;<a href="leyline.html#httpversion">HttpVersion</a>&gt;</code> |  |

**Trait implementations:** <code>Debug</code>


### `Tls`

**Fields**

| Field | Description |
| --- | --- |
| <code>alpn: Option&lt;&amp;'a str&gt;</code> |  |
| <code>cipher: Option&lt;&amp;'a str&gt;</code> |  |
| <code>elapsed: Duration</code> |  |
| <code>host: &amp;'a str</code> |  |
| <code>id: u64</code> |  |
| <code>version: Option&lt;&amp;'a str&gt;</code> |  |

**Trait implementations:** <code>Debug</code>


### `TracingTrace`

**Trait implementations:** <code>Clone</code>, <code>Default</code>, <code>Debug</code>, <code>Copy</code>, <code><a href="#trace">Trace</a></code>


## Enums

### `BodyOutcome`

**Variants**

| Variant | Description |
| --- | --- |
| <code>Complete</code> |  |
| <code>Dropped</code> |  |
| <code>Failed(&amp;'a Error)</code> |  |

**Trait implementations:** <code>Debug</code>


## Traits

### `Trace`

**Methods**

| Method | Description |
| --- | --- |
| <code>body(&amp;self, ev: &amp;<a href="#bodyend">BodyEnd</a>&lt;'_&gt;)</code> |  |
| <code>connect(&amp;self, ev: &amp;<a href="#connect">Connect</a>&lt;'_&gt;)</code> |  |
| <code>dns(&amp;self, ev: &amp;<a href="#dns">Dns</a>&lt;'_&gt;)</code> |  |
| <code>done(&amp;self, ev: &amp;<a href="#done">Done</a>&lt;'_&gt;)</code> |  |
| <code>head(&amp;self, ev: &amp;<a href="#head">Head</a>&lt;'_&gt;)</code> |  |
| <code>sent(&amp;self, ev: &amp;<a href="#sent">Sent</a>&lt;'_&gt;)</code> |  |
| <code>summary(&amp;self, ev: &amp;<a href="#summary">Summary</a>&lt;'_&gt;)</code> | One event per request. |
| <code>tls(&amp;self, ev: &amp;<a href="#tls">Tls</a>&lt;'_&gt;)</code> |  |


---

Generated by truesight from `leyline-http` 0.1.3. See the [API overview](index.md).
