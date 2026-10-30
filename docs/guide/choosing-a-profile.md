# Choosing a profile

A browser session has a profile and a platform. The profile sets the TLS
ClientHello, the HTTP/2 settings, and the request headers. The platform sets
the operating system that the session claims. This page picks both.

## Start with the default

`Browser::default()` is `Browser::chrome()`, the newest bundled
Chrome. `.browser(browser)` claims
the browser's own platform: Windows for Chrome.

```rust,no_run
use leyline::{Browser, Platform, Session};

# fn run() -> leyline::Result<()> {
let windows = Session::from(Browser::default());
let mac = Session::builder()
    .browser(Browser::default()).platform(Platform::MacOS)
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
| Firefox | `.browser(Browser::firefox())` |
| Safari on macOS | `.browser(Browser::Safari26).platform(Platform::MacOS)` |
| Safari on iPhone | `.browser(Browser::safari_ios())`. It needs no platform: the session claims iOS |
| Edge or Opera | `.browser(Browser::edge())` or `.browser(Browser::opera())`, desktop only. See [Sessions](sessions.md#pick-edge-or-opera) |
| Brave | `.browser(Browser::Brave146).platform(Platform::MacOS)` |
| A phone app | `Browser::OkHttpAndroid10` or `Browser::CfnetworkIOS18` |
| A phone browser | `.browser(browser).platform(Platform::Android)` or `Platform::IOS` with a browser that has that platform |
| One fixed version | `.browser(Browser::Chrome150).platform(platform)` |

`.browser(Browser::Safari26).platform(Platform::IOS)` maps to the Safari on
iPhone sibling. [Sessions](sessions.md#platform-defaults-and-call-order)
gives the platform defaults and the call order, and
[Set the languages](sessions.md#set-the-languages) sets `accept-language`.

## Match the platform

The platform sets `Sec-CH-UA-Platform`, `Sec-CH-UA-Mobile`, the `User-Agent`,
and the TCP fingerprint. `Platform::Host` uses the OS the program runs on.

## Keep a saved session stable

The newest-build functions such as `Browser::chrome()` move to newer captures in a release, and a release can
correct the data behind a pinned variant. For an account that must keep one
profile, pin the variant and store `session.profile_id()`. Compare it
with the new session's `profile_id()` on start, or save the session with `Session::save`: `build()` on the builder from `Session::load` returns an error with `err.is_profile_changed()` set to `true` when
the profile changed. To keep the data itself, build the session with `.profile(..)`, and the save stores the profile TOML. See
[Profile data stability](profiles.md#profile-data-stability) and
[Accounts](accounts.md#freeze-a-profile).

## Read the JA4 labels

The [profile reference](profiles.md) labels the JA4 of each profile `gated`
or `estimated`. A Chrome or Brave profile is estimated because Chrome
shuffles the extension order on every connection. JA4 sorts the extensions
before it hashes them, so the JA4 stays the same; JA3 does not. To check a
live session, see
[Fingerprints](fingerprints.md#check-a-fingerprint-with-an-echo-service).

## Next

Read [Requests](requests.md) to build and send a request.
