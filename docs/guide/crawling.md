# Crawling

`Session::crawl` crawls a site for you: it follows links in scope, obeys each
page's own rules, drops copies, and reads robots.txt when you ask it to. The rest of this page
covers the session settings every crawl uses: per-host limits, a proxy pool,
retries, block detection, capped downloads, one log line per request, and
counters for the whole crawl. Every task that shares the session shares the
limits and the proxy health.

## Crawl a site

`session.crawl(seed)` returns a `Crawl`, a lazy stream of `CrawlPage`s. The
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

That crawl stays on the seed's host, sends one request at a time to each host
and at most one a second, pauses a host that answers 429 or 503 and tries
that page up to three times, reads the
site's sitemaps, follows redirects through the same checks as links, skips
error pages, files, and block pages, and drops copies. It obeys robots.txt
and each page's robots rules unless you call `ignore_robots()`. Five
settings change the rest:

```rust,no_run
use leyline::{HostLimits, Respect, Scope, Session};

# async fn run() -> leyline::Result<()> {
let mut crawl = Session::new()
    .crawl("https://docs.example/guide/")
    .respect(Respect::AiTraining)
    .scope(Scope::Prefix)
    .limit(500)
    .host_limits(HostLimits::new().max_in_flight(2).per_second(2.0));
while let Some(page) = crawl.next().await {
    match page {
        Ok(page) => println!("{}", page.url()),
        Err(err) => eprintln!("{}: {}", err.url().map_or("-", |url| url.as_str()), err.category()),
    }
}
println!("{}", crawl.stats());
# Ok(())
# }
```

| Setter | Default | Effect |
| --- | --- | --- |
| `respect(Respect)` | `Robots` | `Robots` obeys robots.txt `Disallow` and `Crawl-delay`, the page's `noindex` and `nofollow`, and `rel="nofollow"`. `AiTraining` does all of that and also obeys the site's AI-training opt-outs |
| `ignore_robots()` | off | Obeys no robots rules: no `Disallow`, `Crawl-delay`, `noindex`, or `nofollow` |
| `scope(Scope)` | `Host` | `Host`: the seed's host. `Domain`: its registrable domain. `Prefix`: its host and directory |
| `limit(n)` | none | Stops after `n` kept pages |
| `host_limits(HostLimits)` | one in flight, one a second, and a 429/503 pause per host, wherever the session's own limits leave them unset | The crawl's pace per host, on top of the session's own limits, which every crawl and request of the session share. The value is used as given; `HostLimits::new()` adds no limits |

The crawl reads each site's robots.txt once, and again after 24 hours, for
its `Sitemap` lines; it obeys the rest unless you call `ignore_robots()`. Robots groups are
matched for the `*` agent. To read the group for a named crawler, call
`Session::robots(url, agent)`. At most 16 requests are in flight across all
hosts.

For each URL, in order, the crawl:

1. Skips a URL that is not `http` or `https`, is out of scope, or was queued
   before. URLs are compared after the fragment is dropped, the query
   parameters are sorted, and percent-encoding is normalized; the request
   goes out as written.
2. Unless robots are ignored, skips a URL that robots.txt disallows, and uses the site's
   `Crawl-delay` as the least spacing between its requests, up to 30
   seconds. With `Respect::AiTraining`, it skips a URL whose site refuses AI
   training in a `Content-Signal` line
   ([Cloudflare's format](https://developers.cloudflare.com/bots/additional-configurations/managed-robots-txt/))
   or a `Content-Usage` line (the IETF
   [aipref draft](https://datatracker.ietf.org/doc/draft-ietf-aipref-attach/)).
3. Sends the request without following redirects. A redirect is queued like
   a link, so its target passes the same checks; a seed redirected to
   another host of the same site adds the scope of its target, and a seed
   redirected to another site yields only that page. A chain longer than
   five redirects is skipped.
4. Skips a block page (`Response::block()`), an error status, and a response
   whose `Content-Type` is not HTML. A response without a `Content-Type` is
   kept when its body starts like HTML, by the
   [MIME Sniffing](https://mimesniff.spec.whatwg.org/) rules.
5. Queues the page's links. Unless robots are ignored, it reads the page's directives
   with `html::Directives::read`, queues nothing from a `nofollow` page, and
   skips links marked `rel="nofollow"`.
6. Unless robots are ignored, skips a `noindex` page. With `Respect::AiTraining`, it
   also skips a `noai` page and a page whose `Content-Usage` header refuses
   AI training. `noai` is a convention some sites use, not a standard. A
   page that names another in-scope page as its canonical URL waits until
   the crawl has that page: it is dropped when the canonical page is kept,
   and kept when the canonical page is skipped or cannot be fetched. A page
   of 20 words or more is skipped when its text, compared word by word,
   matches a kept page.

The crawl reads the sitemaps that each site's robots.txt lists, or
`/sitemap.xml` when it lists none (and, unless robots are ignored, when robots.txt
allows it). Unless robots are ignored, it opens only sitemap files that their own
site's robots.txt allows, and reads that robots.txt first for a sitemap on
another host. It follows a sitemap index to the sitemaps it
names on the same host, reads gzipped and plain-text sitemaps, and reads at most 50 sitemap files and 50 MiB per file;
with a `limit`, it opens no new sitemap file once it has queued that many
URLs. Each `<loc>` URL passes the same checks as a link; image, video, and
news `<loc>` entries in a sitemap's extension namespaces are not pages and
are skipped.

A request that fails comes out of the stream as an `Err`, and the crawl goes
on. `crawl.stats()` returns `CrawlStats`: `requests()`, `pages()`,
`errors()`, and `skipped(SkipReason)` for each reason above. Its `Display`
prints one `key=value` line. `CrawlPage` has `url()`, the address after
redirects, `depth()`, `status()`, `html()`, `text()` from `html::text`, and
`meta(name)`. A page body is capped by the session's
`CompressionConfig::max_body_size`, 100 MiB by default.

`Session::shutdown()` ends the crawl: requests in flight fail with the
shut-down error and nothing new starts. Dropping the `Crawl` cancels its
requests. `Session::crawl` needs the `html` feature, which is on by default.

## Build the crawl session

```rust,no_run
use std::time::Duration;
use leyline::trace::{Fanout, Metrics, TracingTrace};
use leyline::{
    BlockRules, Browser, HostLimits, ProxyPool, RetryPolicy, Session, TimeoutConfig,
};

# fn run() -> leyline::Result<()> {
let blocks = {
    let mut rules = BlockRules::builtin().clone();
    rules.extend(BlockRules::statuses([403, 429]));
    rules
};
let proxies = ProxyPool::new([
    "http://user:pass@proxy-a.example:8080",
    "http://user:pass@proxy-b.example:8080",
    "http://user:pass@proxy-c.example:8080",
])
.sticky_for(Duration::from_secs(600))
.ban_after(3)
.ban_for(Duration::from_secs(120))
.rotate_on_block(blocks.clone());
let metrics = Metrics::new();

let session = Session::builder()
    .browser(Browser::default())
    .host_limits(
        HostLimits::new()
            .max_in_flight(4)
            .per_second(2.0)
            .max_total_in_flight(64)
            .pause_on([429, 503]),
    )
    .proxy_pool(proxies.clone())
    .retry(RetryPolicy::transient().skip_blocks(blocks))
    .timeout(TimeoutConfig::new().body(Duration::from_secs(60)))
    .trace(Fanout::new().with(TracingTrace).with(metrics.clone()))
    .build()?;
# let _ = session;
# Ok(())
# }
```

Clone the session into each worker task. A clone is an `Arc` clone: it shares
the connection pool, the cookie jar, the host limits, and the proxy pool.
`TimeoutConfig::body` bounds each body from its first read, so a slow page fails with a
timeout error. See
[Retries and timeouts](retries-and-timeouts.md#bound-a-whole-download).

## Limit each host

`HostLimits` applies per origin (scheme, host, and port), across HTTP/1.1,
HTTP/2, and HTTP/3. A host name with a trailing dot is the same host as the
name without it. `HostLimits::default()` has no limits.

`HostLimits` is configuration. Each session that you build from it keeps its
own counters, queues, and pauses. Clones and derived sessions of that session
share them; a second session built from the same `HostLimits` value does not.

| Setter | Effect |
| --- | --- |
| `max_in_flight(n)` | At most `n` requests to the origin at the same time. The smallest value is 1 |
| `per_second(rate)` | Requests to the origin start at least `1 / rate` seconds apart. A rate of 0 or less turns the spacing off |
| `max_total_in_flight(n)` | At most `n` requests at the same time across all hosts. The smallest value is 1 |
| `host(host, limits)` | Replaces `max_in_flight` and `per_second` for one host. `*.example.com` matches every subdomain, not `example.com` itself. An exact host wins over a wildcard, and a longer wildcard over a shorter one |
| `pause_on(statuses)` | A response with one of these statuses stops new requests to the origin that sent it, for the wait the server asks for. Requests in flight continue |
| `pause_for(duration)` | The pause when the response asks for no wait. The default is 60 s |
| `max_pause(duration)` | The longest pause a server can ask for. A longer request is cut to this value. The default is 24 h |

The server's wait comes from the same headers the retry policy reads: each
`RetryPolicy::wait_header`, then `Retry-After`. See
[Retries and timeouts](retries-and-timeouts.md#retry-on-a-condition-and-wait-on-a-header).

```rust
use leyline::HostLimits;

let limits = HostLimits::new()
    .max_in_flight(4)
    .per_second(2.0)
    .host("api.shop.example", HostLimits::new().max_in_flight(1).per_second(0.5))
    .host("*.cdn.example", HostLimits::new().max_in_flight(16))
    .max_total_in_flight(64);
# let _ = limits;
```

An override uses only the `max_in_flight` and `per_second` you pass; a
setting it leaves out has no limit for that host. The global cap applies to
every host.

A request is admitted in this order:

1. It waits for an in-flight slot of its origin.
2. It waits while the origin is paused, then for the origin's next rate slot.
3. It waits for a global slot. A request that loses its turn while it waits
   for the global slot goes back to step 2.

No request holds a global slot while it waits for a pause or the rate
spacing. The rate spacing is measured when a request is admitted, and a pause
is checked again just before admission.

The wait counts against the request deadline, and ends in `Kind::Timeout`
when the deadline passes. Each retry waits its turn again. Each redirect hop
is admitted against the origin it goes to, and releases the slot of the hop
before. `pause_on` and the proxy pool's strikes apply to the origin that sent
the response, which after a redirect is not the origin of the first URL. A buffered request releases its slot when
its body has been read; a `.stream()` request keeps it until the body ends,
fails, or is dropped. A streamed body ends at its first error and releases
the slot. A failed attempt releases its slot before the retry wait. Dropping
a waiting request removes it from the queue.

`max_in_flight` counts requests to one origin across every proxy. The
connection pool then applies `PoolConfig::max_h1_conns_per_host` to each
host, port, scheme, proxy, and transport. For HTTP/1.1 through one proxy, or
direct, the smaller limit bounds the requests in flight. HTTP/2 and HTTP/3
multiplex on one connection, so only `max_in_flight` bounds them. See
[Connection pool](network.md#connection-pool).

## Obey robots.txt

`Session::robots(url, agent)` fetches `/robots.txt` for the origin of `url`
and returns `Robots`, the rules for the product token `agent`. Pass `"*"` to
read only the group for every crawler. The rules follow RFC 9309:

| Robots.txt fetch | `Robots::source()` | Result |
| --- | --- | --- |
| 2xx | `Parsed` | The rules of the matching groups |
| 4xx | `Unavailable` | Every path is allowed |
| 5xx, or the request failed | `Unreachable` | Every path is disallowed |

A crawl that obeys robots rules holds a site's pages until its robots.txt is
read, and retries an unreachable robots.txt four times, 1, 2, 4, and 8
seconds apart, before it skips the site. A crawl with `ignore_robots()` does
not wait for robots.txt; it reads the file only for its `Sitemap` lines.

Up to five redirects are followed, and the rules apply to the origin you
asked about. The first 512 KiB of the file are parsed. `Session::robots`
returns an error only for an invalid URL or a shut-down session.

The groups that name `agent`, compared without case, are merged; with no such
group, the `*` groups apply. A path matches the longest rule, and `Allow`
wins a tie. `*` matches any run of characters, `$` ends a pattern, and both
sides are compared with percent-encoding normalized. `/robots.txt` itself is
always allowed.

```rust,no_run
use leyline::{Purpose, Url};

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

`Robots::parse(text, agent)` reads a file you already have.
`allows_use(url, purpose)` reads the AI-use preferences of the matching
group. A `Content-Signal` line uses `search`, `ai-input`, and `ai-train` with
`yes` or `no`; a `Content-Usage` line, from the IETF aipref draft, uses
`search`, `ai-use`, and `train-ai` with `y` or `n`, and can start with a path
pattern that limits it to matching URLs. Both map to `Purpose`. For a
use, the line with the longest path pattern that matches the URL decides;
on a tie, no wins, and a use no line names is allowed. Unknown labels are
ignored. The aipref labels are a draft and may
change. `crawl_delay()` returns the group's `Crawl-delay`; `Session::crawl`
applies it unless robots are ignored. `sitemaps()` lists every `Sitemap` line in
the file.

## Rotate proxies with a pool

A `ProxyPool` keeps an origin on one proxy, strikes and bans failing proxies,
and moves each retry to another healthy proxy. Its settings, strike rules,
`stats()`, and `ProxyPool::identified` are described in
[Use a proxy pool](proxies.md#use-a-proxy-pool).

## Detect a block page

A block is a response, not an error. `Response::block()` checks it against
built-in vendor rules and returns a `BlockSignal` with a `vendor` and a
`kind`. A bare 403 or 429 is not a block signal.

| Vendor rule | Status | Header | Kind |
| --- | --- | --- | --- |
| Cloudflare | any | `cf-mitigated: challenge` | `BlockKind::Challenge` |
| AWS WAF | 202 | `x-amzn-waf-action: challenge` | `BlockKind::Challenge` |
| AWS WAF | 405 | `x-amzn-waf-action: captcha` | `BlockKind::Captcha` |

`BlockRules::from_toml` adds rules of your own. Each `[[rule]]` table has
`vendor` and `kind` (`challenge`, `captcha`, or `block`), and optional
`status`, `header`, and `value`. The value match ignores ASCII case.
`BlockRules::statuses([403, 429])` makes bare statuses count as blocks, with
`BlockKind::Block` and vendor `"status"`. Rules match in order, so add them
after the vendor rules. `BlockRules::builtin()` returns a reference: clone it
before `extend`. `rules.check(&resp)` checks a response against a set. Clones
are cheap, because the rules sit behind an `Arc`.

```rust,no_run
use leyline::{BlockKind, BlockRules};

# async fn run(session: leyline::Session) -> leyline::Result<()> {
let mut rules = BlockRules::builtin().clone();
rules.extend(BlockRules::from_toml(
    r#"
[[rule]]
vendor = "shop"
kind = "block"
status = 403
header = "x-shop-block"
value = "1"
"#,
)?);
rules.extend(BlockRules::statuses([403, 429]));

let resp = session.get("https://shop.example/item/1").send().await?;
match rules.check(&resp) {
    Some(signal) if signal.kind == BlockKind::Captcha => println!("captcha from {}", signal.vendor),
    Some(signal) => println!("blocked by {}", signal.vendor),
    None => println!("{} bytes", resp.text().await?.len()),
}
# Ok(())
# }
```

### Retry a block through another proxy

One `BlockRules` value drives three settings, in this order on each attempt:

1. `ProxyPool::rotate_on_block` sees every attempt. A block is a strike on
   the proxy that carried it and moves the origin off that proxy.
2. `RetryPolicy` decides whether to send again, through another proxy.
   `skip_blocks(rules)` returns a block at once, without a retry, even when
   its status is a trigger such as 429. Calling it again adds rules.
3. `rules.check(&resp)` in your code reports the block on the last response.

Use `skip_blocks` when a retry only repeats the block, as in
[Build the crawl session](#build-the-crawl-session): the pool moves the
origin and your code decides what to do. A status that the retry policy
retries never reaches your code while retries remain. A Cloudflare challenge
is a 403, so a policy with `on_status(403)` retries it before you see it.

Without a pool, `RetryPolicy::rotate_proxies` with `on_status(403)` sends
each retry through the next proxy in a list. See
[Rotate proxies on retry](proxies.md#rotate-proxies-on-retry).

## Read the text of a page

`html::text(document)` returns the text a reader sees, one block per line:
each heading, paragraph, list item, and table cell starts a new line, and
`<br>` breaks a line. Runs of whitespace collapse to one space, except inside
`<pre>`, which keeps its lines and indentation. Entities are decoded. The text
of `<head>`, `<script>`, `<style>`, `<noscript>`, `<template>`, `<svg>`, and
`<textarea>` is left out. It needs the `html` feature, which is on by default.

```rust,no_run
use leyline::html;

# async fn run(session: leyline::Session) -> leyline::Result<()> {
let page = session.get("https://shop.example/about").send().await?.text().await?;
for line in html::text(&page).lines() {
    println!("{line}");
}
# Ok(())
# }
```

`html::text` keeps navigation, footers, and other text that repeats on every
page of a site. To drop it, compare lines across pages and remove the lines
that most pages share.

## Obey a page's own rules

A page can carry rules of its own, in `<meta name="robots">` tags and in
`X-Robots-Tag` headers. `html::Directives::read(document, headers, agent)`
reads both and returns `Directives`:

| Method | `false` when the page says |
| --- | --- |
| `index()` | `noindex` or `none`: do not keep the page |
| `follow()` | `nofollow` or `none`: do not follow its links |
| `ai()` | `noai`: keep the page out of AI datasets |

A meta tag named `robots`, or named for `agent` without case, applies. An
`X-Robots-Tag` value that starts with a product token and a colon, such as
`examplebot: noindex`, applies only to that token. Other directives, such as
`noarchive` or `max-snippet`, are read and ignored.

`html::canonical(document)` returns the `href` of the first
`<link rel="canonical">`, as written in the page. Join it with
`Response::url()` before you compare URLs.

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
    println!("keep {}", canonical.unwrap_or(url));
}
# Ok(())
# }
```

## Group errors

`Error::category()` tells you what to do with a failed URL. See
[Errors](errors.md) for every category.

| Category | Typical action |
| --- | --- |
| `Timeout`, `Connect`, `Tls` | Retry later |
| `Proxy` | The proxy failed. The pool already counted a strike |
| `ProxyTarget` | The proxy works, but the origin is not reachable through it |
| `Dns` | Drop the URL or the host |
| `Status` | An error status from `error_for_status` or `download` |
| `BodyLimit` | The body passed `max_body_size` or your limit |
| Other categories | Log the error and drop the URL |

```rust,no_run
use leyline::{ErrorCategory, Session};

enum Next {
    Done(String),
    RetryLater,
    Drop,
}

async fn crawl_one(session: &Session, url: &str) -> Next {
    let resp = match session.get(url).tag("listing").send().await {
        Ok(resp) => resp,
        Err(err) => {
            eprintln!(
                "{url}: {} after {} attempts via {}",
                err.category(),
                err.attempts(),
                err.proxy().unwrap_or("direct"),
            );
            return match err.category() {
                ErrorCategory::Timeout
                | ErrorCategory::Connect
                | ErrorCategory::Tls
                | ErrorCategory::Proxy
                | ErrorCategory::ProxyTarget => Next::RetryLater,
                _ => Next::Drop,
            };
        }
    };
    if resp.block().is_some() {
        return Next::RetryLater;
    }
    match resp.text().await {
        Ok(text) => Next::Done(text),
        Err(_) => Next::Drop,
    }
}
```

## Download files with a cap

`RequestBuilder::download(path, limit)` streams the decoded body to a
temporary file in the same directory, then renames it to `path`. A status of
400 or more becomes an error with the status, the URL, the headers, and the
start of the body, within the
[status-error limits](responses.md#turn-a-status-into-an-error). The limit is the smaller of `limit` and `max_body_size`.
When the body has no content coding and its `Content-Length` is over the
limit, the download fails with a body-limit error before it reads the body.
Any error or cancel removes the temporary file, so `path` never holds a
partial file. See [Streaming](streaming.md).

```rust,no_run
use std::time::Duration;

# async fn run(session: leyline::Session) -> leyline::Result<()> {
let written = session
    .get("https://shop.example/catalog.csv")
    .tag("catalog")
    .timeout(Duration::from_secs(300))
    .download("catalog.csv", Some(50 * 1024 * 1024))
    .await;
match written {
    Ok(bytes) => println!("{bytes} bytes"),
    Err(err) if err.is_timeout() => println!("download took too long"),
    Err(err) => return Err(err),
}
# Ok(())
# }
```

## Log each request with a tag

`RequestBuilder::tag(tag)` names a request, and `TracingTrace` writes one line
per request with the tag, the redacted proxy, and the browser of the session
that sent it. With `ProxyPool::identified`, that is the identity of the
proxy.

```text
INFO leyline::trace: request id=12 method="GET" url=https://shop.example/item/1 redirects=0 status=200 version=Http2 attempts=1 elapsed_ms=210 streamed=false tag="listing" proxy="http://user:***@proxy-a.example:8080/" browser=Chrome154 outcome="ok"
```

A `.stream()` request logs its body end as a separate event with the same
`id`. See [Logging and tracing](logging.md#streamed-bodies).

## Count requests and watch the queues

`trace::Metrics` counts what a session does. `Metrics::new()` returns an
`Arc<Metrics>`; pass a clone to the session, through `trace::Fanout` when you
also log, and keep one to read. `snapshot()` returns a `MetricsSnapshot`:

| Method | Count |
| --- | --- |
| `requests()`, `attempts()` | Requests, and attempts with retries |
| `status_class(n)` | Responses with status `n`xx |
| `errors(category)`, `errors_total()` | Errors per `ErrorCategory`, and all errors |
| `bodies_complete()`, `bodies_failed()`, `bodies_dropped()` | Streamed body outcomes |
| `latency()` | `(Option<Duration>, u64)` pairs: the upper bound of each bucket and its count |

The latency bounds are 10, 50, 100, 250, 500, 1000, 2500, 5000, and 10000 ms;
a request falls in the first bucket whose bound is at least its elapsed time,
and the last pair, with bound `None`, counts every request over 10 s.
`Display` of a snapshot prints one `key=value` line.

`Session::host_stats()` returns one `HostStats` per origin that the host
limits track, with `origin()`, `in_flight()`, and `waiting()`. A paused
origin's requests count as `waiting`. `Session::pool_stats()` returns
`PoolStats` with `busy` and `idle` connections. An HTTP/2 or HTTP/3
connection is busy while it has an open stream; on HTTP/1.1, `busy` counts
the requests that hold a connection permit.

```rust,no_run
use std::time::Duration;

use leyline::ErrorCategory;
use leyline::trace::Metrics;

# async fn run(session: leyline::Session, metrics: std::sync::Arc<Metrics>) {
loop {
    tokio::time::sleep(Duration::from_secs(10)).await;
    let snapshot = metrics.snapshot();
    println!("{snapshot}");
    if snapshot.errors(ErrorCategory::Proxy) > 100 {
        break;
    }
    for host in session.host_stats() {
        println!("{} in_flight={} waiting={}", host.origin(), host.in_flight(), host.waiting());
    }
}
# }
```

## Rotate identities

`Session::with_identity` derives a session with another browser identity.
Use one session per identity and spread the URLs over them.

```rust,no_run
use leyline::{Browser, Family, Identity, Platform};

# fn run(session: leyline::Session) -> leyline::Result<()> {
let firefox = session.with_identity(Identity::locked(
    Browser::latest(Family::Firefox),
    Platform::Windows,
))?;
let mac_chrome = session.with_identity(Identity::locked(
    Browser::latest(Family::Chrome),
    Platform::MacOS,
))?;
# let _ = (firefox, mac_chrome);
# Ok(())
# }
```

A derived session shares its parent's connection pool, in a partition of its
own, so an identity never reuses another identity's connection. One
`PoolConfig` budget covers every identity, and `pool_stats()` counts them
all. The cookie jar, the HSTS store, the `Alt-Svc` knowledge, the host
limits, and the proxy pool are shared too. Each identity has its own TLS
session cache. To give an identity its own cookies, add `with_cookie_jar`, or
use `ProxyPool::identified`. See [Sessions](sessions.md#derive-a-session).

## Stop the crawl

`Session::shutdown()` stops every clone and derived session: requests in
flight, new requests, and the next read of a body being streamed fail with
`Kind::Request` and the message "session shut down". `Error::is_shut_down()`
tells this error apart, and `Session::is_shut_down()` reports the state.
Shutdown does not close the pooled connections. To stop a single request,
drop its future or its streamed body. See [Cancellation](cancellation.md).

`buffer_unordered(n)` from `futures_util` keeps at most `n` request futures
alive, so a large URL list does not hold one task per URL, and
`max_total_in_flight` keeps the session's cap across every caller. Check
`is_shut_down()` before each URL to stop reading the list:

```rust,no_run
use futures_util::future;
use futures_util::stream::{self, StreamExt};

# async fn run(session: leyline::Session, urls: Vec<String>) {
let stop = session.clone();
tokio::spawn(async move {
    if tokio::signal::ctrl_c().await.is_ok() {
        stop.shutdown();
    }
});

let open = session.clone();
let mut results = stream::iter(urls)
    .take_while(move |_| future::ready(!open.is_shut_down()))
    .map(|url| {
        let session = session.clone();
        async move { (session.get(&url).await, url) }
    })
    .buffer_unordered(32);
while let Some((result, url)) = results.next().await {
    match result {
        Ok(resp) => println!("{url}: {}", resp.status()),
        Err(err) if err.is_shut_down() => {}
        Err(err) => eprintln!("{url}: {}", err.category()),
    }
}
# }
```

A URL stream built with `stream::unfold` over an async reader is not `Unpin`.
Pin the combined stream with `std::pin::pin!` before you call `.next()`.

## Memory bounds

| Item | Bound |
| --- | --- |
| Buffered body | `CompressionConfig::max_body_size`, 100 MiB by default |
| Body kept in a status error | `CompressionConfig::max_error_body`, 64 KiB by default |
| Idle connections | `PoolConfig::idle_timeout`, 300 s by default |
| Connections | `PoolConfig::max_connections` per pool, 2048 by default |
| HTTP/1.1 connections per host | `PoolConfig::max_h1_conns_per_host`, 256 by default |
| Requests that wait for an HTTP/2 stream | 1024 per connection |
| Origins remembered as HTTP/3 | 1024 per pool |
| Origins remembered as HTTP/1.1-only | 1024 per pool |

Stream large bodies with `download` or `copy_decoded_to`, so only one chunk is
in memory.

## Next

Read [Accounts](accounts.md).
