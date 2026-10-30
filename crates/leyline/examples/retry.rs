use std::time::Duration;

use leyline::{Retry, Session};

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "https://httpbin.org/status/503,503,200".to_string());

    let session = Session::builder().build()?;

    let policy = Retry::new(4).backoff(Duration::from_millis(100), Duration::from_secs(2));

    let resp = session
        .request(http::Method::GET, url)
        .retry(policy)
        .send()
        .await?;

    println!("final status: {}", resp.status());
    Ok(())
}
