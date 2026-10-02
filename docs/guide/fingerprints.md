# Fingerprints

A browser profile is a recorded description of how one browser build looks on
the wire. Leyline replays it across four layers. This page covers what a
profile pins, how to audit and check a session's fingerprint, which headers
are safe to override, and how to mix and rotate identities.

## What a profile pins

| Layer | Holds |
| --- | --- |
| TLS (`TlsProfile`) | The cipher list, supported groups, signature algorithms, certificate compression, and extension behavior: GREASE, ECH GREASE, extension permutation or a fixed order, ALPS, record size limit, delegated credentials, OCSP stapling, signed certificate timestamps, session tickets, pre-shared key, padding, trust anchor requests, and the minimum TLS version |
| HTTP/2 (`H2Profile`) | The SETTINGS values and their order, the pseudo-header order, the default priority frame, and per-platform overrides |
| Headers | The `User-Agent` and `sec-ch-ua` of each platform block. The header style in `profiles/headers.toml` and the `Preset` decide the other headers and their order. See [Requests](requests.md) |
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

Build the session with `.audit(true)`, then read `Response::audit()`.
Without the flag, `audit()` returns `None`.

```rust,no_run
use leyline::{Browser, Platform, Session};

# async fn run() -> leyline::Result<()> {
let session = Session::builder()
    .browser(Browser::Chrome152)
    .platform(Platform::Linux)
    .audit(true)
    .build()?;

let resp = session.get("https://example.com/").await?;
if let Some(a) = resp.audit() {
    println!("JA4 {} JA4H {} H2 {}", a.ja4, a.ja4h, a.h2_fingerprint);
}
# Ok(())
# }
```

| `AuditData` field | What it is |
| --- | --- |
| `ja3` | JA3, the MD5 form. It hashes the extensions in the order sent |
| `ja4` | JA4. It sorts the extensions before it hashes them |
| `ja4t` | JA4T from the session's `TcpProfile` values, including fields the running OS does not apply |
| `ja4h` | JA4H over the request line and headers of this response's request. Computed on the first `audit()` call and cached |
| `h2_fingerprint` | The Akamai HTTP/2 fingerprint |
| `permutes_extensions` | `true` when the profile shuffles the TLS extension order on each connection, as Chrome does |
| `request_headers` | The headers the session prepared for the request, in order, before the transport sent them |

The connection fields are computed once per session from the profile. They
report what Leyline built; a packet capture shows what the connection sent,
and the kernel decides what the SYN carries.

## Check a fingerprint with an echo service

A TLS echo service answers with the fingerprints of the connection it
received. `AuditData::compare` checks them against the audit of the same
session.

`audit::Observed::from_json` reads `tls.ja4`, `tls.ja3_hash`,
`http2.akamai_fingerprint`, and the echoed request headers from
`http2.sent_frames[*].headers` and `http1.headers`: the shape that
`https://tls.peet.ws/api/all` returns. A missing field stays `None`. For
another service, build it with
`Observed::new().ja4(..).ja3(..).h2_fingerprint(..).header(name, value)`.

```rust,no_run
use leyline::audit::{FieldOutcome, Observed};
use leyline::{Browser, Session};

# async fn run() -> leyline::Result<()> {
let session = Session::builder()
    .browser(Browser::default())
    .audit(true)
    .build()?;
let resp = session.get("https://tls.peet.ws/api/all").send().await?;
let Some(audit) = resp.audit().cloned() else {
    return Ok(());
};
let observed = Observed::from_json(&resp.text().await?)?;
let report = audit.compare(&observed);
println!("match: {}", report.is_match());
if let FieldOutcome::Mismatch { expected, observed } = &report.ja4 {
    println!("ja4 sent {expected}, service saw {observed}");
}
for header in &report.headers {
    if let FieldOutcome::Mismatch { expected, observed } = &header.outcome {
        println!("{} sent {expected}, service saw {observed}", header.name);
    }
}
# Ok(())
# }
```

`compare` returns a `FingerprintReport` with a `FieldOutcome` for `ja4`,
`ja3`, `h2_fingerprint`, and `header_order`, and a `HeaderOutcome`
(lowercase `name` and `outcome`) in `headers` for each header the session
sent, pseudo-headers excluded. `header_order` compares the order of the
headers that both lists hold.

| `FieldOutcome` | Meaning |
| --- | --- |
| `Match` | The values are equal, ignoring ASCII case |
| `Mismatch { expected, observed }` | The values differ, and they must not |
| `Informational { expected, observed }` | The values differ, and they can |
| `NotReported` | The service did not report the field, or reported no headers at all |
| `Absent { expected }` | The session sent the header and the service reports headers, but not this one. It counts as a mismatch |

JA4 and the Akamai HTTP/2 fingerprint are fixed for every profile, so a
difference is a `Mismatch`. JA3 hashes the extensions in the order sent, and
Chrome shuffles that order, so for a profile with `permutes_extensions` a
JA3 difference is `Informational`; for a fixed-order profile such as Firefox
or Safari it is a `Mismatch`. A header that a proxy or middlebox rewrites
shows as a `Mismatch`. A proxy that ends TLS changes the result.

`is_match()` is `true` when nothing is a `Mismatch` or `Absent`;
`FieldOutcome::is_mismatch()` tests one field. Both types implement
`Display`: `println!("{report}")` prints one line per field and header.

## Override headers safely

On a browser session, a header you set replaces the profile's header of the
same name in its position; [Requests](requests.md#order) gives the placement
of a new name.

| Header | Effect of an override |
| --- | --- |
| `accept-language` | Safe. Prefer [`languages`](sessions.md#set-the-languages) |
| `referer` | Safe. `RequestBuilder::initiator` is better: it also sets `origin` and `sec-fetch-site` |
| `authorization`, `content-type`, application headers such as `x-api-key` | Safe |
| `cookie` | Safe. The jar adds no cookies to that request |
| `user-agent` | Changes the fingerprint. The `sec-ch-ua*` hints still come from the profile, so the two can disagree |
| `sec-ch-ua*`, `accept-encoding`, header order | Changes the fingerprint |
| `accept` | Changes the fingerprint on a navigation. See [Set accept on a script request](#set-accept-on-a-script-request) |
| `sec-fetch-*` | Your value replaces the computed one. Use `preset` and `initiator` instead |
| `content-length` | Computed by Leyline. Your value is replaced |
| `host` | Computed from the URL. Your value is kept on HTTP/1.1 and dropped on HTTP/2 and HTTP/3 |

### Set accept on a script request

Keep the profile's `accept` on `open`, `follow`, and the navigation presets.
A page script sets its own `accept` on a `fetch()` or an XHR, for example
`application/json`. The `Xhr` preset sends what a `fetch()` without an
explicit `accept` sends. When the site's script sets one, set the same value:
the browser sends it too, so the fingerprint does not change.

```rust,no_run
# async fn run(session: leyline::Session) -> leyline::Result<()> {
let tab = session.tab();
tab.open("https://shop.example/cart").await?;
let cart = tab
    .xhr("/api/cart")
    .header("accept", "application/json")
    .send()
    .await?;
# let _ = cart;
# Ok(())
# }
```

## Mix and rotate identities

An `Identity` holds two browsers and a platform. The `tls()` browser supplies
the ClientHello, the HTTP/2 and HTTP/3 settings, and the `ja3`, `ja4`, and
`h2_fingerprint` audit values. The `http()` browser supplies the
`User-Agent`, `sec-ch-ua`, `Accept-Language`, the header shape, and the
header order. The platform selects the OS that the `User-Agent` names, the
HTTP/2 platform overrides, and the `TcpProfile`.

| Function | `tls()` side | `http()` side | `Kind::Config` when |
| --- | --- | --- | --- |
| `Identity::locked(browser, platform)` | `browser` | `browser` | Never; the build checks the platform |
| `rotate_tls(tls)` | `tls` | unchanged | `tls` is of another `Family` than `http()` |
| `rotate_hello()` | the next ClientHello of the family | unchanged | The family has one ClientHello |
| `switch_family(dest)` | `dest` | `dest` | `dest` is of the same `Family`, or has no identity for the platform |

`rotate_hello` moves through `Browser::all()` order and wraps. A profile can
share its ClientHello with another version; its `[meta]` table names that
version in `hello`, `tls()` returns it, and `rotate_hello` counts shared
ClientHellos once.

### Build a session from an identity

`SessionBuilder::identity(identity)` sets both browsers and the platform.
This session sends the headers of Chrome 154 on Linux over the ClientHello,
HTTP/2 settings, and HTTP/3 settings of Chrome 152:

```rust,no_run
use leyline::{Browser, Identity, Platform, Session};

# fn run() -> leyline::Result<()> {
let identity = Identity::locked(Browser::Chrome154, Platform::Linux)
    .rotate_tls(Browser::Chrome152)?;
let session = Session::builder().identity(identity).build()?;
assert_eq!(session.identity().browser(), Some(Browser::Chrome154));
# Ok(())
# }
```

`browser`, `profile`, and `identity` replace each other; the last call wins.
`browser` and `profile` keep the platform in either order. `identity` sets
the platform too, so a `platform` call before it has no effect and one after
it replaces the identity's platform. `build()` returns `Kind::Config` when
the `http()` browser has no identity for the platform, for example Safari 26
on Windows. A session built with `SessionBuilder::profile` sends that one
profile on both sides.

### Change the identity of a session

`Session::with_identity(identity)` derives a session that sends another
identity; the parent keeps its own. The derived session takes the
ClientHello, the HTTP/2 and HTTP/3 settings, the headers, and the audit
values from the identity, as `SessionBuilder::identity` does.

| Part | In the derived session |
| --- | --- |
| Connection pool | Shared with the parent, in its own partition, so connections never cross identities |
| HSTS store and `Alt-Svc` knowledge | Shared with the parent |
| TLS session cache | Its own, empty |
| Cookie jar | The parent's. Give it another with `with_cookie_jar` |
| Brand, proxy, timeouts, `TcpProfile`, other settings | Kept |

A `user-agent` header from `SessionBuilder::headers` still replaces the
identity's `User-Agent`. `Session::identity()` reports the identity the
derived session sends. See [Sessions](sessions.md#derive-a-session).

```rust,no_run
use leyline::{Browser, Family, Identity, Platform, Session};

# fn run() -> leyline::Result<()> {
let chrome = Identity::locked(Browser::latest(Family::Chrome), Platform::Linux);
let session = Session::builder().identity(chrome).build()?;

let next_hello = session.with_identity(chrome.rotate_hello()?)?;

let firefox = chrome.switch_family(Browser::latest(Family::Firefox))?;
let switched = session.with_identity(firefox)?;
# let _ = (next_hello, switched);
# Ok(())
# }
```

`with_identity` returns `Kind::Config` when:

- The session has no `Browser`: a plain session, or one built with
  `SessionBuilder::profile`.
- The `http()` browser has no identity for the platform.
- The session brand, Edge or Opera, has no overlay for the `http()` browser
  on the platform. Only Chromium browsers have one.
- The policy is `ProtocolPolicy::Http3` or `Race` and the `tls()` browser has
  no `[h3]` table. Under `Auto`, `Http1`, or `Http2` that is accepted. See
  [HTTP/3](http3.md).

## Read and test the bundled profiles

`ProfileRegistry::global().get(name, version)` looks up a bundled profile;
`Browser::profile()` is the short path. The
[profile reference](profiles.md) lists what ships and how to load your own.

```rust
use leyline::Browser;
use leyline::profile::ProfileRegistry;

let profile = ProfileRegistry::global()
    .get("chrome", 152)
    .expect("chrome 152 is bundled");
assert_eq!(profile.meta.version, 152);
assert_eq!(Browser::Chrome152.profile().meta.browser, "chrome");
```

The offline conformance test checks each profile's JA4 and HTTP/2 values
against its recorded reference values without the network. It needs the
`bench-internals` feature:

```sh
cargo test -p leyline-http --features bench-internals --test fingerprint_conformance
```

It checks the profile data against the reference values; it does not capture
a browser.

## Next

Read [Features and targets](features-and-targets.md).
