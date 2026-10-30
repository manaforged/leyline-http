use leyline::Session;
use leyline::multipart::{Form, Part};

const URL: &str = "https://httpbin.org/post";
const NOTE_FIELD: &str = "note";
const NOTE: &str = "uploaded with leyline";
const FILE_FIELD: &str = "file";

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "Cargo.toml".to_string());

    let session = Session::new();
    let form = Form::new()
        .part(NOTE_FIELD, Part::text(NOTE))
        .part(FILE_FIELD, Part::file(&path)?);

    let resp = session
        .post(URL)
        .multipart(form)
        .error_for_status()
        .send()
        .await?;

    println!("status: {}", resp.status());
    println!("body:   {}", resp.text().await?);
    Ok(())
}
