# Module `leyline::testing`

| Item | Kind | Description |
| --- | --- | --- |
| [`RecordedRequest`](#recordedrequest) | struct | Requires feature `test-util`. |
| [`TestResponse`](#testresponse) | struct | Requires feature `test-util`. |
| [`TestServer`](#testserver) | struct | Requires feature `test-util`. |
| [`queue`](#queue) | fn | Requires feature `test-util`. |
| [`Handler`](#handler) | type | Requires feature `test-util`. |

## Structs

### `RecordedRequest`

Requires feature `test-util`.

**Methods**

| Method | Description |
| --- | --- |
| <code>header(&amp;self, name: &amp;str) -&gt; Option&lt;&amp;str&gt;</code> | Requires feature `test-util`. |
| <code>header_count(&amp;self, name: &amp;str) -&gt; usize</code> | Count a header. Requires feature `test-util`. |
| <code>header_values(&amp;self, name: &amp;str) -&gt; Vec&lt;&amp;str&gt;</code> | Read every value of a header. Requires feature `test-util`. |
| <code>text(&amp;self) -&gt; Cow&lt;'_, str&gt;</code> | Requires feature `test-util`. |

**Fields**

| Field | Description |
| --- | --- |
| <code>body: Vec&lt;u8&gt;</code> | Requires feature `test-util`. |
| <code>headers: Vec&lt;(String, String)&gt;</code> | Requires feature `test-util`. |
| <code>method: String</code> | Requires feature `test-util`. |
| <code>raw: Vec&lt;u8&gt;</code> | Read the request head as received. Requires feature `test-util`. |
| <code>request_line: String</code> | Read the request line. Requires feature `test-util`. |
| <code>target: String</code> | Requires feature `test-util`. |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Debug</code>, <code>StructuralPartialEq</code>


### `TestResponse`

Requires feature `test-util`.

**Methods**

| Method | Description |
| --- | --- |
| <code>body(self, body: impl Into&lt;Vec&lt;u8&gt;&gt;) -&gt; Self</code> | Requires feature `test-util`. |
| <code>chunks&lt;I, B&gt;(self, parts: I, pause: Duration) -&gt; Self where I: IntoIterator&lt;Item = B&gt;, B: Into&lt;Vec&lt;u8&gt;&gt;</code> | Requires feature `test-util`. |
| <code>close(self) -&gt; Self</code> | Requires feature `test-util`. |
| <code>delay(self, delay: Duration) -&gt; Self</code> | Requires feature `test-util`. |
| <code>header(self, name: impl Into&lt;String&gt;, value: impl Into&lt;String&gt;) -&gt; Self</code> | Requires feature `test-util`. |
| <code>new(status: u16) -&gt; Self</code> | Requires feature `test-util`. |

**Fields**

| Field | Description |
| --- | --- |
| <code>body: Vec&lt;u8&gt;</code> | Requires feature `test-util`. |
| <code>headers: Vec&lt;(String, String)&gt;</code> | Requires feature `test-util`. |
| <code>status: u16</code> | Requires feature `test-util`. |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Default</code>, <code>Debug</code>, <code>StructuralPartialEq</code>


### `TestServer`

Requires feature `test-util`.

**Methods**

| Method | Description |
| --- | --- |
| <code>addr(&amp;self) -&gt; SocketAddr</code> | Requires feature `test-util`. |
| <code>ca_der(&amp;self) -&gt; Option&lt;&amp;[u8]&gt;</code> | Requires feature `test-util`. |
| <code>async http&lt;F&gt;(handler: F) -&gt; Result&lt;Self&gt; where F: Fn(&amp;<a href="#recordedrequest">RecordedRequest</a>) -&gt; <a href="#testresponse">TestResponse</a> + Send + Sync + 'static</code> | Run a local test server. Requires feature `test-util`. |
| <code>http_on&lt;F&gt;(listener: TcpListener, handler: F) -&gt; Result&lt;Self&gt; where F: Fn(&amp;<a href="#recordedrequest">RecordedRequest</a>) -&gt; <a href="#testresponse">TestResponse</a> + Send + Sync + 'static</code> | Run a test server on your listener. Requires feature `test-util`. |
| <code>async https&lt;F&gt;(handler: F) -&gt; Result&lt;Self&gt; where F: Fn(&amp;<a href="#recordedrequest">RecordedRequest</a>) -&gt; <a href="#testresponse">TestResponse</a> + Send + Sync + 'static</code> | Run a local HTTPS test server. Requires feature `test-util`. |
| <code>async next_request(&amp;self) -&gt; Option&lt;<a href="#recordedrequest">RecordedRequest</a>&gt;</code> | Requires feature `test-util`. |
| <code>async requests(&amp;self) -&gt; Vec&lt;<a href="#recordedrequest">RecordedRequest</a>&gt;</code> | Requires feature `test-util`. |
| <code>async shutdown(self)</code> | Requires feature `test-util`. |
| <code>trust(&amp;self) -&gt; <a href="leyline.html#tlstrustconfig">TlsTrustConfig</a></code> | Requires feature `test-util`. |
| <code>url(&amp;self, path: &amp;str) -&gt; String</code> | Requires feature `test-util`. |

**Trait implementations:** <code>Debug</code>, <code>Drop</code>


## Functions

### `queue`

<pre>pub fn <a href="#queue">queue</a>(responses: impl IntoIterator&lt;Item = <a href="#testresponse">TestResponse</a>&gt;) -&gt; impl Fn(&amp;<a href="#recordedrequest">RecordedRequest</a>) -&gt; <a href="#testresponse">TestResponse</a> + Send + Sync + 'static</pre>

Requires feature `test-util`.


## Type aliases

### `Handler`

<pre>pub type <a href="#handler">Handler</a> = Arc&lt;(dyn Fn(&amp;<a href="#recordedrequest">RecordedRequest</a>) -&gt; <a href="#testresponse">TestResponse</a> + Send + Sync)&gt;</pre>

Requires feature `test-util`.


---

Generated by truesight from `leyline-http` 0.1.3. See the [API overview](index.md).
