use serde_json::Value;
use std::error::Error;
use std::thread;
use std::time::Duration;
use ureq::Agent;

const URL: &str = "http://127.0.0.1:18743/t04/flaky";

fn main() -> Result<(), Box<dyn Error>> {
    let agent: Agent = Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into();
    let mut response = agent.get(URL).call()?;
    let mut backoff = Duration::from_millis(100);
    for _ in 0..2 {
        if response.status() != 503 {
            break;
        }
        let wait = match response.headers().get("retry-after") {
            Some(seconds) => Duration::from_secs(seconds.to_str()?.parse()?),
            None => backoff,
        };
        thread::sleep(wait);
        backoff *= 2;
        response = agent.get(URL).call()?;
    }
    let body: Value = response.body_mut().read_json()?;
    println!("{} {}", response.status().as_u16(), body["attempts"]);
    Ok(())
}
