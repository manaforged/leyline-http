use serde_json::Value;
use std::error::Error;
use std::time::Duration;

const URL: &str = "http://127.0.0.1:18743/t04/flaky";

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let client = wreq::Client::new();
    let mut response = client.get(URL).send().await?;
    let mut backoff = Duration::from_millis(100);
    for _ in 0..2 {
        if response.status() != 503 {
            break;
        }
        let wait = match response.headers().get("retry-after") {
            Some(seconds) => Duration::from_secs(seconds.to_str()?.parse()?),
            None => backoff,
        };
        tokio::time::sleep(wait).await;
        backoff *= 2;
        response = client.get(URL).send().await?;
    }
    let status = response.status().as_u16();
    let body: Value = response.json().await?;
    println!("{status} {}", body["attempts"]);
    Ok(())
}
