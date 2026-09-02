# Cookies

Every session owns a `Jar`. It sends the matching cookies on each request and
stores what the response sets.

## Reach the jar

`Session::cookies()` borrows the jar.

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::chrome();
session.get("https://example.com/login").await?;

let jar = session.cookies();
println!("{} cookies", jar.len());
if let Some(id) = jar.get_cookie("https://example.com/", "session_id") {
    println!("session_id={id}");
}
# Ok(())
# }
```

Useful jar methods:

- `set_cookie(url, name, value)` and `get_cookie(url, name)` for one cookie
  scoped to a URL.
- `get_named`, `contains_named`, `set_named`, and `set_named_on` when you know
  the name but not the URL.
- `remove_named`, `remove_all_named`, `remove_named_for_host`, and `clear` to
  delete.
- `all_cookies()` for a snapshot sorted by domain then name.
- `load_cookies(header, url)` and `export_cookies(url)` to move a `Cookie`
  header string in and out.
- `len()` and `is_empty()` to size the jar.

## Sharing and forking

`Jar` is a handle over shared state, so `jar.clone()` gives you another handle
onto the same cookies. Two sessions holding clones of one jar see each other's
logins.

`deep_clone()` forks instead: it copies every cookie into an independent jar,
so later writes do not cross.

```rust,no_run
use leyline::cookie::Jar;

# fn run() -> leyline::Result<()> {
let jar = Jar::new();
jar.set_cookie("https://example.com/", "session_id", "abc");

let shared = jar.clone();
let forked = jar.deep_clone();

jar.set_cookie("https://example.com/", "extra", "1");
assert!(shared.contains_named("extra"));
assert!(!forked.contains_named("extra"));
# Ok(())
# }
```

`merge(other)` copies another jar's cookies into this one.

## Give a session its own jar

`SessionBuilder::cookie_jar(jar)` starts a session from a jar you built,
which is how you restore a saved login. `Session::with_cookie_jar(jar)` derives
a session that keeps the TLS context and the pool but swaps the jar, which is
how you run several identities over one connection pool.

```rust,no_run
use leyline::cookie::Jar;

# fn run() -> leyline::Result<()> {
let jar = Jar::new();
jar.load_cookies("session_id=abc; theme=dark", "https://example.com/");

let session = leyline::Session::builder().cookie_jar(jar).build()?;
let second_identity = session.with_cookie_jar(Jar::new());
# let _ = second_identity;
# Ok(())
# }
```

## What the jar stores

A `Cookie` carries the full RFC 6265bis attribute set: `name`, `value`,
`domain`, `path`, `secure`, `http_only`, `same_site`, `expires`,
`creation_time`, `last_access`, and `host_only`. `SameSite` is `Strict`,
`Lax`, or `None`.

The jar follows Chrome's behavior:

- Cookies are keyed by registrable domain.
- `Secure` cookies go only over HTTPS.
- `SameSite` is enforced against the navigation that started the request, so a
  redirect chain that crosses sites makes the request cross-site.
- Expired cookies are not returned.
- Limits match Chrome: 180 cookies per domain and 3300 overall. Crossing a
  limit evicts the least recently accessed entries, 30 per domain or 300
  globally.

`Jar` and `Cookie` implement `Serialize` and `Deserialize`, so a jar
round-trips through serde with the times stored as unix milliseconds. That is
how you persist a login between runs.

## The public suffix rule

Domain scoping uses the Mozilla Public Suffix List, compiled into the crate.
The registrable domain of `www.example.co.uk` is `example.co.uk`, so a cookie
set there is shared with `shop.example.co.uk`.

The same list blocks a cookie set on a public suffix itself. A `Set-Cookie`
with `Domain=co.uk` is refused, as is one with a domain that has no dot. Such
a cookie is not stored, though `Response::cookies()` still reports the name
and value it carried.

## Next

Read [WebSocket](websocket.md).
