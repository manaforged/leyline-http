The status, headers, and body of one response.

The body is read once, decoded, and capped by `max_body_size`. Each reader consumes the response. For a large file, send with `.stream()` so the body is not buffered, call `download_to`, and bound it with `Timeout::body`; `total` ends at the response headers of a streamed response.

```rust,no_run
use leyline::Session;

# async fn run() -> leyline::Result<()> {
let resp = Session::new().get("https://example.com/big.iso").stream().send().await?;
println!("{} {:?}", resp.status(), resp.header("content-type"));
let written = resp.download_to("big.iso").await?;
# drop(written);
# Ok(())
# }
```

# Errors

A reader returns [`Kind::BodyLimit`](crate::Kind::BodyLimit) over the cap, [`Kind::Decode`](crate::Kind::Decode) for a bad encoding, [`Kind::Json`](crate::Kind::Json) for invalid JSON, and [`Kind::Timeout`](crate::Kind::Timeout) when the body timeout expires.

See also: [`RequestBuilder`](crate::RequestBuilder), [Responses](https://manaforged.github.io/leyline-http/guide/responses.html), [Streaming](https://manaforged.github.io/leyline-http/guide/streaming.html).
