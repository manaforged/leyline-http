use serde_json::{Value, json};
use std::time::Duration;
use wreq::Client;

#[tokio::main]
async fn main() -> wreq::Result<()> {
    let client = Client::new();
    let body = json!({"name": "ada", "age": 36});
    let response = client
        .post("http://127.0.0.1:18743/t02/users")
        .json(&body)
        .timeout(Duration::from_secs(5))
        .send()
        .await?;
    let status = response.status().as_u16();
    let created: Value = response.json().await?;
    println!("{status} {}", created["id"]);
    let slow = client
        .post("http://127.0.0.1:18743/t02/slow")
        .json(&body)
        .timeout(Duration::from_millis(300))
        .send()
        .await;
    match slow {
        Err(error) if error.is_timeout() => println!("timeout"),
        Err(error) => return Err(error),
        Ok(_) => println!("no timeout"),
    }
    Ok(())
}
