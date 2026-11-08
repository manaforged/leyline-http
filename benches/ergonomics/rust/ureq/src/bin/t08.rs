use std::error::Error;
use std::io::{BufRead, BufReader};
use std::thread;
use std::time::Duration;

const URL: &str = "http://127.0.0.1:18743/t08/events";

fn main() -> Result<(), Box<dyn Error>> {
    let mut last_id = String::new();
    loop {
        let mut request = ureq::get(URL).header("Accept", "text/event-stream");
        if !last_id.is_empty() {
            request = request.header("Last-Event-ID", &last_id);
        }
        let reader = BufReader::new(request.call()?.into_body().into_reader());
        let (mut id, mut event, mut data) = (String::new(), String::new(), String::new());
        for line in reader.lines() {
            let line = line?;
            match line.split_once(": ") {
                Some(("id", value)) => id = value.to_string(),
                Some(("event", value)) => event = value.to_string(),
                Some(("data", value)) => data = value.to_string(),
                _ => {}
            }
            if line.is_empty() {
                if event == "done" {
                    return Ok(());
                }
                if !id.is_empty() {
                    last_id = id.clone();
                }
                if !data.is_empty() {
                    println!("{data}");
                }
                (id, event, data) = (String::new(), String::new(), String::new());
            }
        }
        thread::sleep(Duration::from_millis(100));
    }
}
