//! Send a request over HTTP/3 (QUIC).
//!
//! Run with: `cargo run -p leyline --example http3`
//!
//! Swap `URL` for an endpoint you know speaks HTTP/3. example.com does
//! not advertise HTTP/3 at the time of writing.

use leyline::{Browser, Platform, Session};

const URL: &str = "https://example.com";

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let session = Session::builder()
        .browser(Browser::Chrome147)
        .platform(Platform::Linux)
        .http3()
        .build()?;

    let resp = session.get(URL).send().await?;
    println!("status: {}", resp.status());
    Ok(())
}
