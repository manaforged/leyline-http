The one error type of the crate.

[`Error::kind`](crate::Error::kind) returns a [`Kind`](crate::Kind). Check `kind()` or a predicate such as [`Error::is_retryable`](crate::Error::is_retryable); the message text can change between releases.

```rust,no_run
use leyline::{Kind, Session};

# async fn run() {
match Session::new().get("https://example.com/").send().await {
    Ok(resp) => println!("{}", resp.status()),
    Err(err) if err.kind() == Kind::Timeout => println!("timed out"),
    Err(err) => println!("{err}"),
}
# }
```

`multipart::Part::file` returns `std::io::Result`, which has no `Kind`.

See also: [Errors](https://manaforged.github.io/leyline-http/guide/errors.html).
