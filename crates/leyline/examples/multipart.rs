use leyline::Session;
use leyline::multipart::Form;

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
    let form = Form::new().text(NOTE_FIELD, NOTE).file(FILE_FIELD, &path)?;

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
