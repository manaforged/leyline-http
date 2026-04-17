//! Retry with exponential backoff on transient server errors.
//!
//! Run: `cargo run --example retry -- https://httpbin.org/status/503,200`
//! (httpbin's multi-status endpoint answers the requested codes in sequence;
//! exact flakiness depends on what the server echoes.)

use std::time::Duration;

use leyline::{Browser, RetryPolicy, Session};

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "https://httpbin.org/status/503,503,200".to_string());

    let session = Session::builder().browser(Browser::Chrome147).build()?;

    let policy = RetryPolicy::default()
        .with_max_retries(4)
        .with_backoff(Duration::from_millis(100), Duration::from_secs(2));

    let resp = session.get(&url).retry(policy).send().await?;

    println!("final status: {}", resp.status());
    Ok(())
}
