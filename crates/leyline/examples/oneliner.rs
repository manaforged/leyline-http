#[tokio::main]
async fn main() -> leyline::Result<()> {
    let resp = leyline::get("https://httpbin.org/get").await?;
    println!("status: {}", resp.status());
    println!("{}", resp.text().await?);
    Ok(())
}
