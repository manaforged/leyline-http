//! Demonstrate concurrent multiplexing over a single H2 connection.
//!
//! Fires 50 requests in parallel against the same host. The pool reuses
//! one `H2Client` handle across all of them, so every request goes over
//! the same TLS + TCP connection as an independent H2 stream.
//!
//! Run: `cargo run --example concurrent -- https://tls.peet.ws/api/all`

use std::sync::Arc;
use std::time::Instant;

use leyline::{Browser, Platform, Session};

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "https://tls.peet.ws/api/all".to_string());

    let session = Arc::new(
        Session::builder()
            .browser(Browser::Chrome147)
            .platform(Platform::Linux)
            .build()?,
    );

    let n = 50usize;
    let started = Instant::now();

    let mut handles = Vec::with_capacity(n);
    for i in 0..n {
        let session = Arc::clone(&session);
        let url = url.clone();
        handles.push(tokio::spawn(async move {
            let resp = session.get(&url).send().await?;
            Ok::<(usize, u16), leyline::Error>((i, resp.status()))
        }));
    }

    let mut ok = 0usize;
    for h in handles {
        if let Ok(Ok((_, status))) = h.await {
            if status < 400 {
                ok += 1;
            }
        }
    }

    let elapsed = started.elapsed();
    println!(
        "{n} concurrent requests: {ok} succeeded in {:.2?} ({:.0} req/s)",
        elapsed,
        n as f64 / elapsed.as_secs_f64()
    );
    Ok(())
}
