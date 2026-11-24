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

With the `http3` feature, which is on by default, browser profiles race HTTP/3
against HTTP/2 for origins that advertise `h3`. This holds for every bundled
browser profile except Safari iOS 17, on every platform it covers. CFNetwork
and OkHttp profiles do not race. See [HTTP/3](http3.md).

## Pick another browser

| You need | Use |
| --- | --- |
| Firefox | `.browser(Browser::firefox())` |
| Safari on macOS | `.browser(Browser::safari())`. Safari profiles cover macOS only |
| Safari on iPhone | `.browser(Browser::safari_ios())`. It needs no platform: the session claims iOS |
| Edge or Opera | `.browser(Browser::edge())` or `.browser(Browser::opera())`, desktop only. See [Sessions](sessions.md#pick-edge-or-opera) |
| Brave | `.browser(Browser::brave())`. It claims Windows; add `.platform(..)` for macOS or Linux |
| A phone app | `Browser::OkHttpAndroid10` or `Browser::CfnetworkIOS27` |
| A phone browser | `.browser(browser).platform(Platform::Android)` or `Platform::IOS` with a browser that has that platform |
| One fixed version | `.browser(Browser::Chrome158).platform(platform)` |

`.browser(Browser::Safari26).platform(Platform::IOS)` automatically selects
the Safari iOS profile.
[Sessions](sessions.md#platform-defaults-and-call-order) gives the platform
defaults and call order, and [Set the languages](sessions.md#set-the-languages)
sets `accept-language`.

## Match the platform

The platform sets `Sec-CH-UA-Platform`, `Sec-CH-UA-Mobile`, the `User-Agent`,
and the TCP fingerprint. [Choose a platform](sessions.md#choose-a-platform)
lists the platforms, including `Platform::Host`.

## Keep a saved session stable

| Strategy | Call | When to use it |
| --- | --- | --- |
| Always latest | `Browser::chrome()` / `Browser::default()` | Short-lived scrapers that should move to the newest Chrome when you upgrade Leyline |
| Pinned version | `Browser::Chrome158` | Accounts and repeatable tests. `err.is_profile_changed()` reports an edit to the bundled profile data |
| Frozen TOML | `.profile(&toml)` | Long-lived accounts that need the same profile data across upgrades. The browser version it claims grows old |

See [Save and restore a session](sessions.md#save-and-restore-a-session) and
[Profile data stability](profiles.md#profile-data-stability).

## Read the JA4 labels

JA3 and JA4 are hashes of the TLS handshake. Anti-bot web application
firewalls (WAFs) check them. The [profile reference](profiles.md) labels each
profile's offline JA4 test as `gated` or `estimated`:

- Every bundled profile, `gated` or `estimated`, was checked against the live
  echo server `tls.peet.ws`.
- A `gated` profile fixes its TLS extension order, so the offline test checks
  one exact handshake.
- An `estimated` profile shuffles its extension order on each connection, as
  Chrome and Brave do. The offline test cannot pin one handshake.
- JA4 sorts the extensions before it hashes them, so a live Chrome session
  still has one constant JA4. JA3 does not sort them, so it changes on each
  connection.

To verify a live session yourself, see
[Fingerprints](fingerprints.md#audit-a-session).
