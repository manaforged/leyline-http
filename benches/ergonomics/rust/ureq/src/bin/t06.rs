use std::error::Error;
use std::fs::File;
use std::io;

const URL: &str = "http://127.0.0.1:18743/t06/large";

fn main() -> Result<(), Box<dyn Error>> {
    let head = ureq::head(URL).call()?;
    let total: u64 = head.headers()["content-length"].to_str()?.parse()?;
    let mut file = File::create("large.bin")?;
    let mut written = 0;
    while written < total {
        let range = format!("bytes={written}-");
        let response = ureq::get(URL).header("Range", range).call()?;
        let mut reader = response.into_body().into_reader();
        let _ = io::copy(&mut reader, &mut file);
        written = file.metadata()?.len();
    }
    assert_eq!(written, total);
    println!("{written}");
    Ok(())
}
