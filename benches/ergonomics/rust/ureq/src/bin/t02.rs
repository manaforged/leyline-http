use serde_json::{Value, json};
use std::time::Duration;

fn main() -> Result<(), ureq::Error> {
    let body = json!({"name": "ada", "age": 36});
    let mut response = ureq::post("http://127.0.0.1:18743/t02/users")
        .config()
        .timeout_global(Some(Duration::from_secs(5)))
        .build()
        .send_json(&body)?;
    let created: Value = response.body_mut().read_json()?;
    println!("{} {}", response.status().as_u16(), created["id"]);
    let slow = ureq::post("http://127.0.0.1:18743/t02/slow")
        .config()
        .timeout_global(Some(Duration::from_millis(300)))
        .build()
        .send_json(&body);
    match slow {
        Err(ureq::Error::Timeout(_)) => println!("timeout"),
        Err(error) => return Err(error),
        Ok(_) => println!("no timeout"),
    }
    Ok(())
}
