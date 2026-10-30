# Accounts

This page keeps one session across runs the way a browser profile keeps it: the same browser, platform, languages, proxy, cookies, and connection state on every run. `Session::save` writes all of them to one file, `Session::load` reads it back, and `Session::autosave` keeps the file current. A `Tab` keeps the current page, and `leyline::html` reads the login form.

## What a save holds

`session.save(path)` writes one JSON file, `{ "version": 1, .. }`. The write is atomic, and on Unix the file has mode `0600`.

| Field | Holds |
| --- | --- |
| `identity`, `platform` | The browser, platform, and languages the session sends, or the profile TOML of a session built with `.profile(..)` |
| `profile_id` | The session's profile id, which `build()` compares on load |
| `proxy` | The proxy URL without its password |
| `jar` | The cookies |
| `state` | The TLS session tickets, the `Alt-Svc` entries, and the HSTS store. See [The connection state](#the-connection-state) |

A save holds only these fields. Headers, host limits, retry, timeout, redirect, and trace hooks are code, so set them again on the builder that `load` returns.

A session file is a credential: its cookies and TLS session tickets resume the account, and its proxy setting routes every request. Whoever can write the file controls the session, so keep it as private as a password and load only files that your program wrote.

## Save and load

Pin a browser variant, set the languages and one proxy, and turn the proxy environment variables off. Then save the session.

```rust,no_run
use leyline::{Browser, Platform, Proxy, Session};

# fn run() -> leyline::Result<()> {
let proxy = "http://user:secret@proxy.example:8080";
let session = Session::builder()
    .browser(Browser::Chrome154).platform(Platform::Windows).languages(["de-DE", "de", "en"])
    .proxy(Proxy::from(proxy).env(false))
    .build()?;
session.save("account/session.json")?;
# Ok(())
# }
```

`Session::load(path)` returns a `SessionBuilder` with the saved identity, proxy, and cookies. Add any setter, then call `build()`. On a first run the file does not exist and `load` fails with `Kind::Io`, so build a new session when `path.exists()` is false. A loaded builder does not read the proxy environment variables, so a save without a proxy goes direct.

```rust,no_run
use std::time::Duration;
use leyline::Session;

# fn run() -> leyline::Result<()> {
let session = Session::load("account/session.json")?
    .timeout(Duration::from_secs(30))
    .build()?;
# let _ = session;
# Ok(())
# }
```

`build()` returns `Kind::Config` with `err.is_profile_changed()` set to `true` when the session's profile id differs from the saved one, for example after a crate upgrade changed the bundled profile. Otherwise it restores the connection state. Files that older Leyline versions wrote with `SavedSession` load the same way.

| Choice | Why |
| --- | --- |
| `Browser::Chrome154`, not `Browser::chrome()` | `chrome()` moves to newer captures in a release |
| `env(false)` | The session never picks up `HTTPS_PROXY` or `ALL_PROXY` from another machine |

## Save the session as it changes

`session.autosave(path, every)` starts one writer task and returns nothing. `every` is a `Duration`.

| Event | Effect |
| --- | --- |
| A cookie changes | A save runs, at most once per `every` |
| `every` passes and the connection state changed | A save runs, so new TLS tickets, `Alt-Svc`, and HSTS entries reach the file |
| `session.shutdown()` | A last save runs and the writer stops. `shutdown` returns at once; the save completes even when the runtime stops right after |
| The runtime drops the task | The writer saves unsaved changes as it stops |

Each save writes a temporary file, syncs it, and renames it, so a crash never leaves a half-written file. On Unix the file has mode `0600`; on Windows it takes the default ACLs of its directory. `autosave` panics outside a Tokio runtime.

```rust,no_run
use std::time::Duration;
use leyline::Session;

# async fn run() -> leyline::Result<()> {
let session = Session::load("account/session.json")?.build()?;
session.autosave("account/session.json", Duration::from_secs(2));
let tab = session.tab();
tab.open("https://shop.example/account/orders").await?;
session.shutdown();
# Ok(())
# }
```

## Keep your own data

`Session` implements `Serialize`, and `SessionBuilder` implements `Deserialize`, with the same format as `save`. To keep your own data with the session, such as the account email, the tab's page, or values a site keeps in `localStorage`, put the session in your own struct and write it however you like. Leyline does not read your fields.

```rust,no_run
use leyline::{Session, SessionBuilder, Tab, Url};
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
struct AccountOut<'a> {
    session: &'a Session,
    email: String,
    page: Option<Url>,
}

#[derive(Deserialize)]
struct AccountIn {
    session: SessionBuilder,
    email: String,
    page: Option<Url>,
}

# fn run(session: Session, tab: Tab) -> Result<(), Box<dyn std::error::Error>> {
let out = AccountOut { session: &session, email: "me@example.com".into(), page: tab.current() };
std::fs::write("account/account.json", serde_json::to_vec(&out)?)?;

let account: AccountIn = serde_json::from_slice(&std::fs::read("account/account.json")?)?;
let session = account.session.build()?;
let tab = session.tab();
tab.set_current(account.page);
# let _ = account.email;
# Ok(())
# }
```

`tab.set_current(page)` starts the tab on the saved page, so the next request carries it as its `Referer`. `autosave` writes only the session; write your struct yourself.

Leyline keeps no HTTP cache, `localStorage`, `sessionStorage`, IndexedDB, or service workers, because it runs no scripts. Send `if-none-match` or `if-modified-since` yourself.

## Keep the proxy password out

`save`, `autosave`, and `Serialize` never write the proxy password. To use the password again, read it from your own secret store or an environment variable and set the full proxy URL on the loaded builder.

```rust,no_run
use leyline::{Proxy, Session};

# fn run() -> Result<(), Box<dyn std::error::Error>> {
let password = std::env::var("ACCOUNT_PROXY_PASSWORD")?;
let proxy = format!("http://user:{password}@proxy.example:8080");
let session = Session::load("account/session.json")?
    .proxy(Proxy::from(proxy).env(false))
    .build()?;
# let _ = session;
# Ok(())
# }
```

The saved proxy URL is the configured URL, not the exit IP the proxy assigns. To keep the exit IP, fetch it from your own IP echo service and keep it in your own struct.

## Freeze a profile

A crate upgrade can change a bundled profile, and the profile id check on load then fails. To keep the profile data itself, build the session from the data with `.profile(..)`. The save then stores the profile TOML, and a later load uses it instead of the bundled profile.

```rust,no_run
use leyline::{Browser, Platform, Session};

# fn run() -> leyline::Result<()> {
let bundled = Session::builder()
    .browser(Browser::Chrome154).platform(Platform::Windows)
    .build()?;
let toml = bundled.profile().expect("bundled profile");
let session = Session::builder()
    .profile(toml).platform(Platform::Windows)
    .build()?;
session.save("account/session.json")?;
# Ok(())
# }
```

See [Profile data stability](profiles.md#profile-data-stability).

## The connection state

Each save writes the connection state, and `build()` on a loaded builder restores it.

| Part | Effect after load |
| --- | --- |
| TLS session tickets that have not expired | The first handshake resumes, as a returning browser does |
| HTTP/3 `Alt-Svc` entries and their expiry | HTTP/3 is possible on the first request to a known origin |
| The HSTS store | `http://` requests to a known host go to `https`. See [Network](network.md#hsts) |

A ticket key holds the proxy URL without its user name and password, and a hash of them, so the saved state never holds proxy credentials. The saved state contains TLS session secrets, so store the file with the same protection as credentials.

Saved TLS sessions are bound to the trust that the session loaded when it was built: the pins, the contents of every root it loaded from `ca_der`,
`ca_file`, `SSL_CERT_FILE`, and `SSL_CERT_DIR`, the contents of the client
certificate and key files, and whether system roots are on. A saved TLS
session resumes only in a session that loaded the same trust. Otherwise the
connection makes a full handshake and checks the certificate again. A session from `to_builder()` keeps its parent's trust. One with a new pool, such as `to_builder().share_connections(false)`, loads the CA files again. Changes to the operating system's root store are not part of the binding.

## Browse with a tab

A `Tab` sends each request with the current page as its `from_page`, so the `Referer`, `Origin`, and `sec-fetch-site` headers match a real click. [Requests](requests.md#keep-the-page-with-a-tab) lists the calls. For an account:

- `fetch` on a tab with no page fails with `Kind::Request` ("the tab has no page; open one first") and sends nothing. Open a page first.
- A `.stream()` navigation changes the page at the response head. `current()` reads the page, and `set_current(..)` sets it.
- A page script sets its own `accept`. Set it on the `fetch` builder when the site's script does. See [Fingerprints](fingerprints.md#set-accept-on-a-script-request).

```rust,no_run
use leyline::RequestKind;

# async fn run(session: leyline::Session) -> leyline::Result<()> {
let tab = session.tab();
tab.open("https://shop.example/account").await?;
let orders = tab
    .fetch(leyline::http::Method::GET, "/api/orders?page=1")
    .header("accept", "application/json")
    .send()
    .await?;
let logo = tab.fetch(leyline::http::Method::GET, "/static/logo.png").kind(RequestKind::Image).send().await?;
println!("{} {}", orders.status(), logo.status());
# Ok(())
# }
```

## Log in with the page's form

The `html` feature, on by default, adds `leyline::html` and turns on the `multipart` feature. `html::forms(document)` returns every `<form>` as an `html::Form` with the values a browser would submit:

- Hidden inputs, checked checkboxes, text areas, and selected options.
- One checked radio button per name: the last one, as a browser keeps it.
- Controls outside the `<form>` element that name it with `form="id"`. When two elements share the id, the first one is the form.
- No disabled controls, and no controls inside a disabled `<fieldset>` except in the first `<legend>` that is a direct child of that fieldset.
- No options inside a disabled `<optgroup>`. A list box (`<select size>`) with no selected option sends nothing.
- Line breaks in names and values converted to CRLF, as a browser sends them.
- Values with every HTML named character reference decoded, such as `&amp;` and `&eacute;`.

`Form::find(document, name)` returns the first form whose `id` or `name` attribute is `name`.

`Tab::follow` and `Tab::submit_form` drop the session credentials when the target origin differs from the current page: the `Authorization`, `Cookie`, and `Proxy-Authorization` defaults and the session bearer token, as a redirect does.

| Call | Effect |
| --- | --- |
| `action()`, `method()` | The `action` attribute and the method, `http::Method::GET` or `POST` |
| `enctype()` | The encoding as a MIME type: `application/x-www-form-urlencoded`, `multipart/form-data`, or `text/plain` |
| `base()` | The `href` of the page's `<base>` element, if any |
| `fields()`, `field(name)` | Every `(name, value)` pair, or the first value of one field |
| `set(name, value)` | Replaces every value of the field with one value, or adds the field |
| `buttons()`, `press(name)` | The submit button names; `press` adds one button's name and value. It returns `false` for an unknown name, and adds `name.x` and `name.y` for an image button |

`Tab::submit_form(&form)` sends `form.fields()` as they are, with no submit button: call `form.press(name)` first when the site reads the button. The action resolves against the page's `<base href>`, or the current page when there is none. A `Post` form sends the fields with `RequestKind::FormNavigate`, encoded as its `enctype()` says: `application/x-www-form-urlencoded`, `multipart/form-data`, or `text/plain`. A `Get` form replaces the action's query with the fields. Both are navigations that carry the page as the `Referer` and change the current page.

The session redirect policy applies: after a login `POST`, a 301, 302, or 303 continues as a `GET` without the body, and a 307 or 308 repeats the `POST`. The response and the current page are those of the final URL.

```rust,no_run
use leyline::html::{self, Form};

# async fn run(tab: leyline::Tab) -> leyline::Result<()> {
let page = tab.open("https://shop.example/login").await?.text().await?;
let token = html::meta(&page, "csrf-token").unwrap_or_default();
if let Some(mut form) = Form::find(&page, "login") {
    form.set("email", "me@example.com").set("password", "secret");
    form.press("sign_in");
    let resp = tab.submit_form(&form).await?;
    println!("{} at {}", resp.status(), resp.url());
}
let cart = tab.fetch(leyline::http::Method::GET, "/api/cart").header("x-csrf-token", token).send().await?;
println!("{}", cart.status());
# Ok(())
# }
```

`html::meta(document, name)` returns the `content` of the first `<meta>` whose `name` or `property` equals `name`, without case; many sites put a CSRF token there. `html::links(document)` returns every `<a href>` as an `html::Anchor` with `href()`, `text()`, and `rel()`. `href()` is the raw attribute; pass it to `Tab::follow`, which resolves it.

## Check the login, log in again

Leyline cannot tell whether a site considers you logged in. Request an endpoint that only a logged-in user can read, and log in again when it says no. The jar and `Session::autosave` keep the new cookies.

```rust,no_run
use leyline::html::Form;
use leyline::http::StatusCode;

# async fn run(tab: leyline::Tab) -> leyline::Result<()> {
let me = tab.fetch(leyline::http::Method::GET, "https://shop.example/api/me").send().await?;
if me.status() == StatusCode::UNAUTHORIZED {
    let page = tab.open("https://shop.example/login").await?.text().await?;
    if let Some(mut form) = Form::find(&page, "login") {
        form.set("email", "me@example.com").set("password", "secret");
        tab.submit_form(&form).await?;
    }
}
# Ok(())
# }
```

When several tasks share the session, each can see the 401 at once. Hold one lock, such as a `tokio::sync::Mutex`, around the check and the login, and check again after you take it, so one task logs in and the others reuse its cookies. `examples/accounts.rs` combines save and load, autosave, the login form, and the tab.

## Run tasks at the same time

`Session` and `Tab` are `Clone`, `Send`, and `Sync`. Session clones share the pool, the jar, the identity, and the state, so one `Session::autosave` covers every task. Tab clones share the current page, so give each parallel page its own `session.tab()`.

```rust,no_run
# async fn run(session: leyline::Session) -> leyline::Result<()> {
let orders = session.tab();
let messages = session.tab();
let (a, b) = tokio::join!(
    orders.open("https://shop.example/account/orders"),
    messages.open("https://shop.example/account/messages"),
);
println!("{} {}", a?.status(), b?.status());
# Ok(())
# }
```

To stop every task of the account and run the last autosave, call `session.shutdown()`. See [Sessions](sessions.md#stop-a-session).

## Next

Read [Proxies](proxies.md).
