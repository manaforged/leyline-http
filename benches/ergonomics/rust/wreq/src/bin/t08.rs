use futures::StreamExt;
use std::error::Error;
use std::time::Duration;

const URL: &str = "http://127.0.0.1:18743/t08/events";

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let client = wreq::Client::new();
    let mut last_id = String::new();
    loop {
        let mut request = client.get(URL).header("Accept", "text/event-stream");
        if !last_id.is_empty() {
            request = request.header("Last-Event-ID", &last_id);
        }
        let mut body = Box::pin(request.send().await?.bytes_stream());
        let mut buffer = String::new();
        while let Some(Ok(chunk)) = body.next().await {
            buffer.push_str(std::str::from_utf8(&chunk)?);
            while let Some(end) = buffer.find("\n\n") {
                let (mut id, mut event, mut data) = ("", "", "");
                for line in buffer[..end].lines() {
                    match line.split_once(": ") {
                        Some(("id", value)) => id = value,
                        Some(("event", value)) => event = value,
                        Some(("data", value)) => data = value,
                        _ => {}
                    }
                }
                if event == "done" {
                    return Ok(());
                }
                if !id.is_empty() {
                    last_id = id.to_string();
                }
                if !data.is_empty() {
                    println!("{data}");
                }
                buffer.drain(..end + 2);
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
