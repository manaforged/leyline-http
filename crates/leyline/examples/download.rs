use leyline::{ErrorCategory, Session};

const LIMIT: u64 = 50 * 1024 * 1024;

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "https://httpbin.org/bytes/4096".to_string());
    let path = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "download.bin".to_string());

    let session = Session::builder().build()?;
    match session.get(&url).download(&path, Some(LIMIT)).await {
        Ok(written) => println!("saved {written} bytes to {path}"),
        Err(e) => match e.category() {
            ErrorCategory::Status => eprintln!("server answered {:?}", e.status()),
            ErrorCategory::BodyLimit => eprintln!("file is larger than {LIMIT} bytes"),
            ErrorCategory::Timeout => eprintln!("timed out: {e}"),
            ErrorCategory::Dns | ErrorCategory::Connect => eprintln!("network: {e}"),
            _ => return Err(e),
        },
    }
    Ok(())
}
