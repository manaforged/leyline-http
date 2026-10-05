# Module `leyline::audit`

| Item | Kind | Description |
| --- | --- | --- |
| [`AuditData`](#auditdata) | struct |  |
| [`FingerprintReport`](#fingerprintreport) | struct |  |
| [`HeaderOutcome`](#headeroutcome) | struct |  |
| [`Ja3Input`](#ja3input) | struct |  |
| [`Ja4Input`](#ja4input) | struct |  |
| [`Ja4hInput`](#ja4hinput) | struct |  |
| [`Observed`](#observed) | struct |  |
| [`FieldOutcome`](#fieldoutcome) | enum |  |
| [`compute_ja3`](#compute_ja3) | fn |  |
| [`compute_ja4`](#compute_ja4) | fn | Compute a JA4 offline. |
| [`compute_ja4h`](#compute_ja4h) | fn |  |
| [`compute_ja4t`](#compute_ja4t) | fn |  |

## Structs

### `AuditData`

**Methods**

| Method | Description |
| --- | --- |
| <code>compare(&amp;self, observed: &amp;<a href="#observed">Observed</a>) -&gt; <a href="#fingerprintreport">FingerprintReport</a></code> | Compare with an echo service. |

**Fields**

| Field | Description |
| --- | --- |
| <code>h2_fingerprint: String</code> |  |
| <code>ja3: String</code> |  |
| <code>ja4: String</code> |  |
| <code>ja4h: String</code> |  |
| <code>ja4t: String</code> |  |
| <code>permutes_extensions: bool</code> |  |
| <code>request_headers: Vec&lt;(String, String)&gt;</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Debug</code>


### `FingerprintReport`

**Methods**

| Method | Description |
| --- | --- |
| <code>is_match(&amp;self) -&gt; bool</code> |  |

**Fields**

| Field | Description |
| --- | --- |
| <code>h2_fingerprint: <a href="#fieldoutcome">FieldOutcome</a></code> |  |
| <code>header_order: <a href="#fieldoutcome">FieldOutcome</a></code> |  |
| <code>headers: Vec&lt;<a href="#headeroutcome">HeaderOutcome</a>&gt;</code> |  |
| <code>ja3: <a href="#fieldoutcome">FieldOutcome</a></code> |  |
| <code>ja4: <a href="#fieldoutcome">FieldOutcome</a></code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Debug</code>, <code>Display</code>, <code>StructuralPartialEq</code>


### `HeaderOutcome`

**Fields**

| Field | Description |
| --- | --- |
| <code>name: String</code> |  |
| <code>outcome: <a href="#fieldoutcome">FieldOutcome</a></code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Debug</code>, <code>StructuralPartialEq</code>


### `Ja3Input`

**Fields**

| Field | Description |
| --- | --- |
| <code>ciphers: &amp;'a [String]</code> |  |
| <code>curves: &amp;'a [String]</code> |  |
| <code>extension_ids: &amp;'a [u16]</code> |  |
| <code>tls_record_version: u16</code> |  |

**Trait implementations:** <code>Debug</code>


### `Ja4Input`

**Fields**

| Field | Description |
| --- | --- |
| <code>alpn: &amp;'a str</code> |  |
| <code>ciphers: &amp;'a [String]</code> |  |
| <code>curves: &amp;'a [String]</code> |  |
| <code>extension_ids: &amp;'a [u16]</code> |  |
| <code>has_sni: bool</code> |  |
| <code>sigalgs: &amp;'a [String]</code> |  |
| <code>tls_version: &amp;'a str</code> |  |

**Trait implementations:** <code>Debug</code>


### `Ja4hInput`

**Fields**

| Field | Description |
| --- | --- |
| <code>headers: &amp;'a [(String, String)]</code> |  |
| <code>http_version: &amp;'a str</code> |  |
| <code>method: &amp;'a str</code> |  |

**Trait implementations:** <code>Debug</code>


### `Observed`

**Methods**

| Method | Description |
| --- | --- |
| <code>from_json(text: &amp;str) -&gt; <a href="leyline.html#result">Result</a>&lt;<a href="#observed">Observed</a>&gt;</code> |  |
| <code>h2_fingerprint(self, value: impl Into&lt;String&gt;) -&gt; Self</code> |  |
| <code>header(self, name: &amp;str, value: impl Into&lt;String&gt;) -&gt; Self</code> |  |
| <code>ja3(self, value: impl Into&lt;String&gt;) -&gt; Self</code> |  |
| <code>ja4(self, value: impl Into&lt;String&gt;) -&gt; Self</code> |  |
| <code>new() -&gt; Self</code> |  |

**Fields**

| Field | Description |
| --- | --- |
| <code>h2_fingerprint: Option&lt;String&gt;</code> |  |
| <code>headers: Vec&lt;(String, String)&gt;</code> |  |
| <code>ja3: Option&lt;String&gt;</code> |  |
| <code>ja4: Option&lt;String&gt;</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Default</code>, <code>Debug</code>, <code>StructuralPartialEq</code>


## Enums

### `FieldOutcome`

**Methods**

| Method | Description |
| --- | --- |
| <code>is_mismatch(&amp;self) -&gt; bool</code> |  |

**Fields**

| Field | Description |
| --- | --- |
| <code>Absent::expected: String</code> |  |
| <code>Informational::expected: String</code> |  |
| <code>Informational::observed: String</code> |  |
| <code>Mismatch::expected: String</code> |  |
| <code>Mismatch::observed: String</code> |  |

**Variants**

| Variant | Description |
| --- | --- |
| <code>Absent</code> |  |
| <code>Informational</code> |  |
| <code>Match</code> |  |
| <code>Mismatch</code> |  |
| <code>NotReported</code> |  |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Debug</code>, <code>Display</code>, <code>StructuralPartialEq</code>


## Functions

### `compute_ja3`

<pre>pub fn <a href="#compute_ja3">compute_ja3</a>(input: &amp;<a href="#ja3input">Ja3Input</a>&lt;'_&gt;) -&gt; String</pre>


### `compute_ja4`

<pre>pub fn <a href="#compute_ja4">compute_ja4</a>(input: &amp;<a href="#ja4input">Ja4Input</a>&lt;'_&gt;) -&gt; String</pre>

Compute a JA4 offline.


### `compute_ja4h`

<pre>pub fn <a href="#compute_ja4h">compute_ja4h</a>(input: &amp;<a href="#ja4hinput">Ja4hInput</a>&lt;'_&gt;) -&gt; String</pre>


### `compute_ja4t`

<pre>pub fn <a href="#compute_ja4t">compute_ja4t</a>(tcp: &amp;<a href="leyline.html#tcpprofile">TcpProfile</a>) -&gt; String</pre>


---

Generated by truesight from `leyline-http` 0.1.2. See the [API overview](index.md).
