fn main() -> Result<(), ureq::Error> {
    let response = ureq::get("http://127.0.0.1:18743/t01/user").call()?;
    println!("{}", response.status());
    Ok(())
}
