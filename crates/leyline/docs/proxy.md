Which proxy a session or request uses: one URL, rules per scheme, or a list to rotate through.

A URL string converts into a `Proxy` for every scheme, so `.proxy("socks5h://host:1080")` is enough for one proxy. `Proxy::new()` sets a proxy per scheme, a bypass list, and whether the `HTTP_PROXY` family of variables applies. `Proxy::list(urls)` rotates: each request takes the origin's sticky proxy, or else the next healthy one. A proxy error or a block counts a strike; after `ban_after` strikes, 3 by default, the proxy is banned for `ban_for`, 60 s by default. `Proxy::identified` gives each proxy its own [`Browser`](crate::Browser) and [`Platform`](crate::Platform). Clones of a list share its health. The scheme picks the protocol: `http`, `https`, `socks5`, or `socks5h` for name resolution at the proxy; `socks5` and `socks5h` need the `socks` feature. A saved session stores the proxy URL without its password.

```rust,no_run
use leyline::{Proxy, Session};

# fn run() -> leyline::Result<()> {
let proxy = Proxy::new()
    .https("http://user:pass@proxy.example:8080")
    .no_proxy(["localhost", ".internal.example"]);
let session = Session::builder().proxy(proxy).build()?;
# drop(session);
# Ok(())
# }
```

# Errors

[`SessionBuilder::build`](crate::SessionBuilder::build) returns [`Kind::Config`](crate::Kind::Config) for a proxy URL that does not parse, and for an `identified` list on a session with no browser. A request or WebSocket given a list returns `Kind::Config`. A request returns [`Kind::Proxy`](crate::Kind::Proxy) when the proxy connect or authentication fails, and [`Kind::ProxyTarget`](crate::Kind::ProxyTarget) when the proxy cannot reach the origin.

See also: [Proxies](https://manaforged.github.io/leyline-http/guide/proxies.html).
