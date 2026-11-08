use serde_json::Value;
use wreq::Proxy;

#[tokio::main]
async fn main() -> wreq::Result<()> {
    let client = wreq::Client::builder()
        .proxy(Proxy::http("http://127.0.0.1:18743")?)
        .build()?;
    let response = client
        .get("http://ergonomics.invalid/t09/hello")
        .send()
        .await?;
    let status = response.status().as_u16();
    let body: Value = response.json().await?;
    println!("{status} {}", body["via"].as_str().unwrap());
    Ok(())
}
