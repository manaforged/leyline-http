Settings for a [`Session`](crate::Session), checked when you call [`build`](crate::SessionBuilder::build).

Get one from [`Session::builder`](crate::Session::builder), from [`Session::load`](crate::Session::load), or from [`Session::to_builder`](crate::Session::to_builder) to copy a running session. A setter that takes a config type accepts `impl Into<Config>` where a shorthand exists: a `Duration` is a timeout, a `u32` is a retry count, and a URL string is a proxy. `base_url` joins a relative request URL by the rules of `Url::join` (RFC 3986), so keep the trailing slash: `https://api.example/v1/` with `items` gives `/v1/items`, and `/items` replaces the path.

```rust,no_run
use std::time::Duration;

use leyline::{Browser, Protocol, Session};

# fn run() -> leyline::Result<()> {
let session = Session::builder()
    .browser(Browser::Chrome154)
    .protocol(Protocol::Http2)
    .timeout(Duration::from_secs(20))
    .retry(2)
    .proxy("http://proxy.example:8080")
    .build()?;
# drop(session);
# Ok(())
# }
```

# Errors

[`build`](crate::SessionBuilder::build) returns [`Kind::Config`](crate::Kind::Config) for an invalid setting, such as a proxy URL that does not parse or a browser with no data for the platform.

See also: [`Session`](crate::Session), [Sessions](https://manaforged.github.io/leyline-http/guide/sessions.html).
