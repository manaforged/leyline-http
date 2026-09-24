use leyline::profile::Browser;
use leyline::{ProtocolPolicy, Session};

const URL: &str = "https://cloudflare.com";

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let session = Session::builder()
        .browser(Browser::default())
        .protocol(ProtocolPolicy::Http3)
        .build()?;

    let resp = session.get(URL).await?;
    println!("status: {}", resp.status());
    Ok(())
}
