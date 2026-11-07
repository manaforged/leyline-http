use serde_json::Value;

#[tokio::main]
async fn main() -> reqwest::Result<()> {
    let user: Value = reqwest::get("http://127.0.0.1:18743/t01/user")
        .await?
        .json()
        .await?;
    println!("{} {}", user["id"], user["name"].as_str().unwrap());
    Ok(())
}
