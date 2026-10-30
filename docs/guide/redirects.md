# Redirects

A session follows a `301`, `302`, `303`, `307`, or `308` response that has a
`Location` header, up to 10 per request. This chapter covers what a redirect
changes, how to limit or stop redirects, and how to decide each one.

## What a redirect changes

| Status | Next request |
| --- | --- |
| `301`, `302` | A `POST` becomes a `GET` with no body. Other methods keep the method and the body |
| `303` | Every method except `HEAD` becomes a `GET` with no body |
| `307`, `308` | Keeps the method and the body |

- A redirect that keeps a streamed request body fails with `Kind::Redirect`,
  because the stream cannot be sent twice.
- A `Location` with a scheme other than `http` or `https` fails with
  `Kind::Redirect`.
- When the redirect leaves the origin of the first request, the session
  removes the `Authorization`, `Proxy-Authorization`, and `Cookie` headers you
  set. The cookie jar adds the cookies that match the new URL.

A request that follows a URL the server chose, such as a `pages()` next link,
`Tab::follow`, or `Tab::submit_form`, drops the same credentials when its
origin differs from the page it came from: the `Authorization`, `Cookie`, and
`Proxy-Authorization` session defaults and the session bearer token.

`Response::url` is the final URL, and `Response::redirect_chain` lists the
URLs the session left, in order. See
[Responses](responses.md#final-url-and-redirects).

## Limit or stop redirects

`Redirect::limit(n)` sets the limit; `limit(10)` is the default.
`Redirect::none()` follows no redirect. At the limit, the session
returns the last `3xx` response, not an error. Set the policy on the session
with `SessionBuilder::redirect`, or on one request with
`RequestBuilder::redirect`. `Session::execute` reads a `Redirect` from
the request extensions, and `to_builder().redirect(..)` makes a session
with another policy.

```rust,no_run
use leyline::{Browser, Redirect, Session};

# async fn run() -> leyline::Result<()> {
let session = Session::builder()
    .browser(Browser::default())
    .redirect(Redirect::limit(3))
    .build()?;
let resp = session
    .post("https://example.com/login")
    .redirect(Redirect::none())
    .send().await?;
println!("{} {:?}", resp.status(), resp.header("location"));
# Ok(())
# }
```

## Decide each redirect

`Redirect::custom` calls your function for each redirect with an `Attempt`:
`status()`, `url()`, `location()`, and `previous()`, the URLs followed so far.
The URLs carry no user name or password. Return `true` to follow; on `false`,
the session returns the `3xx` response. A custom policy follows at
most 32 redirects; the next one fails with `Kind::Redirect`.

```rust,no_run
use leyline::{Browser, Redirect, Session};

# fn run() -> leyline::Result<()> {
let policy = Redirect::custom(|attempt| {
    let (Some(url), Some(location)) = (attempt.url(), attempt.location()) else {
        return false;
    };
    let same_host = url
        .join(location)
        .is_ok_and(|next| next.host_str() == url.host_str());
    same_host && attempt.previous().len() < 5
});
let session = Session::builder()
    .browser(Browser::default())
    .redirect(policy)
    .build()?;
# let _ = session;
# Ok(())
# }
```
