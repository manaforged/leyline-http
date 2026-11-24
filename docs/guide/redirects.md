# Redirects

A session follows a `301`, `302`, `303`, `307`, or `308` response that has a
`Location` header, up to 10 per request. This chapter covers what a redirect
changes, how to limit or stop redirects, and how to decide each one.

## What a redirect changes

| Status | Next request |
| --- | --- |
| `301`, `302` | `POST` becomes `GET` with no body and strips body headers (`content-length`, `content-type`, `content-encoding`, `content-language`, `content-location`). Other methods keep method and body |
| `303` | Every method except `HEAD` becomes `GET` with no body and strips body headers |
| `307`, `308` | Keeps the method and the body |

- A redirect that keeps a streamed request body fails with `Kind::Redirect`, because the stream cannot be replayed.
- A `Location` with a scheme other than `http` or `https` fails with `Kind::Redirect`.
- When a server-chosen URL — a redirect, a `pages()` next link, `Tab::follow`, or `Tab::submit_form` — leaves the origin it came from, Leyline strips `Authorization`, `Proxy-Authorization`, `Cookie`, and the session bearer/basic auth (custom headers such as `x-api-key` are not stripped automatically). The cookie jar then adds cookies matching the new URL.

`Response::url()` is the final URL, and `Response::redirect_chain()` lists the
URLs followed in order. See [Responses](responses.md#final-url-and-redirects).

## Limit or stop redirects

| Policy | Behavior |
| --- | --- |
| `Redirect::limit(n)` or `n` (`usize`) | Follows up to `n` redirects (default `10`). At the limit, returns the `3xx` response, not an error |
| `Redirect::none()` | Follows no redirects and returns the `3xx` response |
| `Redirect::custom(f)` | Calls `f(&Attempt)` per hop; hard cap of 32 hops (the 33rd fails with `Kind::Redirect`) |

Set the policy with `SessionBuilder::redirect`, override it per request with
`RequestBuilder::redirect` (or a `Redirect` extension on `http::Request` for
`Session::execute`, `request_from`, and Tower `Service`), or derive a new
session with `to_builder().redirect(..)`.

```rust,no_run
use leyline::{Redirect, Session};

# async fn run() -> leyline::Result<()> {
let session = Session::builder()
    .redirect(3)
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

`Redirect::custom` calls your closure for each hop with a `leyline::Attempt`
(URLs have userinfo stripped). `Attempt` is shared with `Retry::when`, so
during redirects only `status()`, `url()`, `location()`, and `previous()` are
populated (`response()` and `error()` are `None`). Return `true` to follow, or
`false` to return the `3xx` response:

| Method | Return type | Value |
| --- | --- | --- |
| `status()` | `Option<StatusCode>` | Redirect status (`301`..`308`) |
| `url()` | `Option<&Url>` | URL that returned the redirect |
| `location()` | `Option<&str>` | Raw `Location` header value |
| `previous()` | `&[Url]` | URLs already followed before this hop |

```rust,no_run
use leyline::{Redirect, Session};

# fn run() -> leyline::Result<()> {
let policy = Redirect::custom(|attempt| {
    let (Some(url), Some(location)) = (attempt.url(), attempt.location()) else {
        return false;
    };
    let same_origin = url
        .join(location)
        .is_ok_and(|next| next.origin() == url.origin());
    same_origin && attempt.previous().len() < 5
});
let session = Session::builder()
    .redirect(policy)
    .build()?;
# let _ = session;
# Ok(())
# }
```
