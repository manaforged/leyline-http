A client that sends requests as one browser, with one connection pool and one cookie jar.

[`Session::new`](crate::Session::new) gives a plain client and `Session::from(browser)` a browser one; neither can fail. [`Session::builder`](crate::Session::builder) returns a [`SessionBuilder`](crate::SessionBuilder) for everything else. A clone shares the connections, the cookies, and the browser, so clone a session rather than build a second one.

```rust,no_run
use std::time::Duration;

use leyline::{Browser, Session};

# async fn run() -> leyline::Result<()> {
let session = Session::builder()
    .browser(Browser::Chrome154)
    .timeout(Duration::from_secs(30))
    .build()?;
let page = session.get("https://example.com/").send().await?.text().await?;
session.save("session.json")?;
# drop(page);
# Ok(())
# }
```

# Errors

[`SessionBuilder::build`](crate::SessionBuilder::build) returns [`Kind::Config`](crate::Kind::Config) for an invalid setting. [`Session::load`](crate::Session::load) returns [`Kind::Io`](crate::Kind::Io) or [`Kind::Json`](crate::Kind::Json) for a file it cannot read, and [`Kind::Config`](crate::Kind::Config) for an invalid saved profile or a version 1 file whose proxy password variable is unset, and its `build()` returns [`Kind::Config`](crate::Kind::Config) with [`Error::is_profile_changed`](crate::Error::is_profile_changed) when the saved profile no longer matches.

See also: [`RequestBuilder`](crate::RequestBuilder), [`SessionBuilder`](crate::SessionBuilder), [Sessions](https://manaforged.github.io/leyline-http/guide/sessions.html).
