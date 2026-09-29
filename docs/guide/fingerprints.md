# Fingerprints

A browser profile is a recorded description of how one browser build looks on
the wire. Leyline replays it across four layers.

## What a profile pins

**TLS.** The `TlsProfile` holds the cipher list, the supported groups, the
signature algorithms, the certificate compression codepoints, and the
extension behavior: GREASE, ECH GREASE, extension permutation or a fixed
extension order, ALPS and its codepoint choice, record size limit, delegated
credentials, OCSP stapling, signed certificate timestamps, session tickets,
pre-shared key, padding, trust anchor requests, and the minimum TLS version.

**HTTP/2.** The `H2Profile` holds the SETTINGS values and the order they are
sent in, the pseudo-header order, and the default priority frame. Per-platform
overrides sit beside it.

**Headers.** Each platform block in a profile supplies the `User-Agent` and
the `sec-ch-ua` brand list. The profile's header shape in
`profiles/headers.toml` and the `Preset` decide the other headers and their
order for the fetch context. See
[Requests](requests.md).

**TCP.** `TcpProfile` carries the socket options that shape the SYN: TTL,
MSS, window size, window scale, the don't-fragment bit, and `TCP_NODELAY`.
The per-OS values live in `profiles/platforms.toml`, and
`Platform::tcp_profile` reads them. The session picks one from its platform,
and `SessionBuilder::tcp_profile` overrides it. Each platform applies what it
can: Linux applies TTL, MSS, don't-fragment, and derives a
`TCP_WINDOW_CLAMP` from the window fields; macOS applies TTL, MSS, and
don't-fragment; Windows applies TTL and don't-fragment. Every platform sets
`TCP_NODELAY` when the profile asks for it. `options` is the SYN
option order that the operating system kernel sends. No platform lets a socket
set it, so Leyline does not apply it; only the audit's JA4T reads it. The audit's
JA4T is computed from the configured values, including fields the running
platform does not apply. `platforms.toml` has rows for Windows, macOS, Linux,
and iOS. Android has no row, so its `TcpProfile` is empty and the kernel
defaults apply.

## Audit a session

Build the session with `.audit(true)`, then read `Response::audit()`. Without
the flag, `audit()` returns `None`.

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
    println!("JA3            {}", a.ja3);
    println!("JA4            {}", a.ja4);
    println!("JA4T           {}", a.ja4t);
    println!("JA4H           {}", a.ja4h);
    println!("H2 fingerprint {}", a.h2_fingerprint);
}
# Ok(())
# }
```

The five fields of `AuditData`:

| Field | What it is |
| --- | --- |
| `ja3` | JA3 TLS fingerprint, the MD5 form. |
| `ja4` | JA4 TLS fingerprint. Derived from the profile's fixed extension order when it has one. |
| `ja4t` | JA4T computed from the session's `TcpProfile` values. |
| `ja4h` | JA4H over the request line and headers this response's request sent. |
| `h2_fingerprint` | Akamai-style HTTP/2 fingerprint. |

The connection-level fields are computed once per session from the profile and
shared by `Arc` with every response. `ja4h` is computed on the first `audit()`
call for a response and cached.

The audit values report what Leyline built from the profile. A packet capture
shows what the connection sent. `ja4t` describes the TCP options that the
`TcpProfile` requests, and the kernel decides what the SYN carries.

## The offline conformance test

The offline test checks each profile's JA4 against a recorded reference value,
without touching the network. The test needs the `bench-internals` feature.
Run it with:

```sh
cargo test -p leyline-http --features bench-internals --test fingerprint_conformance
```

The report checks agreement between the profile data and its recorded reference
values. Establishing browser fidelity also requires a browser capture and a
comparison with Leyline's emitted handshake.

## Read a profile

Profiles are TOML files under `crates/leyline/profiles/<family>/<version>.toml`,
and every bundled one is compiled into the binary.
The [profile reference](profiles.md) lists what ships, which JA4 reference values
are gated, and the update cadence.

`ProfileRegistry::global()` borrows the built-in set, and
`get(name, version)` looks one up by name and version.
`Browser::profile()` is the short path to the same value.

```rust
use leyline::Browser;
use leyline::profile::ProfileRegistry;

let profile = ProfileRegistry::global()
    .get("chrome", 152)
    .expect("chrome 152 is bundled");
assert_eq!(profile.meta.version, 152);

let same = Browser::Chrome152.profile();
assert_eq!(same.meta.browser, "chrome");
```

## Load your own profile directory

`ProfileRegistry::load(dir)` reads a directory laid out like the bundled set,
`<family>/<version>.toml`, and runs the same parse and extension-order
validation as the compiled-in registry. Use it to author and check a profile
for a browser release your installed crate version does not bundle yet.

```rust,no_run
use std::path::Path;
use leyline::profile::ProfileRegistry;

# fn run() -> Result<(), Box<dyn std::error::Error>> {
let registry = ProfileRegistry::load(Path::new("./profiles"))?;
let profile = registry.get("chrome", 153).expect("chrome 153 profile");
println!("{}", profile.meta.name);
# Ok(())
# }
```

`load` reads the directory in sorted order, so two runs load the same set. It
returns a `ProfileError`:

- `Io` when a directory or file cannot be read.
- `Parse` when a TOML file is invalid, or its `extension_permutation`
  disagrees with the extensions its `[tls]` block turns on.
- `Empty` when the directory holds no `<family>/<version>.toml` file.

To parse a single file rather than a directory, call
`BrowserProfile::from_toml`. It runs the same validation.

```rust
use leyline::BrowserProfile;

assert!(BrowserProfile::from_toml("not a profile").is_err());
```

## Mix and rotate identities

An `Identity` holds two browsers and a platform. The `tls()` browser supplies
the ClientHello, the HTTP/2 settings, the HTTP/3 settings, and the `ja3`, `ja4`,
and `h2_fingerprint` audit values. The `http()` browser supplies the
`User-Agent`, `sec-ch-ua`, `Accept-Language`, the header shape, and the header
order. The platform selects the operating system that the `User-Agent` names,
the HTTP/2 platform overrides, and the `TcpProfile`.

| Function | `tls()` side | `http()` side |
| --- | --- | --- |
| `Identity::locked(browser, platform)` | `browser` | `browser` |
| `rotate_tls(tls)` | `tls` | unchanged |
| `rotate_hello()` | the next ClientHello of the family | unchanged |
| `switch_family(dest)` | `dest` | `dest` |

`rotate_hello` moves to the next ClientHello of the family in `Browser::all()`
order and wraps from the last to the first. A profile can share its ClientHello
with another version of the same browser. Its `[meta]` table names that version
in `hello`. `tls()` returns the version that holds the ClientHello, so on an
identity from `locked` it can differ from `http()`. `rotate_hello` counts
versions that share a ClientHello once.

`locked` accepts any browser and platform. `SessionBuilder::build` and
`Session::with_identity` return `Kind::Config` when the `http()` browser has no
identity for the platform, for example Safari 26 on Windows. The three
functions that change an identity return `Kind::Config` at the call when:

- `rotate_tls` gets a `tls` browser of another `Family` than `http()`.
- `rotate_hello` runs on a family with one ClientHello.
- `switch_family` gets a `dest` browser of the same `Family`, or one that has no
  identity for the platform. Use `rotate_tls` to change the ClientHello inside a
  family.

### Build a session from an identity

`SessionBuilder::identity(identity)` sets the `tls()` browser, the `http()`
browser, and the platform in one call. This session sends the headers of
Chrome 154 on Linux over the ClientHello, HTTP/2 settings, and HTTP/3 settings
of Chrome 152:

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

`browser`, `profile`, and `identity` replace each other. The last call of the
three sets the browser or profile and the `http()` browser. `browser` and
`profile` keep the platform in either call order. `identity` sets the platform
too. A `platform` call before `identity` has no effect, and a `platform` call
after it replaces the platform of the identity. A session built with
`SessionBuilder::profile` sends that one profile on both sides.

### Change the identity of a session

`Session::with_identity(identity)` returns a copy of a session that sends
another identity. The session it came from keeps its own. The copy takes these
values from the identity, as `SessionBuilder::identity` does:

- The ClientHello, with an empty TLS session cache.
- The HTTP/2 and HTTP/3 settings.
- The `User-Agent`, `sec-ch-ua`, `Accept-Language`, header shape, and header
  order.
- The `ja3`, `ja4`, and `h2_fingerprint` audit values, when the session was
  built with `.audit(true)`.
- A separate connection pool. The copy opens its own connections.

The copy shares the cookie jar with the session it came from. It keeps the
brand, the proxy, the timeouts, the `TcpProfile`, and every other setting. A
`user-agent` header from `SessionBuilder::headers` replaces the identity's
`User-Agent` in the copy, as it does in the session.
`Session::identity()` reports the identity that the copy sends.

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

- The session has no `Browser`: a bare session, or a session built with
  `SessionBuilder::profile`.
- The `http()` browser has no identity for the platform.
- The session brand, Edge or Opera, has no overlay for the `http()` browser on
  the platform. Only Chromium browsers have one.
- The session policy is `ProtocolPolicy::Http3` or `ProtocolPolicy::Race`, and
  the `tls()` browser has no `[h3]` table.

Under `ProtocolPolicy::Auto`, `Http1`, or `Http2`, `with_identity` accepts a
`tls()` browser with no `[h3]` table. Those policies do not send HTTP/3, so the
copy needs no HTTP/3 transport. See [HTTP/3](http3.md).

## Next

Read [Features and targets](features-and-targets.md).
