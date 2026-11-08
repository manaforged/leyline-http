use serde_json::{Value, json};

fn main() -> Result<(), ureq::Error> {
    let agent = ureq::agent();
    agent
        .post("http://localhost:18743/t03/login")
        .send_json(json!({"user": "ada", "password": "secret"}))?;
    let me: Value = agent
        .get("http://localhost:18743/t03/me")
        .call()?
        .body_mut()
        .read_json()?;
    println!("{}", me["user"].as_str().unwrap());
    Ok(())
}
