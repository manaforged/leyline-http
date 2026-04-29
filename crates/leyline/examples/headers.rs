//! Set custom headers and an `Authorization: Bearer` token on a request.
//!
//! Run with: `cargo run -p leyline --example headers`

use leyline::Client;

const URL: &str = "https://example.com/protected";

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let session = Client::chrome()?;

    let resp = session
        .get(URL)
        .headers([("x-request-id", "ex-001")])
        .accept_language("en-US,en;q=0.9")
        .referer("https://example.com/")
        .bearer_auth("REPLACE_WITH_YOUR_TOKEN")
        .send()
        .await?;

    println!("status: {}", resp.status());
    Ok(())
}
