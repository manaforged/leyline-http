# Cookies

Every session owns a `Jar`. It sends the matching cookies on each request and
stores what each response sets, with the rules Chrome applies. This page
covers reading and seeding the jar, sharing it, and saving it between runs.

## Read and seed the jar

`Session::cookies()` borrows the jar. Every jar method that takes a URL takes
a parsed `&Url` and returns no `Result`. `leyline::Url` is the `url` crate's
`Url`, so parse once with `leyline::Url::parse(..)?` and pass the same value
to each call.

```rust,no_run
# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let session = leyline::Session::new();
session.get("https://www.example.com/login").await?;

let jar = session.cookies();
let url = leyline::Url::parse("https://www.example.com/")?;
if let Some(id) = jar.get_cookie(&url, "session_id") {
    println!("session_id={id}");
}
if let Some(header) = jar.cookie_header(&url) {
    println!("cookie: {header}");
}

jar.store_set_cookie("token=abc; Domain=.example.com; Path=/; Secure", &url);
assert_eq!(jar.remove(&url, "token"), 1);
# Ok(())
# }
```

| Method | Does |
| --- | --- |
| `get_cookie(url, name)`, `set_cookie(url, name, value)` | Read or set one cookie scoped to a URL |
| `cookie_header(url)` | The `Cookie` header the jar sends to a URL |
| `store_set_cookie(set_cookie, url)` | Stores a full `Set-Cookie` value, with `Domain`, `Path`, `Secure`, `HttpOnly`, `SameSite`, `Max-Age`, and `Expires`, through the parser the session uses. `Max-Age=0` deletes the matching cookie |
| `load_cookies(header, url)` | Loads a `Cookie` header string |
| `remove(url, name)` | Deletes every cookie of that name the host of `url` receives, on every path. Returns the number removed |
| `remove_named(name)` | Deletes every cookie of that name on every host. Returns the number removed |
| `clear()` | Deletes every cookie |
| `all_cookies()` | Every stored cookie, sorted by domain then name, expired ones included |
| `snapshot()`, `extend_from(other)` | Fork and merge. See [Share or fork a jar](#share-or-fork-a-jar) |
| `changes()` | A `tokio::sync::watch::Receiver<u64>` whose value increases each time a cookie is added, replaced, or removed through the jar or any clone of it. Sending a `Cookie` header does not change it |

A `Cookie` has the public fields `name`, `value`, `domain`, `path`, `secure`,
`http_only`, `same_site` (`Strict`, `Lax`, or `None`), `expires`,
`creation_time`, and `host_only`. `Cookie::is_expired()` tells whether the
expiry has passed.

## Read the cookies a response set

`Response::cookies()` parses the `Set-Cookie` headers of the final response
into `Cookie` records. It does not read the jar, so it also lists a deletion.
It skips a header that does not parse and a cookie whose `Domain` is a public
suffix or does not cover the host. To read what the session sends, ask the
jar.

```rust,no_run
# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let session = leyline::Session::new();
let resp = session.get("https://shop.example/login").await?;
for cookie in resp.cookies() {
    println!("set {}={} for {}", cookie.name, cookie.value, cookie.domain);
}
# Ok(())
# }
```

## Share or fork a jar

`Jar` is a handle over shared state: `jar.clone()` and session clones see the
same cookies, so a cookie one task receives reaches every other task at once.

`SessionBuilder::cookie_jar(jar)` starts a session from a jar you built.
`session.with_cookie_jar(jar)` derives a session that shares the pool and
every other setting but reads and writes the given jar; use it for a second
identity on warm connections. See [Sessions](sessions.md#derive-a-session).

`snapshot()` is a fork: a new store with a copy of every cookie and all its
attributes. A write to the copy does not reach the original.
`extend_from(&other)` merges the cookies of `other` with their attributes; on
the same name, domain, and path, the cookie from `other` wins. Together they
try a step without touching a stored login:

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

## What the jar does

The jar follows Chrome:

- A cookie is stored under the exact domain the `Set-Cookie` names: its
  `Domain=` value, or the request host.
- `Secure` cookies go only over HTTPS.
- `SameSite` uses the `sec-fetch-site` value the request sends. Leyline
  computes it from the initiator and every URL in the redirect chain, schemes
  included, so `http://` to `https://` on one host is cross-site. The
  initiator is the request's `Referer`; without one, the session's `Referer`
  when it is an absolute URL; otherwise the first URL. A preset or header
  that sets `sec-fetch-site` sets it for cookies too.
- A cross-site request sends no `Strict` cookie, and sends `Lax` cookies only
  on a top-level navigation (`sec-fetch-dest: document`, or none) with `GET`
  or `HEAD`. Firefox profiles also treat a request as cross-site when its
  redirect chain crosses sites.
- A cookie lives at most 400 days. A later `Max-Age` or `Expires`, or an
  `Expires` past the range of the platform clock, is cut to 400 days.
- The limits are 180 cookies per exact domain and 3300 overall. Crossing a
  limit evicts the least recently accessed entries, 30 per domain or 300
  overall.
- An expired cookie stays in memory but is never sent. `get_cookie` skips
  it; `all_cookies()` and `snapshot()` include it.

Domain scoping uses the Mozilla Public Suffix List, compiled into the crate.
A cookie with `Domain=example.co.uk` reaches `shop.example.co.uk`; a host-only
cookie set by `www.example.co.uk` reaches that host only. A `Set-Cookie` with
`Domain=co.uk`, or with a domain that has no dot, is refused.

## Save the jar to a file

`Jar` and `Cookie` implement `Serialize` and `Deserialize`, with times as
unix milliseconds. `Jar::save_to(path)` writes the jar as JSON and
`Jar::load_from(path)` reads it back. The write is atomic: a temporary file,
synced, renamed over `path`, and the directory synced.

- Saving writes every cookie that has not expired, session cookies included,
  as a browser that restores its last session does.
- Loading skips expired cookies, keeps the newest of two cookies with the
  same name, path, and domain, applies the 180 and 3300 limits, and cuts a
  stored expiry to at most 400 days after the load.
- Each cookie keeps its last-access time, so eviction order survives a
  restart. A jar saved without that field loads with the load time.

`Jar::autosave(path, debounce)` starts one writer task and returns a
`JarAutosave` handle. It panics outside a Tokio runtime.

| Event | What the writer does |
| --- | --- |
| A cookie changes | Saves once `debounce` has passed since the first unsaved change, so a burst of `Set-Cookie` headers gives one write |
| `flush().await` | Saves now and returns the result |
| `shutdown().await` | Saves, stops the task, and returns the result |
| The handle is dropped | Saves once more in the background. The runtime must still run |

```rust,no_run
use std::time::Duration;

use leyline::cookie::Jar;

# async fn run() -> leyline::Result<()> {
let jar = if std::path::Path::new("cookies.json").exists() {
    Jar::load_from("cookies.json")?
} else {
    Jar::new()
};
let session = leyline::Session::builder()
    .browser(leyline::Browser::default())
    .cookie_jar(jar)
    .build()?;
let saver = session.cookies().autosave("cookies.json", Duration::from_secs(2));

session.get("https://shop.example/account").await?;

saver.shutdown().await?;
# Ok(())
# }
```

Call `shutdown().await` before the program exits, so the last save is not
lost.

## Keep a logged-in device between runs

A site can tie a login to the browser, platform, proxy, and languages as well
as the cookies. `Device` saves all of them with the jar, and
`Device::autosave` replaces `Jar::autosave`. See [Accounts](accounts.md).

## Save the connection state

TLS session tickets, `Alt-Svc` entries, and the HSTS store live in
`SessionState`, beside the jar. See
[Accounts](accounts.md#keep-the-connection-state).

## Next

Read [WebSocket](websocket.md).
