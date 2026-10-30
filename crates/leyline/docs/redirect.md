How a session follows redirects.

The default follows up to 10 redirects. A policy that stops returns the 3xx response; it is not an error.

```rust,no_run
use leyline::{Redirect, Session};

# fn run() -> leyline::Result<()> {
let session = Session::builder().redirect(Redirect::none()).build()?;
let short = Session::builder().redirect(3).build()?;
# drop(short);
# drop(session);
# Ok(())
# }
```

[`Response::redirect_chain`](crate::Response::redirect_chain) lists the hops.

# Errors

A redirect that cannot be followed returns [`Kind::Redirect`](crate::Kind::Redirect).

See also: [Redirects](https://manaforged.github.io/leyline-http/guide/redirects.html).
