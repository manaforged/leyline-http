# leyline

Async HTTP client. Bare by default. `Session::chrome()` matches Chrome
on TLS, HTTP/2, and HTTP/3.

```toml
[dependencies]
leyline = { git = "https://github.com/manaforged/leyline-http", rev = "<commit-sha>" }
tokio = { version = "1", features = ["full"] }
```

```rust,no_run
use leyline::Session;

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let resp = leyline::get("https://api.example.com/v1").await?;
    let page = Session::chrome().get("https://example.com").await?;
    println!("{} {}", resp.status(), page.status());
    Ok(())
}
```

MSRV 1.86. MIT OR Apache-2.0.
