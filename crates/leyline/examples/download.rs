use leyline::{Kind, Session};

const LIMIT: usize = 50 * 1024 * 1024;

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "https://httpbin.org/bytes/4096".to_string());
    let path = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "download.bin".to_string());

    let session = Session::builder().build()?;
    match async {
        session
            .get(&url)
            .error_for_status()
            .max_body_size(LIMIT)
            .stream()
            .send()
            .await?
            .download_to(&path)
            .await
    }
    .await
    {
        Ok(written) => println!("saved {written} bytes to {path}"),
        Err(e) => match e.kind() {
            Kind::Status => eprintln!("server answered {:?}", e.status()),
            Kind::BodyLimit => eprintln!("file is larger than {LIMIT} bytes"),
            Kind::Timeout => eprintln!("timed out: {e}"),
            Kind::Dns | Kind::Connect => eprintln!("network: {e}"),
            _ => return Err(e),
        },
    }
    Ok(())
}
