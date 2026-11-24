# Crawling

Use `Session::crawl` for a turnkey site crawler (`robots.txt`, sitemaps,
link scoping, deduplication, polite pacing, and checkpoints), or compose
`HostLimits`, `Proxy::list`, `Response::block()`, and `leyline::html` directly
for custom worker loops.

| Need | Use | Module paths |
| --- | --- | --- |
| Crawl a site from a seed URL | `Session::crawl(seed)` (requires default feature `html`) | `leyline::crawl::{Crawl, Page, Respect, Scope, SkipReason}` |
| Check `robots.txt` standalone | `Session::robots(url, agent)` (always compiled) | `leyline::crawl::{Robots, RobotsSource, Purpose}` |
| Rate-limit & queue-cap per origin | `SessionBuilder::host_limits(HostLimits)` | `leyline::HostLimits` |
| Detect blocks / challenges | `Response::block()`, `SessionBuilder::block_rules` | `leyline::Block` |
| Extract text, links, directives | `html::text`, `html::Directives`, `html::canonical` | `leyline::html` (default feature `html`) |

## Crawl a site

`session.crawl(seed)` returns a lazy `Crawl` stream of `Result<Page>`. The
defaults are polite, so the short form is a complete crawler:

```rust,no_run
use leyline::Session;

# async fn run() -> leyline::Result<()> {
let mut crawl = Session::new().crawl("https://docs.example/");
while let Some(page) = crawl.next().await {
    let page = page?;
    println!("{} {} words", page.url(), page.text().split_whitespace().count());
}
# Ok(())
# }
```

By default, a crawl stays on the seed's host, runs 1 request at a time per host
(16 total) at 1 req/s, pauses hosts on 429/503 and retries up to 3 times, reads
`robots.txt` and sitemaps, skips errors/blocks/non-HTML/duplicates, and obeys
page directives unless `respect(None)` is set. Tune any limit on the builder:

```rust,no_run
use leyline::crawl::{Respect, Scope};
use leyline::{HostLimits, Session};

# async fn run() -> leyline::Result<()> {
let mut crawl = Session::new()
    .crawl("https://docs.example/guide/")
    .respect(Respect::AiTrain)
    .scope(Scope::Prefix)
    .limit(500)
    .host_limits(HostLimits::new().max_in_flight(2).per_second(2.0));
while let Some(page) = crawl.next().await {
    match page {
        Ok(page) => println!("{}", page.url()),
        Err(err) => eprintln!("{}: {}", err.url().map_or("-", |url| url.as_str()), err.kind()),
    }
}
println!("{} pages, {} errors", crawl.pages(), crawl.errors());
# Ok(())
# }
```

| Setter | Default | Effect |
| --- | --- | --- |
| `respect(Respect)` or `None` | `Robots` | `Robots` obeys `robots.txt`, `Crawl-delay`, `noindex`, `nofollow`, and `rel="nofollow"`. `AiTrain` also obeys `Content-Signal`, `Content-Usage`, and `noai`. `None` disables robots checks |
| `scope(Scope)` | `Host` | `Host` (seed host), `Domain` (registrable domain), `Prefix` (host + seed directory), or `Any` |
| `filter(keep)` | none | Keeps discovered URLs only when `keep(&Url)` returns `true` (on top of `scope`; never applied to the seed) |
| `limit(n)` or `None` | none | Stops after `n` kept pages |
| `agent(name)` | `*` | Product token matched in `robots.txt` and meta robots (falls back to `*`) |
| `max_body_size(n)` | session's (100 MiB) | Max decoded size of pages, sitemaps (50 MiB when unset), and `robots.txt` |
| `redirect(policy)` | session's, else `5` | Redirect limit for pages, sitemaps, and `robots.txt` |
| `max_depth(n)` or `None` | `64` | Max link depth from seed (deeper links are silently dropped) |
| `max_url_size(n)` or `None` | `2048` | Max URL length in bytes (longer URLs are silently dropped) |
| `max_sitemaps(n)` or `None` | `50` | Max sitemap files to open (`0` disables sitemaps) |
| `max_crawl_delay(d)` or `None` | `30 s` | Cap on `robots.txt` `Crawl-delay` (`Duration::ZERO` ignores it) |
| `robots_max_age(d)` or `None` | `24 h` | Re-fetch interval for `robots.txt` |
| `dedup(bool)` | `true` | Skips canonical aliases and duplicate-text pages |
| `dedup_min_words(n)` or `None` | `20` | Minimum words for duplicate-text matching (`None` disables text dedup) |
| `host_limits(HostLimits)` | 1/host, 16 total, 1/s, pause on 429/503 | Replaces crawl default pace (`HostLimits::new()` removes crawl defaults); session `host_limits` still apply on top |

### Crawl rules and defaults

- **URL pipeline:** Drops non-`http(s)`, out-of-scope, `filter`-rejected, or already-seen URLs (normalized without fragments, with sorted query params and normalized percent-encoding; sent as written). Redirects are re-queued through the same checks (seed redirects to a sibling host widen scope; cross-site seed redirects yield only that page).
- **Content & dedup:** Skips block pages (`Response::block()`), non-2xx statuses, and non-HTML (sniffs missing `Content-Type` per WHATWG MIME Sniffing). Holds canonical aliases until their canonical target resolves, and deduplicates pages of ≥ `dedup_min_words` words.
- **Sitemaps & robots:** Fetches `robots.txt` before crawling each site (retried under session `Retry`, or 4 times at 1/2/4/8 s when unset), then reads listed sitemaps (or `/sitemap.xml`), supporting plain text, gzip, and sitemap indexes while ignoring media extension `<loc>` tags. Missing sitemaps are not errors; failed sitemaps increment `errors()`.
- **Credentials & pacing:** Sends session `Authorization` / `bearer_auth` / default `Cookie` headers only to the seed's origin (jar cookies still follow RFC 6265 rules; avoid placing secrets in custom default headers). Requests wait for both session and crawl `HostLimits` without holding a slot while paused or waiting to retry.
- **Lifecycle:** Transport errors are yielded as `Err` without stopping the crawl. `Session::shutdown()` ends the crawl cleanly; dropping `Crawl` cancels in-flight requests.

### Inspect pages and skip counts

Each `Page` exposes `url()` (final URL), `depth()`, `status()`, `headers()`
(sensitive headers such as `Set-Cookie` stripped), `header(name)`, `html()`,
`text()` (from `html::text`), and `into_html()`.

`Crawl` tracks `requests()`, `pages()`, `errors()`, and `skipped(SkipReason)`:

| `SkipReason` | `as_str()` | Meaning |
| --- | --- | --- |
| `Robots` | `"robots"` | Disallowed by `robots.txt` |
| `Signal` | `"signal"` | Refused by `Content-Signal` or `Content-Usage` under `Respect::AiTrain` |
| `Status` | `"status"` | Non-success status or redirect with no target |
| `NotHtml` | `"not_html"` | Response was not HTML |
| `Blocked` | `"blocked"` | Matched `Response::block()` |
| `NoIndex` | `"noindex"` | Page carried `noindex` / `none` directive |
| `NoAi` | `"noai"` | Page carried `noai` under `Respect::AiTrain` |
| `Duplicate` | `"duplicate"` | Duplicate URL, canonical alias, or duplicate text |
| `Redirects` | `"redirects"` | Exceeded redirect limit |
| `Scope` | `"scope"` | Outside configured `Scope` |
| `Filter` | `"filter"` | Rejected by `Crawl::filter` |

## Filter the links to follow

`Scope::Prefix` keeps the crawl on the seed's host and under its directory
(`/docs/a.html` → `/docs/`; `/docs/a` → `/docs/a/`). Add `Crawl::filter` for
any custom URL rule:

```rust,no_run
use leyline::Session;
use leyline::crawl::{Scope, SkipReason};

# async fn run() -> leyline::Result<()> {
let mut crawl = Session::new()
    .crawl("https://example.com/docs/")
    .scope(Scope::Host)
    .filter(|url| url.path().starts_with("/docs/") && url.query().is_none());
while let Some(page) = crawl.next().await {
    println!("{}", page?.url());
}
println!("{} links rejected", crawl.skipped(SkipReason::Filter));
# Ok(())
# }
```

## Save and resume a crawl

`crawl.save(path).await` stops scheduling new work, drains active fetches, and
writes an atomic JSON checkpoint (versioned, max 64 MiB, mode `0600` on Unix).
Drain any buffered `Err` items from `next()` before calling `save`.

```rust,no_run
use leyline::Session;
use leyline::crawl::Crawl;

# async fn run() -> leyline::Result<()> {
let session = Session::new();
let mut crawl = session.crawl("https://docs.example/");
if let Some(page) = crawl.next().await {
    let page = page?;
    println!("{}", page.url());
}
crawl.save("crawl.json").await?;
drop(crawl);

let mut crawl = Crawl::load(&session, "crawl.json")?;
while let Some(page) = crawl.next().await {
    println!("{}", page?.url());
}
# Ok(())
# }
```

- Checkpoints persist queued URLs, unyielded pages, URL/text dedup history, canonical dependencies, scope, respect policy, agent, limits, pending sitemaps, counters, and retry waits. `robots.txt` is re-fetched on resume.
- Checkpoints do **not** save cookies or closures: restore account cookies separately with `Session::load`, and re-attach `.filter(..)` after `Crawl::load`. Treat checkpoints as sensitive data (they contain URLs and page bodies).

## Build a custom crawl session

Combine `HostLimits`, `Proxy::list`, `block_statuses`, `Retry`, `Timeout::body`,
`trace::Log`, and `trace::Metrics` on one shared `Session`, then clone it into
worker tasks:

```rust,no_run
use std::time::Duration;
use leyline::trace::{Log, Metrics};
use leyline::{Browser, HostLimits, Proxy, Retry, Session, Timeout};

# fn run() -> leyline::Result<()> {
let proxies = Proxy::list([
    "http://user:pass@proxy-a.example:8080",
    "http://user:pass@proxy-b.example:8080",
    "http://user:pass@proxy-c.example:8080",
])
.sticky_for(Duration::from_secs(600))
.ban_after(3)
.ban_for(Duration::from_secs(120));
let metrics = std::sync::Arc::new(Metrics::new());

let session = Session::builder()
    .browser(Browser::Chrome158)
    .host_limits(
        HostLimits::new()
            .max_in_flight(4)
            .per_second(2.0)
            .max_total_in_flight(64)
            .pause_on([429, 503]),
    )
    .proxy(proxies)
    .block_statuses([403, 429])
    .retry(Retry::new(3).when(|attempt| attempt.is_transient() && !attempt.is_block()))
    .timeout(Timeout::new().body(Duration::from_secs(60)))
    .trace(Log)
    .trace(metrics)
    .build()?;
# let _ = session;
# Ok(())
# }
```

## Limit each host

`HostLimits` applies per origin (scheme, host, port) across HTTP/1.1, HTTP/2,
and HTTP/3. Clones and derived sessions share its live counters, queues, and
pauses; `HostLimits::default()` sets no limits.

| Setter | Effect |
| --- | --- |
| `max_in_flight(n)` | Max concurrent requests per origin (minimum 1) |
| `per_second(rate)` | Minimum spacing of `1 / rate` seconds between starts per origin (`<= 0` disables) |
| `max_total_in_flight(n)` | Max concurrent requests across all origins (minimum 1) |
| `host_max_in_flight(host, n)` | Per-host override for `max_in_flight` (`*.example.com` matches subdomains only; exact host wins over wildcard, longer wildcard over shorter) |
| `host_per_second(host, rate)` | Per-host override for `per_second` (same host/wildcard precedence) |
| `max_pending(n)` or `None` | Max requests queued waiting for a per-host slot or HTTP/1.1 connection (default `1024`; `None` removes the cap; excess fails with `Kind::Overloaded`) |
| `max_total_pending(n)` or `None` | Max requests queued waiting for `max_total_in_flight` across all origins (excess fails with `Kind::Overloaded`) |
| `pause_on(statuses)` | Pauses new requests to the responding origin using `Retry::wait_header` / `Retry-After` (or `pause_for`) |
| `pause_for(duration)` | Fallback pause duration when no wait header is sent (default `60 s`) |
| `max_pause(duration)` | Cap on server-requested pauses (default `24 h`) |

```rust
use leyline::HostLimits;

let limits = HostLimits::new()
    .max_in_flight(4)
    .per_second(2.0)
    .host_max_in_flight("api.shop.example", 1)
    .host_per_second("api.shop.example", 0.5)
    .host_max_in_flight("*.cdn.example", 16)
    .max_total_in_flight(64);
# let _ = limits;
```

- A host override uses only the overrides set for it; unset fields are unlimited for that host while `max_total_in_flight` still applies.
- Requests acquire slots in order: per-origin in-flight → origin pause & rate spacing → global in-flight (never holding a global slot while paused or waiting for rate spacing).
- Queue waits count toward request deadlines (`Kind::Timeout`). Buffered requests release slots after the body is read; `.stream()` requests hold them until the body ends, errors, or drops. Dropping a waiting request removes it from the queue.

## Obey robots.txt

`Session::robots(url, agent)` fetches `/robots.txt` for the origin of `url`
(RFC 9309, up to 512 KiB or `max_body_size`, following up to 5 redirects by
default) and returns `leyline::crawl::Robots`. Pass `"*"` for the wildcard
group. Fails only on an invalid URL or shut-down session.

| `robots.txt` fetch | `Robots::source()` | Result |
| --- | --- | --- |
| 2xx | `Parsed` | Matching group rules (case-insensitive `agent`, else `*`; longest pattern wins, `Allow` wins ties) |
| 4xx | `Unavailable` | Every path allowed |
| 5xx or transport failure | `Unreachable` | Every path disallowed |

```rust,no_run
use leyline::Url;
use leyline::crawl::Purpose;

# async fn run(session: leyline::Session) -> leyline::Result<()> {
let robots = session.robots("https://shop.example/", "examplebot").await?;
let page = Url::parse("https://shop.example/private/report.html")?;
if robots.allows(&page) && robots.allows_use(&page, Purpose::AiTrain) {
    let _ = session.get(page.as_str()).send().await?;
}
println!("delay {:?}, sitemaps {:?}", robots.crawl_delay(), robots.sitemaps());
# Ok(())
# }
```

`Robots::parse(text, agent)` parses text you already have. `allows_use(url, purpose)`
evaluates `Content-Signal` (`search`, `ai-input`, `ai-train`) and IETF aipref
`Content-Usage` (`search`, `ai-use`, `train-ai`) rules (longest path wins; `no`
wins ties; unspecified purposes are allowed). `crawl_delay()` and `sitemaps()`
expose the parsed `Crawl-delay` and `Sitemap` directives.

## Classify a block response

`Response::block()` returns `Some(Block)` when a response matches a built-in or
custom block rule (`None` otherwise). A block is a normal response, not an
error; `Session::crawl` skips it as `SkipReason::Blocked`.

| Vendor | Status | Header | `Block` |
| --- | --- | --- | --- |
| Cloudflare | any | `cf-mitigated: challenge` | `Challenge("cloudflare")` |
| AWS WAF | 202 | `x-amzn-waf-action: challenge` | `Challenge("aws-waf")` |
| AWS WAF | 405 | `x-amzn-waf-action: captcha` | `Captcha("aws-waf")` |

Add bare status rules with `SessionBuilder::block_statuses([403, 429])`
(`Block::Denied("status")`) or TOML `[[rule]]` entries with
`SessionBuilder::block_rules(toml)` (`vendor`, `kind`, and optional `status`,
`header`, `value`; evaluated in order after built-in rules; invalid TOML fails
`build()` with `Kind::Config`):

```rust,no_run
use leyline::{Block, Session};

# async fn run() -> leyline::Result<()> {
let session = Session::builder()
    .block_rules(
        r#"
[[rule]]
vendor = "shop"
kind = "denied"
status = 403
header = "x-shop-block"
value = "1"
"#,
    )
    .block_statuses([403, 429])
    .build()?;

let resp = session.get("https://shop.example/item/1").send().await?;
match resp.block() {
    Some(Block::Captcha(vendor)) => println!("captcha from {vendor}"),
    Some(block) => println!("blocked by {}", block.vendor()),
    None => println!("{} bytes", resp.text().await?.len()),
}
# Ok(())
# }
```

### Blocks, retries, and proxies

On each attempt: (1) a block response counts as a strike against its proxy in a
`Proxy::list`, (2) `Retry::when` inspects `attempt.is_block()` to decide
whether to retry (for example `attempt.is_transient() && !attempt.is_block()`
stops immediately), and (3) the final response exposes `resp.block()`. See
[Use a proxy list](proxies.md#use-a-proxy-list).

## Read page text and directives

With the default `html` feature:

- `html::text(document)` returns readable text one block per line (collapsing whitespace outside `<pre>`, decoding entities, skipping `<head>`/`<script>`/`<style>`/`<noscript>`/`<template>`/`<svg>`/`<textarea>`).
- `html::Directives::read(document, headers, agent)` merges `<meta name="robots">` and `X-Robots-Tag` headers into `index()`, `follow()`, and `ai()`.
- `html::canonical(document)` returns the raw `<link rel="canonical">` `href` (join against `resp.url()`).

```rust,no_run
use leyline::html::{self, Directives};

# async fn run(session: leyline::Session) -> leyline::Result<()> {
let resp = session.get("https://shop.example/item/1").send().await?;
let url = resp.url().clone();
let headers = resp.headers().clone();
let page = resp.text().await?;
let rules = Directives::read(&page, &headers, "examplebot");
if rules.index() && rules.ai() {
    let canonical = html::canonical(&page).and_then(|href| url.join(&href).ok());
    println!("keep {} ({} words)", canonical.unwrap_or(url), html::text(&page).split_whitespace().count());
}
# Ok(())
# }
```

## Building custom worker loops

For large custom URL streams, combine `futures_util::StreamExt::buffer_unordered(n)`
with `session.is_shut_down()` and the session primitives owned by their
respective chapters:

- **Proxy rotation & per-proxy identities:** [Use a proxy list](proxies.md#use-a-proxy-list) (`Proxy::list`, `Proxy::identified`)
- **Browser rotation & graceful shutdown:** [Sessions](sessions.md#derive-a-session) (`to_builder().browser(..)`), [Stop a session](sessions.md#stop-a-session)
- **Error classification:** [Errors](errors.md#match-on-the-kind) (`Error::kind()`, `attempts()`, `proxy()`)
- **Resumable capped file downloads:** [Resume a download](streaming.md#resume-a-download) (`RequestBuilder::download_to`)
- **Per-request tags, `Log`, `Metrics`, and `Session::stats()`:** [Logging and tracing](logging.md#count-requests-and-inspect-queues)
- **Pool and body memory bounds:** [Connection pool](network.md#connection-pool) and [Size cap and content coding](responses.md#size-cap-and-content-coding)

## Next

Read [Accounts](accounts.md).
