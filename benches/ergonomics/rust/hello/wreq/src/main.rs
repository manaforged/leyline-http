#[tokio::main]
async fn main() -> wreq::Result<()> {
    let response = wreq::get("http://127.0.0.1:18743/t01/user").send().await?;
    println!("{}", response.status());
    Ok(())
}
