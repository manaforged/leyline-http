use leyline::Session;

const URL: &str = "https://cloudflare.com";

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let session = Session::builder().chrome().http3().build()?;

    let resp = session.get(URL).await?;
    println!("status: {}", resp.status());
    Ok(())
}
