# Cookies

Every session owns a `Jar`. It sends the matching cookies on each request and
stores what the response sets.

## Reach the jar

`Session::cookies()` borrows the jar.

```rust,no_run
# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let session = leyline::Session::new();
session.get("https://example.com/login").await?;

let jar = session.cookies();
let live = jar.all_cookies().iter().filter(|c| !c.is_expired()).count();
println!("{live} cookies");
let url = url::Url::parse("https://example.com/")?;
if let Some(id) = jar.get_cookie(&url, "session_id") {
    println!("session_id={id}");
}
# Ok(())
# }
```

Every jar method that takes a URL takes a parsed `&url::Url`. The methods
cannot fail on URL input, so none of them returns a `Result`. Parse the URL
once and pass the same value to each call.

Useful jar methods:

- `set_cookie(url, name, value)` and `get_cookie(url, name)` for one cookie
  scoped to a URL.
- `store_set_cookie(set_cookie, url)` to seed a cookie from a full
  `Set-Cookie` value, with `Domain`, `Path`, `Secure`, `HttpOnly`,
  `SameSite`, `Max-Age`, and `Expires`. It is the same parser the session
  uses for responses. `Max-Age=0` deletes the matching cookie.
- `remove(url, name)` to delete every cookie with that name that the host of
  `url` receives, on every path. It returns the number removed.
- `remove_named(name)` to delete every cookie with that name on every host.
  It returns the number removed. `clear` deletes all cookies.
- `all_cookies()` for a list of every stored cookie, sorted by domain then
  name. The list includes expired cookies. Filter them with
  `Cookie::is_expired()`.
- `snapshot()` for an independent copy of the jar with every attribute. The
  copy includes expired cookies.
- `extend_from(other)` to merge the cookies of another jar.
- `load_cookies(header, url)` to load a `Cookie` header string, and
  `cookie_header(url)` for the `Cookie` header the jar sends to a URL.

## Sharing and forking

`Jar` is a handle over shared state, so `jar.clone()` gives you another handle
onto the same cookies. Two sessions holding clones of one jar see each other's
logins.

```rust,no_run
use leyline::cookie::Jar;

# fn run() -> Result<(), url::ParseError> {
let url = url::Url::parse("https://example.com/")?;
let jar = Jar::new();
jar.set_cookie(&url, "session_id", "abc");

let shared = jar.clone();
jar.set_cookie(&url, "extra", "1");
assert!(shared.get_cookie(&url, "extra").is_some());
# Ok(())
# }
```

`jar.snapshot()` is a fork: a new store with a copy of every cookie and all
of its attributes (domain, path, `Secure`, `HttpOnly`, `SameSite`, expiry,
host-only, and creation time). A write to the copy does not reach the
original. `jar.extend_from(&other)` merges the cookies of `other` into `jar`
with their attributes. On the same name, domain, and path, the cookie from
`other` replaces the one in `jar`.

Use the pair to try a step without touching a stored login: run the step on a
session that uses a snapshot, and merge the snapshot back only when the step
succeeds.

```rust,no_run
# async fn run(session: leyline::Session) -> leyline::Result<()> {
let probe_jar = session.cookies().snapshot();
let probe = session.with_cookie_jar(probe_jar.clone());
if probe.get("https://example.com/check").await?.status().is_success() {
    session.cookies().extend_from(&probe_jar);
}
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

# fn run() -> Result<(), Box<dyn std::error::Error>> {
let jar = Jar::new();
jar.load_cookies("session_id=abc; theme=dark", &url::Url::parse("https://example.com/")?);

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
jar.store_set_cookie("token=abc; Domain=.example.com; Path=/; Secure", &url);
assert_eq!(jar.remove(&url, "token"), 1);
# Ok(())
# }
```

## What the jar stores

A `Cookie` has these public fields: `name`, `value`, `domain`, `path`,
`secure`, `http_only`, `same_site`, `expires`, `creation_time`, and
`host_only`. `SameSite` is `Strict`, `Lax`, or `None`. `Cookie::is_expired()`
tells whether the expiry has passed.

The jar follows Chrome's behavior:

- Cookies are stored under the exact domain the `Set-Cookie` names: its
  `Domain=` value when present, the request host otherwise. The 180-cookie
  limit applies per exact domain.
- `Secure` cookies go only over HTTPS.
- `SameSite` is enforced against the navigation that started the request, so a
  redirect chain that crosses sites makes the request cross-site.
- The jar sends no expired cookie.
- Limits match Chrome: 180 cookies per domain and 3300 overall. Crossing a
  limit evicts the least recently accessed entries, 30 per domain or 300
  globally.

The jar keeps a cookie after its expiry passes. `get_cookie` skips expired
cookies, and the `Cookie` header omits them. `all_cookies()`, `snapshot()`, and
serde include them.

`Jar` and `Cookie` implement `Serialize` and `Deserialize`, so a jar
round-trips through serde with the times stored as unix milliseconds. Serde
writes every stored cookie, expired ones included. That is how you persist a
login between runs.

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
