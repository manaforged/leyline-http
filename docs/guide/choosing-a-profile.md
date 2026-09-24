# Choosing a profile

A profile decides the TLS ClientHello, the HTTP/2 settings, and the headers
that a session sends. The platform decides the operating system that the
session claims. Pick both.

## Start with the default

`Session::chrome()` is the default choice. It selects
`Browser::default_browser()`, which is Chrome 152, on Windows, and it races
HTTP/3 against HTTP/2.

```rust,no_run
use leyline::{Platform, Session};

# fn run() -> leyline::Result<()> {
let windows = Session::chrome();
let mac = Session::builder().chrome().platform(Platform::MacOS).build()?;
# let _ = (windows, mac);
# Ok(())
# }
```

## Pick another browser

| You need | Use |
| --- | --- |
| The most common desktop browser | `.chrome()` |
| Firefox | `.firefox()`, which selects `Browser::default_firefox()` |
| Safari on macOS | `.safari()` |
| Edge, Opera, or Vivaldi | `.edge()`, `.opera()`, or `.vivaldi()`: a brand on a Chrome profile |
| Brave | `.brave()` |
| A phone | `.android()` or `.ios()` with a browser that has that platform, or an app profile such as `Browser::OkHttpAndroid10` or `Browser::CfnetworkIOS18` |
| One fixed version | `.browser(Browser::Chrome150)`, then `.platform(...)` |

`Browser::latest(family)` returns the newest bundled version of a family. See
[Sessions](sessions.md) for the platform defaults and the call order.

## Match the platform

The platform sets `Sec-CH-UA-Platform`, `Sec-CH-UA-Mobile`, the `User-Agent`,
and the TCP fingerprint. A site can compare the claimed OS with the TCP
fingerprint, so a claim that differs from the real host is visible. Use
`Platform::Host` to claim the OS you build for.

## What the JA4 labels mean

The [profile reference](profiles.md) labels each profile's JA4 as `gated` or
`reconnaissance`.

- **Gated** profiles fix their TLS extension order. The offline test fails
  when the profile's JA4 differs from the recorded reference value. Firefox,
  Safari 26, and the CFNetwork profiles are gated.
- **Reconnaissance** profiles do not fix the extension order. Chrome
  randomizes the extension order on every connection, so a Chrome profile
  cannot fix one. JA4 sorts the extensions before it hashes them, so the
  label says nothing about how closely the profile matches Chrome. The live
  `tls_peet` suite still compares the JA4 that Leyline sends with the
  recorded value.

The label is not a trust ranking. The default Chrome profile is labeled
`reconnaissance` for the reason above.
