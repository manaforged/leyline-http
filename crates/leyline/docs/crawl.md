A crawl from a seed URL that yields pages in order.

[`Session::crawl`](crate::Session::crawl) starts it; call [`next`](crate::Crawl::next) until it returns `None`. A crawl stays inside its [`Scope`](crate::Scope), honors `robots.txt` unless you change [`respect`](crate::Crawl::respect), and keeps per-host rate limits.

```rust,no_run
use leyline::{Scope, Session};

# async fn run() -> leyline::Result<()> {
let mut crawl = Session::new().crawl("https://example.com/docs/").scope(Scope::Prefix).limit(100);
while let Some(page) = crawl.next().await {
    let page = page?;
    println!("{} {}", page.status(), page.url());
}
# Ok(())
# }
```

See also: [Crawling](https://manaforged.github.io/leyline-http/guide/crawling.html).
