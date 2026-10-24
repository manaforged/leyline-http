# Module `leyline::html`

| Item | Kind | Description |
| --- | --- | --- |
| [`Anchor`](#anchor) | struct | Requires feature `html`. |
| [`Directives`](#directives) | struct | Requires feature `html`. |
| [`Form`](#form) | struct | Requires feature `html`. |
| [`FormEnctype`](#formenctype) | enum | Form encoding. Requires feature `html`. |
| [`FormMethod`](#formmethod) | enum | Requires feature `html`. |
| [`canonical`](#canonical) | fn | Read a page's canonical URL. Requires feature `html`. |
| [`forms`](#forms) | fn | Parse forms. Requires feature `html`. |
| [`links`](#links) | fn | Read the links of a page. Requires feature `html`. |
| [`meta`](#meta) | fn | Read the meta tags of a page. Requires feature `html`. |
| [`text`](#text) | fn | Read the visible text of a page. Requires feature `html`. |

## Structs

### `Anchor`

Requires feature `html`.

**Methods**

| Method | Description |
| --- | --- |
| <code>href(&amp;self) -&gt; &amp;str</code> | Requires feature `html`. |
| <code>rel(&amp;self) -&gt; Option&lt;&amp;str&gt;</code> | Requires feature `html`. |
| <code>text(&amp;self) -&gt; &amp;str</code> | Requires feature `html`. |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Debug</code>, <code>StructuralPartialEq</code>


### `Directives`

Requires feature `html`.

**Methods**

| Method | Description |
| --- | --- |
| <code>ai(&amp;self) -&gt; bool</code> | Requires feature `html`. |
| <code>follow(&amp;self) -&gt; bool</code> | Requires feature `html`. |
| <code>index(&amp;self) -&gt; bool</code> | Requires feature `html`. |
| <code>read(document: &amp;str, headers: &amp;HeaderMap, agent: &amp;str) -&gt; <a href="#directives">Directives</a></code> | Read a page's robots directives. Requires feature `html`. |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Debug</code>, <code>Copy</code>, <code>StructuralPartialEq</code>


### `Form`

Requires feature `html`.

**Methods**

| Method | Description |
| --- | --- |
| <code>action(&amp;self) -&gt; &amp;str</code> | Requires feature `html`. |
| <code>base(&amp;self) -&gt; Option&lt;&amp;str&gt;</code> | Requires feature `html`. |
| <code>buttons(&amp;self) -&gt; Vec&lt;&amp;str&gt;</code> | Requires feature `html`. |
| <code>enctype(&amp;self) -&gt; <a href="#formenctype">FormEnctype</a></code> | Requires feature `html`. |
| <code>field(&amp;self, name: &amp;str) -&gt; Option&lt;&amp;str&gt;</code> | Requires feature `html`. |
| <code>fields(&amp;self) -&gt; &amp;[(String, String)]</code> | Requires feature `html`. |
| <code>find(document: &amp;str, selector: &amp;str) -&gt; Option&lt;<a href="#form">Form</a>&gt;</code> | Find a form by id or name. Requires feature `html`. |
| <code>id(&amp;self) -&gt; Option&lt;&amp;str&gt;</code> | Requires feature `html`. |
| <code>method(&amp;self) -&gt; <a href="#formmethod">FormMethod</a></code> | Requires feature `html`. |
| <code>name(&amp;self) -&gt; Option&lt;&amp;str&gt;</code> | Requires feature `html`. |
| <code>press(&amp;mut self, button: &amp;str) -&gt; bool</code> | Requires feature `html`. |
| <code>set(&amp;mut self, name: impl Into&lt;String&gt;, value: impl Into&lt;String&gt;) -&gt; &amp;mut Self</code> | Fill a form. Requires feature `html`. |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Default</code>, <code>Debug</code>, <code>StructuralPartialEq</code>


## Enums

### `FormEnctype`

Form encoding. Requires feature `html`.

**Methods**

| Method | Description |
| --- | --- |
| <code>as_str(self) -&gt; &amp;'static str</code> | Requires feature `html`. |

**Variants**

| Variant | Description |
| --- | --- |
| <code>Multipart</code> | Requires feature `html`. |
| <code>TextPlain</code> | Requires feature `html`. |
| <code>UrlEncoded</code> | Requires feature `html`. |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Default</code>, <code>Debug</code>, <code>Hash</code>, <code>Copy</code>, <code>StructuralPartialEq</code>


### `FormMethod`

Requires feature `html`.

**Variants**

| Variant | Description |
| --- | --- |
| <code>Get</code> | Requires feature `html`. |
| <code>Post</code> | Requires feature `html`. |

**Trait implementations:** <code>Clone</code>, <code>Eq</code>, <code>PartialEq</code>, <code>Default</code>, <code>Debug</code>, <code>Hash</code>, <code>Copy</code>, <code>StructuralPartialEq</code>


## Functions

### `canonical`

<pre>pub fn <a href="#canonical">canonical</a>(document: &amp;str) -&gt; Option&lt;String&gt;</pre>

Read a page's canonical URL. Requires feature `html`.


### `forms`

<pre>pub fn <a href="#forms">forms</a>(document: &amp;str) -&gt; Vec&lt;<a href="#form">Form</a>&gt;</pre>

Parse forms. Requires feature `html`.


### `links`

<pre>pub fn <a href="#links">links</a>(document: &amp;str) -&gt; Vec&lt;<a href="#anchor">Anchor</a>&gt;</pre>

Read the links of a page. Requires feature `html`.


### `meta`

<pre>pub fn <a href="#meta">meta</a>(document: &amp;str, name: &amp;str) -&gt; Option&lt;String&gt;</pre>

Read the meta tags of a page. Requires feature `html`.


### `text`

<pre>pub fn <a href="#text">text</a>(document: &amp;str) -&gt; String</pre>

Read the visible text of a page. Requires feature `html`.


---

Generated by truesight from `leyline-http` 0.1.6. See the [API overview](index.md).
