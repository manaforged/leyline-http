use wreq::redirect::Policy;

#[tokio::main]
async fn main() -> wreq::Result<()> {
    let client = wreq::Client::builder()
        .redirect(Policy::limited(5))
        .build()?;
    let response = client
        .get("http://127.0.0.1:18743/t10/start")
        .send()
        .await?;
    println!("{}", response.uri());
    match client.get("http://127.0.0.1:18743/t10/loop/0").send().await {
        Err(error) if error.is_redirect() => println!("capped"),
        Err(error) => return Err(error),
        Ok(_) => println!("uncapped"),
    }
    Ok(())
}
