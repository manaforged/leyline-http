use leyline::Session;
use serde_json::{Value, json};
use std::time::Duration;

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let session = Session::new();
    let body = json!({"name": "ada", "age": 36});
    let response = session
        .post("http://127.0.0.1:18743/t02/users")
        .json(&body)
        .timeout(Duration::from_secs(5))
        .send()
        .await?;
    let status = response.status().as_u16();
    let created: Value = response.json().await?;
    println!("{status} {}", created["id"]);
    let slow = session
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
