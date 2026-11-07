use futures::StreamExt;
use reqwest_eventsource::{Error, Event, EventSource};

#[tokio::main]
async fn main() {
    let mut source = EventSource::get("http://127.0.0.1:18743/t08/events");
    while let Some(event) = source.next().await {
        match event {
            Ok(Event::Open) | Err(Error::StreamEnded) => {}
            Ok(Event::Message(message)) if message.event == "done" => source.close(),
            Ok(Event::Message(message)) => println!("{}", message.data),
            Err(error) => panic!("{error}"),
        }
    }
}
