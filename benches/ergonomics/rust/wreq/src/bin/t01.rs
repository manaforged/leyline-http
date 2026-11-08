use serde_json::Value;

#[tokio::main]
async fn main() -> wreq::Result<()> {
    let user: Value = wreq::get("http://127.0.0.1:18743/t01/user")
        .send()
        .await?
        .json()
        .await?;
    println!("{} {}", user["id"], user["name"].as_str().unwrap());
    Ok(())
}
