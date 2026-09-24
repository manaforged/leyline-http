#[tokio::main]
async fn main() {
    match leyline::Session::new()
        .get("https://tls.peet.ws/api/all")
        .await
    {
        Ok(mut resp) => {
            println!("Status: {}", resp.status());
            let text = resp.text().await.unwrap();
            if text.len() > 100 {
                println!("Body: {}... ({} bytes)", &text[..100], text.len());
            }
            println!("\nThat's it. One function call. No Session, no Builder, no config.");
        }
        Err(e) => eprintln!("Error: {e}"),
    }
}
