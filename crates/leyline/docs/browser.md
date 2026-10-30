A bundled browser profile, one variant per captured browser version.

A `Browser` builds a session on its usual platform, so `Session::from(Browser::Chrome154)` is a complete browser session. One function per product returns its newest build: [`Browser::chrome`](crate::Browser::chrome), `edge`, `opera`, `brave`, `firefox`, `safari`, `safari_ios`, `cfnetwork`, and `okhttp`. `Browser::default()` equals `Browser::chrome()`, and a release can move these to newer captures. Edge and Opera are variants such as `Browser::Edge154` and `Browser::Opera139`, built on the Chrome captures for Windows, macOS, and Linux. Pin a variant such as `Browser::Chrome154`; a release changes a pinned variant only to correct its capture. The [profile reference](https://manaforged.github.io/leyline-http/guide/profiles.html#bundled-profiles) lists every variant and its capture.

```rust
use leyline::{Browser, Platform};

let pinned = Browser::Chrome154;
let newest = Browser::firefox();
assert_eq!("edge-154".parse::<Browser>().ok(), Some(Browser::Edge154));
assert_eq!(Browser::Opera139.version(), 139);
# drop(newest);
let on_mac = pinned.for_platform(Platform::MacOS);
# drop(on_mac);
```

See also: [`SessionBuilder::browser`](crate::SessionBuilder::browser), [`Platform`](crate::Platform), [Choose a browser](https://manaforged.github.io/leyline-http/guide/sessions.html#choose-a-browser).
