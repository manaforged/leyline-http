# Cookies

Every session owns a `Jar`. It sends the matching cookies on each request and
stores what the response sets.

## Reach the jar

`Session::cookies()` borrows the jar.

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
session.get("https://example.com/login").await?;

let jar = session.cookies();
println!("{} cookies", jar.all_cookies().len());
if let Some(id) = jar.get_cookie("https://example.com/", "session_id") {
    println!("session_id={id}");
}
# Ok(())
# }
```

Useful jar methods:

- `set_cookie(url, name, value)` and `get_cookie(url, name)` for one cookie
  scoped to a URL.
- `store_set_cookie(set_cookie, url)` to seed a cookie from a full
  `Set-Cookie` value, with `Domain`, `Path`, `Secure`, `HttpOnly`,
  `SameSite`, `Max-Age`, and `Expires`. It is the same parser the session
  uses for responses. `Max-Age=0` deletes the matching cookie.
- `remove_named(name)` to delete every cookie with that name on every host.
  It returns the number removed. `clear` deletes all cookies.
- `all_cookies()` for a snapshot sorted by domain then name.
- `load_cookies(header, url)` and `export_cookies(url)` to move a `Cookie`
  header string in and out.
- `cookie_header(url)` for the `Cookie` header the jar sends to a URL.

## Sharing and forking

`Jar` is a handle over shared state, so `jar.clone()` gives you another handle
onto the same cookies. Two sessions holding clones of one jar see each other's
logins.

```rust,no_run
use leyline::cookie::Jar;

# fn run() -> leyline::Result<()> {
let jar = Jar::new();
jar.set_cookie("https://example.com/", "session_id", "abc");

let shared = jar.clone();
jar.set_cookie("https://example.com/", "extra", "1");
assert!(shared.get_cookie("https://example.com/", "extra").is_some());
# Ok(())
# }
```

## Give a session its own jar

`SessionBuilder::cookie_jar(jar)` starts a session from a jar you built,
which is how you restore a saved login. To run a second identity on the same
connections, call `session.with_cookie_jar(jar)`. It derives a session that
shares the pool and all other settings but reads and writes the given jar.

```rust,no_run
use leyline::cookie::Jar;

# fn run() -> leyline::Result<()> {
let jar = Jar::new();
jar.load_cookies("session_id=abc; theme=dark", "https://example.com/");

let session = leyline::Session::builder().cookie_jar(jar).build()?;
let second_identity = session.with_cookie_jar(Jar::new());
# let _ = (session, second_identity);
# Ok(())
# }
```

Seed a domain cookie that `set_cookie` cannot express:

```rust,no_run
use leyline::cookie::Jar;

# fn run() -> Result<(), url::ParseError> {
let jar = Jar::new();
let url = url::Url::parse("https://www.example.com/")?;
jar.store_set_cookie("token=abc; Domain=.example.com; Path=/; Secure", &url);
jar.store_set_cookie("token=; Domain=.example.com; Path=/; Max-Age=0", &url);
assert_eq!(jar.remove_named("token"), 0);
# Ok(())
# }
```

## What the jar stores

A `Cookie` carries the full RFC 6265bis attribute set: `name`, `value`,
`domain`, `path`, `secure`, `http_only`, `same_site`, `expires`,
`creation_time`, `last_access`, and `host_only`. `SameSite` is `Strict`,
`Lax`, or `None`.

The jar follows Chrome's behavior:

- Cookies are stored under the exact domain the `Set-Cookie` names: its
  `Domain=` value when present, the request host otherwise. The 180-cookie
  limit applies per exact domain.
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
A cookie with `Domain=example.co.uk` is shared with `shop.example.co.uk`,
because the list accepts `example.co.uk` as a registrable domain. A host-only
cookie set by `www.example.co.uk` reaches that host only.

The same list blocks a cookie set on a public suffix itself. A `Set-Cookie`
with `Domain=co.uk` is refused, as is one with a domain that has no dot. Such
a cookie is not stored, and `Response::cookies()` does not list it.

## Next

Read [WebSocket](websocket.md).
