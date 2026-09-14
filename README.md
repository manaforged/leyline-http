# Leyline

HTTP client that mimics browsers on the wire.

## Requirements

Rust 1.98 or later and Tokio. Prebuilt BoringSSL libraries and Rust bindings
are included for:

- `aarch64-apple-darwin`
- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`
- `x86_64-pc-windows-msvc`

Other targets require porting both the native libraries and the Rust bindings.
Setting `BORING_BSSL_PATH` alone does not add target support.

## Install

The first crates.io release, `0.1.0`, is in preparation. Until it is published,
use the repository:

```toml
[dependencies]
leyline-http = { git = "https://github.com/manaforged/leyline-http", branch = "main" }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

The package name is `leyline-http`; the Rust import is `leyline`. The crate
family has five packages. A registry install becomes possible once they are
published in this order: `leyline-bssl-sys`, `leyline-bssl`, then
`leyline-bssl-tokio` and `leyline-quiche`, then `leyline-http`. A Git
dependency also checks out the vendored BoringSSL source submodule (about
500 MB); the published packages use the included prebuilt libraries and do
not need it.

## Usage

```rust,no_run
use leyline::Session;

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let session = Session::chrome();
    let mut response = session.get("https://example.com/").await?;
    println!("{}", response.text().await?);
    Ok(())
}
```

Reuse a session to share connections and cookies. `Session::chrome()` selects
the latest bundled Chrome profile with a Windows identity. Use the builder to
select another browser or platform.

## Limits

Profiles cover selected TLS, HTTP/2, and HTTP header properties. Capture
status and known gaps are listed in the
[profile reference](https://github.com/manaforged/leyline-http/blob/main/crates/leyline/docs/PROFILES.md).
Safari 26 uses a WKWebView capture with a synthesized Safari HTTP identity.

HTTP/3 proxy support is not implemented. The HTTP/3 QPACK decoder uses no
dynamic table; that differs from Chromium's QPACK parameters and is a known
protocol fingerprint difference. Opt-in `response.audit()` values describe
the configured profile and request; they are not packet captures.

## Docs

Read the [user guide](https://github.com/manaforged/leyline-http/blob/main/crates/leyline/docs/guide/README.md)
for sessions, requests, streaming, retries, proxies, and fingerprints.
The [changelog](https://github.com/manaforged/leyline-http/blob/main/CHANGELOG.md)
states the version policy.

## License

Leyline is [MIT licensed](https://github.com/manaforged/leyline-http/blob/main/LICENSE).
See [NOTICE](https://github.com/manaforged/leyline-http/blob/main/NOTICE) for the
BoringSSL and quiche forks and their licenses.
