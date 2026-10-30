# Fingerprints

A browser profile is a recorded description of how one browser build looks on
the wire. Leyline replays it across four layers. This page covers what a
profile pins, how to audit and check a session's fingerprint, which headers
are safe to override, and how to mix and rotate identities.

## What a profile pins

| Layer | Holds |
| --- | --- |
| TLS (`[tls]`) | The cipher list, supported groups, signature algorithms, certificate compression, and extension behavior: GREASE, ECH GREASE, extension permutation or a fixed order, ALPS, record size limit, delegated credentials, OCSP stapling, signed certificate timestamps, session tickets, pre-shared key, padding, trust anchor requests, and the minimum TLS version |
| HTTP/2 (`[h2]`) | The SETTINGS values and their order, the pseudo-header order, the default priority frame, and per-platform overrides |
| Headers | The `User-Agent` and `sec-ch-ua` of each platform block. The header style in `profiles/headers.toml` and the `RequestKind` decide the other headers and their order. See [Requests](requests.md) |
| TCP (`TcpProfile`) | The socket options that shape the SYN: TTL, MSS, window size, window scale, the don't-fragment bit, and `TCP_NODELAY` |

The session takes its `TcpProfile` from `profiles/platforms.toml` through
`Platform::tcp_profile`; `SessionBuilder::tcp_profile` overrides it. Each
OS applies what it can:

| OS | Applies |
| --- | --- |
| Linux | TTL, MSS, don't-fragment, and a `TCP_WINDOW_CLAMP` derived from the window fields |
| macOS | TTL, MSS, and don't-fragment |
| Windows | TTL and don't-fragment |

Every OS sets `TCP_NODELAY` when the profile asks for it. The SYN option
order in `options` is chosen by the kernel and cannot be set from a socket,
so only the audit's JA4T reads it. `platforms.toml` has rows for Windows,
macOS, Linux, and iOS. Android has none, so its `TcpProfile` is empty and the
kernel defaults apply.

## Audit a session

Build the session with `.audit(true)`, then read `Response::fingerprint()`.
Without the flag, `fingerprint()` returns `None`.

```rust,no_run
use leyline::{Browser, Platform, Session};

# async fn run() -> leyline::Result<()> {
let session = Session::builder()
    .browser(Browser::Chrome152).platform(Platform::Linux)
    .audit(true)
    .build()?;

let resp = session.get("https://example.com/").send().await?;
if let Some(sent) = resp.fingerprint() {
    println!("JA4 {} JA4H {} H2 {}", sent.ja4(), sent.ja4h(), sent.akamai());
}
# Ok(())
# }
```

| `Fingerprint` method | What it returns |
| --- | --- |
| `ja3()` | JA3, the MD5 form. It hashes the extensions in the order sent |
| `ja4()` | JA4. It sorts the extensions before it hashes them |
| `ja4t()` | JA4T from the session's `TcpProfile` values, including fields the running OS does not apply |
| `ja4h()` | JA4H over the request line and headers of this response's request. Computed on the first `fingerprint()` call and cached |
| `akamai()` | The Akamai HTTP/2 fingerprint |
| `headers()` | The request headers as sent, lowercase names |

A TLS echo service answers with the fingerprints of the connection it
received. `Fingerprint::from_json` reads `tls.ja4`, `tls.ja3_hash`,
`http2.akamai_fingerprint`, and the echoed request headers from
`http2.sent_frames[*].headers` and `http1.headers`: the shape that
`https://tls.peet.ws/api/all` returns. A field the service leaves out stays
empty. `sent.diff(&observed)` lists each difference as a
`(field, expected, observed)` tuple; an empty list means a match.

```rust,no_run
use leyline::{Browser, Fingerprint, Session};

# async fn run() -> leyline::Result<()> {
let session = Session::builder()
    .browser(Browser::default())
    .audit(true)
    .build()?;
let resp = session.get("https://tls.peet.ws/api/all").send().await?;
let Some(sent) = resp.fingerprint().cloned() else {
    return Ok(());
};
let observed = Fingerprint::from_json(&resp.text().await?)?;
for (field, expected, seen) in sent.diff(&observed) {
    println!("{field}: sent {expected}, service saw {seen}");
}
# Ok(())
# }
```

`diff` compares `ja4`, `ja3`, `akamai`, the header order of the headers both
sides hold, and each header the session sent, pseudo-headers excluded. Values
match when they are equal ignoring ASCII case. A field the service did not
report is skipped. A header the session sent that the service does not list,
while it lists others, appears with an empty observed value.

JA4 and the Akamai HTTP/2 fingerprint are fixed for every profile, so any
difference is listed. JA3 hashes the extensions in the order sent, and Chrome
shuffles that order, so for a Chrome-family profile a JA3 difference is not
listed; for a fixed-order profile such as Firefox or Safari it is. A header
that a proxy or middlebox rewrites is listed. A proxy that ends TLS changes
the result.

## Override headers safely

On a browser session, a header you set replaces the profile's header of the
same name in its position; [Requests](requests.md#order) gives the placement
of a new name.

| Header | Effect of an override |
| --- | --- |
| `accept-language` | Safe. Prefer [`languages`](sessions.md#set-the-languages) |
| `referer` | Safe. `RequestBuilder::from_page` is better: it also sets `origin` and `sec-fetch-site` |
| `authorization`, `content-type`, application headers such as `x-api-key` | Safe |
| `cookie` | Safe. The jar adds no cookies to that request |
| `user-agent` | Changes the fingerprint. A session `user-agent` drops the profile's `sec-ch-ua*` hints, so the two never name different browsers |
| `sec-ch-ua*`, `accept-encoding`, header order | Changes the fingerprint |
| `accept` | Changes the fingerprint on a navigation. See [Set accept on a script request](#set-accept-on-a-script-request) |
| `sec-fetch-*` | Your value replaces the computed one. Use `kind` and `from_page` instead |
| `content-length` | Computed by Leyline. Your value is replaced |
| `host` | Computed from the URL. Your value is kept on HTTP/1.1 and dropped on HTTP/2 and HTTP/3 |

### Set accept on a script request

Keep the profile's `accept` on `open`, `follow`, and the navigation presets.
A page script sets its own `accept` on a `fetch()` or an XHR, for example
`application/json`. The `Xhr` kind sends what a `fetch()` without an
explicit `accept` sends. When the site's script sets one, set the same value:
the browser sends it too, so the fingerprint does not change.

```rust,no_run
# async fn run(session: leyline::Session) -> leyline::Result<()> {
let tab = session.tab();
tab.open("https://shop.example/cart").await?;
let cart = tab
    .fetch(leyline::http::Method::GET, "/api/cart")
    .header("accept", "application/json")
    .send()
    .await?;
# let _ = cart;
# Ok(())
# }
```

## Mix the hello and the headers

A browser session sends two browsers and a platform. The hello browser
supplies the ClientHello, the HTTP/2 and HTTP/3 settings, and the `ja3`,
`ja4`, and `h2_fingerprint` audit values. The header browser supplies the
`User-Agent`, `sec-ch-ua`, `Accept-Language`, the header shape, and the
header order. The platform selects the OS that the `User-Agent` names, the
HTTP/2 platform overrides, and the `TcpProfile`.

| Setter | Hello side | Header side | `Kind::Config` at `build()` when |
| --- | --- | --- | --- |
| `browser(b)` | `b` | `b` | `b` has no profile for the platform |
| `hello(b)` | `b` | unchanged | `b` is of another product family than the header browser |
| `platform(p)` | unchanged | unchanged | The header browser has no profile for `p` |

`.browser(b)` picks the browser's native platform unless `.platform(p)` is
set, before or after; `.platform(p)` always wins. A second `.browser(..)`
replaces the first. Call `.hello(..)` after `.browser(..)`.

`Browser::next_hello()` returns the next distinct ClientHello of the same
product in `Browser::all()` order, and wraps. It returns `None` when the
product has one ClientHello. A profile can share its ClientHello with
another version; its `[meta]` table names that version in `hello`.
`next_hello` returns that version and counts shared ClientHellos once.

### Set the browser and platform

This session sends the headers of Chrome 154 on Linux over the ClientHello,
HTTP/2 settings, and HTTP/3 settings of Chrome 152:

```rust,no_run
use leyline::{Browser, Platform, Session};

# fn run() -> leyline::Result<()> {
let session = Session::builder()
    .browser(Browser::Chrome154)
    .platform(Platform::Linux)
    .hello(Browser::Chrome152)
    .build()?;
assert_eq!(session.browser(), Some(Browser::Chrome154));
assert_eq!(session.hello(), Some(Browser::Chrome152));
# Ok(())
# }
```

`build()` returns `Kind::Config` when the header browser has no profile
for the platform, for example Safari 26 on Windows. A session built with
`.profile(..)` sends that one profile on both sides.

### Change the browser of a session

`session.to_builder().browser(b).build()` derives a session that sends
another browser; the parent keeps its own. `hello` and `platform` work on
the derived builder as they do on `Session::builder()`. The derived session
takes the ClientHello, the HTTP/2 and HTTP/3 settings, the headers, and the
audit values from the new setters.

| Part | In the derived session |
| --- | --- |
| Connection pool | Shared with the parent, in its own partition, so connections never cross identities |
| HSTS store and `Alt-Svc` knowledge | Shared with the parent |
| TLS session cache | Its own, empty |
| Cookie jar | The parent's. Give it another with `to_builder().cookies` |
| Proxy, timeouts, `TcpProfile`, other settings | Kept |

A `user-agent` header from `SessionBuilder::headers` still replaces the
browser's `User-Agent`. `Session::browser()`, `hello()`, and `platform()`
report what the derived session sends. See [Sessions](sessions.md#derive-a-session).

```rust,no_run
use leyline::{Browser, Platform, Session};

# fn run() -> leyline::Result<()> {
let chrome = Browser::chrome();
let session = Session::builder()
    .browser(chrome)
    .platform(Platform::Linux)
    .build()?;

let mut next_hello = session.to_builder();
if let Some(hello) = chrome.next_hello() {
    next_hello = next_hello.hello(hello);
}
let next_hello = next_hello.build()?;

let switched = session.to_builder().browser(Browser::firefox()).build()?;
# let _ = (next_hello, switched);
# Ok(())
# }
```

`build()` after `to_builder()` returns `Kind::Config` when:

- The header browser has no profile for the platform, such as Edge or
  Opera on Android or iOS.
- The policy is `Protocol::Http3` or `Race` and the hello browser has
  no `[h3]` table. Under `Auto`, `Http1`, or `Http2` that is accepted. See
  [HTTP/3](http3.md).

## Read and test the bundled profiles

A variant such as `Browser::Chrome152` names a bundled browser, and
`profiles/chrome/152.toml` in the repository holds its profile.
`Session::profile()` returns the profile TOML a session uses. The
[profile reference](profiles.md) lists what ships and how to load your own.

```rust,no_run
use leyline::{Browser, Session};

# fn main() -> leyline::Result<()> {
let session = Session::builder().browser(Browser::Chrome152).build()?;
let toml = session.profile().expect("bundled profile");
assert!(toml.contains("[tls]"));
# Ok(())
# }
```

The offline conformance test checks each profile's JA4 and HTTP/2 values
against its recorded reference values without the network. It needs the
`bench-internals` feature:

```sh
cargo test -p leyline-http --features bench-internals --test it fingerprint_conformance::
```

It checks the profile data against the reference values; it does not capture
a browser.

## Next

Read [Features and targets](features-and-targets.md).
