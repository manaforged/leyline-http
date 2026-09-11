use std::borrow::Cow;
use std::future::{Future, poll_fn};
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Instant;

use http::{Method, StatusCode, Uri};
use tower_layer::Layer;
use tower_service::Service;

use crate::core::body::Body;
use crate::core::error::{Error, Kind, Result};
use crate::core::headers::HeaderList;
use crate::core::response::HttpVersion;
use crate::core::transport::{Prepared, TransportBody, TransportResponse};
use crate::core::{ResponseTiming, Session};
use crate::profile::preset::HeaderPair;

pub type Pending = Pin<Box<dyn Future<Output = Result<Reply>> + Send>>;

pub struct Call {
    method: Method,
    uri: Uri,
    headers: HeaderList,
    body: Body,
    proxy: Option<String>,
    stream: bool,
    url: url::Url,
    session: Session,
}

impl Call {
    pub fn method(&self) -> &Method {
        &self.method
    }

    pub fn uri(&self) -> &Uri {
        &self.uri
    }

    pub fn headers(&self) -> &HeaderList {
        &self.headers
    }

    pub fn headers_mut(&mut self) -> &mut HeaderList {
        &mut self.headers
    }

    pub fn body(&self) -> &Body {
        &self.body
    }

    pub fn proxy(&self) -> Option<&str> {
        self.proxy.as_deref()
    }

    pub fn stream(&self) -> bool {
        self.stream
    }

    pub(crate) fn new(session: Session, req: Prepared<'_>) -> Result<Self> {
        let Prepared {
            method,
            url,
            headers,
            body,
            proxy,
            stream_response,
        } = req;
        let uri: Uri = url
            .as_str()
            .parse()
            .map_err(|e| Error::new(Kind::Request).with_source(e))?;
        Ok(Self {
            method: Method::from_bytes(method.as_bytes())
                .map_err(|e| Error::new(Kind::Request).with_source(e))?,
            uri,
            headers: HeaderList::from_pairs(
                headers
                    .into_iter()
                    .map(|(k, v)| (k.into_owned(), v.into_owned()))
                    .collect(),
            )?,
            body,
            proxy: proxy.map(str::to_string),
            stream: stream_response,
            url: url.clone(),
            session,
        })
    }

    async fn run(self) -> Result<Reply> {
        let Self {
            method,
            headers,
            body,
            proxy,
            stream,
            url,
            session,
            ..
        } = self;
        let pairs: Vec<HeaderPair> = headers
            .iter()
            .map(|(name, value)| {
                (
                    Cow::Owned(name.as_str().to_string()),
                    Cow::Owned(String::from_utf8_lossy(value.as_bytes()).into_owned()),
                )
            })
            .collect();
        let resp = session
            .send_with_policy(Prepared {
                method: method.as_str(),
                url: &url,
                headers: pairs,
                body,
                proxy: proxy.as_deref(),
                stream_response: stream,
            })
            .await?;
        Ok(Reply { inner: resp })
    }
}

pub struct Reply {
    inner: TransportResponse,
}

impl Reply {
    pub fn new(status: StatusCode) -> Self {
        Self {
            inner: TransportResponse {
                status,
                headers: Vec::new(),
                trailers: Vec::new(),
                body: TransportBody::Buffered(Vec::new()),
                final_url: String::new(),
                version: HttpVersion::Http1_1,
                tls_alpn: None,
                peer_cert_der: None,
                tls_version: None,
                tls_cipher: None,
                timing: ResponseTiming::accumulator(),
            },
        }
    }

    pub fn header(
        mut self,
        name: impl TryInto<http::HeaderName>,
        value: impl TryInto<http::HeaderValue>,
    ) -> Result<Self> {
        let name = crate::core::headers::name(name)?;
        let value = crate::core::headers::value(value)?;
        self.inner.headers.push((name, value));
        Ok(self)
    }

    pub fn body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.inner.body = TransportBody::Buffered(body.into());
        self
    }

    pub fn status(&self) -> StatusCode {
        self.inner.status
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.inner
            .headers
            .iter()
            .find(|(k, _)| k.as_str().eq_ignore_ascii_case(name))
            .and_then(|(_, v)| v.to_str().ok())
    }

    pub(crate) fn seal(mut self, url: &url::Url) -> TransportResponse {
        if self.inner.final_url.is_empty() {
            self.inner.final_url = url.to_string();
        }
        self.inner
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Transport;

impl Service<Call> for Transport {
    type Response = Reply;
    type Error = Error;
    type Future = Pending;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, call: Call) -> Pending {
        Box::pin(call.run())
    }
}

pub(crate) trait Stack: Send + Sync {
    fn call(&self, call: Call) -> Pending;
}

pub(crate) struct Hold<S>(pub(crate) S);

impl<S> Stack for Hold<S>
where
    S: Service<Call, Response = Reply, Error = Error> + Clone + Send + Sync + 'static,
    S::Future: Send + 'static,
{
    fn call(&self, call: Call) -> Pending {
        let mut svc = self.0.clone();
        Box::pin(async move {
            poll_fn(|cx| svc.poll_ready(cx)).await?;
            svc.call(call).await
        })
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Log;

impl<S> Layer<S> for Log {
    type Service = Logged<S>;

    fn layer(&self, inner: S) -> Logged<S> {
        Logged { inner }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Logged<S> {
    inner: S,
}

impl<S> Service<Call> for Logged<S>
where
    S: Service<Call, Response = Reply, Error = Error>,
    S::Future: Send + 'static,
{
    type Response = Reply;
    type Error = Error;
    type Future = Pending;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<()>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, call: Call) -> Pending {
        let method = call.method.clone();
        let host = call.url.host_str().unwrap_or("").to_string();
        let start = Instant::now();
        let next = self.inner.call(call);
        Box::pin(async move {
            let out = next.await;
            let status = match &out {
                Ok(reply) => reply.status().as_u16(),
                Err(_) => 0,
            };
            tracing::info!(
                target: "leyline::layer",
                method = %method,
                host = %host,
                status,
                elapsed_ms = start.elapsed().as_millis() as u64,
                "call"
            );
            out
        })
    }
}

#[cfg(test)]
#[path = "layer_tests.rs"]
mod tests;
