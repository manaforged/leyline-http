# Fingerprints

A browser profile is a recorded description of how one browser build looks on
the wire. Leyline replays it across four layers. This page covers what a
profile pins, how to audit and check a session's fingerprint, which headers
are safe to override, and how to mix and rotate identities.

## What a profile pins

| Layer | Holds |
| --- | --- |
| TLS (`[tls]`) | ClientHello ciphers, curves, signature algorithms, certificate compression, and extension ordering/randomization that form JA3 and JA4 |
| HTTP/2 (`[h2]`) | `SETTINGS` values and order, pseudo-header order, default priority frame, and per-platform overrides (Akamai H2 fingerprint) |
| Headers | `User-Agent`, `sec-ch-ua`, and per-`RequestKind` headers and order (JA4H). See [Requests](requests.md) |
| TCP (`TcpProfile`) | Socket options shaping the TCP SYN packet: TTL, MSS, window size/scale, don't-fragment bit, and `TCP_NODELAY` (JA4T) |

The session takes its `TcpProfile` from `Platform::tcp_profile`;
`SessionBuilder::tcp_profile` overrides it. Because operating systems restrict
raw TCP SYN control from unprivileged sockets, `TcpProfile` is best-effort per
OS:

| OS | Applies |
| --- | --- |
| Linux | TTL, MSS, don't-fragment, and a `TCP_WINDOW_CLAMP` derived from the window fields |
| macOS | TTL and don't-fragment |
| Windows | TTL and don't-fragment |

Every OS applies `TCP_NODELAY` when the profile asks for it. The kernel, not
Leyline, writes the TCP option order in the SYN. A Linux host that claims
`Platform::Windows` still sends Linux's option order. `Fingerprint::ja4t()`
reports the configured values, not what the kernel sent.

### Customize `TcpProfile`

Build a custom `leyline::profile::TcpProfile` from a JA4T string
(`window_options_mss_wscale`) with `TcpProfile::from_ja4t`, or from TOML with
`str::parse`. `TcpProfile::new()` returns an empty profile that keeps every OS
default; it has no setters. Inspect fields with
`ja4t()`, `ttl()`, `mss()`, `window_size()`, `window_scale()`, `df()`,
`no_delay()`, and `options()`. Unknown TOML keys or out-of-range values return
`Kind::Config`.

```rust,no_run
use leyline::Session;
use leyline::profile::TcpProfile;

# fn run() -> leyline::Result<()> {
let from_ja4t = TcpProfile::from_ja4t("64240_2-4-8-1-3_1380_10")?;
let from_toml: TcpProfile = "ttl = 64\nmss = 1380\ndf = true".parse()?;
let session = Session::builder().tcp_profile(from_ja4t).build()?;
println!("{} ttl={:?} no_delay={}", from_toml.ja4t(), from_toml.ttl(), from_toml.no_delay());
# drop(session);
# Ok(())
# }
```

## Audit a session

Build the session with `.audit(true)`, then read `Response::fingerprint()`.
Without the flag, `fingerprint()` returns `None`.

```rust,no_run
use leyline::{Browser, Platform, Session};

# async fn run() -> leyline::Result<()> {
let session = Session::builder()
    .browser(Browser::Chrome158).platform(Platform::Linux)
    .audit(true)
    .build()?;

let resp = session.get("https://example.com/").send().await?;
if let Some(sent) = resp.fingerprint() {
    println!("JA4 {} JA4H {} H2 {}", sent.ja4(), sent.ja4h(), sent.akamai());
}
# Ok(())
# }
```

| `Fingerprint` method | Layer & what it measures |
| --- | --- |
| `ja3()` | TLS ClientHello MD5 hash (extension order sensitive; varies per Chrome connection) |
| `ja4()` | Modern TLS ClientHello hash (sorts extensions so Chrome produces one stable JA4) |
| `ja4t()` | TCP SYN packet parameters (`window_options_mss_wscale`) from the session's `TcpProfile` |
| `ja4h()` | HTTP request method, version, header count, language, and header-order hash (computed lazily) |
| `akamai()` | HTTP/2 `SETTINGS`, `WINDOW_UPDATE`, priority, and pseudo-header order (`1:65536;...\|m,a,s,p`) |
| `headers()` | Regular request headers as prepared, lowercase names |

A TLS echo service answers with the fingerprints of the connection it
received. `Fingerprint::from_json` reads `tls.ja4`, `tls.ja3_hash`,
`http2.akamai_fingerprint`, and the echoed request headers from
`http2.sent_frames[*].headers` and `http1.headers`: the shape that
`https://tls.peet.ws/api/all` returns. A field the service leaves out stays
empty. `sent.diff(&observed)` lists each difference as a
`(field, expected, observed)` tuple; an empty list means a match.

```rust,no_run
use leyline::profile::Fingerprint;
use leyline::{Browser, Session};

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

`diff` compares `ja4`, `ja3`, `akamai`, header order, and each regular header
(pseudo-headers, `ja4h`, and `ja4t` are not compared). Values match
case-insensitively, and fields omitted by the echo server are skipped.

JA4 and the Akamai HTTP/2 string are fixed for every profile. Chrome shuffles
its TLS extension order per connection, so `diff` ignores JA3 on Chrome-family
profiles. It checks JA3 on fixed-order profiles such as Firefox and Safari. A MITM proxy
that terminates TLS changes the observed fingerprint.

### If your diff is empty but you are still blocked

A clean `tls.peet.ws` diff confirms TLS, HTTP/2, and header order. Anti-bot
systems also check layers outside that one request:

1. TCP options. The kernel writes the SYN option order, so a Linux host that claims `Platform::Windows` still looks like Linux. See [What a profile pins](#what-a-profile-pins).
2. Fetch context. A plain `session.get(...)` sends `Sec-Fetch-Site`, `Referer`, and `Origin` like a typed URL. Use a `Tab` or `from_page(...)` for in-page requests.
3. Cookies across exit IPs. `Proxy::list` shares one cookie jar across all proxies. `Proxy::identified` gives each proxy its own jar, and `.cookies(None)` stores none.
4. HTTP/3 behind an HTTP proxy. An `http://` or `https://` proxy keeps `Protocol::Race` on TCP. Check HTTP/3 directly or through a `socks5://` proxy against `https://quic.browserleaks.com/?minify=1`.
5. JavaScript. Leyline is an HTTP client, not a browser. It cannot solve JavaScript proof-of-work or Turnstile challenges.

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
| `user-agent` | Changes the fingerprint. It also drops the profile's `sec-ch-ua*` hints unless you set them, so the two never disagree |
| `sec-ch-ua*`, `accept-encoding`, header order | Changes the fingerprint |
| `accept` | Changes the fingerprint on a navigation. See [Set accept on a script request](#set-accept-on-a-script-request) |
| `sec-fetch-*` | Your value replaces the computed one. Use `kind` and `from_page` instead |
| `content-length` | Computed by Leyline. Your value is replaced |
| `host` | Computed from the URL. Your value is kept on HTTP/1.1 and dropped on HTTP/2 and HTTP/3 |

### Set accept on a script request

Keep the profile's `accept` on `open`, `follow`, and the navigation request
kinds. A page script sets its own `accept` on a `fetch()` or an XHR, for example
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

Use this to match a browser release whose `User-Agent` changed before its TLS
ClientHello did, or to rotate ClientHellos within one browser family. A browser
session has two halves. `hello` picks the TLS ClientHello and the HTTP/2 and
HTTP/3 settings, so it sets `ja3()`, `ja4()`, and `akamai()`. `browser` picks
`User-Agent`, `sec-ch-ua`, `Accept-Language`, and header order. Both default to
the same version. `platform` picks the OS headers, the HTTP/2 platform
overrides, and the `TcpProfile`.

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
product has one ClientHello. Some versions reuse an older version's
ClientHello, and their `[meta] hello` key names it. `next_hello` skips those
versions, so each step changes the ClientHello.

### Set the browser and platform

This session sends the headers of Chrome 158 on Linux over the ClientHello,
HTTP/2 settings, and HTTP/3 settings of Chrome 157:

```rust,no_run
use leyline::{Browser, Platform, Session};

# fn run() -> leyline::Result<()> {
let session = Session::builder()
    .browser(Browser::Chrome158)
    .platform(Platform::Linux)
    .hello(Browser::Chrome157)
    .build()?;
assert_eq!(session.browser(), Some(Browser::Chrome158));
assert_eq!(session.hello(), Some(Browser::Chrome157));
# Ok(())
# }
```

`build()` returns `Kind::Config` when the header browser has no profile
for the platform, for example Safari 26 on Windows. A session built with
`.profile(..)` sends that one profile on both sides.

### Change the browser of a session

`session.to_builder().browser(b).build()` derives a session that sends another
browser (leaving the parent unchanged) in its own partition of the shared
connection pool with a fresh TLS session cache; see
[Derive a session](sessions.md#derive-a-session) for the full sharing table.
A session `user-agent` still replaces the browser's `User-Agent`, as in
[Override headers safely](#override-headers-safely). `Session::browser()`, `hello()`,
and `platform()` report the resolved identity.

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
- The hello browser has no `[h3]` table and the policy needs one. See
  [HTTP/3](http3.md#protocol-policy).

## Read and test the bundled profiles

A variant such as `Browser::Chrome158` names a bundled browser, and
`crates/leyline/profiles/chrome/158.toml` in the repository holds its profile.
`Session::profile()` returns the profile TOML a session uses. The
[profile reference](profiles.md) lists what ships and how to load your own.

```rust,no_run
use leyline::{Browser, Session};

# fn main() -> leyline::Result<()> {
let session = Session::builder().browser(Browser::Chrome158).build()?;
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
