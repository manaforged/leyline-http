One request, built from a [`Session`](crate::Session) and sent with [`send`](crate::RequestBuilder::send).

Request setters override the session for this request only. Nothing is sent until `send().await`.

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
