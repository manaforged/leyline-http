# Crawling

`Session::crawl` crawls a site for you: it follows links in scope, obeys each
page's own rules, drops copies, and obeys robots.txt unless you turn it off. The rest of this page
covers the session settings every crawl uses: per-host limits, a proxy pool,
retries, block detection, capped downloads, one log line per request, and
counters for the whole crawl. Every task that shares the session shares the
limits and the proxy health.

## Crawl a site

`session.crawl(seed)` returns a `Crawl`, a lazy stream of `Page`s. The
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
and each page's robots rules unless you call `respect(None)`. Five
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
        Err(err) => eprintln!("{}: {}", err.url().map_or("-", |url| url.as_str()), err.kind()),
    }
}
println!("{} pages, {} errors", crawl.pages(), crawl.errors());
# Ok(())
# }
```

| Setter | Default | Effect |
| --- | --- | --- |
| `respect(Respect)` or `respect(None)` | `Robots` | `Robots` obeys robots.txt `Disallow` and `Crawl-delay`, the page's `noindex` and `nofollow`, and `rel="nofollow"`. `AiTraining` does all of that and also obeys the site's AI-training opt-outs. `None` obeys no robots rules: no `Disallow`, `Crawl-delay`, `noindex`, or `nofollow` |
| `scope(Scope)` | `Host` | `Host`: the seed's host. `Domain`: its registrable domain. `Prefix`: its host and directory; a last path segment with a dot and a letter after it is a file, so end a directory seed with a slash |
| `limit(n)` | none | Stops after `n` kept pages |
| `host_limits(HostLimits)` | one in flight, one a second, and a 429/503 pause per host, wherever the session's own limits leave them unset | The crawl's pace per host. A value you pass replaces the crawl's defaults, so `HostLimits::new()` adds no limits. The session's own limits, which every crawl and request of the session share, still apply on top |

Unless you call `respect(None)`, the crawl reads each site's robots.txt
once, and again after 24 hours, obeys its rules, and reads its `Sitemap`
lines. Robots groups are
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
   Both count as `SkipReason::Signal`.
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
   AI training, counted as `SkipReason::NoAi` and `SkipReason::Signal`. `noai` is a convention some sites use, not a standard. A
   page that names another in-scope page as its canonical URL waits until
   the crawl has that page: it is dropped when the canonical page is kept,
   and kept when the canonical page is skipped or cannot be fetched. A held page counts
   toward `limit`; when the limit leaves no room to fetch the canonical page,
   the held page is kept. A page
   of 20 words or more is skipped when its text, compared word by word,
   matches a kept page.

The crawl reads the sitemaps that each site's robots.txt lists on the same
site, or
`/sitemap.xml` when it lists none (and, unless robots are ignored, when robots.txt
allows it). Unless robots are ignored, it opens only sitemap files that their own
site's robots.txt allows, and reads that robots.txt first for a sitemap on
another host. It follows a sitemap index to the sitemaps it
names on the same host, reads gzipped and plain-text sitemaps, and reads at most 50 sitemap files and 50 MiB per file;
with a `limit`, it opens no new sitemap file once it has queued that many
URLs. A sitemap that redirects is queued again at its new address and
passes the same checks. Each `<loc>` URL passes the same checks as a link; image, video, and
news `<loc>` entries in a sitemap's extension namespaces are not pages and
are skipped.

A request that fails comes out of the stream as an `Err`, and the crawl goes
on. `Crawl` counts as it goes: `requests()`, `pages()`, `errors()`, and
`skipped(SkipReason)` for each reason above. `Page` has `url()`, the address after
redirects, `depth()`, `status()`, `html()`, `text()` from `html::text`, and
`into_html()`, which returns the body as an owned `String`. A page body is capped by the session's `max_body_size`, 100 MiB
by default.

`Session::shutdown()` ends the crawl: requests in flight fail with the
shut-down error and nothing new starts. Dropping the `Crawl` cancels its
requests. `Session::crawl` needs the `html` feature, which is on by default.

## Build the crawl session

```rust,no_run
use std::time::Duration;
use leyline::trace::{Metrics, Log};
use leyline::{
    Browser, HostLimits, Proxy, Retry, Session, Timeout,
};

# fn run() -> leyline::Result<()> {
let proxies = Proxy::list([
    "http://user:pass@proxy-a.example:8080",
    "http://user:pass@proxy-b.example:8080",
    "http://user:pass@proxy-c.example:8080",
])
.sticky_for(Duration::from_secs(600))
.ban_after(3)
.ban_for(Duration::from_secs(120));
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
    .proxy(proxies.clone())
    .block_statuses([403, 429])
    .retry(Retry::new(3).when(|attempt| attempt.is_transient() && !attempt.is_block()))
    .timeout(Timeout::new().body(Duration::from_secs(60)))
    .trace(Log)
    .trace(metrics.clone())
    .build()?;
# let _ = session;
# Ok(())
# }
```

Clone the session into each worker task. A clone is an `Arc` clone: it shares
the connection pool, the cookie jar, the host limits, and the proxy pool.
`Timeout::body` bounds each body from its first read, so a slow page fails with a
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
`Retry::wait_header`, then `Retry-After`. See
[Retries and timeouts](retries-and-timeouts.md#wait-on-a-header).

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

A host override uses only the `host_max_in_flight` and `host_per_second` set for it; a
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
connection pool then applies `max_connections_per_host` to each
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

A crawl sends the session's `Authorization` header (including
`bearer_auth`) and a `Cookie` default header only to the seed's origin;
requests to other hosts go without them. Cookies from the session's cookie jar
follow cookie rules, which match by host and path but not by port (RFC 6265,
section 8.5). Other default headers go to every host, so do not put a
credential in a custom default header for a crawl.

A crawl request waits for the session's own host limits and the crawl's pace
together, and is sent only when both allow it. A 429 or 503 that the crawl gets pauses the
host in both, so the session's own requests to that host wait out the same
pause. A host that is paused, waiting
for its next turn, or out of free slots holds no crawl slot. When a page or a
sitemap fails in a way the retry policy retries (the session's
`Retry`, or three tries on transient errors, 429, and 503 when it has
none), the crawl puts it back in the queue and tries again after the policy's
wait, without holding a slot. A redirect that leaves the scope counts as
`SkipReason::Scope`. A URL longer than 2048 bytes, or more than 64 links from
the seed, is dropped without a count.

A crawl that obeys robots rules holds a site's pages until its robots.txt is
read, and retries an unreachable robots.txt four times, waiting up to 1, 2,
4, and 8 seconds, before it skips the site. A crawl with `respect(None)` never
asks for robots.txt; it reads `/sitemap.xml` directly. A missing sitemap is not
an error; a sitemap that fails after its retries counts in `errors()`.

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

A proxy list keeps an origin on one proxy, strikes and bans failing proxies,
and moves each retry to another healthy proxy. Its settings, strike rules,
its counters in `Session::stats()`, and `Proxy::identified` are described in
[Use a proxy list](proxies.md#use-a-proxy-list).

## Classify a block response

`Response::block()` returns a `Block` when the response matches a block
rule, and `None` otherwise. A block is a response, not an error. A crawl skips
block pages and counts them under `SkipReason::Blocked`.

| Vendor | Status | Header | `Block` |
| --- | --- | --- | --- |
| Cloudflare | any | `cf-mitigated: challenge` | `Challenge("cloudflare")` |
| AWS WAF | 202 | `x-amzn-waf-action: challenge` | `Challenge("aws-waf")` |
| AWS WAF | 405 | `x-amzn-waf-action: captcha` | `Captcha("aws-waf")` |

`SessionBuilder::block_statuses([403, 429])` makes bare statuses blocks, as
`Block::Denied("status")`. `SessionBuilder::block_rules(toml)` adds rules.
Each `[[rule]]` table has `vendor` and `kind` (`challenge`, `captcha`, or
`denied`), and optional `status`, `header`, and `value`. The value match
ignores ASCII case. Rules match in order, after the built-in rules. Invalid
TOML fails at `build()` with `Kind::Config`.

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

On each attempt:

1. A block response counts as a failure for its proxy in a proxy list.
2. `Retry` decides whether to send again. `Attempt::is_block()` is true for a
   block response, so a rule with `!attempt.is_block()` returns blocks without
   a retry, as in [Build the crawl session](#build-the-crawl-session).
3. `resp.block()` returns the block for the response you get.

A status that the retry rule retries reaches your code only after the
retries run out. [Which proxy an attempt uses](proxies.md#which-proxy-an-attempt-uses)
covers how a retry picks its proxy.

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

`Error::kind()` tells you what to do with a failed URL. See
[Errors](errors.md) for every kind.

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
use leyline::{Kind, Session};

enum Next {
    Done(String),
    RetryLater,
    Drop,
}

async fn crawl_one(session: &Session, url: &str) -> Next {
    let resp = match session.get(url).label("listing").send().await {
        Ok(resp) => resp,
        Err(err) => {
            eprintln!(
                "{url}: {} after {} attempts via {}",
                err.kind(),
                err.attempts(),
                err.proxy().unwrap_or("direct"),
            );
            return match err.kind() {
                leyline::Kind::Timeout
                | leyline::Kind::Connect
                | Kind::Tls
                | Kind::Proxy
                | Kind::ProxyTarget => Next::RetryLater,
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

`Response::download_to(path)` streams the decoded body to a temporary file
in the same directory, then renames it to `path`. With
`RequestBuilder::error_for_status`, a status of 400 or more becomes an error
with the status, the URL, the headers, and the start of the body, within the
[status-error limits](responses.md#turn-a-status-into-an-error).
`RequestBuilder::max_body_size` sets the limit for this request. When the
body has no content coding and its `Content-Length` is over the limit, the
download fails with a body-limit error before it reads the body.
Any error or cancel removes the temporary file, so `path` never holds a
partial file. See [Streaming](streaming.md).

```rust,no_run
use std::time::Duration;

# async fn run(session: leyline::Session) -> leyline::Result<()> {
let written = session
    .get("https://shop.example/catalog.csv")
    .label("catalog")
    .timeout(Duration::from_secs(300))
    .error_for_status()
    .max_body_size(50 * 1024 * 1024)
    .stream()
    .send()
    .await?
    .download_to("catalog.csv")
    .await;
match written {
    Ok(bytes) => println!("{bytes} bytes"),
    Err(err) if err.kind() == leyline::Kind::Timeout => println!("download took too long"),
    Err(err) => return Err(err),
}
# Ok(())
# }
```

## Log each request with a tag

`RequestBuilder::label(text)` names a request, and `Log` writes one line
per request with the tag, the redacted proxy, and the browser of the session
that sent it. With `Proxy::identified`, that is the identity of the
proxy.

```text
INFO leyline::trace: request id=12 method="GET" url=https://shop.example/item/1 redirects=0 status=200 version=Http2 attempts=1 elapsed_ms=210 streamed=false label="listing" proxy="http://user:***@proxy-a.example:8080/" browser=Chrome154 outcome="ok"
```

A `.stream()` request logs its body end as a separate event with the same
`id`. See [Logging and tracing](logging.md#streamed-bodies).

## Count requests and watch the queues

`trace::Metrics` counts what a session does. `Metrics::new()` returns an
`Arc<Metrics>`; pass a clone to `trace`, which adds it beside any other
hook, and keep one to read it:

| Method | Count |
| --- | --- |
| `requests()`, `attempts()` | Requests, and attempts with retries |
| `status_class(n)` | Responses with status `n`xx |
| `errors(category)`, `errors_total()` | Errors per `Kind`, and all errors |
| `bodies_complete()`, `bodies_failed()`, `bodies_dropped()` | Streamed body outcomes |
| `latency()` | `(Option<Duration>, u64)` pairs: the upper bound of each bucket and its count |

The latency bounds are 10, 50, 100, 250, 500, 1000, 2500, 5000, and 10000 ms;
a request falls in the first bucket whose bound is at least its elapsed time,
and the last pair, with bound `None`, counts every request over 10 s.
`Display` of `Metrics` prints one `key=value` line.

`Session::stats()` returns a `Stats` for the moment of the call. `hosts()`
lists each origin the host limits track, and `in_flight(host)` and
`waiting(host)` count its requests. A paused origin's requests count as
`waiting`. `busy()` and `idle()` count connections. An HTTP/2 or HTTP/3
connection is busy while it has an open stream; on HTTP/1.1, `busy` counts
the requests that hold a connection permit.

```rust,no_run
use std::time::Duration;

use leyline::Kind;
use leyline::trace::Metrics;

# async fn run(session: leyline::Session, metrics: std::sync::Arc<Metrics>) {
loop {
    tokio::time::sleep(Duration::from_secs(10)).await;
    println!("{metrics}");
    if metrics.errors(Kind::Proxy) > 100 {
        break;
    }
    let stats = session.stats();
    for host in stats.hosts() {
        println!("{host} in_flight={} waiting={}", stats.in_flight(host), stats.waiting(host));
    }
}
# }
```

## Rotate browsers

`session.to_builder().browser(..).platform(..)` makes a session with another browser and platform.
Use one session per browser and spread the URLs over them.

```rust,no_run
use leyline::{Browser, Platform};

# fn run(session: leyline::Session) -> leyline::Result<()> {
let firefox = session.to_builder().browser(Browser::firefox()).platform(Platform::Windows).build()?;
let mac_chrome = session.to_builder().browser(Browser::chrome()).platform(Platform::MacOS).build()?;
# let _ = (firefox, mac_chrome);
# Ok(())
# }
```

A derived session shares its parent's connection pool, in a partition of its
own, so an identity never reuses another identity's connection. One
pool budget covers every identity, and `stats()` counts them
all. The cookie jar, the HSTS store, the `Alt-Svc` knowledge, the host
limits, and the proxy pool are shared too. Each identity has its own TLS
session cache. To give an identity its own cookies, add
`to_builder().cookies`, or use `Proxy::identified`. See [Sessions](sessions.md#derive-a-session).

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
        async move { (session.get(&url).send().await, url) }
    })
    .buffer_unordered(32);
while let Some((result, url)) = results.next().await {
    match result {
        Ok(resp) => println!("{url}: {}", resp.status()),
        Err(err) if err.is_shut_down() => {}
        Err(err) => eprintln!("{url}: {}", err.kind()),
    }
}
# }
```

A URL stream built with `stream::unfold` over an async reader is not `Unpin`.
Pin the combined stream with `std::pin::pin!` before you call `.next()`.

## Memory bounds

| Item | Bound |
| --- | --- |
| Buffered body | `max_body_size`, 100 MiB by default |
| Body kept in a status error | `max_error_body_size`, 64 KiB by default |
| Idle connections | `pool_idle_timeout`, 300 s by default |
| Connections | `max_connections` per pool, 2048 by default |
| HTTP/1.1 connections per host | `max_connections_per_host`, 256 by default |
| Requests that wait for an HTTP/2 stream | 1024 per connection |
| Origins remembered as HTTP/3 | 1024 per pool |
| Origins remembered as HTTP/1.1-only | 1024 per pool |

Stream large bodies with `download_to` or `write_to`, so only one chunk is in
memory.

## Next

Read [Accounts](accounts.md).
