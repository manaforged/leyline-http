use std::fmt;
use std::sync::{Arc, OnceLock};

use http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use url::Url;

use crate::core::body_stream::BodyStream;
use crate::core::error::{Error, Kind};
use crate::trace::masked;
use crate::util::redact;

mod body;
mod download;
mod link;
mod relay;

pub use link::Link;
#[cfg(feature = "bench-internals")]
pub(crate) use link::parse_links;
pub use relay::{RelayBody, relay_headers};

pub(crate) enum ResponseBody {
    Buffered(Vec<u8>),
    Streaming(BodyStream),
    Taken,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum HttpVersion {
    Http1_1,
    Http2,
    Http3,
}

impl HttpVersion {
    pub(crate) fn ja4h_token(self) -> &'static str {
        match self {
            Self::Http1_1 => "1",
            Self::Http2 => "2",
            Self::Http3 => "3",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Http1_1 => "HTTP/1.1",
            Self::Http2 => "HTTP/2",
            Self::Http3 => "HTTP/3",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ResponseTiming {
    pub reused: bool,
    pub connect_ms: Option<u32>,
    pub send_ms: u32,
    pub total_ms: u32,
}

impl ResponseTiming {
    pub(crate) fn accumulator() -> Self {
        Self {
            reused: true,
            connect_ms: None,
            send_ms: 0,
            total_ms: 0,
        }
    }

    pub(crate) fn leg(started: std::time::Instant, connect_ms: Option<u32>) -> Self {
        let total_ms = Self::millis(started);
        Self {
            reused: connect_ms.is_none(),
            connect_ms,
            send_ms: total_ms.saturating_sub(connect_ms.unwrap_or(0)),
            total_ms,
        }
    }

    pub(crate) fn millis(start: std::time::Instant) -> u32 {
        u32::try_from(start.elapsed().as_millis()).unwrap_or(u32::MAX)
    }

    pub(crate) fn add_leg(&mut self, leg: &ResponseTiming) {
        self.total_ms = self.total_ms.saturating_add(leg.total_ms);
        self.send_ms = self.send_ms.saturating_add(leg.send_ms);
        if let Some(c) = leg.connect_ms {
            self.connect_ms = Some(self.connect_ms.unwrap_or(0).saturating_add(c));
        }
        self.reused &= leg.reused;
    }
}

pub struct Response {
    pub(crate) status: StatusCode,
    pub(crate) version: HttpVersion,
    pub(crate) headers: HeaderMap,
    pub(crate) trailers: Vec<(HeaderName, HeaderValue)>,
    pub(crate) body: ResponseBody,
    pub(crate) url: Url,
    pub(crate) redirect_chain: Vec<Url>,
    pub(crate) request_headers: Vec<(String, String)>,
    pub(crate) tls: Option<crate::pool::TlsInfo>,
    pub(crate) request_method: String,
    pub(crate) timing: ResponseTiming,
    pub(crate) audit_tls: Option<Arc<crate::audit::AuditTlsCache>>,
    pub(crate) audit_cache: OnceLock<crate::audit::AuditData>,
    pub(crate) compression: crate::core::CompressionConfig,
    pub(crate) attempts: u32,
    pub(crate) proxy: Option<String>,
}

impl Response {
    pub fn status(&self) -> StatusCode {
        self.status
    }

    pub fn version(&self) -> HttpVersion {
        self.version
    }

    pub fn timing(&self) -> &ResponseTiming {
        &self.timing
    }

    pub fn attempts(&self) -> u32 {
        self.attempts
    }

    pub(crate) fn set_attempts(&mut self, attempts: u32) {
        self.attempts = attempts.max(1);
    }

    pub fn proxy(&self) -> Option<&str> {
        self.proxy.as_deref()
    }

    pub(crate) fn set_proxy(&mut self, proxy: Option<&str>) {
        self.proxy = proxy.map(redact);
    }

    pub fn block(&self) -> Option<crate::core::block::BlockSignal> {
        crate::core::block::BlockRules::builtin().check(self)
    }

    pub fn url(&self) -> &Url {
        &self.url
    }

    pub fn redirect_chain(&self) -> &[Url] {
        &self.redirect_chain
    }

    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    pub fn trailers(&self) -> impl Iterator<Item = (&HeaderName, &HeaderValue)> {
        self.trailers.iter().map(|(k, v)| (k, v))
    }

    pub fn cookies(&self) -> impl Iterator<Item = crate::cookie::Cookie> + '_ {
        let url = &self.url;
        self.headers
            .get_all(http::header::SET_COOKIE)
            .iter()
            .filter_map(move |value| {
                crate::cookie::parse::parse_set_cookie(value.to_str().ok()?, url)
            })
    }

    pub fn tls(&self) -> Option<&crate::pool::TlsInfo> {
        self.tls.as_ref()
    }

    pub fn request_headers(&self) -> impl Iterator<Item = (&str, &str)> {
        self.request_headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
    }

    pub fn content_length(&self) -> Option<u64> {
        self.header("content-length").and_then(|v| v.parse().ok())
    }

    pub fn error_for_status(self) -> crate::core::Result<Self> {
        match self.status_error() {
            Some(err) => Err(err),
            None => Ok(self),
        }
    }

    pub fn error_for_status_ref(&self) -> crate::core::Result<&Self> {
        match self.status_error() {
            Some(err) => Err(err),
            None => Ok(self),
        }
    }

    fn status_error(&self) -> Option<Error> {
        if !is_error_status(self.status) {
            return None;
        }
        Some(
            Error::new(Kind::Status)
                .with_status(self.status)
                .with_url(self.url.clone())
                .with_headers(self.headers.clone())
                .with_trail(self.attempts, self.proxy.clone()),
        )
    }

    pub fn audit(&self) -> Option<&crate::audit::AuditData> {
        let tls = self.audit_tls.as_ref()?;
        Some(self.audit_cache.get_or_init(|| {
            let ja4h = crate::audit::compute_ja4h(&crate::audit::Ja4hInput {
                method: &self.request_method,
                http_version: self.version.ja4h_token(),
                headers: &self.request_headers,
            });
            crate::audit::AuditData {
                ja4: tls.ja4.clone(),
                ja3: tls.ja3.clone(),
                h2_fingerprint: tls.h2_fingerprint.clone(),
                ja4t: tls.ja4t.clone(),
                ja4h,
                permutes_extensions: tls.permutes_extensions,
                request_headers: self.request_headers.clone(),
            }
        }))
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }
}

impl fmt::Debug for Response {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let chain: Vec<String> = self
            .redirect_chain
            .iter()
            .map(|url| redact(url.as_str()))
            .collect();
        f.debug_struct("Response")
            .field("status", &self.status)
            .field("version", &self.version)
            .field("url", &redact(self.url.as_str()))
            .field("redirect_chain", &chain)
            .field(
                "headers",
                &masked(self.headers.iter().map(|(k, v)| (k.as_str(), v.as_bytes()))),
            )
            .field(
                "trailers",
                &masked(
                    self.trailers
                        .iter()
                        .map(|(k, v)| (k.as_str(), v.as_bytes())),
                ),
            )
            .field(
                "request_headers",
                &masked(
                    self.request_headers
                        .iter()
                        .map(|(k, v)| (k.as_str(), v.as_bytes())),
                ),
            )
            .field("timing", &self.timing)
            .field("attempts", &self.attempts)
            .field("proxy", &self.proxy)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests;

pub(crate) fn is_error_status(status: StatusCode) -> bool {
    status.as_u16() >= 400
}
