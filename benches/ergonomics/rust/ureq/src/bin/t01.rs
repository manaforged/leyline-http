use serde_json::Value;

fn main() -> Result<(), ureq::Error> {
    let user: Value = ureq::get("http://127.0.0.1:18743/t01/user")
        .call()?
        .body_mut()
        .read_json()?;
    println!("{} {}", user["id"], user["name"].as_str().unwrap());
    Ok(())
}
