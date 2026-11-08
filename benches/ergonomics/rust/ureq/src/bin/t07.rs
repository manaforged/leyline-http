use serde_json::Value;
use ureq::unversioned::multipart::Form;

fn main() -> Result<(), ureq::Error> {
    let form = Form::new()
        .text("note", "hello")
        .file("file", "upload.bin")?;
    let mut response = ureq::post("http://127.0.0.1:18743/t07/upload").send(form)?;
    let body: Value = response.body_mut().read_json()?;
    println!(
        "{} {} {}",
        response.status().as_u16(),
        body["size"],
        body["note"].as_str().unwrap()
    );
    Ok(())
}
