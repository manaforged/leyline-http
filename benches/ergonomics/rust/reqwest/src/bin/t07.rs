use reqwest::multipart::Form;
use serde_json::Value;
use std::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let form = Form::new()
        .text("note", "hello")
        .file("file", "upload.bin")
        .await?;
    let response = reqwest::Client::new()
        .post("http://127.0.0.1:18743/t07/upload")
        .multipart(form)
        .send()
        .await?;
    let status = response.status().as_u16();
    let body: Value = response.json().await?;
    println!(
        "{status} {} {}",
        body["size"],
        body["note"].as_str().unwrap()
    );
    Ok(())
}
