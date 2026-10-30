use leyline::trace::{Event, Trace};
use leyline::{Browser, Session};

const URL: &str = "https://example.com";

struct Printer;

impl Trace for Printer {
    fn event(&self, event: &Event<'_>) {
        match event {
            Event::Dns {
                id,
                host,
                port,
                addrs,
                elapsed,
                ..
            } => println!("[{id}] dns      {host}:{port} -> {addrs} addrs in {elapsed:?}"),
            Event::Connect {
                id,
                host,
                port,
                reused,
                elapsed,
                ..
            } => println!("[{id}] connect  {host}:{port} reused={reused} in {elapsed:?}"),
            Event::Tls {
                id,
                version,
                cipher,
                alpn,
                elapsed,
                ..
            } => println!(
                "[{id}] tls      {} {} alpn={} in {elapsed:?}",
                version.unwrap_or("?"),
                cipher.unwrap_or("?"),
                alpn.unwrap_or("?"),
            ),
            Event::Sent {
                id,
                protocol,
                elapsed,
                ..
            } => println!("[{id}] sent     {protocol:?} in {elapsed:?}"),
            Event::Head {
                id,
                status,
                protocol,
                elapsed,
                ..
            } => println!("[{id}] head     {status} {protocol:?} in {elapsed:?}"),
            Event::Done {
                id,
                elapsed,
                outcome,
                ..
            } => match outcome {
                Ok(()) => println!("[{id}] done     ok in {elapsed:?}"),
                Err(e) => println!("[{id}] done     {:?}: {e}", e.kind()),
            },
            _ => {}
        }
    }
}

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let session = Session::builder()
        .browser(Browser::Chrome147)
        .trace(Printer)
        .build()?;

    drop(session.get(URL).send().await?);
    println!("--- second request, pooled ---");
    drop(session.get(URL).send().await?);

    Ok(())
}
