use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use http::{Request as HttpRequest, Response as HttpResponse};

use crate::core::decompress::DECODED_HEADERS;
use crate::core::response::Response;
use crate::core::session::Session;
use crate::error::{Kind, Result};
use crate::wire::Body;

fn adapt(resp: Response) -> Result<HttpResponse<Body>> {
    let decoded = resp.decodes_body();
    let mut builder = HttpResponse::builder()
        .status(resp.status())
        .version(resp.version())
        .extension(resp.url().clone());
    for (name, value) in resp.headers() {
        if decoded && DECODED_HEADERS.contains(name) {
            continue;
        }
        builder = builder.header(name, value);
    }
    let body = resp.bytes_stream()?;
    builder
        .body(body)
        .map_err(|e| crate::error::Error::new(Kind::Request).with_source(e))
}

impl tower_service::Service<HttpRequest<Body>> for Session {
    type Response = HttpResponse<Body>;
    type Error = crate::error::Error;
    type Future = Pin<Box<dyn Future<Output = Result<HttpResponse<Body>>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: HttpRequest<Body>) -> Self::Future {
        let session = self.clone();
        Box::pin(async move { adapt(session.http_request(req).stream().send().await?) })
    }
}
