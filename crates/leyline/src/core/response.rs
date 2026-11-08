use std::fmt;
use std::sync::{Arc, OnceLock};

use http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use url::Url;

use crate::core::body_stream::BodyStream;
use crate::error::{Error, Kind};
use crate::util::masked;
use crate::util::redact;
use crate::wire::ResponseTiming;
use crate::wire::is_error_status;

mod body;
mod download;
mod link;
mod prefix;
mod relay;

pub use link::Link;
#[cfg(feature = "bench-internals")]
pub(crate) use link::parse_links;
pub(crate) use prefix::StopReason;
pub use relay::relay_headers;

pub(crate) enum ResponseBody {
    Buffered(Vec<u8>),
    Streaming(Box<BodyStream>),
    Taken,
}

#[doc = include_str!("../../docs/response.md")]
pub struct Response {
    pub(crate) parts: Box<Parts>,
}

pub(crate) struct Parts {
    pub(crate) status: StatusCode,
    pub(crate) version: http::Version,
    pub(crate) headers: crate::wire::Fields,
    pub(crate) trailers: Option<Box<HeaderMap>>,
    pub(crate) body: ResponseBody,
    pub(crate) url: Arc<Url>,
    pub(crate) redirect_chain: Vec<Url>,
    pub(crate) request_headers: Vec<(String, String)>,
    pub(crate) tls: Option<crate::wire::TlsInfo>,
    pub(crate) request_method: Option<http::Method>,
    pub(crate) timing: ResponseTiming,
    pub(crate) audit_tls: Option<Arc<crate::audit::AuditTlsCache>>,
    pub(crate) audit_cache: OnceLock<crate::audit::Fingerprint>,
    pub(crate) compression: crate::core::BodyRules,
    pub(crate) attempts: u32,
    pub(crate) proxy: Option<String>,
    pub(crate) block: Option<crate::core::block::Block>,
}

impl Response {
    pub(crate) fn from_parts(parts: Parts) -> Self {
        Self {
            parts: Box::new(parts),
        }
    }

    /// Returns the response status.
    pub fn status(&self) -> StatusCode {
        self.parts.status
    }

    /// Returns the HTTP version of the response.
    pub fn version(&self) -> http::Version {
        self.parts.version
    }

    /// Returns whether every attempt reused a pooled connection.
    pub fn reused(&self) -> bool {
        self.parts.timing.reused
    }

    /// Returns the time spent opening new connections, or `None` when every attempt reused one.
    pub fn connect_time(&self) -> Option<std::time::Duration> {
        self.parts
            .timing
            .connect_ms
            .map(|ms| std::time::Duration::from_millis(u64::from(ms)))
    }

    /// Returns the time spent sending the request and waiting for the response.
    pub fn send_time(&self) -> std::time::Duration {
        std::time::Duration::from_millis(u64::from(self.parts.timing.send_ms))
    }

    /// Returns the total time from the start of the request to the response headers.
    pub fn elapsed(&self) -> std::time::Duration {
        std::time::Duration::from_millis(u64::from(self.parts.timing.total_ms))
    }

    /// Returns the number of attempts made, including retries.
    pub fn attempts(&self) -> u32 {
        self.parts.attempts
    }

    pub(crate) fn set_attempts(&mut self, attempts: u32) {
        self.parts.attempts = attempts.max(1);
    }

    /// Returns the redacted proxy address the response came through.
    pub fn proxy(&self) -> Option<&str> {
        self.parts.proxy.as_deref()
    }

    pub(crate) fn set_proxy(&mut self, proxy: Option<&str>) {
        self.parts.proxy = proxy.map(redact);
    }

    /// Returns the block detected in the response, if any.
    pub fn block(&self) -> Option<&crate::core::block::Block> {
        self.parts.block.as_ref()
    }

    /// Returns the final URL after redirects.
    pub fn url(&self) -> &Url {
        &self.parts.url
    }

    /// Returns the URLs of each redirect followed, in order.
    pub fn redirect_chain(&self) -> &[Url] {
        &self.parts.redirect_chain
    }

    /// Returns the response headers.
    pub fn headers(&self) -> &HeaderMap {
        self.parts.headers.map()
    }

    /// Returns the response trailers.
    pub fn trailers(&self) -> impl Iterator<Item = (&HeaderName, &HeaderValue)> {
        self.parts
            .trailers
            .iter()
            .flat_map(|trailers| trailers.iter())
    }

    /// Parses each `Set-Cookie` header of the response into a cookie.
    pub fn cookies(&self) -> impl Iterator<Item = crate::cookie::Cookie> + '_ {
        let url = &self.parts.url;
        self.parts
            .headers
            .map()
            .get_all(http::header::SET_COOKIE)
            .iter()
            .filter_map(move |value| {
                crate::cookie::parse::parse_set_cookie(value.to_str().ok()?, url)
            })
    }

    /// Returns the negotiated TLS version.
    #[must_use]
    pub fn tls_version(&self) -> Option<&str> {
        self.parts
            .tls
            .as_ref()
            .and_then(|tls| tls.version.as_deref())
    }

    /// Returns the negotiated TLS cipher suite.
    #[must_use]
    pub fn tls_cipher(&self) -> Option<&str> {
        self.parts
            .tls
            .as_ref()
            .and_then(|tls| tls.cipher.as_deref())
    }

    /// Returns the DER bytes of the server certificate.
    #[must_use]
    pub fn peer_certificate(&self) -> Option<&[u8]> {
        self.parts
            .tls
            .as_ref()
            .and_then(|tls| tls.peer_cert_der.as_deref())
    }

    /// Returns the headers that were sent with the request.
    pub fn request_headers(&self) -> impl Iterator<Item = (&str, &str)> {
        self.parts
            .request_headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// Returns the `content-length` header value as a number.
    pub fn content_length(&self) -> Option<u64> {
        self.header("content-length")
            .and_then(crate::util::content_length)
    }

    /// Returns the response, or an error of kind `Kind::Status` when the status is an error.
    pub fn error_for_status(&self) -> crate::error::Result<&Self> {
        match self.status_error() {
            Some(err) => Err(err),
            None => Ok(self),
        }
    }

    fn status_error(&self) -> Option<Error> {
        if !is_error_status(self.parts.status.as_u16()) {
            return None;
        }
        Some(
            Error::new(Kind::Status)
                .with_status(self.parts.status)
                .with_url(Url::clone(&self.parts.url))
                .with_headers(self.parts.headers.map().clone())
                .with_trail(self.parts.attempts, self.parts.proxy.clone()),
        )
    }

    /// Returns the request fingerprints, or `None` when auditing is off.
    pub fn fingerprint(&self) -> Option<&crate::audit::Fingerprint> {
        let tls = self.parts.audit_tls.as_ref()?;
        Some(self.parts.audit_cache.get_or_init(|| {
            tls.for_request(
                self.parts
                    .request_method
                    .as_ref()
                    .map_or("", http::Method::as_str),
                self.parts.version,
                &self.parts.request_headers,
            )
        }))
    }

    /// Returns the named header value as text, or `None` when it is absent or not visible ASCII.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.parts
            .headers
            .map()
            .get(name)
            .and_then(|v| v.to_str().ok())
    }
}

impl fmt::Debug for Response {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let chain: Vec<String> = self
            .parts
            .redirect_chain
            .iter()
            .map(|url| redact(url.as_str()))
            .collect();
        f.debug_struct("Response")
            .field("status", &self.parts.status)
            .field("version", &self.parts.version)
            .field("url", &redact(self.parts.url.as_str()))
            .field("redirect_chain", &chain)
            .field(
                "headers",
                &masked(
                    self.parts
                        .headers
                        .map()
                        .iter()
                        .map(|(k, v)| (k.as_str(), v.as_bytes())),
                ),
            )
            .field(
                "trailers",
                &masked(
                    self.parts
                        .trailers
                        .iter()
                        .flat_map(|trailers| trailers.iter())
                        .map(|(k, v)| (k.as_str(), v.as_bytes())),
                ),
            )
            .field(
                "request_headers",
                &masked(
                    self.parts
                        .request_headers
                        .iter()
                        .map(|(k, v)| (k.as_str(), v.as_bytes())),
                ),
            )
            .field("timing", &self.parts.timing)
            .field("attempts", &self.parts.attempts)
            .field("proxy", &self.parts.proxy)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests;
