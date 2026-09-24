use leyline::{Browser, Platform, Session};

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let session = Session::builder()
        .browser(Browser::Chrome147)
        .platform(Platform::Windows)
        .build()?;

    let page = session.get("https://example.com/").await?;
    println!("navigate: {}", page.status());

    let js = session
        .get("https://example.com/app.js")
        .preset(leyline::Preset::Script)
        .await?;
    println!(
        "script:   {} ({} bytes)",
        js.status(),
        js.bytes().await?.len()
    );

    let api = session
        .get("https://example.com/api/state")
        .preset(leyline::Preset::Xhr)
        .await?;
    println!("xhr get:  {}", api.status());

    let posted = session
        .post("https://example.com/api/items")
        .preset(leyline::Preset::Xhr)
        .body("payload=p%3D1")
        .await?;
    println!("xhr post: {}", posted.status());

    let created = session
        .post("https://example.com/users")
        .json(&serde_json::json!({ "name": "ada" }))
        .await?;
    println!("json:     {}", created.status());

    match api.error_for_status() {
        Ok(resp) => println!("ok:       {}", resp.status()),
        Err(e) if e.is_status() => println!("status:   {:?}", e.status()),
        Err(e) if e.is_timeout() => println!("timed out"),
        Err(e) if e.is_connect() => println!("connect failure: {e}"),
        Err(e) => return Err(e),
    }

    Ok(())
}
