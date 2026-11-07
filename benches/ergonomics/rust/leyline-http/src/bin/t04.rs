use serde_json::Value;

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let response = leyline::Session::new()
        .get("http://127.0.0.1:18743/t04/flaky")
        .retry(2)
        .send()
        .await?;
    let status = response.status().as_u16();
    let body: Value = response.json().await?;
    println!("{status} {}", body["attempts"]);
    Ok(())
}
