//! The simplest possible Leyline usage.

#[tokio::main]
async fn main() {
    match leyline::get("https://tls.peet.ws/api/all").await {
        Ok(resp) => {
            println!("Status: {}", resp.status());
            // Parse the fingerprint from the response
            let text = resp.text();
            if text.len() > 100 {
                println!("Body: {}... ({} bytes)", &text[..100], text.len());
            }
            println!("\nThat's it. One function call. No Session, no Builder, no config.");
        }
        Err(e) => eprintln!("Error: {e}"),
    }
}
