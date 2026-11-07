use serde_json::Value;
use ureq::{Agent, Proxy};

fn main() -> Result<(), ureq::Error> {
    let proxy = Proxy::new("http://127.0.0.1:18743")?;
    let agent: Agent = Agent::config_builder().proxy(Some(proxy)).build().into();
    let mut response = agent.get("http://ergonomics.invalid/t09/hello").call()?;
    let body: Value = response.body_mut().read_json()?;
    println!(
        "{} {}",
        response.status().as_u16(),
        body["via"].as_str().unwrap()
    );
    Ok(())
}
