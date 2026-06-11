# Leyline

An HTTP client for Rust. Plain and honest by default — a `leyline/<version>`
client on the host OS, for ordinary API calls. Ask for a browser and it
matches that browser's TLS, HTTP/2, and TCP shape on the wire: Chrome,
Firefox, Safari, OkHttp. That's the path for servers that fingerprint the
client and block a plain one.

Status: `1.0.0-alpha.1`. The API still moves; pin exact versions.

## Example

```toml
[dependencies]
leyline = "1.0.0-alpha.1"
serde_json = "1"
tokio = { version = "1", features = ["full"] }
```

```rust,no_run
use leyline::Session;

#[tokio::main]
async fn main() -> leyline::Result<()> {
    // Bare by default — no browser fingerprint.
    let resp = leyline::get("https://api.example.com/v1").await?;
    println!("{}", resp.status());

    // Browser parity when you want it. Constructors are infallible.
    let session = Session::chrome();
    let resp = session
        .post("https://example.com/api")
        .json(&serde_json::json!({ "hello": "world" }))
        .bearer_auth("token")
        .send()
        .await?
        .error_for_status()?;
    println!("{}", resp.status());
    Ok(())
}
```

MSRV: Rust 1.85.

## Build dependencies

Leyline transitively builds BoringSSL from source on the first `cargo
build` (~5–15 min, then cached). You will need a C/C++ toolchain plus
CMake and Perl.

- **Linux (Debian / Ubuntu):**
  `sudo apt-get install build-essential cmake perl pkg-config libclang-dev musl-tools git`
- **macOS:** `xcode-select --install` and `brew install cmake`
- **Windows MSVC:** Visual Studio Build Tools + Strawberry Perl
  (`choco install strawberryperl`)

If you already have a built BoringSSL tree, point `BORING_BSSL_PATH` at it
to skip the source compile.

## Profiles

| Profile          | Versions           |
| ---------------- | ------------------ |
| Chrome           | 145, 146, 147, 148 |
| Aloha            | 138                |
| Brave            | 146                |
| Firefox          | 148, 150, 151      |
| Safari (macOS)   | 18                 |
| Safari (iOS)     | 15, 17, 18         |
| OkHttp (Android) | 7, 10              |

The default `Session::chrome()` tracks the newest Chrome here (148).

## More

Full README, examples, contributing guide, and changelog at
[github.com/manaforged/leyline-http](https://github.com/manaforged/leyline-http).

## License

Dual MIT / Apache-2.0.
