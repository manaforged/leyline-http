use std::time::Duration;

use leyline::trace::{Event, Metrics, Trace};
use leyline::{Browser, HostLimits, Kind, Proxy, Retry, Session, Timeout};

const BODY_LIMIT: usize = 5 * 1024 * 1024;
const MAX_IN_FLIGHT: usize = 16;
const PROXIES: [&str; 2] = [
    "http://user:pass@proxy1.example:8080",
    "http://user:pass@proxy2.example:8080",
];
const PAGES: usize = 40;
const BLOCK_STATUSES: [u16; 2] = [403, 429];
const BODY_TIMEOUT: Duration = Duration::from_secs(30);

struct Log;

impl Trace for Log {
    fn event(&self, event: &Event<'_>) {
        match event {
            Event::Summary {
                id,
                label,
                method,
                original_url,
                proxy,
                status,
                elapsed,
                attempts,
                ..
            } => println!(
                "#{id} [{}] {method} {} via {} -> {} in {elapsed:?} ({attempts} attempts)",
                label.unwrap_or("-"),
                leyline::redact_url(original_url),
                proxy.as_deref().unwrap_or("direct"),
                status.map_or(0, |s| s.as_u16()),
            ),
            Event::Body {
                id,
                bytes,
                elapsed,
                outcome,
                ..
            } => {
                let outcome = match outcome {
                    Ok(()) => "complete".to_owned(),
                    Err(Some(err)) => format!("failed: {err}"),
                    Err(None) => "dropped".to_owned(),
                };
                println!("#{id} body {bytes} bytes in {elapsed:?}: {outcome}");
            }
            _ => {}
        }
    }
}

async fn fetch(session: Session, index: usize, url: String) {
    let resp = match session
        .get(&url)
        .label(format!("page-{index}"))
        .max_body_size(BODY_LIMIT)
        .stream()
        .send()
        .await
    {
        Ok(resp) => resp,
        Err(e) => return report(&url, &e),
    };
    let via = resp.proxy().unwrap_or("direct").to_owned();
    if let Some(block) = resp.block() {
        println!("{url} via {via}: {block:?}");
        return;
    }
    let mut sink = tokio::io::sink();
    match resp.write_to(&mut sink).await {
        Ok(n) => println!("{url} via {via}: {n} bytes"),
        Err(e) => report(&url, &e),
    }
}

fn report(url: &str, e: &leyline::Error) {
    let via = e.proxy().unwrap_or("direct");
    let attempts = e.attempts();
    match e.kind() {
        Kind::Proxy => {
            eprintln!("{url}: proxy {via} failed after {attempts} attempts, the pool will rotate")
        }
        Kind::BodyLimit => eprintln!("{url}: page larger than {BODY_LIMIT} bytes"),
        other => eprintln!("{url}: {other} via {via} after {attempts} attempts: {e}"),
    }
}

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let proxies = Proxy::list(PROXIES)
        .sticky_for(Duration::from_secs(300))
        .ban_after(3)
        .ban_for(Duration::from_secs(120));
    let metrics = Metrics::new();

    let session = Session::builder()
        .browser(Browser::default())
        .host_limits(
            HostLimits::new()
                .max_in_flight(4)
                .per_second(2.0)
                .max_total_in_flight(MAX_IN_FLIGHT),
        )
        .proxy(proxies.clone())
        .block_statuses(BLOCK_STATUSES)
        .retry(Retry::new(3).when(|attempt| attempt.is_transient() && !attempt.is_block()))
        .timeout(Timeout::new().body(BODY_TIMEOUT))
        .trace(Log)
        .trace(metrics.clone())
        .build()?;

    let tasks: Vec<_> = (1..=PAGES)
        .map(|page| format!("https://shop.example/products?page={page}"))
        .enumerate()
        .map(|(index, url)| tokio::spawn(fetch(session.clone(), index, url)))
        .collect();
    for task in tasks {
        if let Err(e) = task.await {
            eprintln!("fetch task failed: {e}");
        }
    }

    println!("{metrics}");
    let stats = session.stats();
    for proxy in stats.proxies() {
        println!(
            "{proxy}: {} failures, banned until {:?}",
            stats.proxy_failures(proxy),
            stats.proxy_banned_until(proxy)
        );
    }
    Ok(())
}
