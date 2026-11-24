use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use http::{Request as HttpRequest, Response as HttpResponse};

use crate::core::session::Session;
use crate::error::Result;
use crate::wire::Body;

impl tower_service::Service<HttpRequest<Body>> for Session {
    type Response = HttpResponse<Body>;
    type Error = crate::error::Error;
    type Future = Pin<Box<dyn Future<Output = Result<HttpResponse<Body>>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: HttpRequest<Body>) -> Self::Future {
        let session = self.clone();
        Box::pin(async move { session.request_from(req).stream().send().await?.into_http() })
    }
}
