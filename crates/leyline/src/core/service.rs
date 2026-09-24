use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use http::{Request as HttpRequest, Response as HttpResponse};

use crate::core::body::Body;
use crate::core::{Kind, Response, Result, Session};

#[derive(Clone)]
pub struct LeylineService {
    session: Session,
}

impl LeylineService {
    pub fn new(session: Session) -> Self {
        Self { session }
    }
}

fn adapt(resp: Response) -> Result<HttpResponse<Body>> {
    let mut builder = HttpResponse::builder().status(resp.status());
    for (name, value) in resp.headers() {
        builder = builder.header(name, value);
    }
    let body = Body::stream(resp.into_stream()?, None);
    builder
        .body(body)
        .map_err(|e| crate::core::Error::new(Kind::Request).with_source(e))
}

impl tower_service::Service<HttpRequest<Body>> for LeylineService {
    type Response = HttpResponse<Body>;
    type Error = crate::core::Error;
    type Future = Pin<Box<dyn Future<Output = Result<HttpResponse<Body>>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: HttpRequest<Body>) -> Self::Future {
        let session = self.session.clone();
        Box::pin(async move { adapt(session.execute(req).await?) })
    }
}
