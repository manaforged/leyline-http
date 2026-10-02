use leyline::Session;
use serde::Serialize;

const URL: &str = "https://example.com/users";

#[derive(Serialize)]
struct CreateUser {
    name: &'static str,
    age: u32,
}

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let session = Session::builder().build()?;

    let payload = CreateUser {
        name: "ada",
        age: 36,
    };

    let resp = session.post(URL).json(&payload).send().await?;

    println!("status: {}", resp.status());
    println!("body:   {}", resp.text().await?);
    Ok(())
}
