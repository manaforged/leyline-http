When and how often a request is sent again.

A `u32` converts into a retry count with the default rules: an error with [`Error::is_retryable`](crate::Error::is_retryable), or status 429, 502, 503, or 504, is retried for idempotent methods, and for any method when the request was never sent. `retry(3)` sends the first try and up to 3 retries. The wait backs off from 100 ms to 1 s, and a `Retry-After` header is honored up to `max_wait`, 60 s by default.

```rust,no_run
use std::time::Duration;

use leyline::{Retry, Session};

# fn run() -> leyline::Result<()> {
let retry = Retry::new(4)
    .backoff(Duration::from_millis(200), Duration::from_secs(5))
    .max_wait(Duration::from_secs(30));
let session = Session::builder().retry(retry).build()?;
# drop(session);
# Ok(())
# }
```

When the retries run out on a retryable status, `send` returns the last response; with `error_for_status`, it returns a status error whose [`Error::retries_exhausted`](crate::Error::retries_exhausted) is true. An error on the last try is returned as it is.

See also: [`Timeout`](crate::Timeout), [Retries and timeouts](https://manaforged.github.io/leyline-http/guide/retries-and-timeouts.html).
