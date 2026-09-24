use leyline::Session;

const URL: &str = "https://example.com/protected";

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let session = Session::new();

    let resp = session
        .get(URL)
        .headers([("x-request-id", "ex-001")])
        .header("accept-language", "en-US,en;q=0.9")
        .header("referer", "https://example.com/")
        .bearer_auth("REPLACE_WITH_YOUR_TOKEN")
        .send()
        .await?;

    println!("status: {}", resp.status());
    Ok(())
}
