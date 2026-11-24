One request, built from a [`Session`](crate::Session) and sent with `.await`, [`send`](crate::RequestBuilder::send), [`text`](crate::RequestBuilder::text), or [`bytes`](crate::RequestBuilder::bytes).

Request setters override the session for this request only. Sending is lazy.

[`build`](crate::RequestBuilder::build) converts the request into an `http::Request` without sending it, and [`Session::request_from`](crate::Session::request_from) converts one back. [`try_clone`](crate::RequestBuilder::try_clone) copies a request whose body is not a stream.

`download_to(path).await` sends and saves a resumable download. `events()` returns a lazy SSE stream; awaiting its `next()` sends the request.

```rust,no_run
use std::time::Duration;

use leyline::Session;

# async fn run() -> leyline::Result<()> {
let session = Session::new();
let resp = session
    .post("https://api.example.com/items")
    .bearer_auth("token")
    .json(&serde_json::json!({ "id": 7 }))
    .timeout(Duration::from_secs(5))
    .error_for_status()
    .send()
    .await?;
# drop(resp);
# Ok(())
# }
```

# Errors

[`send`](crate::RequestBuilder::send) returns the [`Error`](crate::Error) of the failed step; [`Error::kind`](crate::Error::kind) names it. Without `error_for_status`, a 4xx or 5xx is a normal [`Response`](crate::Response).

See also: [`Response`](crate::Response), [Requests](https://manaforged.github.io/leyline-http/guide/requests.html).
