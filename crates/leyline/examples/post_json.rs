//! POST a JSON body.

use leyline::{Browser, Platform, Session};
use serde::Serialize;

const URL: &str = "https://example.com/users";

#[derive(Serialize)]
struct CreateUser {
    name: &'static str,
    age: u32,
}

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let session = Session::builder()
        .browser(Browser::Chrome147)
        .platform(Platform::Linux)
        .build()?;

    let payload = CreateUser {
        name: "ada",
        age: 36,
    };

    let resp = session.post(URL).json(&payload).send().await?;

    println!("status: {}", resp.status());
    println!("body:   {}", resp.text().unwrap());
    Ok(())
}
