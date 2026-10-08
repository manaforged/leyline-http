use std::sync::LazyLock;
use std::time::Duration;

use leyline::trace::{BodyEnd, BodyOutcome, Fanout, Metrics, Summary, Trace};
use leyline::{
    BlockRules, Browser, ErrorCategory, HostLimits, ProxyPool, RetryPolicy, Session, TimeoutConfig,
};

const BODY_LIMIT: u64 = 5 * 1024 * 1024;
const MAX_IN_FLIGHT: usize = 16;
const PROXIES: [&str; 2] = [
    "http://user:pass@proxy1.example:8080",
    "http://user:pass@proxy2.example:8080",
];
const PAGES: usize = 40;
const BLOCK_STATUSES: [u16; 2] = [403, 429];
const BODY_TIMEOUT: Duration = Duration::from_secs(30);

static BLOCKS: LazyLock<BlockRules> = LazyLock::new(|| {
    let mut rules = BlockRules::builtin().clone();
    rules.extend(BlockRules::statuses(BLOCK_STATUSES));
    rules
});

struct Log;

impl Trace for Log {
    fn summary(&self, ev: &Summary<'_>) {
        let status = ev.status.map(|s| s.as_u16()).unwrap_or(0);
        println!(
            "#{} [{}] {} {} via {} -> {status} in {:?} ({} attempts)",
            ev.id,
            ev.tag.unwrap_or("-"),
            ev.method,
            leyline::redact_url(ev.original_url),
            ev.proxy.as_deref().unwrap_or("direct"),
            ev.elapsed,
            ev.attempts,
        );
    }

    fn body(&self, ev: &BodyEnd<'_>) {
        let outcome = match ev.outcome {
            BodyOutcome::Complete => "complete".to_owned(),
            BodyOutcome::Failed(err) => format!("failed: {err}"),
            BodyOutcome::Dropped => "dropped".to_owned(),
            _ => "ended".to_owned(),
        };
        println!(
            "#{} body {} bytes in {:?}: {outcome}",
            ev.id, ev.bytes, ev.elapsed
        );
    }
}

async fn fetch(session: Session, index: usize, url: String) {
    let resp = match session
        .get(&url)
        .tag(format!("page-{index}"))
        .stream()
        .send()
        .await
    {
        Ok(resp) => resp,
        Err(e) => return report(&url, &e),
    };
    let via = resp.proxy().unwrap_or("direct").to_owned();
    if let Some(signal) = BLOCKS.check(&resp) {
        println!("{url} via {via}: {:?} by {}", signal.kind, signal.vendor);
        return;
    }
    let mut sink = tokio::io::sink();
    match resp.copy_decoded_to(&mut sink, Some(BODY_LIMIT)).await {
        Ok(n) => println!("{url} via {via}: {n} bytes"),
        Err(e) => report(&url, &e),
    }
}

fn report(url: &str, e: &leyline::Error) {
    let via = e.proxy().unwrap_or("direct");
    let attempts = e.attempts();
    match e.category() {
        ErrorCategory::Proxy => {
            eprintln!("{url}: proxy {via} failed after {attempts} attempts, the pool will rotate")
        }
        ErrorCategory::BodyLimit => eprintln!("{url}: page larger than {BODY_LIMIT} bytes"),
        other => eprintln!("{url}: {other} via {via} after {attempts} attempts: {e}"),
    }
}

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let proxies = ProxyPool::new(PROXIES)
        .sticky_for(Duration::from_secs(300))
        .ban_after(3)
        .ban_for(Duration::from_secs(120))
        .rotate_on_block(BLOCKS.clone());
    let metrics = Metrics::new();

    let session = Session::builder()
        .browser(Browser::default())
        .host_limits(
            HostLimits::new()
                .max_in_flight(4)
                .per_second(2.0)
                .max_total_in_flight(MAX_IN_FLIGHT),
        )
        .proxy_pool(proxies.clone())
        .retry(RetryPolicy::transient().skip_blocks(BLOCKS.clone()))
        .timeout(TimeoutConfig::new().body(BODY_TIMEOUT))
        .trace(Fanout::new().with(Log).with(metrics.clone()))
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

    println!("{}", metrics.snapshot());
    for health in proxies.stats() {
        println!(
            "{}: {} failures, banned until {:?}",
            health.proxy, health.failures, health.banned_until
        );
    }
    Ok(())
}
