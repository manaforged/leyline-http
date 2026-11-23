A crawl from a seed URL that yields pages in order.

[`Session::crawl`](crate::Session::crawl) starts it; call [`next`](crate::crawl::Crawl::next) until it returns `None`. A crawl stays inside its [`Scope`](crate::crawl::Scope) and follows only the links its [`filter`](crate::crawl::Crawl::filter) accepts, honors `robots.txt` unless you change [`respect`](crate::crawl::Crawl::respect), and keeps polite per-host and global defaults that [`host_limits`](crate::crawl::Crawl::host_limits) and the setters on `Crawl` and [`SessionBuilder`](crate::SessionBuilder) can tune or lift.

`save(path).await` drains current fetches and writes an atomic checkpoint. `Crawl::load(&session, path)` restores queued and buffered pages without replaying completed URLs. A checkpoint does not save the `filter`, so set it again after `Crawl::load`. Save account cookies separately with `Session::save`.

```rust,no_run
use leyline::Session;
use leyline::crawl::Scope;

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
