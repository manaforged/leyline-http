use http::{HeaderMap, StatusCode};

use crate::error::{Error, Kind, Result};

mod body;
mod failure;
mod fields;
mod finish;
pub(crate) mod h1;
mod method;
mod mode;
mod request_headers;
mod scheme;
mod state;
mod timing;
mod tls;
mod upload;

pub use body::Body;
pub(crate) use body::{BodyKind, BoxedStream, Length};
pub(crate) use failure::Failure;
pub use fields::Fields;
pub(crate) use fields::{Seen, text};
#[cfg(feature = "http3")]
pub(crate) use finish::Chunk;
pub(crate) use finish::{STREAM_CHANNEL_DEPTH, body_channel, has_body, length_mismatch};
pub(crate) use method::{has_body_by_default, is_idempotent, is_retrieval, is_safe};
pub use mode::{ErrorBudget, ResponseMode};
pub(crate) use request_headers::{HeaderBlock, RequestHeaders};
pub(crate) use scheme::{Scheme, authority, host_port};
pub(crate) use state::{BlockReader, ResponseBlock, ResponseState};
pub(crate) use timing::ResponseTiming;
pub use tls::{TlsFacts, TlsInfo};
pub(crate) use upload::Upload;

/// One request header field as a name and a value.
pub type HeaderPair = (std::borrow::Cow<'static, str>, crate::header_str::HeaderStr);

pub(crate) const DEFAULT_MAX_BODY_SIZE: usize = 100 * 1024 * 1024;
pub(crate) const DEFAULT_MAX_HEADER_LIST_BYTES: usize = 256 * 1024;

/// The hop-by-hop fields of RFC 9110 section 7.6.1, which never travel past one connection.
pub(crate) const HOP_BY_HOP: [&str; 6] = [
    "connection",
    "proxy-connection",
    "keep-alive",
    "te",
    "transfer-encoding",
    "upgrade",
];

const NOT_ON_MUX: [&str; 2] = ["host", "http2-settings"];

pub(crate) fn is_error_status(status: u16) -> bool {
    status >= 400
}

pub(crate) fn status(code: u16) -> Result<StatusCode> {
    StatusCode::from_u16(code).map_err(|e| {
        Error::new(Kind::Request)
            .with_message(format!("invalid status code {code}"))
            .with_source(e)
    })
}

pub(crate) fn trailers(fields: HeaderMap) -> Option<Box<HeaderMap>> {
    (!fields.is_empty()).then(|| Box::new(fields))
}

#[cfg(feature = "websocket")]
pub(crate) fn string_pairs(fields: &HeaderMap) -> Vec<(String, String)> {
    fields
        .iter()
        .map(|(k, v)| {
            (
                k.as_str().to_owned(),
                String::from_utf8_lossy(v.as_bytes()).into_owned(),
            )
        })
        .collect()
}

pub(crate) type Pump = std::pin::Pin<Box<dyn Future<Output = ()> + Send>>;

/// A streamed response body as a transport hands it over: its chunks and the task that feeds them.
pub struct BodyChannel {
    pub(crate) rx: tokio::sync::mpsc::Receiver<std::io::Result<bytes::Bytes>>,
    pub(crate) pump: Option<Box<Pump>>,
}

impl BodyChannel {
    pub(crate) fn new(rx: tokio::sync::mpsc::Receiver<std::io::Result<bytes::Bytes>>) -> Self {
        Self { rx, pump: None }
    }

    pub(crate) fn pumped_by(mut self, pump: impl Future<Output = ()> + Send + 'static) -> Self {
        self.pump = Some(Box::new(Box::pin(pump)));
        self
    }
}

impl futures_util::Stream for BodyChannel {
    type Item = std::io::Result<bytes::Bytes>;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if let Some(pump) = this.pump.as_mut()
            && (**pump).as_mut().poll(cx).is_ready()
        {
            this.pump = None;
        }
        this.rx.poll_recv(cx)
    }
}

/// A response body as a transport returns it.
pub enum TransportBody {
    /// The whole body, read before return.
    Buffered(Vec<u8>),
    /// The body, streamed after the head.
    Streaming(BodyChannel),
}

impl std::fmt::Debug for TransportBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Buffered(b) => f.debug_struct("Buffered").field("len", &b.len()).finish(),
            Self::Streaming(_) => f.debug_struct("Streaming").finish_non_exhaustive(),
        }
    }
}

/// A response as a transport returns it, before decoding and cookies.
#[derive(Debug)]
pub struct TransportResponse {
    /// The response status.
    pub status: StatusCode,
    /// The response header fields.
    pub headers: Fields,
    /// The response trailer fields, when the response has any.
    pub trailers: Option<Box<HeaderMap>>,
    /// The response body.
    pub body: TransportBody,
    pub(crate) version: http::Version,
    pub(crate) tls: Option<TlsInfo>,
    pub(crate) timing: ResponseTiming,
}

const _: () = assert!(
    size_of::<TransportResponse>() <= 160,
    "TransportResponse moves through every response channel; keep it at most 160 bytes"
);

pub(crate) fn check_framing(headers: &[HeaderPair]) -> Result<()> {
    let (mut te, mut cl) = (0, 0);
    for (name, _) in headers {
        match name.len() {
            17 if name.eq_ignore_ascii_case("transfer-encoding") => te += 1,
            14 if name.eq_ignore_ascii_case("content-length") => cl += 1,
            _ => {}
        }
    }
    if te > 1 {
        return Err(Error::config(format!(
            "{te} Transfer-Encoding headers on one request: framing would be ambiguous"
        )));
    }
    if te > 0 && cl > 0 {
        return Err(Error::config(
            "Transfer-Encoding together with Content-Length on one request: framing would be ambiguous",
        ));
    }
    Ok(())
}

pub(crate) fn h2_ready(headers: &[HeaderPair]) -> bool {
    check_framing(headers).is_ok()
        && headers.iter().all(|(name, value)| {
            !crate::util::has_ascii_uppercase(name) && sent_on(http::Version::HTTP_2, name, value)
        })
}

pub(crate) fn strip_connection_specific_headers(headers: &mut Vec<HeaderPair>) -> Result<()> {
    check_framing(headers)?;
    headers.retain_mut(|(name, value)| {
        if crate::util::has_ascii_uppercase(name) {
            *name = name.to_lowercase().into();
        }
        sent_on(http::Version::HTTP_2, name, value)
    });
    Ok(())
}

pub(crate) fn sent_on(version: http::Version, name: &str, value: &str) -> bool {
    if version == http::Version::HTTP_11 {
        return !name.eq_ignore_ascii_case("priority");
    }
    if name.eq_ignore_ascii_case("te") {
        return value.eq_ignore_ascii_case("trailers");
    }
    !HOP_BY_HOP
        .iter()
        .chain(&NOT_ON_MUX)
        .any(|field| name.eq_ignore_ascii_case(field))
}

#[cfg(test)]
mod h2_header_hygiene_tests;
