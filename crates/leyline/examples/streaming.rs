use bytes::Bytes;
use futures_util::StreamExt;
use leyline::{Body, Browser, Session};
use tokio::io::AsyncWriteExt;

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "https://httpbin.org/post".to_string());
    let file_path = std::env::args().nth(2);

    let session = Session::builder().browser(Browser::Chrome147).build()?;

    let body = if let Some(path) = file_path {
        let meta = tokio::fs::metadata(&path).await.expect("stat input file");
        let file = tokio::fs::File::open(&path).await.expect("open input file");
        let reader = tokio_util::io::ReaderStream::new(file).map(|r| r);
        Body::stream_with_length(reader, meta.len())
    } else {
        let chunks = (0..16).map(|_| Ok::<Bytes, std::io::Error>(Bytes::from(vec![b'A'; 65536])));
        let stream = futures_util::stream::iter(chunks);
        Body::stream_with_length(stream, 16 * 65536)
    };

    let resp = session
        .post(&url)
        .body(body)
        .header("content-type", "application/octet-stream")
        .stream()
        .send()
        .await?;

    eprintln!("status {}", resp.status());

    let mut stream = resp.into_stream()?;
    let mut stdout = tokio::io::stdout();
    let mut total = 0u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(leyline::Error::from)?;
        total += chunk.len() as u64;
        stdout.write_all(&chunk).await.expect("stdout");
    }
    eprintln!("\nreceived {total} bytes");
    Ok(())
}
