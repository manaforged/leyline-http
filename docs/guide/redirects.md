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

`Response::redirect_chain` lists the URLs that the session left, in order.
`Response::url` is the final URL.

## Set the limit

`max_redirects(n)` sets the limit. `max_redirects(0)` turns redirects off.
When the limit is reached, the session returns the last `3xx` response. It
does not return an error.

```rust,no_run
use leyline::{RedirectPolicy, Session};

# fn run() -> leyline::Result<()> {
let session = Session::builder().chrome().max_redirects(3).build()?;
let no_redirects = session.with_redirect_policy(RedirectPolicy::none());
# let _ = no_redirects;
# Ok(())
# }
```

`RedirectPolicy::limited(n)` is the same as `max_redirects(n)`.
`RedirectPolicy::none()` follows no redirect. `Session::with_redirect_policy`
returns a copy of the session with another policy. The copy shares the pool
and the cookie jar.

## Decide each redirect

`RedirectPolicy::custom` calls your function for each redirect. The function
gets a `RedirectAttempt` with the status, the current URL, the `Location`
value, and the URLs followed so far. It returns `RedirectAction::Follow` or
`RedirectAction::Stop`. On `Stop`, the session returns the `3xx` response.

```rust,no_run
use leyline::{RedirectAction, RedirectPolicy, Session};

# fn run() -> leyline::Result<()> {
let policy = RedirectPolicy::custom(|attempt| {
    let same_host = attempt
        .location
        .is_some_and(|location| location.starts_with('/'));
    if same_host && attempt.previous.len() < 5 {
        RedirectAction::Follow
    } else {
        RedirectAction::Stop
    }
});
let session = Session::builder().chrome().redirect_policy(policy).build()?;
# let _ = session;
# Ok(())
# }
```

A custom policy can follow at most 32 redirects. The next redirect fails with
`Kind::Redirect`.
