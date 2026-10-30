The time limits of a request.

A `Duration` converts into a `total` limit, and the other limits keep their defaults. `Timeout::new()` sets each limit on its own.

```rust
use std::time::Duration;

use leyline::Timeout;

let timeout = Timeout::new()
    .connect(Duration::from_secs(5))
    .response_header(Duration::from_secs(10))
    .total(Duration::from_secs(60));
# drop(timeout);
```

| Limit | Default | Covers |
| --- | --- | --- |
| `total` | 300 s | One `send`: every redirect, retry, backoff, and buffered body read; for a `.stream()` response it ends at the response headers |
| `connect` | 10 s | One new connection: DNS, TCP, proxy, and the TLS or QUIC handshake |
| `response_header` | none | From dispatch to the response headers, per redirect |
| `read` | none | The longest gap between two body chunks, when the transport delivers the body in pieces |
| `body` | none | The whole body from its first read, when the transport delivers it in pieces; bounds `download_to` |
| `error_body` | 10 s | The body that `error_for_status` keeps in a status error |

# Errors

An expired limit returns [`Kind::Timeout`](crate::Kind::Timeout).

See also: [`Retry`](crate::Retry), [Retries and timeouts](https://manaforged.github.io/leyline-http/guide/retries-and-timeouts.html).
