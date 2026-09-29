# Browser profiles

A profile is one TOML file that describes a single browser build's TLS
ClientHello, HTTP/2 SETTINGS, and per-platform identity. Leyline compiles the
bundled profiles into the crate and indexes them in `ProfileRegistry`. You
select a bundled profile with `SessionBuilder::browser`.

Profiles live under `crates/leyline/profiles/<family>/<version>.toml`.

## What the fingerprint columns mean

A profile records a JA4 reference value under `[tls.fingerprint]` and an Akamai
HTTP/2 reference value under `[h2.fingerprint]`. The offline
`fingerprint_conformance` test computes both values from the profile's own
`[tls]` and `[h2]` tables and compares them with the reference values. The
`JA4` column of the table below shows how the JA4 check treats each profile:

- **Gated.** The profile sets `extension_permutation`, so the ClientHello
  extension order is fixed and the computed JA4 is exact. The test fails when
  it differs from the reference value. The test gates the HTTP/2 value of
  every profile the same way.
- **Estimated.** The profile leaves the extension order to BoringSSL, so the
  computed JA4 is an estimate. The test reports a difference and passes.

The live `tls_peet` suite sends a request with each profile to a fingerprint
echo server and compares the JA4 and HTTP/2 values that the server reports
with the reference values.

The reference values come from the capture that the profile's `capture` and
`captured_against` keys name. Neither test captures a browser. A matching JA4
does not prove that every ClientHello field is equal.

`captured_against` records the exact browser build a profile was captured from.
A profile without it logs a `tracing` warning when it loads. The warning says
the capture build is unrecorded.

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
| Safari 27 | `Safari27` | `safari-27.0-21625.1.29.18.28` | gated |
| Safari iOS 17 | `SafariIOS17` | `safari-ios-17.5-21F79-simulator` | gated |
| Safari iOS 18 | `SafariIOS18` | `safari-ios-18.6-22G86-simulator` | gated |
| Safari iOS 27 | `SafariIOS27` | `safari-ios-27.0-iphone16,2-device` | gated |
| OkHttp4 Android 10+ | `OkHttpAndroid10` | `okhttp-4.12.0` | gated |
| CFNetwork iOS 18 | `CfnetworkIOS18` | `cfnetwork-3826.600.41-ios18.6-simulator` | gated |
| CFNetwork iOS 27 | `CfnetworkIOS27` | `cfnetwork-3892.100.1-ios27.0-device` | gated |
| CFNetwork macOS 26 | `CfnetworkMacOS26` | `cfnetwork-3860.700.1-macos26.6.2-vm` | gated |

## Provenance

Each profile comes from one of six sources:

- **Browser capture.** A capture of the named browser, with the build recorded
  in `captured_against`.
- **Native stack capture.** A capture of an operating system HTTP stack, such
  as URLSession for CFNetwork, with the build recorded in `captured_against`.
- **Non-browser build capture.** A capture of a related build that is not the
  shipped browser, such as `chrome-headless-shell` or a WKWebView host.
- **Emulator capture.** A capture of the shipped app on an Android emulator or
  an iOS simulator. The app and the operating system's own TLS stack are real,
  but the device is not a physical phone. The build and OS version are
  recorded in `captured_against`.
- **Inferred.** No capture of this version. Values come from a neighboring
  version.
- **Self-referential.** The reference values are Leyline's own past output, so
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
| Safari 27 | Browser capture | `browser` | `safari-27.0-21625.1.29.18.28`, Safari.app 27.0 on macOS 26.6.2 in a VM through safaridriver |
| Safari iOS 17 | Emulator capture | `emulator` | `safari-ios-17.5-21F79-simulator`, Mobile Safari in the iOS 17.5 simulator |
| Safari iOS 18 | Emulator capture | `emulator` | `safari-ios-18.6-22G86-simulator`, Mobile Safari in the iOS 18.6 simulator |
| Safari iOS 27 | Browser capture | `browser` | `safari-ios-27.0-iphone16,2-device`, Mobile Safari 27.0 on an iPhone 15 Pro Max (iOS 27.0); the iOS 27.0 simulator capture matches it |
| OkHttp4 Android 10+ | Emulator capture | `emulator` | `okhttp-4.12.0`, test app on the platform TLS stack of an Android 17 emulator |
| CFNetwork iOS 18 | Emulator capture | `emulator` | `cfnetwork-3826.600.41-ios18.6-simulator`, URLSession test binary in the iOS 18.6 simulator |
| CFNetwork iOS 27 | Native stack capture | `native` | `cfnetwork-3892.100.1-ios27.0-device`, URLSession through Shortcuts on an iPhone 15 Pro Max (iOS 27.0) |
| CFNetwork macOS 26 | Native stack capture | `native` | `cfnetwork-3860.700.1-macos26.6.2-vm`, URLSession test binary on macOS 26.6.2 in a VM; macOS 26.2 on hardware sends the same ClientHello |

Chromium-family profiles do not store `sec-ch-ua`. Leyline derives it from the
major version and the `ch_ua_brand` field in `[meta]`, with the same GREASE
brand, version, and order rule that Chromium uses.

`Browser::latest` returns the newest profile of a family whose `capture` is in
the family's `latest_capture` list in `families.toml`. The list defaults to
`browser`. CFNetwork uses `native` and `emulator`, and Safari iOS and OkHttp
add `emulator`. The build fails when a family has no such profile. A
`platform_browser` target resolves the same way, with the target family's
`latest_capture` list. `Session::new()` uses `Browser::latest(Family::Chrome)`,
the newest Chrome in the table above. You can pin the product line instead of
a version:

```rust
use leyline::{Browser, Family};

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
  Xvfb, Brave 146 sent 0.8 and 0.6. Brave 154 sent 0.6 in three navigations
  (`brave-154.1.96.59-linux-headful.json` and two other runs) and 0.9 and 0.5 in
  the preset captures. The Brave 154 Linux row uses 0.6, the value seen most.
  Brave picks the q value at random, so any fixed value matches only some
  requests.
- **Edge 154 and Opera 136 on Linux.** Edge 154.0.4258.37 and Opera
  136.0.6008.52 (Chromium 152), captured headful under Xvfb with no user agent
  override. They send the Chrome user agent form with `Edg/154.0.0.0` and
  `OPR/136.0.0.0`, `sec-ch-ua-platform: "Linux"`, and the Chrome navigate
  header order.
- **Edge 145 to 152 on Windows.** Edge 145.0.3800.97, 146.0.3856.117,
  147.0.3912.98, 148.0.3967.96, 149.0.4022.98, 150.0.4078.105,
  151.0.4129.107, and 152.0.4191.66 come from the Microsoft Edge Enterprise
  x64 MSI packages on the Microsoft download CDN. Each MSI matches the SHA-256
  in the winget-pkgs manifest. The embedded installer matches the SHA-256 in
  the offline manifest of the MSI. The MSI, the installer, and `msedge.exe`
  carry a valid Authenticode signature from Microsoft Corporation. Each build
  ran unpacked, headful, twice, from an empty profile, with no user agent
  override (`edge-<version>-windows-run<n>.json`). Each build sends the
  ClientHello, HTTP/2 settings, and header order of Chrome on Windows for the
  same major, `Edg/<major>.0.0.0`, and the `Microsoft Edge` brand in
  `sec-ch-ua`. Edge 152 does not send the Trust Anchor Identifiers extension
  that Chrome 152 sends.
- **Edge 145 to 152 on macOS.** Edge 145.0.3800.97, 146.0.3856.97,
  147.0.3912.98, 148.0.3967.96, 149.0.4022.98, 150.0.4078.105,
  151.0.4129.107, and 152.0.4191.66 come from the macOS DMGs on the Microsoft
  download CDN. Each DMG matches its published SHA-256, and Gatekeeper accepts
  each app as notarized Developer ID, Microsoft Corporation (UBF8T346G9). Each
  build ran headful, twice, from an empty profile
  (`edge-<version>-macos-run<n>.json`), and sends the ClientHello and HTTP/2
  settings of Chrome on macOS for the same major, with `Edg/<major>.0.0.0`.
- **Opera 129 to 135 on Linux.** Opera 129.0.5823.65, 130.0.5847.92,
  131.0.5877.116, 132.0.5905.114, 133.0.5932.85, 134.0.5954.66, and
  135.0.5973.142 from get.geo.opera.com. Each `.rpm` matches Opera's
  `.sha256sum`, and its header signature verifies with Opera's RPM repository
  key `6C86BE214648376680CA957B11EE8C00B693A745`. Each build ran headful under
  Xvfb twice, from an empty profile, with no user agent override. Each sends
  the ClientHello, HTTP/2 settings, and header order of Chrome on Linux for
  the same Chromium major (145 to 151), `OPR/<major>.0.0.0`, the `Opera`
  brand in `sec-ch-ua`, and `sec-ch-ua-platform: "Linux"`
  (`opera-<version>-linux-headful-run<n>.json`).
- **Firefox 148 to 156.** Official Mozilla builds captured on macOS and
  Linux with `--headless` on 2026-09-25, and on Windows headful from the
  `win64` installers, with the SHA256SUMS signature and the Authenticode
  signature checked. The Windows builds send
  `Windows NT 10.0; Win64; x64` and the same TLS and H2 values as macOS and
  Linux. Every build sends three key shares
  (X25519MLKEM768, X25519, P-256) and a HEADERS PRIORITY with weight 42 and no
  exclusive bit, sent as 41.
- **Firefox 150.** Firefox 150.0 sends 17 cipher suites
  (`t13d1717h2_5b57614c22b0_3cbfd9057e0d`), and the profile follows that
  capture. Firefox 150.0.3 on Android and Firefox 151 to 153 on desktop send 16,
  without `TLS_ECDHE_ECDSA_WITH_AES_128_CBC_SHA`. No desktop capture of a
  Firefox 150 point release exists, so the cipher list of desktop 150.0.3 is
  unconfirmed.
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
  the same result both times. Edge sends the same headers as Chrome. Brave adds
  `sec-gpc: 1` after `accept` on navigations, scripts, and plain fetches,
  and after `sec-ch-ua-mobile` on CORS fetches, and drops
  `application/signed-exchange` from the navigate `accept`. A
  parser-blocking script carries `priority: u=1` in Chrome and `u=2` in
  Firefox. Firefox sends `priority: u=4` on each `fetch()`. Every browser
  sends the cookie last. The `native` preset describes a client that is not a
  browser, so no browser capture applies to it. The captures are
  `captures/presets-<browser>-<version>-linux-run<n>.json`.
  Safari 18.6 (20621.3.11.11.3) on macOS 15.7.7, and Safari 26.6.2
  (21624.5.1.11.3) and Safari 27.0 (21625.1.29.18.28) on macOS 26.6.2 ran in
  VMs through safaridriver, twice each, with the same result both times.
  Safari 26.6.2 and 27.0 send identical presets. Safari 18.6 differs only in
  an `accept-encoding` without `zstd`. Safari sends `priority: u=1, i` on a
  script, `u=3, i` on each `fetch()`, and no `cache-control` on a form
  `POST`. Mobile Safari 17.5 in the iOS 17.5 simulator (21F79) sends no
  `priority` header. Its form `POST` came from page script, because the
  simulator's Web Inspector listed no pages. Mobile Safari 27.0 on an iPhone
  (`presets-safari-ios-27.0-iphone16,2-device.json`) sends the macOS Safari
  27 order. The iOS 27.0 simulator (24A434) puts `sec-fetch-site` before
  `origin` on CORS fetches. The Safari captures are
  `captures/presets-safari-<version>-<build>-<os>-run<n>.json`.
- **Safari 18.** Safari 18.6 on macOS 15.7.7 sends the same ClientHello as
  CFNetwork iOS 18: AES_128 first in the TLS 1.3 ciphers, no X25519MLKEM768,
  TLS 1.0 and 1.1 in `supported_versions`, and the padding extension last. The
  HEADERS frame carries a priority with weight 256. The capture ran in a VM, so
  it gives no TCP SYN values for a Mac. The profile takes only its TLS, HTTP/2,
  and header values from the capture.
- **Safari 26.** Safari 26.6.2 (21624.5.1.11.3) on macOS 26.6.2 sends the
  same ClientHello and HTTP/2 SETTINGS as Safari 26.2. Its header list adds
  `zstd` to `accept-encoding`, so the profile uses the `webkit-26` header
  style. Safari 18 and Mobile Safari 17 and 18 keep the `webkit` style. The
  capture ran in a macOS 26.6.2 VM, so it gives no TCP SYN values for a Mac.
  The profile takes only its TLS, HTTP/2, and header values from the capture.
  Two safaridriver runs against `tls.peet.ws/api/all` gave the same JA3, JA4,
  peetprint, and Akamai HTTP/2 fingerprint.
- **Safari iOS 27 and CFNetwork iOS 27.** The iOS 27.0 simulator loads
  `CFNetwork`, `Network`, `libcoretls`, and `libboringssl` from its own iOS
  runtime. Its ClientHello is the same as Safari 26 and CFNetwork macOS 26.
  Mobile Safari and Safari 26.6.2 send the `webkit-26` header style, where
  `accept-encoding` adds `zstd`. Safari iOS 27 and CFNetwork iOS 27 were then
  captured on an iPhone 15 Pro Max running iOS 27.0. Mobile Safari matches the
  simulator on TLS, HTTP/2, HTTP/3, and headers. CFNetwork on the device sends
  `CFNetwork/3892.100.1 Darwin/27.0.0` and a 512 KiB initial stream window.
  The device request came from the Shortcuts app and the simulator request from
  a plain `URLSession` binary, so the window difference may belong to the app
  rather than the device. The simulator also reported the host's Darwin
  version. The profile follows the device. Real apps send
  `<App>/<version> CFNetwork/... Darwin/...`; prefix the profile's user agent
  with your app's token.
- **Safari iOS 18 and Safari iOS 17.** Mobile Safari 18.6 sends the same
  ClientHello and HTTP/2 SETTINGS as Safari 18.6 on macOS. Mobile Safari 17.5
  adds `ecdsa_sha1` to the signature algorithms, sends no SETTINGS parameter 9,
  uses the `m,s,p,a` pseudo-header order, and a priority weight of 255. Its
  header order is `accept`, `sec-fetch-site`, `accept-encoding`,
  `sec-fetch-mode`, `user-agent`, `accept-language`, `sec-fetch-dest`. The
  `webkit-17` header style sends that navigate order; its other presets come
  from the iOS 17.5 simulator captures. Mobile Safari 17.5 did not use
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

- **GREASE signature algorithm.** Chrome 152 to 154, Brave 1.96.59, Edge
  153 and 154, and Opera 136 put one GREASE value first in
  `signature_algorithms` on every captured platform. Chrome 145 to 151 and
  Brave 1.88.138 do not. The value is random per connection and independent
  of the cipher GREASE. `sigalg_grease = true` in `[tls]` turns it on; it
  needs `grease = true`. BoringSSL patch
  `0004-leyline-grease-signature-algorithms.patch` adds it. JA4 and JA3 drop
  GREASE, so only the raw list shows it. The QUIC ClientHello of these
  builds has no GREASE value, so `[h3.tls]` leaves it off.
- **Resumed ClientHello.** The second request to `tls.peet.ws` in the same
  browser process resumes with `pre_shared_key` (41). Official Linux builds,
  two fresh processes each, on 2026-09-25 (`<browser>-<version>-linux-resumed-run<n>.json`),
  and Chrome 154.0.8037.58 and Firefox 156.0.1 on Windows
  (`-windows-resumed-run<n>.json`), give `resumed_ja4`:
  Chrome 145 to 149 and Brave 1.88.138 `t13d1517h2_8daaf6152771_b6f405a00624`;
  Chrome 150, 151 and Brave 1.96.59 `t13d1517h2_8daaf6152771_a87ad97598a9`;
  Chrome 152 to 154 `t13d1518h2_8daaf6152771_e2d80978ab2e`; Firefox 148 to
  150 `t13d1717h2_5b57614c22b0_e6dcd7ae0a9e`; Firefox 151 to 153
  `t13d1617h2_86a278354501_e6dcd7ae0a9e`; Firefox 154 to 156
  `t13d1517h2_8daaf6152771_e6dcd7ae0a9e`. Firefox drops `session_ticket` (35)
  when it adds `pre_shared_key`. Safari 18.6, 26.6.2, and 27.0 under
  safaridriver sent no `pre_shared_key` on a second connection in two fresh
  processes each (`safari-<version>-<build>-macos-<version>-vm-resumed-run<n>.json`),
  so the Safari profiles keep `pre_shared_key = false`.
- **CFNetwork headers.** URLSession sends a fixed header list with no
  `sec-*` headers. CFNetwork iOS 18 sends `user-agent`, `accept: */*`,
  `accept-language`, `accept-encoding: gzip, deflate, br` (the `cfnetwork`
  style; CFNetwork 1496 on iOS 17.5 sends the same order). CFNetwork macOS 26
  and iOS 27 send `accept: */*`, `user-agent`, `priority: u=3`,
  `accept-language`, `accept-encoding: gzip, deflate, br` (the
  `cfnetwork-26` style). The captured `priority: u=3` appears on iOS 27 and on
  macOS 26.
- **Android.** Chrome 145.0.7632.218, preinstalled and Google-signed in an
  Android 17 (API 37) Google Play emulator image, sends the same ClientHello
  and HTTP/2 values as Chrome 145 on desktop, the `Android 10; K` user agent,
  and `accept-language: en-US,en;q=0.9`
  (`chrome-android-145.0.7632.218-emulator-run<n>.json`). Firefox for Android
  148.0.2, 149.0.2, 150.0.3, 151.0.4, 152.0.6, 153.0.4, 154.0.1, 155.0.1, and
  156.0.1 from archive.mozilla.org (arm64-v8a APKs, signer Mozilla Release
  Engineering, certificate SHA-256
  `a78b62a5165b4494b2fead9e76a280d22d937fee6251aece599446b2ea319b04`; the
  Fenix releases publish no SHA256SUMS) ran twice each in the same emulator
  under geckodriver. All send `Android 17; Mobile`, `accept-language: en-US`,
  and the HTTP/2 SETTINGS `1:4096;2:0;4:32768;5:16384`, which
  `[h2.platforms.android]` of Firefox 148 to 156 sets. Firefox 155 and 156 on
  Android send the desktop ClientHello. Firefox 148 to 154 on Android omit
  `signed_certificate_timestamp` (148 and 149:
  `t13d1716h2_5b57614c22b0_eeeea6562960`; 150 to 153:
  `t13d1616h2_86a278354501_eeeea6562960`; 154:
  `t13d1516h2_8daaf6152771_eeeea6562960`). A profile has no per-platform TLS
  table, so Leyline sends the desktop ClientHello of each version on Android.
- **Opera on Windows.** Opera 136.0.6008.52 from get.geo.opera.com
  (Authenticode signer Opera Norway AS, checksum from Opera's `.sha256sum`)
  sends the Chrome 152 ClientHello, `OPR/136.0.0.0`, the `Opera` brand in
  `sec-ch-ua`, and `accept-language: en-US,en;q=0.9`
  (`opera-136.0.6008.52-windows-run<n>.json`).
- **TCP rows.** `platforms.toml` has Windows, macOS, Linux, and iOS rows. The
  option order, window, window scale, and TTL of the Windows, macOS, and Linux
  rows match every browser SYN captured from that host. The browser captures
  show MSS 1400 because the path clamps it. The hosts' own SYNs show the
  default: the Linux host (MTU 1500) sends MSS 1460, window 64240, and options
  `mss, sackOK, TS, nop, wscale 10`, and the Windows VM (MTU 1500) sends MSS
  1460, window 64240, and `mss, nop, wscale 8, nop, nop, sackOK`. No host SYN
  capture exists for macOS, so its MSS 1460 is unconfirmed. The iOS row comes
  from an iPhone 15 Pro Max on iOS 27.0: window 65535, window scale 6, TTL 64,
  and `mss, nop, wscale, nop, nop, TS, sackOK, eol`. Its SYN shows MSS 1400
  because of the path, and the row's MSS 1460 is the interface default, which
  no capture shows. Android has no row: the emulator sends the SYNs of its
  host.

## HTTP/3 captures

Every `[h3]` table comes from QUIC captures against
`https://quic.browserleaks.com/?minify=1`. The raw files end in `-h3.json` or
`-h3-run<n>.json`.

- Chrome 145 to 154, Brave 1.88.138 and 1.96.59, and Firefox 148 to 156:
  official Linux builds, headful under Xvfb, two fresh processes each, eight
  for Chrome 154 and Firefox 156, on 2026-09-25. Every build used HTTP/3 on
  the first navigation with no QUIC flag or preference.
- Chrome 154.0.8037.58 and Firefox 156.0.1 on Windows, two runs each. The
  HTTP/3 HEADERS carry `sec-ch-ua-platform: "Windows"`.
- Chrome 154, Brave 1.96.59, Edge 154, Firefox 156, Safari 26.6.2, and Safari
  27.0 in a macOS 26.6.2 VM, and Safari 18.6 in a macOS 15.7.7 VM.
- Mobile Safari 18.6 in the iOS 18.6 simulator, two runs with the HTTP/3
  frames (`safari-ios-18.6-22G86-simulator-h3-run<n>.json`), and Mobile
  Safari 27.0 in the iOS 27.0 simulator.

Each profile reproduces its capture: the QUIC ClientHello through
`[h3.tls]`, the transport parameter set, values, and order policy, the
connection ID lengths, the SETTINGS list, the control stream frames, and the
pseudo-header order. `[h3.tls]` lists only the TLS 1.3 ciphers, in the
captured order, and no `min_tls_version` or `padding`, because QUIC sends
neither. Values the browser picks at random per connection are random in
Leyline in the same way. See [HTTP/3](http3.md) for the keys.

- **Chromium.** The transport parameters come in a random order. All builds
  send `version_information` with a GREASE version at a random position,
  `max_datagram_frame_size` 65536, `initial_max_streams_uni` 103, and a GREASE
  parameter of up to 15 bytes. Chrome 145 adds `google_version` (18258) with
  QUICv1. Chrome 149 and later add `google_connection_options` `ORIG`.
  Chrome 152 to 154 add Trust Anchor Identifiers to the QUIC ClientHello;
  Brave 1.96.59 does not. SETTINGS are `1:65536;6:262144;7:100;51:1` and a
  GREASE setting, followed by a GREASE frame of 0 to 3 bytes and a
  PRIORITY_UPDATE frame. The connection IDs are 8 and 0 bytes. Each Initial
  packet is 1250 bytes (`initial_datagram_size = 1250`).
- **Firefox.** The transport parameter order is fixed. Firefox 155 and 156
  add parameter 29 and offer QUICv2 in `version_information` (GREASE, QUICv2,
  QUICv1); Firefox 148 to 154 offer GREASE and QUICv1. Every run of every
  build, 27 in all, sends GREASE-form parameter 4278378010 with the value
  1000, so the profiles pin it. The source connection ID has 3 bytes. The
  destination connection ID length varies per connection; `dcid_length`
  weights are the 27 observed lengths: 8 (12), 10 (4), 11 (1), 13 (5), 15
  (2), 16 (1), 19 (1), 20 (1). Firefox 156 adds the ML-DSA schemes to the
  QUIC signature algorithms and delegated credentials; 148 to 155 do not.
  The QUIC ClientHello keeps `extended_master_secret` and
  `renegotiation_info`. Firefox shuffles the QUIC ClientHello extensions on
  every connection and keeps `quic_transport_parameters` and ECH last, so
  `[h3.tls]` sets `permute_extensions` and `extension_tail = [57, 65037]`.
  The Initial packets carry no PADDING frames, so the profiles set no
  `initial_datagram_size`. Every capture shows two Initial packets that
  differ by 3 or 4 bytes: the first holds two CRYPTO frames, the second
  holds one. Firefox splits the ClientHello evenly and cuts it at the
  midpoint of the server name, so the profiles set
  `initial_crypto_split = "even"` and
  `initial_crypto_reorder = "sni_midpoint"`.
- **Safari.** The transport parameter order rotates between connections.
  Safari 18 and iOS 18 add Apple parameter `0xff080808` last. SETTINGS are
  `1:16383;7:100` and a GREASE setting, with `m,s,a,p` pseudo-headers. The
  source connection ID is empty. Each Initial packet is 1200 bytes
  (`initial_datagram_size = 1200`).

The Chrome 154 capture negotiated real ECH, because its browser had the
server's ECH configuration. Leyline sends an ECH GREASE extension with the
same extension ID, so the JA4 is the same.

## Uncaptured values

These values have no capture of the official build on their own platform.
Leyline keeps them so that the API stays complete, but no capture backs them.

- Chrome `[identity.android]` 146 to 154: only Chrome 145 on Android is
  captured. The rows follow the reduced `Android 10; K` user agent.
- The MSS 1460 of the macOS and iOS TCP rows.
- Header presets on Windows and macOS for the Chromium and Gecko styles.

## Update cadence

- **Chrome and Firefox:** Leyline adds a profile for each new stable major
  release. Both browsers ship a major release every four weeks.
- **Safari:** Leyline adds a profile when Apple ships an OS release, because
  Safari's TLS stack changes with macOS and iOS.
- **Brave, OkHttp, and CFNetwork:** Leyline recaptures the profile when the
  upstream engine version in `captured_against` changes.

Every new profile records the exact build in `captured_against`, and the JA4
and HTTP/2 reference values from that capture.

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
defines every header style. Each top-level table is one style, and its key is
the `header_style` value: `chromium`, `gecko`, `webkit`, `webkit-26`,
`webkit-17`, `okhttp`, `cfnetwork`, `cfnetwork-26`, `brave`, and `brave-154`.
The build generates the `HeaderStyle` enum from this file, so a new style needs
only a new table. The build fails when a profile or a brand names a style that
the file does not define.

A style has these keys:

- `variant`: the `HeaderStyle` variant name.
- `default`: `true` on the one style that a profile without `header_style`
  uses. That style is `chromium`.
- `fallback`: the header list for a request without a preset, or for a preset
  that the style does not list.
- `presets`: one header list for each `Preset`, with names, order, and values.
- `extends`: another style. The style takes each preset and the fallback that
  it does not define from that style.
- `append`: headers that every request of the style carries. A session or
  request header of the same name wins.
- `order`: a header order that the session applies after the merge, including
  the cookie and caller headers. `RequestBuilder::header_order` and
  `FingerprintSpec::header_order` replace it.

The placeholders `{user_agent}`, `{sec_ch_ua}`, `{sec_ch_ua_mobile}`,
`{sec_ch_ua_platform}`, `{accept_language}`, `{origin}`, `{referer}`, and
`{fetch_site}` take the values of the session and the request. The `brave`
style extends `chromium`. It sends
`sec-gpc: 1`, a navigate `accept` without `application/signed-exchange`, and
the header order of the Brave 146 capture of 2026-09-25 from tls.peet.ws. The
`brave-154` style carries every preset from the Brave 1.96.59 capture, with
`sec-gpc` in the position that Brave sends it, and has no `order`. A brand row
in `profiles/brands.toml` can set `header_style` to replace the profile's
style.

`key_shares` in `[tls]` lists the groups that the ClientHello `key_share`
extension carries, in order. Each group must also be in `curves`. Without it,
BoringSSL sends key shares for the first one or two groups.

A brand in `profiles/brands.toml` can set a `[<brand>.tls]` table. Its
`request_trust_anchors` value replaces the base profile's value when the brand
is active. Edge sets it to `false`: Edge 152 and 154 do not send the trust anchors
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
- `header_order(names)`: replaces the order of the profile's header style.
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
- A cipher, curve, or signature algorithm ID that is not in Leyline's
  built-in table fails. An extension or SETTINGS ID that Leyline cannot send
  also fails.

Every failure returns `Kind::Config` with a message that names the format and
the field.

## Next

Read [MSRV](msrv.md) for the Rust version Leyline supports.
