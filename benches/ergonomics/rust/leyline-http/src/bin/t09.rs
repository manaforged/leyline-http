use serde_json::Value;

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let response = leyline::Session::new()
        .get("http://ergonomics.invalid/t09/hello")
        .proxy("http://127.0.0.1:18743")
        .send()
        .await?;
    let status = response.status().as_u16();
    let body: Value = response.json().await?;
    println!("{status} {}", body["via"].as_str().unwrap());
    Ok(())
}
