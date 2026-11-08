use serde_json::{Value, json};

#[tokio::main]
async fn main() -> wreq::Result<()> {
    let client = wreq::Client::builder().cookie_store(true).build()?;
    client
        .post("http://localhost:18743/t03/login")
        .json(&json!({"user": "ada", "password": "secret"}))
        .send()
        .await?;
    let me: Value = client
        .get("http://localhost:18743/t03/me")
        .send()
        .await?
        .json()
        .await?;
    println!("{}", me["user"].as_str().unwrap());
    Ok(())
}
