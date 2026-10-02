# Choosing a profile

A browser session has a profile and a platform. The profile sets the TLS
ClientHello, the HTTP/2 settings, and the request headers. The platform sets
the operating system that the session claims. This page picks both.

## Start with the default

`Browser::default()` is `Browser::latest(Family::Chrome)`, the newest bundled
Chrome. Without `.platform()`, a Chrome session claims Windows.

```rust,no_run
use leyline::{Browser, Platform, Session};

# fn run() -> leyline::Result<()> {
let windows = Session::browser(Browser::default());
let mac = Session::builder()
    .browser(Browser::default())
    .platform(Platform::MacOS)
    .build()?;
# let _ = (windows, mac);
# Ok(())
# }
```

With the `http3` feature, a Chrome session races HTTP/3 against HTTP/2 for an
origin that advertised `h3`. See [HTTP/3](http3.md).

## Pick another browser

| You need | Use |
| --- | --- |
| Firefox | `.browser(Browser::latest(Family::Firefox))` |
| Safari on macOS | `.browser(Browser::Safari26).platform(Platform::MacOS)` |
| Safari on iPhone | `.browser(Browser::latest(Family::SafariIos))`. It needs no `.platform()`: the session claims iOS |
| Edge or Opera | `.brand(ChromiumBrand::Edge)` or `.brand(ChromiumBrand::Opera)` on a Chrome profile. See [Sessions](sessions.md#apply-a-brand-overlay) |
| Brave | `.browser(Browser::Brave146).platform(Platform::MacOS)` |
| A phone app | `Browser::OkHttpAndroid10` or `Browser::CfnetworkIOS18` |
| A phone browser | `.platform(Platform::Android)` or `.platform(Platform::IOS)` with a browser that has that platform |
| One fixed version | `.browser(Browser::Chrome150)`, then `.platform(...)` |

`.browser(Browser::Safari26).platform(Platform::IOS)` maps to the Safari on
iPhone sibling. [Sessions](sessions.md#platform-defaults-and-call-order)
gives the platform defaults and the call order, and
[Set the languages](sessions.md#set-the-languages) sets `accept-language`.

## Match the platform

The platform sets `Sec-CH-UA-Platform`, `Sec-CH-UA-Mobile`, the `User-Agent`,
and the TCP fingerprint. A site can compare the claimed OS with the TCP
fingerprint, so a claim that differs from the real host is visible. Use
`Platform::Host` to claim the OS you build for.

## Keep a device stable

`Browser::latest` moves to newer captures in patch releases, and a release can
correct the data behind a pinned variant. For an account that must keep one
device, pin the variant and store `session.identity().profile_id()`. Pass it
to `SessionBuilder::expect_profile_id` on start: `build()` then fails with
`Kind::Config` when the profile changed, and `err.is_profile_changed()` is
`true`. To keep the data itself, freeze it with `Device::pin_profile`. See
[Profile data stability](profiles.md#profile-data-stability) and
[Accounts](accounts.md#create-the-device-once).

## Read the JA4 labels

The [profile reference](profiles.md) labels the JA4 of each profile `gated`
or `estimated`. A Chrome or Brave profile is estimated because Chrome
shuffles the extension order on every connection. JA4 sorts the extensions
before it hashes them, so the JA4 stays the same; JA3 does not. To check a
live session, see
[Fingerprints](fingerprints.md#check-a-fingerprint-with-an-echo-service).

## Next

Read [Requests](requests.md) to build and send a request.
