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
| Chrome 145 | `Chrome145` | `chrome-android-145.0.7632.218-android-17-emulator` | estimated |
| Chrome 146 | `Chrome146` | unrecorded | estimated |
| Chrome 147 | `Chrome147` | unrecorded | estimated |
| Chrome 148 | `Chrome148` | `chrome-148` | estimated |
| Chrome 149 | `Chrome149` | unrecorded | estimated |
| Chrome 150 | `Chrome150` | `chrome-150.0.7871.128` | estimated |
| Chrome 151 | `Chrome151` | `chrome-headless-shell-151.0.7922.138` | estimated |
| Chrome 152 | `Chrome152` | `chrome-headless-shell-152.0.7977.64` | estimated |
| Chrome 153 | `Chrome153` | `chrome-153.0.8010.53` | estimated |
| Brave (Chromium 146) | `Brave146` | `brave-146` | estimated |
| Firefox 148 | `Firefox148` | `firefox-148.0.2` | gated |
| Firefox 149 | `Firefox149` | `firefox-149.0` | gated |
| Firefox 150 | `Firefox150` | `firefox-150.0` | gated |
| Firefox 151 | `Firefox151` | `firefox-151.0` | gated |
| Firefox 152 | `Firefox152` | `firefox-152.0` | gated |
| Firefox 153 | `Firefox153` | `firefox-153.0.1` | gated |
| Firefox 154 | `Firefox154` | `firefox-154.0.1` | gated |
| Safari 18 | `Safari18` | unrecorded | reconnaissance, needs recapture |
| Safari 26 | `Safari26` | `safari-26.2-21623.1.14.11.9` | gated |
| Safari iOS 17 | `SafariIOS17` | unrecorded | reconnaissance, needs recapture |
| Safari iOS 18 | `SafariIOS18` | unrecorded | reconnaissance, needs recapture |
| OkHttp4 Android 10+ | `OkHttpAndroid10` | `okhttp-4.12.0-android-17-emulator` | estimated |
| CFNetwork iOS 18 | `CfnetworkIOS18` | `CFNetwork-3826.600.41-iOS-18.6-22G86-sim` | gated |
| CFNetwork macOS 26 | `CfnetworkMacOS26` | `CFNetwork-3860.600.21-Darwin-25.5.0-macOS-26.5.1-25F80` | gated |

## Provenance

Each profile comes from one of six sources:

- **Browser capture.** A capture of the named browser, with the build recorded
  in `captured_against`.
- **Native stack capture.** A capture of an operating system HTTP stack, such
  as a URLSession app for CFNetwork, with the build recorded in
  `captured_against`.
- **Non-browser build capture.** A capture of a related build that is not the
  shipped browser, such as `chrome-headless-shell` or a WKWebView host.
- **Emulator capture.** A capture of the shipped app on an Android emulator.
  The app and Android's own TLS stack are real, but the device is not a
  physical phone. The build and Android version are recorded in
  `captured_against`.
- **Inferred.** No capture of this version. Values come from a neighbouring
  version.
- **Self-referential golden.** The JA4 golden is Leyline's own past output, so
  the offline test does not compare the profile with a browser.

The `capture` key in each profile's `[meta]` table records the source:
`browser`, `native`, `headless-shell`, `webview`, `emulator`, `inferred`, or
`self-referential`.

| Profile | Provenance | `capture` | Source |
| --- | --- | --- | --- |
| Chrome 145 | Emulator capture | `emulator` | `chrome-android-145.0.7632.218-android-17-emulator` for TLS and Android H2; desktop H2 from the Opera 129 (Chromium 145) capture |
| Chrome 146 | Inferred | `inferred` | No capture reference |
| Chrome 147 | Inferred | `inferred` | Chrome 148 TLS block |
| Chrome 148 | Browser capture | `browser` | `chrome-148`, exact build not recorded |
| Chrome 149 | Inferred | `inferred` | Chrome 148 TLS block |
| Chrome 150 | Browser capture | `browser` | `chrome-150.0.7871.128` |
| Chrome 151 | Non-browser build capture | `headless-shell` | `chrome-headless-shell-151.0.7922.138` |
| Chrome 152 | Non-browser build capture | `headless-shell` | `chrome-headless-shell-152.0.7977.64` |
| Chrome 153 | Browser capture | `browser` | `chrome-153.0.8010.53`, macOS, `--headless=new` |
| Brave (Chromium 146) | Browser capture | `browser` | `brave-146` |
| Firefox 148 | Browser capture | `browser` | `firefox-148.0.2` |
| Firefox 149 | Browser capture | `browser` | `firefox-149.0` |
| Firefox 150 | Browser capture | `browser` | `firefox-150.0` |
| Firefox 151 | Browser capture | `browser` | `firefox-151.0` |
| Firefox 152 | Browser capture | `browser` | `firefox-152.0` |
| Firefox 153 | Browser capture | `browser` | `firefox-153.0.1` |
| Firefox 154 | Browser capture | `browser` | `firefox-154.0.1` |
| Safari 18 | Self-referential golden | `self-referential` | Leyline output |
| Safari 26 | Browser capture | `browser` | `safari-26.2-21623.1.14.11.9`, Safari.app through safaridriver |
| Safari iOS 17 | Self-referential golden | `self-referential` | Leyline output |
| Safari iOS 18 | Self-referential golden | `self-referential` | Leyline output |
| OkHttp4 Android 10+ | Emulator capture | `emulator` | `okhttp-4.12.0-android-17-emulator`, test app on the platform TLS stack |
| CFNetwork iOS 18 | Native stack capture | `native` | `CFNetwork-3826.600.41-iOS-18.6-22G86-sim`, iOS simulator |
| CFNetwork macOS 26 | Native stack capture | `native` | `CFNetwork-3860.600.21-Darwin-25.5.0-macOS-26.5.1-25F80` |

Chromium-family profiles do not store `sec-ch-ua`. Leyline derives it from the
major version and the `ch_ua_brand` field in `[meta]`, with the same GREASE
brand, version, and order rule that Chromium uses.

`Browser::latest` returns the newest profile of a family whose `capture` is in
the family's `latest_capture` list in `families.toml`. The list defaults to
`browser`; CFNetwork uses `native`. A family with no such profile returns its
newest profile. `Session::new()` uses `Browser::latest(Family::Chrome)`, which
is Chrome 153. You can pin the product line instead of a version:

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
Safari 26 capture (`safari-26.2-21623.1.14.11.9`). Their goldens are not evidence about Safari.
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

## Build a profile from a JA3 or Akamai string

`BrowserProfile::from_fingerprint` builds a profile from fingerprint strings
instead of a TOML file. It returns the same `BrowserProfile` type, and
`SessionBuilder::profile` sends it the same way.

`FingerprintSpec` takes these inputs:

- `ja3(raw)`: a raw JA3 string, `version,ciphers,extensions,curves,point
  formats`. It sets the cipher list, the curve list, and the exact extension
  order.
- `ja4_r(raw)`: a raw JA4_r or JA4_ro string. It sets the cipher list, the
  extension set, and the signature algorithms. The hashed JA4 form
  (`t13d1516h2_8daaf6152771_806a8c22fdea`) cannot be inverted, so
  `from_fingerprint` rejects it.
- `akamai(raw)`: an Akamai HTTP/2 string,
  `SETTINGS|WINDOW_UPDATE|PRIORITY|pseudo-header order`. It sets the SETTINGS
  order and values, the connection window, and the pseudo-header order.
- `base(profile)`: the profile to start from. The result keeps every value
  the strings do not carry: the identity tables, the headers, HTTP/3, the
  signature algorithms for a JA3 string, and the curves for a JA4_r string.
  Without a base, the bare profile is the start.
- `user_agent(ua)` and `header_order(names)`: replace the user agent and the
  request header order in every identity table.
- `name(name)`: the profile name in logs and errors.

This example takes the JA3 and Akamai values of the bundled Chrome 152
profile and applies them to the Chrome 148 profile:

```rust
use leyline::profile::FingerprintSpec;
use leyline::{Browser, BrowserProfile, Session};

# fn main() -> leyline::Result<()> {
let ja3 = "771,4865-4866-4867-49195-49199-49196-49200-52393-52392-49171-49172-156-157-47-53,\
           0-23-65281-11-35-16-51-43-45-10-13-5-18-27-17613-65037,4588-29-23-24,0";
let akamai = "1:65536;2:0;4:6291456;6:262144|15663105|0|m,a,s,p";
let spec = FingerprintSpec::new()
    .base(Browser::Chrome148.profile().clone())
    .ja3(ja3)
    .akamai(akamai)
    .name("Chrome 152 from strings");
let profile = BrowserProfile::from_fingerprint(spec)?;

let chrome152 = Browser::Chrome152.profile();
assert_eq!(profile.tls.ciphers, chrome152.tls.ciphers);
assert_eq!(profile.tls.curves, chrome152.tls.curves);
assert_eq!(chrome152.expected_h2_fingerprint(), Some(akamai));

let session = Session::builder().profile(profile).audit(true).build()?;
# drop(session);
# Ok(())
# }
```

`Response::audit` confirms the result. A session built from these strings
reports the same JA3 and Akamai values as the Chrome 152 session:

```rust,no_run
# use leyline::profile::FingerprintSpec;
# use leyline::{Browser, BrowserProfile, Session};
# async fn run(ja3: &str, akamai: &str) -> leyline::Result<()> {
let spec = FingerprintSpec::new()
    .base(Browser::Chrome148.profile().clone())
    .ja3(ja3)
    .akamai(akamai);
let custom = Session::builder()
    .profile(BrowserProfile::from_fingerprint(spec)?)
    .audit(true)
    .build()?;
let reference = Session::builder()
    .browser(Browser::Chrome152)
    .audit(true)
    .build()?;

let url = "https://tls.peet.ws/api/all";
let custom = custom.get(url).await?;
let reference = reference.get(url).await?;
let (Some(custom), Some(reference)) = (custom.audit(), reference.audit()) else {
    return Ok(());
};
assert_eq!(custom.ja3, reference.ja3);
assert_eq!(custom.h2_fingerprint, reference.h2_fingerprint);
# Ok(())
# }
```

These rules apply to the strings:

- A GREASE value in any list sets `grease = true` and is not stored. A string
  without GREASE keeps the base value.
- Each extension ID turns on the `[tls]` setting that sends it. An extension
  that carries data (`application_settings`, `compress_certificate`,
  `delegated_credentials`, `record_size_limit`) takes its value from the base
  profile. `from_fingerprint` fails when the base has no value.
- `pre_shared_key` (41) sets `pre_shared_key = true`. `padding` (21) sets
  `padding = true` and must be the last extension.
- The JA3 version must be 771 and the point formats must be `0`. The Akamai
  PRIORITY field must be `0`.
- An ID that is not in the leyline IANA registry fails. An extension or
  SETTINGS ID that leyline cannot send also fails.

Every failure returns `Kind::Config` with a message that names the format and
the field.
