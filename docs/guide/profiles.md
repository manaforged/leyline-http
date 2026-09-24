# Browser profiles

A profile is one TOML file that describes a single browser build's TLS
ClientHello, HTTP/2 SETTINGS, and per-platform identity. Leyline compiles the
bundled profiles into the crate and indexes them in `ProfileRegistry`. You
select a bundled profile with `SessionBuilder::browser`.

Profiles live under `crates/leyline/profiles/<family>/<version>.toml`.

## What the fingerprint columns mean

Each profile can pin a JA4 golden under `[tls.fingerprint]`. The offline
`fingerprint_conformance` test compares that golden against the fingerprint
leyline reconstructs from the profile's own `[tls]` block, and reports one of
two trust levels:

- **Gated.** The profile declares `extension_permutation`. The test compares
  the reconstructed fingerprint with the configured golden and fails on a
  mismatch. This check does not establish the golden's browser provenance.
- **Reconnaissance.** The profile leaves the order to BoringSSL, so the
  reconstruction is an estimate. A mismatch is reported, not failed. The live
  `tls_peet` suite checks Leyline's emission against the configured golden.

A browser capture provides the reference for either category. Neither the
offline test nor a live Leyline request captures the browser itself. Matching
JA4 does not establish equality of every ClientHello field.

`captured_against` records the exact browser build a profile was captured from.
A missing value logs a load-time warning and means the capture build is
unrecorded, not that the profile is wrong.

## Bundled profiles

| Profile | `Browser` variant | `captured_against` | JA4 |
| --- | --- | --- | --- |
| Chrome 145 | `Chrome145` | unrecorded | estimated |
| Chrome 146 | `Chrome146` | unrecorded | estimated |
| Chrome 147 | `Chrome147` | unrecorded | estimated |
| Chrome 148 | `Chrome148` | `chrome-148` | estimated |
| Chrome 149 | `Chrome149` | unrecorded | estimated |
| Chrome 150 | `Chrome150` | `chrome-150.0.7871.128` | estimated |
| Chrome 151 | `Chrome151` | `chrome-headless-shell-151.0.7922.138` | estimated |
| Chrome 152 | `Chrome152` | `chrome-headless-shell-152.0.7977.64` | estimated |
| Brave (Chromium 146) | `Brave146` | `brave-146` | estimated |
| Firefox 148 | `Firefox148` | unrecorded | gated (self-referential golden) |
| Firefox 149 | `Firefox149` | `firefox-149.0` | gated |
| Firefox 150 | `Firefox150` | `firefox-150.0` | gated |
| Firefox 151 | `Firefox151` | `firefox-151.0` | gated |
| Firefox 152 | `Firefox152` | `firefox-152.0` | gated |
| Firefox 153 | `Firefox153` | `firefox-153.0.1` | gated |
| Firefox 154 | `Firefox154` | `firefox-154.0.1` | gated |
| Safari 18 | `Safari18` | unrecorded | reconnaissance, needs recapture |
| Safari 26 | `Safari26` | `webkit-26.5` | gated |
| Safari iOS 17 | `SafariIOS17` | unrecorded | reconnaissance, needs recapture |
| Safari iOS 18 | `SafariIOS18` | unrecorded | reconnaissance, needs recapture |
| OkHttp4 Android 10+ | `OkHttpAndroid10` | unrecorded | estimated |
| CFNetwork iOS 18 | `CfnetworkIOS18` | `CFNetwork-3826.600.41-iOS-18.6-22G86-sim` | gated |
| CFNetwork macOS 26 | `CfnetworkMacOS26` | `CFNetwork-3860.600.21-Darwin-25.5.0-macOS-26.5.1-25F80` | gated |

## Provenance

Each profile comes from one of four sources:

- **Browser capture.** A capture of the named browser, with the build recorded
  in `captured_against`.
- **Non-browser build capture.** A capture of a related build that is not the
  shipped browser, such as `chrome-headless-shell` or a WKWebView host.
- **Inferred.** No capture of this version. Values come from a neighbouring
  version.
- **Self-referential golden.** The JA4 golden is Leyline's own past output, so
  the offline test does not compare the profile with a browser.

| Profile | Provenance | Source |
| --- | --- | --- |
| Chrome 145 | Inferred | Opera 129 (Chromium 145) capture |
| Chrome 146 | Inferred | No capture reference |
| Chrome 147 | Inferred | Chrome 148 TLS block |
| Chrome 148 | Browser capture | `chrome-148`, exact build not recorded |
| Chrome 149 | Inferred | Chrome 148 TLS block |
| Chrome 150 | Browser capture | `chrome-150.0.7871.128` |
| Chrome 151 | Non-browser build capture | `chrome-headless-shell-151.0.7922.138` |
| Chrome 152 | Non-browser build capture | `chrome-headless-shell-152.0.7977.64` |
| Brave (Chromium 146) | Browser capture | `brave-146` |
| Firefox 148 | Self-referential golden | Leyline output |
| Firefox 149 | Browser capture | `firefox-149.0` |
| Firefox 150 | Browser capture | `firefox-150.0` |
| Firefox 151 | Browser capture | `firefox-151.0` |
| Firefox 152 | Browser capture | `firefox-152.0` |
| Firefox 153 | Browser capture | `firefox-153.0.1` |
| Firefox 154 | Browser capture | `firefox-154.0.1` |
| Safari 18 | Self-referential golden | Leyline output |
| Safari 26 | Non-browser build capture | `webkit-26.5` (WKWebView), synthesized HTTP identity |
| Safari iOS 17 | Self-referential golden | Leyline output |
| Safari iOS 18 | Self-referential golden | Leyline output |
| OkHttp4 Android 10+ | Self-referential golden | Leyline output |
| CFNetwork iOS 18 | Browser capture | `CFNetwork-3826.600.41-iOS-18.6-22G86-sim`, iOS simulator |
| CFNetwork macOS 26 | Browser capture | `CFNetwork-3860.600.21-Darwin-25.5.0-macOS-26.5.1-25F80` |

Chromium-family profiles do not store `sec-ch-ua`. Leyline derives it from the
major version and the `ch_ua_brand` field in `[meta]`, with the same GREASE
brand, version, and order rule that Chromium uses.

`Browser::latest` returns the highest bundled version of a family, so you can
pin the product line instead of a version:

```rust
use leyline::profile::{Browser, Family};

let chrome = Browser::latest(Family::Chrome);
```

## Profiles that need a recapture

Three WebKit profiles pin a JA4 golden taken from Leyline's own output, not
from a Safari capture:

- `safari/18.toml`
- `safari/ios17.toml`
- `safari/ios18.toml`

These profiles send no ALPS extension and no ECH GREASE, the same as the
Safari 26 capture (`webkit-26.5`). Their goldens are not evidence about Safari.
The fix is a fresh capture from Safari.app and Mobile Safari.

## Update cadence

- **Chrome and Firefox:** Leyline adds profiles for new stable releases. Both
  browsers ship every four weeks.
- **Safari:** add a profile when Apple ships an OS release. Safari's TLS stack
  moves with macOS and iOS, not with the browser version alone.
- **Brave, OkHttp, and CFNetwork:** recapture when the upstream engine version
  in `captured_against` moves, not on a fixed schedule.

Every new profile carries `captured_against` with the exact build string, and a
JA4 golden taken from that capture.

## Load your own profile directory

`ProfileRegistry::load` reads a directory laid out the same way as the bundled
set, `<family>/<version>.toml`, and runs the same parse and extension-order
validation as the compiled-in registry. `SessionBuilder::profile` sends a
loaded profile:

```rust,no_run
use std::path::Path;
use leyline::{Platform, Session};
use leyline::profile::ProfileRegistry;

# fn main() -> leyline::Result<()> {
let registry = ProfileRegistry::load(Path::new("./profiles"))
    .expect("profile directory loads");
let profile = registry.get("chrome", 153).expect("chrome 153 profile").clone();
let session = Session::builder()
    .profile(profile)
    .platform(Platform::MacOS)
    .build()?;
# drop(session);
# Ok(())
# }
```

`BrowserProfile::from_toml` parses one profile from a string, and its result
goes to `SessionBuilder::profile` the same way.

`load` returns `ProfileError::Io` when a file cannot be read,
`ProfileError::Parse` when a TOML file is invalid or its
`extension_permutation` disagrees with the extensions its `[tls]` block turns
on, and `ProfileError::Empty` when the directory holds no profile.

A loaded profile carries its own `[identity.<platform>]` tables. The session
reads the user agent, `sec-ch-ua`, and extra headers for the chosen platform
from those tables. `build` returns `Kind::Config` when the profile has no
table for that platform. Without `.platform()`, the platform is Windows. A
brand overlay (`SessionBuilder::brand`) needs `chromium_major` in `[meta]`.
The request header order follows `header_style` in `[meta]`.
