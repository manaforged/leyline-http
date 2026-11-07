use serde_json::{Value, json};

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let session = leyline::Session::new();
    session
        .post("http://localhost:18743/t03/login")
        .json(&json!({"user": "ada", "password": "secret"}))
        .send()
        .await?;
    let me: Value = session
        .get("http://localhost:18743/t03/me")
        .send()
        .await?
        .json()
        .await?;
    println!("{}", me["user"].as_str().unwrap());
    Ok(())
}
