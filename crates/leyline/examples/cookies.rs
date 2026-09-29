use leyline::cookie::Jar;
use leyline::{Browser, Platform, Session};

const URL_SET: &str = "https://example.com/login";
const URL_READ: &str = "https://example.com/me";

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let jar = Jar::new();

    let session = Session::builder()
        .browser(Browser::Chrome147)
        .platform(Platform::Linux)
        .cookie_jar(jar)
        .build()?;

    drop(session.get(URL_SET).await?);

    let resp = session.get(URL_READ).await?;
    println!("status: {}", resp.status());

    Ok(())
}
