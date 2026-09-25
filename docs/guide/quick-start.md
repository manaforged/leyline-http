# Quick start

This chapter takes you from an empty project to a request and a parsed
response.

## Add the dependency

The crate is `leyline-http`. The library it builds is `leyline`, so that is the
name you import.

```toml
[dependencies]
leyline-http = "0.1"
tokio = { version = "1", features = ["full"] }
```

Leyline is async and runs on Tokio. Every send is an `async fn`. The JSON
example below also needs `serde_json = "1"` in the same manifest.

## Send a GET request

`Session::new()` builds a session that impersonates the newest captured
Chrome, currently Chrome 154, on Windows.

```rust,no_run
use leyline::Session;

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let session = Session::new();
    let resp = session.get("https://example.com/").await?;
    println!("{}", resp.status());
    Ok(())
}
```

## Read the body as text

`Response::text` decodes the body with the charset from the `Content-Type`
header, and falls back to UTF-8.

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let body = session.get("https://example.com/").await?.text().await?;
println!("{} bytes", body.len());
# Ok(())
# }
```

## Read the body as JSON

`Response::json` deserializes the body into any type that implements
`serde::de::DeserializeOwned`.

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();
let value: serde_json::Value = session
    .get("https://example.com/api/state")
    .await?
    .json()
    .await?;
println!("{value}");
# Ok(())
# }
```

## Await the builder, or call send

`Session::get` and its siblings return a `RequestBuilder`. The builder
implements `IntoFuture`, so awaiting it sends the request. Call `.send()` when
you want the send to be explicit, or when you need to hold the future in a
variable first.

```rust,no_run
# async fn run() -> leyline::Result<()> {
let session = leyline::Session::new();

// Await the builder.
let a = session.get("https://example.com/one").await?;

// Or send it yourself. Same request, same result.
let b = session.get("https://example.com/two").send().await?;

# let _ = (a, b);
# Ok(())
# }
```

## Next

Read [Sessions](sessions.md) to choose which browser you present as.
