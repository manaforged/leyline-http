A session that keeps the current page, as a browser tab does.

Each request from a tab except `open` sends the current page as its `from_page`, so the request headers match the kind of request. [`open`](crate::Tab::open) loads a typed URL, [`follow`](crate::Tab::follow) clicks a link, [`submit`](crate::Tab::submit) posts form fields, and [`submit_form`](crate::Tab::submit_form) posts an [`html::Form`](crate::html::Form) read from the page; each sets the next page. [`fetch`](crate::Tab::fetch) is a script request and leaves the page alone; add `.kind(..)` for an image or script. Relative URLs resolve against the current page. Get one with [`Session::tab`](crate::Session::tab); it shares the session's cookies and connections, and clones of a tab share the page.

```rust,no_run
use leyline::{Browser, Session};

# async fn run() -> leyline::Result<()> {
let tab = Session::from(Browser::Chrome154).tab();
tab.open("https://example.com/").await?;
let next = tab.follow("https://example.com/about").await?;
# drop(next);
# Ok(())
# }
```

# Errors

`fetch` on a tab with no page fails with [`Kind::Request`](crate::Kind::Request) and sends nothing.

See also: [`Session`](crate::Session), [Requests](https://manaforged.github.io/leyline-http/guide/requests.html#request-kinds).
