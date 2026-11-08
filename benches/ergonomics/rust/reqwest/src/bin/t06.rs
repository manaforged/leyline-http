use std::error::Error;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

const URL: &str = "http://127.0.0.1:18743/t06/large";

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let client = reqwest::Client::new();
    let total: u64 = client.head(URL).send().await?.headers()["content-length"]
        .to_str()?
        .parse()?;
    let mut file = File::create("large.bin").await?;
    let mut written = 0;
    while written < total {
        let range = format!("bytes={written}-");
        let mut response = client.get(URL).header("Range", range).send().await?;
        while let Ok(Some(chunk)) = response.chunk().await {
            file.write_all(&chunk).await?;
            written += chunk.len() as u64;
        }
    }
    file.flush().await?;
    assert_eq!(written, total);
    println!("{written}");
    Ok(())
}
