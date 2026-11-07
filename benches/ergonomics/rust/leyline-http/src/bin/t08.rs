#[tokio::main]
async fn main() -> leyline::Result<()> {
    let mut events = leyline::Session::new()
        .get("http://127.0.0.1:18743/t08/events")
        .events()?;
    while let Some(event) = events.next().await {
        let event = event?;
        if event.event() == "done" {
            break;
        }
        println!("{}", event.data());
    }
    Ok(())
}
