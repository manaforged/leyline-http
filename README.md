# leyline-tls

Async HTTP client for Rust. Bare by default. `Session::chrome()` matches
Chrome on TLS, HTTP/2, and HTTP/3.

The crate is `leyline`. MSRV 1.86. Four targets ship prebuilt BoringSSL
(linux x64/arm64, windows x64, mac arm64).

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

Matching a captured browser on the wire is the contract. Beating every
classifier is not.

MIT OR Apache-2.0. Notices in [NOTICE](NOTICE).
