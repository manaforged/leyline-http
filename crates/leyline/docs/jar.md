The cookie store of a session, with RFC 6265bis rules.

[`Session::cookies`](crate::Session::cookies) returns the session's jar; clones of the session share it. A jar saves with serde, and a saved session includes it.

```rust
use leyline::Url;
use leyline::cookie::Jar;

let jar = Jar::new();
let url = Url::parse("https://example.com/").unwrap();
jar.set(&url, "theme", "dark");
assert_eq!(jar.get(&url, "theme").as_deref(), Some("dark"));
```

See also: [`Session`](crate::Session), [Cookies](https://manaforged.github.io/leyline-http/guide/cookies.html).
