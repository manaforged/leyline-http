//! Persist cookies across requests with a `Jar`.
//!
//! Run with: `cargo run -p leyline --example cookies`
//!
//! Swap `URL_SET` and `URL_READ` for endpoints that actually round-trip a
//! `Set-Cookie` header (e.g. a local test server). example.com will not
//! echo cookies back.

use leyline::cookie::Jar;
use leyline::{Browser, Platform, Session};

const URL_SET: &str = "https://example.com/login";
const URL_READ: &str = "https://example.com/me";

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let jar = Jar::new();

    let session = Session::builder()
        .browser(Browser::Chrome147)
        .platform(Platform::Linux)
        .cookie_jar(jar)
        .build()?;

    // First request: server may set cookies on the jar.
    let _ = session.get(URL_SET).send().await?;

    // Second request: any cookies the jar captured are sent automatically.
    let resp = session.get(URL_READ).send().await?;
    println!("status: {}", resp.status());

    Ok(())
}
