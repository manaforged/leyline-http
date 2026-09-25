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
don't-fragment; Windows applies TTL and don't-fragment. The audit's JA4T is
computed from the configured values, including fields the running platform
does not apply.

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
| `ja4t` | JA4T from the TCP options Leyline applies with `setsockopt`, not captured from the SYN. |
| `ja4h` | JA4H over the request line and headers this response's request sent. |
| `h2_fingerprint` | Akamai-style HTTP/2 fingerprint. |

The connection-level fields are computed once per session from the profile and
shared by `Arc` with every response. `ja4h` is computed on the first `audit()`
call for a response and cached.

Read these as a self-report: they say what Leyline built from the profile, not
what a packet capture observed. `ja4t` in particular describes the options
requested, and the kernel decides what the SYN carries.

## The offline conformance test

The offline test checks each profile's JA4 against a recorded reference value,
without touching the network. Run it with
`cargo test -p leyline-http --test fingerprint_conformance`. The report checks
agreement between the profile data and its recorded reference values.
Establishing browser fidelity also requires a browser capture and a comparison
with Leyline's emitted handshake.

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

In 0.1 the session builder still selects a profile by `Browser` variant, so a
loaded profile is available for inspection and validation but is not usable in a
session. Sending one needs a crate release that adds the variant.

To parse a single file rather than a directory, call
`BrowserProfile::from_toml`. It runs the same validation.

```rust
use leyline::BrowserProfile;

assert!(BrowserProfile::from_toml("not a profile").is_err());
```

## Next

Read [Features and targets](features-and-targets.md).
