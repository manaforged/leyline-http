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
| Chrome 145 | `Chrome145` | `chrome-145.0.7632.160` | estimated |
| Chrome 146 | `Chrome146` | `chrome-146.0.7680.178` | estimated |
| Chrome 147 | `Chrome147` | `chrome-147.0.7727.138` | estimated |
| Chrome 148 | `Chrome148` | `chrome-148.0.7778.216` | estimated |
| Chrome 149 | `Chrome149` | `chrome-149.0.7827.201` | estimated |
| Chrome 150 | `Chrome150` | `chrome-150.0.7871.187` | estimated |
| Chrome 151 | `Chrome151` | `chrome-151.0.7922.174` | estimated |
| Chrome 152 | `Chrome152` | `chrome-152.0.7977.83` | estimated |
| Chrome 153 | `Chrome153` | `chrome-153.0.8010.53` | estimated |
| Chrome 154 | `Chrome154` | `chrome-154.0.8037.58` | estimated |
| Brave (Chromium 146) | `Brave146` | `brave-146.1.88.138` | estimated |
| Brave (Chromium 154) | `Brave154` | `brave-154.1.96.59` | estimated |
| Firefox 148 | `Firefox148` | `firefox-148.0.2` | gated |
| Firefox 149 | `Firefox149` | `firefox-149.0.2` | gated |
| Firefox 150 | `Firefox150` | `firefox-150.0` | gated |
| Firefox 151 | `Firefox151` | `firefox-151.0.4` | gated |
| Firefox 152 | `Firefox152` | `firefox-152.0.6` | gated |
| Firefox 153 | `Firefox153` | `firefox-153.0.4` | gated |
| Firefox 154 | `Firefox154` | `firefox-154.0.1` | gated |
| Firefox 155 | `Firefox155` | `firefox-155.0.1` | gated |
| Firefox 156 | `Firefox156` | `firefox-156.0.1` | gated |
| Safari 18 | `Safari18` | `safari-18.6-20621.3.11.11.3` | gated |
| Safari 26 | `Safari26` | `safari-26.6.2-21624.5.1.11.3` | gated |
| Safari iOS 17 | `SafariIOS17` | `safari-ios-17.5-21F79-simulator` | gated |
| Safari iOS 18 | `SafariIOS18` | `safari-ios-18.6-22G86-simulator` | gated |
| Safari iOS 27 | `SafariIOS27` | `safari-ios-27.0-24A434-simulator` | gated |
| OkHttp4 Android 10+ | `OkHttpAndroid10` | `okhttp-4.12.0-android-17-emulator` | estimated |
| CFNetwork iOS 18 | `CfnetworkIOS18` | `CFNetwork-3826.600.41-iOS-18.6-22G86-sim` | gated |
| CFNetwork iOS 27 | `CfnetworkIOS27` | `CFNetwork-3896.100.1.2.1-iOS-27.0-24A434-sim` | gated |
| CFNetwork macOS 26 | `CfnetworkMacOS26` | `CFNetwork-3860.700.1-Darwin-25.6.0-macOS-26.6.2-25G83` | gated |

## Provenance

Each profile comes from one of six sources:

- **Browser capture.** A capture of the named browser, with the build recorded
  in `captured_against`.
- **Native stack capture.** A capture of an operating system HTTP stack, such
  as a URLSession app for CFNetwork, with the build recorded in
  `captured_against`.
- **Non-browser build capture.** A capture of a related build that is not the
  shipped browser, such as `chrome-headless-shell` or a WKWebView host.
- **Emulator capture.** A capture of the shipped app on an Android emulator or
  an iOS simulator. The app and the operating system's own TLS stack are real,
  but the device is not a physical phone. The build and OS version are
  recorded in `captured_against`.
- **Inferred.** No capture of this version. Values come from a neighbouring
  version.
- **Self-referential golden.** The JA4 golden is Leyline's own past output, so
  the offline test does not compare the profile with a browser.

The `capture` key in each profile's `[meta]` table records the source:
`browser`, `native`, `headless-shell`, `webview`, `emulator`, `inferred`, or
`self-referential`.

| Profile | Provenance | `capture` | Source |
| --- | --- | --- | --- |
| Chrome 145 | Browser capture | `browser` | `chrome-145.0.7632.160` macOS and `145.0.7632.160` Windows headful; `145.0.7632.159` Linux headful |
| Chrome 146 | Browser capture | `browser` | `chrome-146.0.7680.178` macOS and `146.0.7680.178` Windows headful; `146.0.7680.177` Linux headful |
| Chrome 147 | Browser capture | `browser` | `chrome-147.0.7727.138` macOS and `147.0.7727.138` Windows headful; `147.0.7727.137` Linux headful |
| Chrome 148 | Browser capture | `browser` | `chrome-148.0.7778.216` macOS and `148.0.7778.217` Windows headful; `148.0.7778.215` Linux headful |
| Chrome 149 | Browser capture | `browser` | `chrome-149.0.7827.201` macOS and `149.0.7827.201` Windows headful; `149.0.7827.200` Linux headful |
| Chrome 150 | Browser capture | `browser` | `chrome-150.0.7871.187` macOS and `150.0.7871.187` Windows headful; `150.0.7871.186` Linux headful |
| Chrome 151 | Browser capture | `browser` | `chrome-151.0.7922.174` macOS and `151.0.7922.174` Windows headful; `151.0.7922.173` Linux headful |
| Chrome 152 | Browser capture | `browser` | `chrome-152.0.7977.83` macOS and `152.0.7977.83` Windows headful; `152.0.7977.82` Linux headful |
| Chrome 153 | Browser capture | `browser` | `chrome-153.0.8010.53` macOS and `153.0.8010.53` Windows headful; `153.0.8010.52` Linux headful |
| Chrome 154 | Browser capture | `browser` | `chrome-154.0.8037.58` macOS and `154.0.8037.58` Windows headful; `154.0.8037.57` Linux headful |
| Brave (Chromium 146) | Browser capture | `browser` | `brave-146.1.88.138`, macOS, Windows, and Linux headful |
| Brave (Chromium 154) | Browser capture | `browser` | `brave-154.1.96.59`, macOS, Windows, and Linux headful |
| Firefox 148 | Browser capture | `browser` | `firefox-148.0.2`, macOS and Linux `--headless`, Windows headful |
| Firefox 149 | Browser capture | `browser` | `firefox-149.0.2`, macOS and Linux `--headless`, Windows headful |
| Firefox 150 | Browser capture | `browser` | `firefox-150.0`, macOS and Linux `--headless`, Windows headful |
| Firefox 151 | Browser capture | `browser` | `firefox-151.0.4`, macOS and Linux `--headless`, Windows headful |
| Firefox 152 | Browser capture | `browser` | `firefox-152.0.6`, macOS and Linux `--headless`, Windows headful |
| Firefox 153 | Browser capture | `browser` | `firefox-153.0.4`, macOS and Linux `--headless`, Windows headful |
| Firefox 154 | Browser capture | `browser` | `firefox-154.0.1`, macOS and Linux `--headless`, Windows headful |
| Firefox 155 | Browser capture | `browser` | `firefox-155.0.1`, macOS and Linux `--headless`, Windows headful |
| Firefox 156 | Browser capture | `browser` | `firefox-156.0.1`, macOS and Linux `--headless`, Windows headful |
| Safari 18 | Browser capture | `browser` | `safari-18.6-20621.3.11.11.3`, Safari.app on macOS 15.7.7 in a VM through safaridriver |
| Safari 26 | Browser capture | `browser` | `safari-26.6.2-21624.5.1.11.3`, Safari.app on macOS 26.6.2 in a VM through safaridriver |
| Safari iOS 17 | Emulator capture | `emulator` | `safari-ios-17.5-21F79-simulator`, Mobile Safari in the iOS 17.5 simulator |
| Safari iOS 18 | Emulator capture | `emulator` | `safari-ios-18.6-22G86-simulator`, Mobile Safari in the iOS 18.6 simulator |
| Safari iOS 27 | Emulator capture | `emulator` | `safari-ios-27.0-24A434-simulator`, Mobile Safari in the iOS 27.0 simulator |
| OkHttp4 Android 10+ | Emulator capture | `emulator` | `okhttp-4.12.0-android-17-emulator`, test app on the platform TLS stack |
| CFNetwork iOS 18 | Native stack capture | `native` | `CFNetwork-3826.600.41-iOS-18.6-22G86-sim`, iOS simulator |
| CFNetwork iOS 27 | Emulator capture | `emulator` | `CFNetwork-3896.100.1.2.1-iOS-27.0-24A434-sim`, URLSession test binary in the iOS 27.0 simulator |
| CFNetwork macOS 26 | Native stack capture | `native` | `CFNetwork-3860.700.1-Darwin-25.6.0-macOS-26.6.2-25G83`, URLSession test binary on macOS 26.6.2 in a VM; macOS 26.2 on hardware sends the same ClientHello |

Chromium-family profiles do not store `sec-ch-ua`. Leyline derives it from the
major version and the `ch_ua_brand` field in `[meta]`, with the same GREASE
brand, version, and order rule that Chromium uses.

`Browser::latest` returns the newest profile of a family whose `capture` is in
the family's `latest_capture` list in `families.toml`. The list defaults to
`browser`. CFNetwork uses `native` and `emulator`, and Safari iOS and OkHttp
add `emulator`. The build fails when a family has no such profile. A
`platform_browser` target resolves the same way, with the target family's
`latest_capture` list. `Session::new()` uses `Browser::latest(Family::Chrome)`, which
is Chrome 154. You can pin the product line instead of a version:

```rust
use leyline::profile::{Browser, Family};

let chrome = Browser::latest(Family::Chrome);
```

## Capture notes

These facts come from the captures behind the bundled profiles.

- **Chrome 145 to 154.** Branded Google Chrome builds from Google's update
  server and apt repository, captured on 2026-09-25. macOS and Windows ran
  headful with no user agent override, after the variations seed arrived.
  Linux ran headful under Xvfb with no user agent override, from the official
  `.deb` packages extracted without installation. The Linux captures back the
  user agent (`X11; Linux x86_64`), `sec-ch-ua-platform: "Linux"`, the
  navigate header order, and the TLS and H2 values. Every
  build sends the four core SETTINGS (1, 2, 4, 6) on every platform, without
  `max_concurrent_streams` and without setting 8. The ClientHello has three
  forms: 145 to 149 send no ML-DSA signature schemes and no Trust Anchor
  Identifiers (`t13d1516h2_8daaf6152771_d8a2da3f94cd`); 150 and 151 add the
  ML-DSA schemes (`t13d1516h2_8daaf6152771_806a8c22fdea`); 152 and later also
  send Trust Anchor Identifiers (0xCA34) with an empty list
  (`t13d1517h2_8daaf6152771_cb7bf5808d99`). Captures with a fresh profile
  and with the variations seed are the same.
- **Chrome 150.** Chrome 150 puts the ML-DSA signature schemes (0x0904,
  0x0905, 0x0906) before the classical list. It needs a BoringSSL revision
  with `SSL_SIGN_ML_DSA_*` (3a9254f or later). Chrome changes the extension
  order and the GREASE values on each connection, so the profile keeps
  BoringSSL permutation and random GREASE. Each request HEADERS frame carries
  PRIORITY with the exclusive bit and weight 256, sent as 255.
- **Brave 146 and 154.** Brave 1.88.138 (Chromium 146) and 1.96.59
  (Chromium 154), captured headful on macOS and Windows with no user agent
  override. Brave sends the four core SETTINGS and the Chrome user agent form.
  Brave 154 sends the Chromium 150 ClientHello (ML-DSA, no Trust Anchor
  Identifiers) and moves `accept-language` after `accept-encoding`. Brave
  changes the `accept-language` q value between requests (macOS 0.9 and 0.5,
  Windows 0.8 and 0.6). The profiles use the value that both versions sent on
  each platform: 0.9 on macOS and 0.8 on Windows. On Linux, headful under
  Xvfb, Brave 146 sent 0.8 and 0.6 and Brave 154 sent 0.6 and 0.8; the Linux
  rows use 0.8.
- **Edge 154 and Opera 136 on Linux.** Edge 154.0.4258.37 and Opera
  136.0.6008.52 (Chromium 152), captured headful under Xvfb with no user agent
  override. They send the Chrome user agent form with `Edg/154.0.0.0` and
  `OPR/136.0.0.0`, `sec-ch-ua-platform: "Linux"`, and the Chrome navigate
  header order.
- **Firefox 148 to 156.** Official Mozilla builds captured on macOS and
  Linux with `--headless` on 2026-09-25, and on Windows headful from the
  `win64` installers, with the SHA256SUMS signature and the Authenticode
  signature checked. The Windows builds send
  `Windows NT 10.0; Win64; x64` and the same TLS and H2 values as macOS and
  Linux. Every build sends three key shares
  (X25519MLKEM768, X25519, P-256) and a HEADERS PRIORITY with weight 42 and no
  exclusive bit, sent as 41.
- **Firefox 149.** Firefox 149.0 (BuildID 20260318190823) on macOS aarch64
  matches the Firefox 150 capture field for field, cold and resumed.
- **Firefox 150.** Firefox 150.0 sends 17 cipher suites
  (`t13d1717h2_5b57614c22b0_3cbfd9057e0d`). The resumed ClientHello adds
  `pre_shared_key` (41). Firefox 150.0.3 drops
  `TLS_ECDHE_ECDSA_WITH_AES_128_CBC_SHA` (`t13d1617h2_86a278354501_3cbfd9057e0d`);
  the profile follows 150.0. Both builds are captured
  (`firefox-150.0` and `firefox-150.0.3`); a Firefox 150 user on the last
  point release sends the 16-cipher hello.
- **Firefox 151 to 153.** The TLS values are the same in all three. The
  cipher list drops `TLS_ECDHE_ECDSA_WITH_AES_128_CBC_SHA`.
- **Firefox 156.** Firefox 156 drops `ffdhe2048` and `ffdhe3072` from the
  supported groups. The JA4 is the same as Firefox 154 and 155.
- **Request header presets.** tls.peet.ws records only the top-level
  navigation. The other presets come from a local HTTPS server that logs
  each HTTP/2 HEADERS frame in wire order. Its page loads a `<script src>`,
  sends a JSON `POST` (`xhr`), a form-encoded
  `fetch()` `POST` (`form`), a `fetch()` to a sibling subdomain
  (`same-site`) and to another site (`cross-origin`), and then a clicked
  form `POST` (`form-navigate`). Chrome 154.0.8037.57, Edge 154.0.4258.37,
  Brave 1.96.59, and Firefox 156.0.1 ran headful on Linux, twice each, with
  the same result both times. Edge sends the Chrome shapes. Brave adds
  `sec-gpc: 1` after `accept` on navigations, scripts, and plain fetches,
  and after `sec-ch-ua-mobile` on CORS fetches, and drops
  `application/signed-exchange` from the navigate `accept`. A
  parser-blocking script carries `priority: u=1` in Chrome and `u=2` in
  Firefox. Firefox sends `priority: u=4` on each `fetch()`. Every browser
  sends the cookie last. The `native` preset describes a client that is not a
  browser, so no browser capture applies to it. The captures are
  `captures/presets-<browser>-<version>-linux-run<n>.json`.
- **Safari 18.** Safari 18.6 on macOS 15.7.7 sends the same ClientHello as
  CFNetwork iOS 18: AES_128 first in the TLS 1.3 ciphers, no X25519MLKEM768,
  TLS 1.0 and 1.1 in `supported_versions`, and the padding extension last. The
  HEADERS frame carries a priority with weight 256. The capture ran in a VM, so
  its TCP SYN is not a Mac's; only the TLS, HTTP/2, and header values come from
  it.
- **Safari 26.** Safari 26.6.2 (21624.5.1.11.3) on macOS 26.6.2 sends the
  same ClientHello and HTTP/2 SETTINGS as Safari 26.2. Its header list adds
  `zstd` to `accept-encoding`, so the profile uses the `webkit-26` header
  style. Safari 18 and Mobile Safari 17 and 18 keep the `webkit` style. The
  capture ran in a VM from the `macos-tahoe-base` image, so its TCP SYN is not
  a Mac's; only the TLS, HTTP/2, and header values come from it. Two
  safaridriver runs against `tls.peet.ws/api/all` gave the same JA3, JA4,
  peetprint, and Akamai HTTP/2 fingerprint.
- **Safari iOS 27 and CFNetwork iOS 27.** The iOS 27.0 simulator loads
  `CFNetwork`, `Network`, `libcoretls`, and `libboringssl` from the iOS runtime,
  not from macOS. Its ClientHello is the same as Safari 26 and CFNetwork macOS
  26. Mobile Safari and Safari 26.6.2 send the `webkit-26` header style, where
  `accept-encoding` adds `zstd`. CFNetwork iOS 27
  sends a 2 MiB initial stream window, where macOS sends 4 MiB. The simulator
  takes the Darwin version in the CFNetwork user agent from the host kernel, so
  `Darwin/25.6.0` is the host's value, not a phone's.
- **Safari iOS 18 and Safari iOS 17.** Mobile Safari 18.6 sends the same
  ClientHello and HTTP/2 SETTINGS as Safari 18.6 on macOS. Mobile Safari 17.5
  adds `ecdsa_sha1` to the signature algorithms, sends no SETTINGS parameter 9,
  uses the `m,s,p,a` pseudo-header order, and a priority weight of 255. Its
  header order is `accept`, `sec-fetch-site`, `accept-encoding`,
  `sec-fetch-mode`, `user-agent`, `accept-language`, `sec-fetch-dest`, which
  the `webkit` header style does not produce. Mobile Safari 17.5 did not use
  HTTP/3 after an Alt-Svc response in three fresh simulators, so its profile
  has no `[h3]` table. The iOS 18.6 simulator ran on a macOS 15 host, so
  `CFNetwork/3826.600.41 Darwin/24.6.0` carries the Darwin version of iOS 18.
- **CFNetwork macOS 26.** The ClientHello has one GREASE cipher first, TLS 1.3
  ciphers in the order AES_256, CHACHA20, AES_128, and `rsa_pss_rsae_sha384`
  (0x0805) twice in the signature algorithms. The groups are GREASE, then
  X25519MLKEM768; the key shares are GREASE, X25519MLKEM768, and X25519.
  Certificate compression is zlib. The ClientHello has no ALPS, no ECH GREASE,
  and no padding. `extension_order` omits the GREASE extensions, which
  BoringSSL places itself. `initial_connection_window_size` is 10551295 so
  that the WINDOW_UPDATE increment is 10 MiB.
- **CFNetwork iOS 18.** Compared with macOS 26: no X25519MLKEM768 (groups are
  GREASE, X25519, P-256, P-384, P-521; key shares are GREASE and X25519), no
  SETTINGS parameter 9, a 2 MiB initial stream window, AES_128 first in the TLS
  1.3 ciphers, TLS 1.0 and 1.1 in `supported_versions`, and the padding
  extension (21) last. BoringSSL padding to 512 bytes matches this ClientHello
  at SNI `tls.peet.ws`. The padding target for a public destination (about
  704) has one data point and is not confirmed. The iOS 18 build shares the
  duplicate 0x0805, zlib compression, the leading GREASE cipher, and the
  `m,s,p,a` pseudo-header order with macOS.

## HTTP/3 captures

The `[h3]` tables of Chrome 154, Brave (Chromium 154), Firefox 156, Safari 18,
Safari 26, Safari iOS 18, and Safari iOS 27 come from QUIC captures against
`https://quic.browserleaks.com/?minify=1`. The raw files end in `-h3.json`. The
desktop browsers ran in a macOS 26.6.2 VM (Safari 18 in a macOS 15.7.7 VM).
Mobile Safari captures are QUIC Initial packets, decrypted with the RFC 9001
initial keys. The `[h3]` tables of the other profiles have no QUIC capture.

Each of these profiles reproduces its capture: the QUIC ClientHello through
`[h3.tls]`, the transport parameter set, values, and order policy, the
connection ID lengths, the SETTINGS list, the control stream frames, and the
pseudo-header order. Values the browser picks at random per connection
(GREASE IDs and values, the Chromium parameter shuffle, the Safari rotation,
the Firefox destination connection ID length) are random in Leyline in the
same way. See [HTTP/3](http3.md) for the keys.

- **Chromium.** The transport parameters come in a random order, with
  `version_information`, `max_datagram_frame_size`, `google_connection_options`
  and a GREASE parameter. SETTINGS end with a GREASE setting, and a GREASE
  frame and a PRIORITY_UPDATE frame follow them. The source connection ID is
  empty.
- **Firefox.** The transport parameters include `max_ack_delay` 20,
  `version_information`, `reset_stream_at`, `min_ack_delay`, and
  `max_datagram_frame_size` 65535. The destination connection ID is 8 to 20
  bytes, biased to 8, and the source connection ID has 3 bytes. The QUIC
  ClientHello keeps `extended_master_secret` and `renegotiation_info`.
- **Safari.** The transport parameter order rotates between connections.
  Safari 18 and iOS 18 add Apple parameter `0xff080808` last. SETTINGS end with
  a GREASE setting. The source connection ID is empty.

The Chrome 154 capture negotiated real ECH, because its browser had the
server's ECH configuration. Leyline sends an ECH GREASE extension with the
same extension ID, so the JA4 is the same.

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
reads the user agent, `sec-ch-ua`, and `accept_language` for the chosen
platform from those tables. `build` returns `Kind::Config` when the profile has no
table for that platform. Without `.platform()`, the platform is Windows. A
brand overlay (`SessionBuilder::brand`) needs `chromium_major` in `[meta]`.
The request headers follow `header_style` in `[meta]`. `profiles/headers.toml`
is the one owner of request header shapes. Each top-level table is one shape,
and its key is the `header_style` value: `chromium`, `gecko`, `webkit`,
`webkit-26`, `okhttp`, `brave`, and `brave-154`. The build generates the `HeaderStyle` enum from this
file, so a new shape needs only a new table. The build fails when a profile or
a brand names a shape that the file does not define.

A shape has these keys:

- `variant`: the `HeaderStyle` variant name.
- `default`: `true` on the one shape that a profile without `header_style`
  uses. That shape is `chromium`.
- `fallback`: the header list for a request without a preset, or for a preset
  that the shape does not list.
- `presets`: one header list for each `Preset`, with names, order, and values.
- `extends`: another shape. The shape takes each preset and the fallback that
  it does not define from that shape.
- `append`: headers that every request of the shape carries. A session or
  request header of the same name wins.
- `order`: a header order that the session applies after the merge, including
  the cookie and caller headers. `RequestBuilder::header_order` and
  `FingerprintSpec::header_order` replace it.

The placeholders `{user_agent}`, `{sec_ch_ua}`, `{sec_ch_ua_mobile}`,
`{sec_ch_ua_platform}`, `{accept_language}`, `{origin}`, and `{referer}` take
the session values. The `brave` shape extends `chromium`. It sends
`sec-gpc: 1`, a navigate `accept` without `application/signed-exchange`, and
the header order of the Brave 146 capture of 2026-09-25 from tls.peet.ws. The
`brave-154` shape carries every preset from the Brave 1.96.59 capture, with
`sec-gpc` in the position that Brave sends it, and has no `order`. A
brand row in `profiles/brands.toml` can set `header_style` to replace the
profile's shape.

`key_shares` in `[tls]` lists the groups that the ClientHello `key_share`
extension carries, in order. Each group must also be in `curves`. Without it,
BoringSSL sends key shares for the first one or two groups.

A brand in `profiles/brands.toml` can set a `[<brand>.tls]` table. Its
`request_trust_anchors` value replaces the base profile's value when the brand
is active. Edge sets it to `false`: Edge 154 does not send the trust anchors
extension that Chrome 153 sends.

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
- `user_agent(ua)`: replaces the user agent in every identity table.
- `header_order(names)`: replaces the order of the profile's header shape.
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
