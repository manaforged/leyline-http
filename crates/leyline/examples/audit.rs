//! Read the per-connection wire-fingerprint audit block off a response.
//!
//! Run with: `cargo run -p leyline --example audit`
//!
//! `AuditData` carries the JA3, JA4, JA4T, JA4H, and HTTP/2 Akamai
//! fingerprint that leyline actually negotiated with the server. Use it
//! to verify the wire shape matches the browser profile you picked.

use leyline::{Browser, Platform, Session};

const URL: &str = "https://example.com";

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let session = Session::builder()
        .browser(Browser::Chrome147)
        .platform(Platform::Linux)
        .build()?;

    let resp = session.get(URL).send().await?;

    if let Some(audit) = resp.audit() {
        println!("JA3:            {}", audit.ja3);
        println!("JA4:            {}", audit.ja4);
        println!("JA4T:           {}", audit.ja4t);
        println!("JA4H:           {}", audit.ja4h);
        println!("H2 fingerprint: {}", audit.h2_fingerprint);
    } else {
        eprintln!("no audit data on this response");
    }

    Ok(())
}
