use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use http::{Request as HttpRequest, Response as HttpResponse};

use crate::core::body::Body;
use crate::core::session::decompress::{Decoder, content_codings};
use crate::core::{HttpVersion, Kind, Response, Result, Session};

#[derive(Clone, Debug)]
pub struct LeylineService {
    session: Session,
}

impl LeylineService {
    pub fn new(session: Session) -> Self {
        Self { session }
    }
}

fn http_version(version: HttpVersion) -> http::Version {
    match version {
        HttpVersion::Http1_1 => http::Version::HTTP_11,
        HttpVersion::Http2 => http::Version::HTTP_2,
        HttpVersion::Http3 => http::Version::HTTP_3,
    }
}

fn adapt(resp: Response) -> Result<HttpResponse<Body>> {
    let encoding = content_codings(resp.headers().get_all(http::header::CONTENT_ENCODING));
    let decoded = Decoder::new(encoding.as_deref(), &resp.compression)?.is_some();
    let mut builder = HttpResponse::builder()
        .status(resp.status())
        .version(http_version(resp.version()))
        .extension(resp.url().clone())
        .extension(resp.version())
        .extension(*resp.timing());
    for (name, value) in resp.headers() {
        if decoded
            && (name == http::header::CONTENT_ENCODING || name == http::header::CONTENT_LENGTH)
        {
            continue;
        }
        builder = builder.header(name, value);
    }
    let body = Body::stream(resp.into_decoded_stream(None)?, None);
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
        Box::pin(async move { adapt(session.http_request(req).stream().send().await?) })
    }
}
