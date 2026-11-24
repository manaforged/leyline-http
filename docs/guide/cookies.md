# Cookies

Every session owns a `leyline::cookie::Jar`, unless you call
`SessionBuilder::cookies(None)`. `Session::save` records that choice. Cookie types
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

### Import cookies from Playwright or Chrome

To move a logged-in session from Playwright or Chrome into Leyline, map each
exported cookie into `Cookie::builder`. Insert it against an `https://` URL for
the cookie's domain. Playwright marks a session cookie with `expires: -1`, so
the example sets an expiry only when it is positive. The `SameSite` match
accepts both Playwright and Chrome spellings.

```rust,no_run
use leyline::cookie::{Cookie, SameSite};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportedCookie {
    name: String,
    value: String,
    domain: String,
    path: String,
    secure: bool,
    http_only: bool,
    same_site: Option<String>,
    #[serde(default, alias = "expirationDate")]
    expires: Option<f64>,
}

# fn run(json: &str) -> Result<(), Box<dyn std::error::Error>> {
let session = leyline::Session::new();
let exported: Vec<ExportedCookie> = serde_json::from_str(json)?;
for item in exported {
    let host = item.domain.trim_start_matches('.');
    let origin = format!("https://{host}/");
    let same_site = match item.same_site.as_deref() {
        Some("Strict" | "strict") => SameSite::Strict,
        Some("None" | "no_restriction") => SameSite::None,
        _ => SameSite::Lax,
    };
    let mut builder = Cookie::builder(&item.name, &item.value)
        .path(&item.path)
        .secure(item.secure)
        .http_only(item.http_only)
        .same_site(same_site);
    if item.domain.starts_with('.') {
        builder = builder.domain(&item.domain);
    }
    if let Some(secs) = item.expires.filter(|secs| *secs > 0.0) {
        builder = builder.expires(std::time::UNIX_EPOCH + std::time::Duration::from_secs_f64(secs));
    }
    session.cookies().insert(&origin, builder.build());
}
# Ok(())
# }
```

To run a session with no cookies at all, see
[Turn cookies off](sessions.md#turn-cookies-off).

## Read the cookies a response set

`Response::cookies()` parses the `Set-Cookie` headers of the final response
into `Cookie` records. It does not read the jar, so it also includes deletion
headers (such as `Max-Age=0`). It skips headers that do not parse or whose
`Domain` is a public suffix or does not cover the host. To read what the
session will send next, query the jar.

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
`session.to_builder().cookies(jar).build()?` shares the pool with a different
jar. To keep accounts fully apart, see
[Sessions](sessions.md#derive-a-session).

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

The jar follows Chrome's rules:

- A cookie is stored under the exact domain the `Set-Cookie` names: its
  `Domain=` value, or the request host.
- `Secure` cookies go only over HTTPS, or over HTTP to `localhost`, a
  `*.localhost` host, or a loopback IP (`127.0.0.0/8` or `[::1]`).
- `SameSite` uses the `sec-fetch-site` value the request sends. Leyline
  computes it from the sending page and every URL in the redirect chain, schemes
  included, so `http://` to `https://` on one host is cross-site. Leyline
  takes the sending page from the request's `Referer`. Without one, it uses the
  session's `Referer` if that is an absolute URL, and otherwise the request URL.
  A request kind or header that sets `sec-fetch-site` sets it for cookies too.
- A cross-site request sends no `Strict` cookie, and sends `Lax` cookies only
  on a top-level navigation (`sec-fetch-dest: document`, or none) with `GET`
  or `HEAD`. Firefox profiles also treat a request as cross-site when its
  redirect chain crosses sites.
- A cookie lives at most 400 days. A later `Max-Age` or `Expires`, or an
  `Expires` past the range of the platform clock, is cut to 400 days.
- The jar refuses a cookie whose name and value together pass 4096 octets.
- The jar ignores an attribute whose value passes 1024 octets. An oversized
  `Path` falls back to the default path from the request URL, which has no cap.
- The jar refuses a cookie whose stored domain passes 1024 octets.
- `merge` and loading a saved jar apply the same limits and skip a cookie
  that fails them.
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
