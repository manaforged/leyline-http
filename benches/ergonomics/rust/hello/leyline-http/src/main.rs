#[tokio::main]
async fn main() -> leyline::Result<()> {
    let response = leyline::Session::new()
        .get("http://127.0.0.1:18743/t01/user")
        .send()
        .await?;
    println!("{}", response.status());
    Ok(())
}
