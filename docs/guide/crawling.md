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

By default, a crawl stays on the seed's host and reads `robots.txt` and
sitemaps. It obeys page directives unless `respect(None)` is set. It retries a
page up to 2 times. It skips blocks, non-2xx, non-HTML, and duplicate pages,
and yields transport errors as `Err` without stopping.

A crawl paces itself with the session's `HostLimits`. Any field the session
leaves unset gets a polite default: 1 request at a time per host, 16 in total,
1 request per second, and a pause on 429 and 503. Tune any limit on the builder:

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
        Err(err) => eprintln!("crawl error ({}): {err}", err.kind()),
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
| `limit(n: usize)` or `None` | none | Stops after `n` kept pages |
| `agent(name)` | `*` | Product token matched in `robots.txt` and meta robots (falls back to `*`) |
| `max_body_size(n)` or `None` | session's (100 MiB) | Max decoded size of pages, sitemaps, and `robots.txt`. When neither the crawl nor the session sets it, sitemaps get 50 MiB and `robots.txt` gets 512 KiB. `None` removes the cap |
| `redirect(policy)` | session's, else `5` | Redirect limit for pages, sitemaps, and `robots.txt` |
| `max_depth(n: usize)` or `None` | `64` | Max link depth from seed (deeper links are silently dropped) |
| `max_url_size(n: usize)` or `None` | `2048` | Max URL length in bytes (longer URLs are silently dropped) |
| `max_sitemaps(n: usize)` or `None` | `50` | Max sitemap files to open (`0` disables sitemaps) |
| `max_crawl_delay(d)` or `None` | `30 s` | Cap on `robots.txt` `Crawl-delay` (`Duration::ZERO` ignores it) |
| `robots_max_age(d)` or `None` | `24 h` | Re-fetch interval for `robots.txt` |
| `dedup(bool)` | `true` | Skips canonical aliases and duplicate-text pages |
| `dedup_min_words(n: usize)` or `None` | `20` | Minimum words for duplicate-text matching (`None` disables text dedup) |
| `host_limits(HostLimits)` | session's limits plus the polite defaults above | Replaces the crawl's own pace. The session's `HostLimits` still apply. Pass `HostLimits::new()` to drop the polite defaults |

### Crawl rules and defaults

- **URL checks:** The crawl drops non-`http(s)`, out-of-scope, `filter`-rejected, and already-seen URLs.
- **Duplicate URLs:** Dedup compares URLs without fragments, with sorted query parameters and normalized percent-encoding. Requests still use the URL as written.
- **Redirects:** A redirect target goes through the same checks. A seed that redirects to a sibling host widens the scope. A seed that redirects cross-site yields only that page.
- **Content & dedup:** Skips block pages (`Response::block()`), non-2xx statuses, and non-HTML (sniffs missing `Content-Type` per WHATWG MIME Sniffing). Holds canonical aliases until their canonical target resolves, and deduplicates pages of ≥ `dedup_min_words` words.
- **Sitemaps & robots:** Fetches `robots.txt` before crawling each site (retried under the session `Retry`, or, when unset, up to 4 times on transient errors and 5xx with jittered backoff from 1 s to 8 s), then reads listed sitemaps (or `/sitemap.xml`), supporting plain text, gzip, and sitemap indexes while ignoring media extension `<loc>` tags. Missing sitemaps are not errors; failed sitemaps increment `errors()`.
- **Credentials, pacing & multi-host seeds:** Sends session `Authorization` / `bearer_auth` / default `Cookie` headers only to the seed's origin (jar cookies still follow RFC 6265 rules). Requests wait for both session and crawl `HostLimits`. To crawl multiple seed domains concurrently, spawn one `session.crawl(seed)` per domain on a shared cloned `Session` so all crawls share connection pools, `HostLimits`, and proxy health.
- **Lifecycle:** Transport errors are yielded as `Err` without stopping the crawl. `Session::shutdown()` ends the crawl cleanly; dropping `Crawl` cancels in-flight requests.

### Inspect pages and skip counts

Each `Page` exposes `url()` (final URL), `depth()`, `status()`, `headers()`
(sensitive headers such as `Set-Cookie` stripped), `header(name)`, `html()`,
`text()` (from `html::text`), and `into_html()`.

`Crawl` counters return `u64`: `requests()`, `pages()`, `errors()`, and
`skipped(SkipReason)`:

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

`crawl.save(path).await` (async) stops scheduling new work, drains active
fetches, and writes an atomic JSON checkpoint. The file is versioned, has mode
`0600` on Unix, and may be up to 64 MiB, or the session's `max_body_size` when
that is larger. `Crawl::load(&session, path)` (synchronous, no `.await`)
restores it.

`save` fails with `Kind::Config` and keeps the old checkpoint if the crawl holds
an error you have not read with `next()`. That includes an error that arrives
while `save` waits for active fetches.

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
worker tasks. To use it with `session.crawl(seed)`, pass
`.host_limits(HostLimits::new())` if the crawl's polite defaults should not
fill the fields your session leaves unset.

- **Polite single-IP crawl:** set `HostLimits::pause_on([429, 503])` and stop retrying on blocks (`!attempt.is_block()`).
- **Rotating proxy pool:** retry blocks and leave out `pause_on`. See [Use a proxy list](proxies.md#use-a-proxy-list).

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
    .cookies(None)
    .host_limits(
        HostLimits::new()
            .max_in_flight(4)
            .per_second(2.0)
            .max_total_in_flight(64),
    )
    .proxy(proxies)
    .block_statuses([403, 429])
    .retry(Retry::new(3).when(|attempt| attempt.is_transient() || attempt.is_block()))
    .timeout(Timeout::new().body(Duration::from_secs(60)))
    .trace(Log)
    .trace(metrics)
    .build()?;
# let _ = session;
# Ok(())
# }
```

## Limit each host

`HostLimits` caps per-origin concurrency, request rate and queue depth (excess
fails with `Kind::Overloaded`), and can pause an origin on statuses you list
with `pause_on`. See
[Limit each host](network.md#limit-each-host) for the full setter reference and
examples.

## Obey robots.txt

`Session::robots(url, agent)` fetches `/robots.txt` for the origin of `url`
(RFC 9309, up to 512 KiB or `max_body_size`, following up to 5 redirects by
default) and returns `leyline::crawl::Robots`. Pass `"*"` for the wildcard
group. Fails only on an invalid URL or shut-down session.

| `robots.txt` fetch | `Robots::source()` | Result |
| --- | --- | --- |
| 2xx | `Parsed` | Matching group rules (case-insensitive `agent`, else `*`; longest pattern wins, `Allow` wins ties) |
| Other non-2xx (4xx except 429) | `Unavailable` | Every path allowed |
| 5xx, 429, or transport failure | `Unreachable` | Every path disallowed except `robots.txt` |

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

On each attempt, a block response strikes its proxy in a `Proxy::list`. Your
`Retry::when` closure decides whether to retry: return `attempt.is_block()` to
rotate on blocks, or `!attempt.is_block()` to stop. The final response still
exposes `resp.block()`. See [Use a proxy list](proxies.md#use-a-proxy-list).

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

For a custom URL stream, drive requests with
`futures_util::StreamExt::buffer_unordered(n)` and stop when
`session.is_shut_down()` is true. These pages cover the building blocks:

- **Proxy rotation & per-proxy identities:** [Use a proxy list](proxies.md#use-a-proxy-list) (`Proxy::list`, `Proxy::identified`)
- **Browser rotation & graceful shutdown:** [Sessions](sessions.md#derive-a-session) (`to_builder().browser(..)`), [Stop a session](sessions.md#stop-a-session)
- **Error classification:** [Errors](errors.md#match-on-the-kind) (`Error::kind()`, `attempts()`, `proxy()`)
- **Resumable capped file downloads:** [Download a file](streaming.md#download-a-file) (`RequestBuilder::download_to`)
- **Per-request tags, `Log`, `Metrics`, and `Session::stats()`:** [Logging and tracing](logging.md#count-requests-and-inspect-queues)
- **Pool and body memory bounds:** [Connection pool](network.md#connection-pool) and [Size cap and content coding](responses.md#size-cap-and-content-coding)
