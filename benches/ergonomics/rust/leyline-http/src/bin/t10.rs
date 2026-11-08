#[tokio::main]
async fn main() -> leyline::Result<()> {
    let session = leyline::Session::builder().redirect(5).build()?;
    let response = session
        .get("http://127.0.0.1:18743/t10/start")
        .send()
        .await?;
    println!("{}", response.url());
    let looping = session
        .get("http://127.0.0.1:18743/t10/loop/0")
        .send()
        .await?;
    if looping.status().is_redirection() {
        println!("capped");
    }
    Ok(())
}
