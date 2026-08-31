# leyline

HTTP client that mimics browsers on the wire.

crates.io package is `leyline-http` (`leyline` is a different crate). Import `leyline`. Rust 1.86. First crates.io version is **0.1.0** because the bundled BoringSSL crate is still `1.0.0-alpha.3`.

Bundled: Chrome 152, Firefox 154, Safari 26 (WKWebView TLS/H2; HTTP identity is synthesized when the dump omits `Safari/`). Edge is a brand overlay on Chrome (same ClientHello on Windows and macOS; UA platform tokens differ). `Session::chrome()` races H3 against H2 when `http3` is on. That is leyline, not Chrome Alt-Svc. HTTP/3 QPACK advertises a 0-capacity table; the decoder has no dynamic table. MIT. `leyline-bssl` is Apache-2.0. `leyline-bssl-tokio` is MIT OR Apache-2.0. BoringSSL and quiche licenses live in [NOTICE](NOTICE).

```toml
[dependencies]
leyline-http = "0.1"
tokio = { version = "1", features = ["full"] }
```

```rust,no_run
use leyline::Session;

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let session = Session::builder()
        .chrome()
        .proxy("http://127.0.0.1:8080")
        .build()?;
    let resp = session
        .post("https://example.com/api")
        .header("x-request-id", "1")
        .body(r#"{"ok":true}"#)
        .await?;
    println!("{}", resp.status());
    Ok(())
}
```
