# Choosing a profile

A session that impersonates a browser has a profile and a platform. The
profile sets the TLS ClientHello, the HTTP/2 settings, and the headers that
the session sends. The platform sets the operating system that the session
claims.

## Start with the default

`Session::new()` selects `Browser::default()`, the newest bundled Chrome, on
Windows. With the `http3` feature, it races HTTP/3 against HTTP/2 for an
origin that advertised `h3` in an `Alt-Svc` header.

```rust,no_run
use leyline::{Browser, Platform, Session};

# fn run() -> leyline::Result<()> {
let windows = Session::new();
let mac = Session::builder()
    .browser(Browser::default())
    .platform(Platform::MacOS)
    .build()?;
# let _ = (windows, mac);
# Ok(())
# }
```

## Pick another browser

| You need | Use |
| --- | --- |
| The most common desktop browser | `.browser(Browser::default())` |
| Firefox | `.browser(Browser::latest(leyline::Family::Firefox))` |
| Safari on macOS | `.browser(Browser::Safari26).platform(Platform::MacOS)` |
| Edge or Opera | `.brand(ChromiumBrand::Edge)` or `.brand(ChromiumBrand::Opera)` on a Chrome profile. [Sessions](sessions.md#apply-a-brand-overlay) lists the Chrome profiles that each brand accepts |
| Brave | `.browser(Browser::Brave146).platform(Platform::MacOS)` |
| A phone | `.platform(Platform::Android)` or `.platform(Platform::IOS)` with a browser that has that platform, or an app profile such as `Browser::OkHttpAndroid10` or `Browser::CfnetworkIOS18` |
| One fixed version | `.browser(Browser::Chrome150)`, then `.platform(...)` |

`Browser::latest(family)` returns the newest bundled version of a family. See
[Sessions](sessions.md) for the platform defaults and the call order.

## Match the platform

The platform sets `Sec-CH-UA-Platform`, `Sec-CH-UA-Mobile`, the `User-Agent`,
and the TCP fingerprint. A site can compare the claimed OS with the TCP
fingerprint, so a claim that differs from the real host is visible. Use
`Platform::Host` to claim the OS you build for.

## What the JA4 labels mean

The [profile reference](profiles.md) labels the JA4 of each profile `gated` or
`estimated`.

- A `gated` profile has a fixed extension order. Its JA4 equals the value
  recorded from the browser. The Firefox, Safari, OkHttp, and CFNetwork
  profiles are gated.
- An `estimated` profile has a randomized extension order. Chrome randomizes
  the extension order on every connection, so a Chrome profile cannot fix one.
  The Chrome and Brave profiles are estimated.

JA4 sorts the extensions before it hashes them, so the label says nothing
about how closely a profile matches its browser. The default Chrome profile is
`estimated` for the reason above.

## Next

Read [Requests](requests.md) to build and send a request.
