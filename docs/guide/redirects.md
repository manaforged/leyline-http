# Redirects

A session follows redirects by default, up to 10 per request. It follows a
`301`, `302`, `303`, `307`, or `308` response that has a `Location` header.

## What a redirect changes

- After a `301`, `302`, or `303`, the next request is a `GET` with no body.
- After a `307` or `308`, the next request keeps the method and the body.
- A streamed request body cannot be sent twice. A `307` or `308` on a streamed
  body fails with `Kind::Redirect`.
- A `Location` with a scheme other than `http` or `https` fails with
  `Kind::Redirect`.
- When the redirect leaves the origin of the first request, the session
  removes the `Authorization`, `Proxy-Authorization`, and `Cookie` headers
  that you set. The cookie jar still adds the cookies that match the new URL.

`Response::redirect_chain` lists the URLs that the session left, in order, as
`url::Url` values without a user name or password. `Response::url` is the final
URL, also a `url::Url`.

## Set the limit

`.redirect(RedirectPolicy::limited(n))` sets the limit.
`RedirectPolicy::none()` turns redirects off.
When the limit is reached, the session returns the last `3xx` response. It
does not return an error.

```rust,no_run
use leyline::{Browser, RedirectPolicy, Session};

# fn run() -> leyline::Result<()> {
let session = Session::builder()
    .browser(Browser::default())
    .redirect(RedirectPolicy::limited(3))
    .build()?;
let no_redirects = Session::builder()
    .browser(Browser::default())
    .redirect(RedirectPolicy::none())
    .build()?;
# let _ = (session, no_redirects);
# Ok(())
# }
```

`RedirectPolicy::limited(10)` is the default. `RedirectPolicy::none()` follows
no redirect. The session policy is set at build time.

## Override one request

`RequestBuilder::redirect` sets the policy for one request. The session
default applies to every other request. `Session::execute` reads a
`RedirectPolicy` from the request extensions in the same way.

```rust,no_run
use leyline::RedirectPolicy;

# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let resp = session
    .post("https://example.com/login")
    .redirect(RedirectPolicy::none())
    .await?;
let location = resp.header("location");
# let _ = location;
# Ok(())
# }
```

## Decide each redirect

`RedirectPolicy::custom` calls your function for each redirect. The function
gets a `RedirectAttempt` with the status, the current URL as a `&url::Url`, the
`Location` value as a string, and the URLs followed so far as a `&[url::Url]`.
Both URL fields have no user name or password. The function returns
`RedirectAction::Follow` or `RedirectAction::Stop`. On `Stop`, the session
returns the `3xx` response.

```rust,no_run
use leyline::{Browser, RedirectAction, RedirectPolicy, Session};

# fn run() -> leyline::Result<()> {
let policy = RedirectPolicy::custom(|attempt| {
    let same_host = attempt
        .location
        .and_then(|location| attempt.url.join(location).ok())
        .is_some_and(|next| next.host_str() == attempt.url.host_str());
    if same_host && attempt.previous.len() < 5 {
        RedirectAction::Follow
    } else {
        RedirectAction::Stop
    }
});
let session = Session::builder()
    .browser(Browser::default())
    .redirect(policy)
    .build()?;
# let _ = session;
# Ok(())
# }
```

A custom policy can follow at most 32 redirects. The next redirect fails with
`Kind::Redirect`.
