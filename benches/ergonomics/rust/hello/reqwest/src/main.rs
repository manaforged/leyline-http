#[tokio::main]
async fn main() -> reqwest::Result<()> {
    let response = reqwest::get("http://127.0.0.1:18743/t01/user").await?;
    println!("{}", response.status());
    Ok(())
}
