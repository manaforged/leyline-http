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

With the `http3` feature, bundled desktop and iOS 18+ browser profiles race
HTTP/3 against HTTP/2 for origins that advertise `h3`. See [HTTP/3](http3.md).

## Pick another browser

| You need | Use |
| --- | --- |
| Firefox | `.browser(Browser::firefox())` |
| Safari on macOS | `.browser(Browser::Safari26).platform(Platform::MacOS)` |
| Safari on iPhone | `.browser(Browser::safari_ios())`. It needs no platform: the session claims iOS |
| Edge or Opera | `.browser(Browser::edge())` or `.browser(Browser::opera())`, desktop only. See [Sessions](sessions.md#pick-edge-or-opera) |
| Brave | `.browser(Browser::Brave158).platform(Platform::MacOS)` |
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
and the TCP fingerprint. `Platform::Host` uses the OS the program runs on.

## Keep a saved session stable

| Strategy | Call | When to use it |
| --- | --- | --- |
| Always latest | `Browser::chrome()` / `Browser::default()` | Ephemeral scrapers that should automatically advance to newer Chrome releases on upgrade |
| Pinned version | `Browser::Chrome158` | Accounts and reproducible tests (detects bundled-profile data edits via `profile_id()` / `err.is_profile_changed()`) |
| Frozen TOML | `.profile(&toml)` | Long-lived accounts that must keep byte-identical profile data across upgrades (note: frozen profiles eventually age) |

See [Save and restore a session](sessions.md#save-and-restore-a-session) and
[Profile data stability](profiles.md#profile-data-stability).

## Read the JA4 labels

JA3 and JA4 are TLS handshake hashes that anti-bot WAFs check. The
[profile reference](profiles.md) labels each profile's offline JA4 test as
`gated` or `estimated`:

- Both `gated` and `estimated` profiles are verified against live echo servers (`tls.peet.ws`).
- Chrome and Brave are marked `estimated` only because Chrome randomizes TLS extension order on every connection (so an offline static hash can only estimate one ordering). Because JA4 sorts extensions before hashing, a live Chrome session still produces one constant JA4 (while JA3 changes per connection).

To verify a live session yourself, see
[Fingerprints](fingerprints.md#audit-a-session).
