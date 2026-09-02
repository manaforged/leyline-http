//! Print one line per request lifecycle event.

use leyline::trace::{Connect, Dns, Done, Head, Sent, Tls, Trace};
use leyline::{Browser, Session};

const URL: &str = "https://example.com";

/// Prints every event the client reports.
struct Printer;

impl Trace for Printer {
    fn dns(&self, ev: &Dns<'_>) {
        println!(
            "[{}] dns      {}:{} -> {} addrs in {:?}",
            ev.id, ev.host, ev.port, ev.addrs, ev.elapsed
        );
    }

    fn connect(&self, ev: &Connect<'_>) {
        println!(
            "[{}] connect  {}:{} reused={} in {:?}",
            ev.id, ev.host, ev.port, ev.reused, ev.elapsed
        );
    }

    fn tls(&self, ev: &Tls<'_>) {
        println!(
            "[{}] tls      {} {} alpn={} in {:?}",
            ev.id,
            ev.version.unwrap_or("?"),
            ev.cipher.unwrap_or("?"),
            ev.alpn.unwrap_or("?"),
            ev.elapsed
        );
    }

    fn sent(&self, ev: &Sent<'_>) {
        println!(
            "[{}] sent     {} in {:?}",
            ev.id,
            ev.protocol.as_str(),
            ev.elapsed
        );
    }

    fn head(&self, ev: &Head<'_>) {
        println!(
            "[{}] head     {} {} in {:?}",
            ev.id,
            ev.status,
            ev.protocol.as_str(),
            ev.elapsed
        );
    }

    fn done(&self, ev: &Done<'_>) {
        match ev.outcome {
            Ok(()) => println!("[{}] done     ok in {:?}", ev.id, ev.elapsed),
            Err(e) => println!("[{}] done     {:?}: {e}", ev.id, e.kind()),
        }
    }
}

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let session = Session::builder()
        .browser(Browser::Chrome147)
        .trace(Printer)
        .build()?;

    drop(session.get(URL).await?);
    println!("--- second request, pooled ---");
    drop(session.get(URL).await?);

    Ok(())
}
