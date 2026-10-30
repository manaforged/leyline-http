#[tokio::main]
async fn main() -> leyline::Result<()> {
    let resp = leyline::Session::new()
        .get("https://httpbin.org/get")
        .send()
        .await?;
    println!("status: {}", resp.status());
    println!("{}", resp.text().await?);
    Ok(())
}
