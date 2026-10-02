use std::time::Duration;

use leyline::{RetryPolicy, Session};

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "https://httpbin.org/status/503,503,200".to_string());

    let session = Session::builder().build()?;

    let policy = RetryPolicy::transient()
        .max_retries(4)
        .initial_backoff(Duration::from_millis(100))
        .max_backoff(Duration::from_secs(2));

    let resp = session
        .request(http::Method::GET, url)
        .retry(policy)
        .send()
        .await?;

    println!("final status: {}", resp.status());
    Ok(())
}
