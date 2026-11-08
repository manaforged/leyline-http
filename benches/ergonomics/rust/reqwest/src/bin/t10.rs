use reqwest::redirect::Policy;

#[tokio::main]
async fn main() -> reqwest::Result<()> {
    let client = reqwest::Client::builder()
        .redirect(Policy::limited(5))
        .build()?;
    let response = client
        .get("http://127.0.0.1:18743/t10/start")
        .send()
        .await?;
    println!("{}", response.url());
    match client.get("http://127.0.0.1:18743/t10/loop/0").send().await {
        Err(error) if error.is_redirect() => println!("capped"),
        Err(error) => return Err(error),
        Ok(_) => println!("uncapped"),
    }
    Ok(())
}
