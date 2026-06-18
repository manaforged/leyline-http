//! Browser-shaped request presets — one helper per fetch context.
//!
//! Run with: `cargo run -p leyline --example presets`

use leyline::{Browser, Platform, Session};

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let session = Session::builder()
        .browser(Browser::Chrome147)
        .platform(Platform::Windows)
        .build()?;

    // Top-level document fetch (Sec-Fetch-Mode: navigate).
    let page = session.navigate("https://example.com/").await?;
    println!("navigate: {}", page.status());

    // Sub-resource load, as a `<script src=…>` tag (no-cors, dest script).
    let js = session.get_script("https://example.com/app.js").await?;
    println!("script:   {} ({} bytes)", js.status(), js.bytes().len());

    // fetch()/XHR GET (cors, empty dest).
    let api = session.get_xhr("https://example.com/api/state").await?;
    println!("xhr get:  {}", api.status());

    // fetch()/XHR POST with a raw, non-JSON body (e.g. a telemetry beacon).
    let beacon = session
        .post_xhr("https://example.com/collect", "payload=p%3D1")
        .await?;
    println!("xhr post: {}", posted.status());

    // fetch()/XHR POST with a JSON body.
    let created = session
        .post_json(
            "https://example.com/users",
            &serde_json::json!({ "name": "ada" }),
        )
        .await?;
    println!("json:     {}", created.status());

    // Error classification, reqwest-style.
    match api.error_for_status() {
        Ok(resp) => println!("ok:       {}", resp.status()),
        Err(e) if e.is_status() => println!("status:   {:?}", e.status()),
        Err(e) if e.is_timeout() => println!("timed out"),
        Err(e) if e.is_connect() => println!("connect failure: {e}"),
        Err(e) => return Err(e),
    }

    Ok(())
}
