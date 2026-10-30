The operating system a session reports: `Windows`, `MacOS`, `Linux`, `Android`, `IOS`, or `Host` for the machine the code runs on.

The platform sets the platform headers and the TCP profile. A browser profile covers some platforms; [`SessionBuilder::build`](crate::SessionBuilder::build) returns [`Kind::Config`](crate::Kind::Config) for one it does not cover.

```rust
use leyline::{Browser, Platform, Session};

let session = Session::builder()
    .browser(Browser::Chrome154)
    .platform(Platform::Linux)
    .build()
    .unwrap();
assert_eq!(session.platform(), Platform::Linux);
```

See also: [`SessionBuilder::platform`](crate::SessionBuilder::platform), [Supported platforms](https://manaforged.github.io/leyline-http/guide/platforms.html).
