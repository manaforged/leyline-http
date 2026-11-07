#[tokio::main]
async fn main() -> leyline::Result<()> {
    let length = leyline::get("http://127.0.0.1:18743/t06/large")
        .retry(5)
        .download_to("large.bin")
        .await?;
    println!("{length}");
    Ok(())
}
