# Cookies

Every session owns a `leyline::cookie::Jar` (unless disabled with
`SessionBuilder::cookies(None)`, which `Session::save` persists). Cookie types
live in `leyline::cookie::{Jar, Cookie, CookieBuilder, SameSite}`. The jar
follows Chrome's rules for sending and storing cookies.

## Read and seed the jar

`Session::cookies()` borrows the jar. Every `Jar` method that takes a URL
accepts `impl IntoUrl` (a `&str`, `String`, or `&leyline::Url`), returns no
`Result`, and ignores URLs that fail to parse. Parse once with
`leyline::Url::parse(..)?` if you reuse a URL across calls.

```rust,no_run
# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let session = leyline::Session::new();
session.get("https://www.example.com/login").send().await?;

let jar = session.cookies();
let url = leyline::Url::parse("https://www.example.com/")?;
if let Some(id) = jar.get(&url, "session_id") {
    println!("session_id={id}");
}
if let Some(header) = jar.header_for(&url) {
    println!("cookie: {header}");
}

jar.store(&url, "token=abc; Domain=.example.com; Path=/; Secure");
assert_eq!(jar.remove(&url, "token"), 1);
# Ok(())
# }
```

| Method | Does |
| --- | --- |
| `get(url, name)`, `set(url, name, value)` | Read or set one cookie scoped to a URL |
| `cookies_for(url)` | Returns matching non-expired `Cookie` records in send order |
| `header_for(url)` | The `Cookie` header string the jar sends to a URL |
| `store(url, set_cookie)` | Parses and stores a raw `Set-Cookie` value (`Max-Age=0` deletes matching cookie) |
| `insert(url, cookie)` | Stores a `Cookie` you built or parsed, applying the same rules as `Set-Cookie` |
| `remove(url, name)`, `remove_everywhere(name)` | Delete cookies matching `name` for one host or across all hosts; returns count removed |
| `clear()` | Deletes every cookie |
| `len()`, `is_empty()` | Count or emptiness of stored cookies without cloning |
| `all()` | Every stored cookie, sorted by domain then name (expired ones included) |
| `snapshot()`, `merge(other)` | Fork and merge jars; see [Share or fork a jar](#share-or-fork-a-jar) |

A `Cookie` has a getter for each attribute: `name()`, `value()`, `domain()`,
`path()`, `secure()`, `http_only()`, `same_site()` (`Strict`, `Lax`, or
`None`), `expires()`, `creation_time()`, `host_only()`, and `is_expired()`.

### Build or parse a cookie

- `Cookie::new(name, value)` makes a host-only session cookie with path `/` and `SameSite::Lax`.
- `Cookie::builder(name, value)` sets `domain`, `path`, `expires`, `secure`, `http_only`, and `same_site`, then `.build()`.
- `Cookie::parse(url, set_cookie)` parses a raw `Set-Cookie` header against `url` into `Option<Cookie>` without inserting it into a jar.

```rust,no_run
use leyline::cookie::Cookie;

# fn run() -> Result<(), Box<dyn std::error::Error>> {
let session = leyline::Session::new();
let cookie = Cookie::builder("sid", "abc")
    .path("/app")
    .secure(true)
    .http_only(true)
    .build();
session.cookies().insert("https://www.example.com/", cookie);

if let Some(parsed) = Cookie::parse("https://www.example.com/", "prefs=dark; Path=/; Secure") {
    session.cookies().insert("https://www.example.com/", parsed);
}
println!("{} cookies", session.cookies().len());
# Ok(())
# }
```

Without `domain` the cookie matches only the host of the URL; with it the
cookie also matches subdomains. The jar applies the rules it
applies to a `Set-Cookie` header from that URL, and ignores a cookie the URL
may not set: for example, it drops a `secure` cookie for an `http://` URL.

To run a session with no cookies at all, see
[Turn cookies off](sessions.md#turn-cookies-off).

## Read the cookies a response set

`Response::cookies()` parses the `Set-Cookie` headers of the final response
into `Cookie` records. It does not read the jar, so it also lists a deletion.
It skips a header that does not parse and a cookie whose `Domain` is a public
suffix or does not cover the host. To read what the session sends, ask the
jar.

```rust,no_run
# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let session = leyline::Session::new();
let resp = session.get("https://shop.example/login").send().await?;
for cookie in resp.cookies() {
    println!("set {}={} for {}", cookie.name(), cookie.value(), cookie.domain());
}
# Ok(())
# }
```

## Share or fork a jar

`Jar` is a handle over shared state: `jar.clone()` and session clones see the
same cookies, so a cookie one task receives reaches every other task at once.

`SessionBuilder::cookies(jar)` starts a session from a jar you built.
`session.to_builder().cookies(jar).build()?` makes a session that shares
the pool and every other setting but reads and writes the given jar; use it for a second
identity on warm connections. See [Sessions](sessions.md#derive-a-session).

`snapshot()` is a fork: a new store with a copy of every cookie and all its
attributes. A write to the copy does not reach the original.
`merge(&other)` merges the cookies of `other` with their attributes; on
the same name, domain, and path, the cookie from `other` wins. Together they
try a step without touching a stored login:

```rust,no_run
# async fn run(session: leyline::Session) -> leyline::Result<()> {
let probe_jar = session.cookies().snapshot();
let probe = session.to_builder().cookies(probe_jar.clone()).build()?;
if probe.get("https://example.com/check").send().await?.status().is_success() {
    session.cookies().merge(&probe_jar);
}
# Ok(())
# }
```

## What the jar does

The jar follows Chrome:

- A cookie is stored under the exact domain the `Set-Cookie` names: its
  `Domain=` value, or the request host.
- `Secure` cookies go only over HTTPS, or over HTTP to `localhost`, `127.0.0.1`, `[::1]`, or a `*.localhost` host.
- `SameSite` uses the `sec-fetch-site` value the request sends. Leyline
  computes it from the sending page and every URL in the redirect chain, schemes
  included, so `http://` to `https://` on one host is cross-site. The
  sending page is the request's `Referer`; without one, the session's `Referer`
  when it is an absolute URL; otherwise the first URL. A request kind or header
  that sets `sec-fetch-site` sets it for cookies too.
- A cross-site request sends no `Strict` cookie, and sends `Lax` cookies only
  on a top-level navigation (`sec-fetch-dest: document`, or none) with `GET`
  or `HEAD`. Firefox profiles also treat a request as cross-site when its
  redirect chain crosses sites.
- A cookie lives at most 400 days. A later `Max-Age` or `Expires`, or an
  `Expires` past the range of the platform clock, is cut to 400 days.
- A cookie whose name and value together pass 4096 octets is refused. An
  attribute whose value passes 1024 octets is ignored, so an oversized
  `Path` falls back to the default path, which comes from the request URL
  and has no cap. A cookie whose stored domain passes 1024 octets is
  refused. `merge` and loading a saved jar apply the same limits
  and skip a cookie that fails them.
- The limits are 180 cookies per exact domain and 3300 overall. Crossing a
  limit evicts the least recently accessed entries, 30 per domain or 300
  overall.
- An expired cookie stays in memory but is never sent. `get` skips
  it; `all()` and `snapshot()` include it.

Domain scoping uses the Mozilla Public Suffix List, compiled into the crate.
A cookie with `Domain=example.co.uk` reaches `shop.example.co.uk`; a host-only
cookie set by `www.example.co.uk` reaches that host only. A `Set-Cookie` with
`Domain=co.uk`, or with a domain that has no dot, is refused.

## Save cookies between runs

To persist cookies together with browser identity, proxy, TLS session tickets,
`Alt-Svc`, and HSTS in one file, use `Session::save`, `Session::load`, and
`Session::autosave`; see [Accounts](accounts.md).

For a custom cookie store, `Jar` and `Cookie` implement `Serialize` and
`Deserialize` (timestamps as Unix milliseconds):

- Saving writes every non-expired cookie, session cookies included.
- Loading skips expired or oversized cookies, keeps the newest copy of duplicate `(name, path, domain)` keys, enforces the 180/3300 caps, and clamps expiry to 400 days from load time.
- Last-access times are preserved so eviction order survives restarts.

## Next

Read [Accounts](accounts.md).
