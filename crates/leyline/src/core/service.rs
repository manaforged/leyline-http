use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use http::{HeaderName, Request as HttpRequest, Response as HttpResponse};

use crate::core::body::Body;
use crate::core::{Kind, Request, Response, Result, Session};

#[derive(Clone)]
pub struct LeylineService {
    session: Session,
}

impl LeylineService {
    pub fn new(session: Session) -> Self {
        Self { session }
    }

    pub fn session(&self) -> &Session {
        &self.session
    }
}

impl From<HttpRequest<Body>> for Request {
    fn from(req: HttpRequest<Body>) -> Self {
        let (parts, body) = req.into_parts();
        let mut out = Request::new(parts.method, parts.uri);
        let mut last: Option<HeaderName> = None;
        for (name, value) in parts.headers {
            if name.is_some() {
                last = name;
            }
            if let Some(name) = last.clone() {
                drop(out.headers.append(name, value));
            }
        }
        out.body = body;
        out
    }
}

fn adapt(resp: Response) -> Result<HttpResponse<Body>> {
    let mut builder = HttpResponse::builder().status(resp.status());
    for (name, value) in resp.headers() {
        builder = builder.header(name, value);
    }
    let body = Body::stream(resp.into_stream()?);
    builder
        .body(body)
        .map_err(|e| crate::core::Error::new(Kind::Request).with_source(e))
}

impl tower_service::Service<Request> for LeylineService {
    type Response = Response;
    type Error = crate::core::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Response>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request) -> Self::Future {
        let session = self.session.clone();
        Box::pin(async move { session.execute(req).await })
    }
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
        Box::pin(async move { adapt(session.execute(Request::from(req)).await?) })
    }
}
