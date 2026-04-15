//! Manual HTTP/3 sanity check against public QUIC endpoints.

use leyline_profile::{Browser, ProfileRegistry};
use leyline_quic::{H3Config, H3Connection};

#[tokio::main]
async fn main() {
    println!("=== HTTP/3 QUIC Test ===\n");

    let reg = ProfileRegistry::builtin();
    let profile = reg.get_browser(Browser::Chrome147).unwrap();
    let config = H3Config::chrome();

    // Cloudflare supports HTTP/3
    println!("1. GET https://cloudflare-quic.com/ via H3...");
    match H3Connection::request(
        &config,
        profile,
        "GET",
        "cloudflare-quic.com",
        443,
        "/",
        vec![],
        None,
    )
    .await
    {
        Ok(resp) => {
            println!("   Status: {}", resp.status);
            println!("   Body: {} bytes", resp.body.len());
            println!("   ✓ HTTP/3 works!");
        }
        Err(e) => println!("   ✗ {e}"),
    }

    // Google also supports HTTP/3
    println!("\n2. GET https://www.google.com/ via H3...");
    match H3Connection::request(
        &config,
        profile,
        "GET",
        "www.google.com",
        443,
        "/",
        vec![("accept".into(), "text/html".into())],
        None,
    )
    .await
    {
        Ok(resp) => {
            println!("   Status: {}", resp.status);
            println!("   Body: {} bytes", resp.body.len());
            println!("   ✓ HTTP/3 works!");
        }
        Err(e) => println!("   ✗ {e}"),
    }
}
