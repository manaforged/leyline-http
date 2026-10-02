# Accounts

This page keeps one logged-in account the way a browser profile keeps it:
the same browser, platform, languages, proxy, cookies, and connection state
on every run. A `Device` holds all of them in one file. `Device::autosave`
keeps the file current, a `Tab` keeps the current page, and `leyline::html`
reads the login form.

## What a device holds

| Field | Holds |
| --- | --- |
| `identity`, `platform`, `brand` | The browser the session sends. `Device::capture` stores `Some(ChromiumBrand::Chrome)` for a Chrome session with no `.brand()`, and `None` for another family |
| `profile_toml` | The frozen profile, after `pin_profile` |
| `profile_id` | The id that `check` compares |
| `user_agent` | The `User-Agent` of the session |
| `languages` | The tags from `SessionBuilder::languages` |
| `proxy` | The proxy URL, password included. `Debug` and `Display` mask the password |
| `proxy_password_env` | An environment variable that holds the proxy password |
| `env_proxy` | Whether to read proxy environment variables. Off by default |
| `strict` | When `true`, `check` refuses a device with no `profile_id`, or with no `proxy` while `env_proxy` is `false`. Off by default |
| `jar_path`, `jar` | The jar file, or a jar kept in the device file |
| `state` | The `SessionState`. See [Keep the connection state](#keep-the-connection-state) |
| `page` | The current page of the account's tab |
| `app` | Data of your own, a `BTreeMap<String, serde_json::Value>` |

| Call | Does |
| --- | --- |
| `Device::capture(&session, proxy)` | Reads the device from a session, with the proxy URL you pass |
| `pin_profile(&session)` | Stores the profile TOML in the device |
| `save_to(path)`, `load_from(path)` | Writes and reads the device as JSON, `{ "version": 1, .. }`. The write is atomic, and on Unix the file has mode `0600` |
| `open()` | Builds the session, runs `check`, loads the jar, and restores `state` |
| `session_builder()` | Returns the builder, for more settings before `build()` |
| `check(&session)` | Returns `Kind::Config` when the session differs from the device |
| `tab(&session)` | Returns a `Tab` whose current page is `page` |
| `autosave(&session, path, options)` | Saves the device, its jar, and the session state as they change |

## Create the device once

Pin a browser variant, set the languages and one proxy, and turn the proxy
environment variables off. Then capture the device and freeze its profile.

```rust,no_run
use leyline::{Browser, Device, Platform, ProxyConfig, ProxyUrl, Session};

# fn run() -> leyline::Result<()> {
let proxy = ProxyUrl::parse("http://user:secret@proxy.example:8080")?;
let session = Session::builder()
    .browser(Browser::Chrome154)
    .platform(Platform::Windows)
    .languages(["de-DE", "de", "en"])
    .proxy(ProxyConfig::from(proxy.clone()).env(false))
    .build()?;

let mut device = Device::capture(&session, Some(proxy));
device.pin_profile(&session)?;
device.strict = true;
device.jar_path = Some("account/cookies.json".into());
device.save_to("account/device.json")?;
# Ok(())
# }
```

| Choice | Why |
| --- | --- |
| `Browser::Chrome154`, not `Browser::latest` | `latest` moves to newer captures in patch releases |
| `pin_profile` | A crate upgrade cannot change the ClientHello or the headers: a pinned device loads its own profile TOML and keeps its `profile_id`. It fails for a session whose TLS hello comes from another browser. See [Profile data stability](profiles.md#profile-data-stability) |
| `env_proxy` stays `false` | The device never picks up `HTTPS_PROXY` or `ALL_PROXY` from another machine, and a device without a proxy goes direct |
| `strict` | A strict device cannot go direct by mistake |

`Session::proxy_url()` returns the configured proxy, password included, so
`Device::capture(&session, session.proxy_url())` captures it too. `check`
compares the configured URL, not the exit IP the proxy assigns: a rotating
proxy behind one URL passes with a new IP. To pin the exit IP, fetch it from
your own IP echo service and keep it in `Device::app`.

### Keep the proxy password out of the file

`ProxyUrl` serializes the full URL, password included. To keep the password
out of the device file, set `proxy_password_env` to the name of an
environment variable. `save_to` and `Device::autosave` then write the proxy
URL without its password, and `open` and `session_builder` read it back from
the variable. When the variable is not set, they return `Kind::Config` with
a message that names it.

```rust,no_run
use leyline::{Browser, Device, Platform, ProxyConfig, ProxyUrl, Session, Url};

const PROXY: &str = "http://user@proxy.example:8080";
const PASSWORD_VAR: &str = "ACCOUNT_PROXY_PASSWORD";

# async fn run() -> Result<(), Box<dyn std::error::Error>> {
let mut url = Url::parse(PROXY)?;
url.set_password(Some(&std::env::var(PASSWORD_VAR)?))
    .map_err(|()| "the proxy URL cannot carry a password")?;
let proxy = ProxyUrl::parse(url.as_str())?;

let session = Session::builder()
    .browser(Browser::Chrome154)
    .platform(Platform::Windows)
    .proxy(ProxyConfig::from(proxy).env(false))
    .build()?;
session.get("https://shop.example/login").await?;

let mut device = Device::capture(&session, session.proxy_url());
device.proxy_password_env = Some(PASSWORD_VAR.to_owned());
device.pin_profile(&session)?;
device.save_to("account/device.json")?;
# Ok(())
# }
```

## Open the device on each run

`Device::open` builds the session, runs `check`, loads the cookies, and
restores the saved connection state. A `jar_path` whose file does not exist
yet gives an empty jar. A device with no `jar_path` uses the jar in the
device file, or an empty jar. Then start `Device::autosave` and call its
`shutdown` before the process exits.

```rust,no_run
use std::time::Duration;
use leyline::Device;

# async fn run() -> leyline::Result<()> {
let device = Device::load_from("account/device.json")?;
let session = device.open()?;
let tab = device.tab(&session);
let autosave = device.autosave(&session, "account/device.json", Duration::from_secs(2));
autosave.track(&tab);

tab.follow("/account/orders").await?;
autosave.shutdown().await?;
# Ok(())
# }
```

To add settings before the build, call `device.session_builder()?`, change
the builder, build the session, then call `device.check(&session)?` and
`device.state.restore_into(&session)`.

`check` returns `Kind::Config` and names each field that differs:

| Field | Compared with |
| --- | --- |
| `profile_id` | The session's profile id, when the device has one. It covers the profile data, the platform, and the brand |
| `identity`, `platform`, `brand` | The session's HTTP profile, TLS profile, platform, and brand. Only without a `profile_id` |
| `user_agent` | The session's `User-Agent`, when the device has one and no `profile_id` |
| `languages` | The session's languages |
| `proxy` | `Session::proxy_url()`, unless `env_proxy` is set. The scheme, user name, host, and port must match; the password is not compared |
| `strict` | When `true`: a `profile_id` is present, and a `proxy` is present unless `env_proxy` is set |

`Error::is_profile_changed()` is `true` when the profile differs: the
`profile_id`, or, without one, the identity, platform, brand, or user agent.
It is `false` when only the languages, the proxy, or a `strict` gap differ.

## Save the device as it changes

`Device::autosave(&session, path, options)` starts one writer task and
returns a `DeviceAutosave` handle. `options` is a `DeviceAutosaveOptions`, or
a `Duration` that sets its `interval`:

| Option | Default | Effect |
| --- | --- | --- |
| `interval` | Set by `DeviceAutosaveOptions::new(interval)` | After a change, saves at most once per `interval` |
| `state_interval` | 300 s | Saves at this period, so the connection state stays current |

 Each save writes the device file with the
session's current `SessionState`. With a `jar_path`, it also writes the jar
file when a cookie changed; without one, the jar goes into the device file.
For a jar without a device, use `Jar::autosave`; see
[Cookies](cookies.md#save-the-jar-to-a-file).

| Event | Effect |
| --- | --- |
| A cookie changes, or `update` or `track` runs | A save runs when `interval` has passed since the first unsaved change. Later changes do not move the save |
| `state_interval` passes | A save runs, so new TLS tickets, `Alt-Svc`, and HSTS entries reach the file |
| `flush().await` | Saves now and returns the result |
| `shutdown().await` | Saves and stops the writer |
| The handle is dropped | A last save runs in the background. The runtime must still run |
| The runtime shuts down | The writer task saves unsaved changes as it stops |

Each save writes a temporary file, syncs it, and renames it, so a crash never
leaves a half-written file. On Unix the files have mode `0600`; on Windows
they take the default ACLs of their directory. `autosave` panics outside a Tokio runtime.

The writer keeps its own copy of the device. Change it with
`DeviceAutosave::update(|device| ..)`; the change reaches the file with the
next save. A change to the `Device` you called `autosave` on does not.
`DeviceAutosave::device()` returns a snapshot of the writer's copy.

`DeviceAutosave::track(&tab)` stores the tab's current page in
`Device::page` on each save. The writer keeps one tab: a second `track`
replaces the first. On the next run, `Device::tab(&session)` returns a tab
on that page, so the next request carries it as its `Referer`.

## Keep your own data

`Device::app` holds your own data in the same atomic save, such as the
account email or values a site keeps in `localStorage`. Leyline does not
read it.

```rust,no_run
use leyline::Device;
use serde_json::json;

# fn run(mut device: Device) -> leyline::Result<()> {
device.app.insert("email".into(), json!("me@example.com"));
device.app.insert("local_storage".into(), json!({ "cart_id": "c-1842" }));
let email = device.app.get("email").and_then(|v| v.as_str());
# let _ = email;
device.save_to("account/device.json")?;
# Ok(())
# }
```

Leyline keeps no HTTP cache, `localStorage`, `sessionStorage`, IndexedDB, or
service workers, because it runs no scripts. Send `if-none-match` or
`if-modified-since` yourself.

## Keep the connection state

`Session::state()` returns a `SessionState`, which implements `Serialize` and
`Deserialize`. `state.restore_into(&session)` loads it, and `is_empty()` is
`true` when there is nothing to save.

| Part | Effect after a restore |
| --- | --- |
| TLS session tickets that have not expired | The first handshake resumes, as a returning browser does |
| HTTP/3 `Alt-Svc` entries and their expiry | HTTP/3 is possible on the first request to a known origin |
| The HSTS store | `http://` requests to a known host go to `https`. See [Network](network.md#hsts) |

`Device::autosave` refreshes `device.state` on each save. Before a manual
`save_to`, set `device.state = session.state()`. A ticket key holds the
proxy URL without its user name and password, and a hash of them, so the saved
state never holds proxy credentials.

## Browse with a tab

A `Tab` sends each request with the current page as its initiator, so the
`Referer`, `Origin`, and `sec-fetch-site` headers match a real click.
[Requests](requests.md#keep-the-page-with-a-tab) lists the calls. For an
account:

- `xhr`, `fetch`, `post_json`, and `subresource` on a tab with no page fail
  with `Kind::Request` ("the tab has no page; open one first") and send
  nothing. Open a page first.
- A `.stream()` navigation changes the page at the response head.
  `current()` reads the page, and `set_current(..)` sets it.
- A page script sets its own `accept`. Set it on the `xhr` builder when the
  site's script does. See
  [Fingerprints](fingerprints.md#set-accept-on-a-script-request).

```rust,no_run
use leyline::Preset;

# async fn run(session: leyline::Session) -> leyline::Result<()> {
let tab = session.tab();
tab.open("https://shop.example/account").await?;
let orders = tab
    .xhr("/api/orders?page=1")
    .header("accept", "application/json")
    .send()
    .await?;
let logo = tab.subresource("/static/logo.png", Preset::Image).send().await?;
println!("{} {}", orders.status(), logo.status());
# Ok(())
# }
```

## Log in with the page's form

The `html` feature, on by default, adds `leyline::html` and turns on the
`multipart` feature. `html::forms(document)` returns every `<form>` as an
`html::Form` with the values a browser would submit:

- Hidden inputs, checked checkboxes, text areas, and selected options.
- One checked radio button per name: the last one, as a browser keeps it.
- Controls outside the `<form>` element that name it with `form="id"`.
- No disabled controls, and no controls inside a disabled `<fieldset>`
  except in its first `<legend>`.
- Values with every HTML named character reference decoded, such as `&amp;`
  and `&eacute;`.

`Form::find(document, key)` returns the first form whose `id` or `name` is
`key`.

| Call | Effect |
| --- | --- |
| `action()`, `method()` | The `action` attribute and the `FormMethod`, `Get` or `Post` |
| `enctype()` | The `FormEnctype`: `UrlEncoded`, `Multipart`, or `TextPlain` |
| `base()` | The `href` of the page's `<base>` element, if any |
| `fields()`, `field(name)` | Every `(name, value)` pair, or the first value of one field |
| `set(name, value)` | Replaces every value of the field with one value, or adds the field |
| `buttons()`, `press(name)` | The submit button names; `press` adds one button's name and value. It returns `false` for an unknown name, and adds `name.x` and `name.y` for an image button |

`Tab::submit_form(&form)` sends `form.fields()` as they are, with no submit
button: call `form.press(name)` first when the site reads the button. The
action resolves against the page's `<base href>`, or the current page when
there is none. A `Post` form sends the fields with `Preset::FormNavigate`,
encoded as its `enctype()` says: `application/x-www-form-urlencoded`,
`multipart/form-data`, or `text/plain`. A `Get` form replaces the action's
query with the fields. Both are navigations that carry
the page as the `Referer` and change the current page.

The session redirect policy applies: after a login `POST`, a 301, 302, or
303 continues as a `GET` without the body, and a 307 or 308 repeats the
`POST`. The response and the current page are those of the final URL.

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
let cart = tab.xhr("/api/cart").header("x-csrf-token", token).send().await?;
println!("{}", cart.status());
# Ok(())
# }
```

`html::meta(document, name)` returns the `content` of the first `<meta>`
whose `name` or `property` equals `name`, without case; many sites put a CSRF
token there. `html::links(document)` returns every `<a href>` as an
`html::Anchor` with `href()`, `text()`, and `rel()`. `href()` is the raw
attribute; pass it to `Tab::follow`, which resolves it.

## Check the login, log in again

Leyline cannot tell whether a site considers you logged in. Request an
endpoint that only a logged-in user can read, and log in again when it says
no. The jar and `Device::autosave` keep the new cookies.

```rust,no_run
use leyline::html::Form;
use leyline::http::StatusCode;

# async fn run(tab: leyline::Tab) -> leyline::Result<()> {
let me = tab.xhr("https://shop.example/api/me").send().await?;
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

When several tasks share the session, each can see the 401 at once. Hold one
lock, such as a `tokio::sync::Mutex`, around the check and the login, and
check again after you take it, so one task logs in and the others reuse its
cookies. `examples/device.rs` combines the device, autosave, the login form,
and the tab.

## Run tasks at the same time

`Session` and `Tab` are `Clone`, `Send`, and `Sync`. Session clones share the
pool, the jar, the identity, and the state, so one `Device::autosave` covers
every task. Tab clones share the current page, so give each parallel page its
own `session.tab()`.

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

To stop every task of the account, call `session.shutdown()`, then
`autosave.shutdown().await` for the last save. See
[Sessions](sessions.md#stop-a-session).

## Next

Read [Proxies](proxies.md).
