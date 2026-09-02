//! Wrap every request attempt in a Tower middleware stack. `Log` writes through `tracing`, so install a subscriber to see its line.

use std::task::{Context, Poll};

use leyline::layer::{Call, Log, Pending, Reply};
use leyline::{Browser, Error, Session};
use tower_layer::{Layer, Stack};
use tower_service::Service;

const URL: &str = "https://example.com";

/// Layer that stamps one header on every attempt.
#[derive(Clone, Copy)]
struct Tag;

impl<S> Layer<S> for Tag {
    type Service = Tagged<S>;

    fn layer(&self, inner: S) -> Tagged<S> {
        Tagged { inner }
    }
}

/// The service [`Tag`] installs.
#[derive(Clone, Copy)]
struct Tagged<S> {
    inner: S,
}

impl<S> Service<Call> for Tagged<S>
where
    S: Service<Call, Response = Reply, Error = Error>,
    S::Future: Send + 'static,
{
    type Response = Reply;
    type Error = Error;
    type Future = Pending;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut call: Call) -> Pending {
        if let Err(e) = call.headers_mut().append("x-example", "layer") {
            return Box::pin(async move { Err(e) });
        }
        Box::pin(self.inner.call(call))
    }
}

#[tokio::main]
async fn main() -> leyline::Result<()> {
    let session = Session::builder()
        .browser(Browser::Chrome147)
        .layer(Stack::new(Tag, Log))
        .build()?;

    let resp = session.get(URL).await?;
    println!("{} {}", resp.status(), resp.url());

    Ok(())
}
