use std::sync::{Arc, OnceLock};

use bytes::Bytes;
use http::{HeaderMap, HeaderName, HeaderValue, StatusCode};

use crate::core::body_stream::BodyStream;
use crate::core::error::{Error, Kind, Result};
use crate::core::session::decompress::{Decoder, decompress_body, drain_stream_into_vec};

pub(crate) enum ResponseBody {
    Buffered(Vec<u8>),
    Streaming(BodyStream),
    Taken,
}

impl std::fmt::Debug for ResponseBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Buffered(b) => f
                .debug_struct("ResponseBody::Buffered")
                .field("len", &b.len())
                .finish(),
            Self::Streaming(_) => f
                .debug_struct("ResponseBody::Streaming")
                .finish_non_exhaustive(),
            Self::Taken => f.debug_struct("ResponseBody::Taken").finish(),
        }
    }
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

#[derive(Debug)]
pub struct Response {
    pub(crate) status: StatusCode,
    pub(crate) version: HttpVersion,
    pub(crate) headers: HeaderMap,
    pub(crate) trailers: Vec<(HeaderName, HeaderValue)>,
    pub(crate) body: ResponseBody,
    pub(crate) url: String,
    pub(crate) redirect_chain: Vec<String>,
    pub(crate) request_headers: Vec<(String, String)>,
    pub(crate) tls: Option<crate::pool::TlsInfo>,
    pub(crate) request_method: String,
    pub(crate) timing: ResponseTiming,
    pub(crate) audit_tls: Option<Arc<crate::audit::AuditTlsCache>>,
    pub(crate) audit_cache: OnceLock<crate::audit::AuditData>,
    pub(crate) compression: crate::core::CompressionConfig,
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

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn redirect_chain(&self) -> &[String] {
        &self.redirect_chain
    }

    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    pub fn trailers(&self) -> impl Iterator<Item = (&HeaderName, &HeaderValue)> {
        self.trailers.iter().map(|(k, v)| (k, v))
    }

    pub fn cookies(&self) -> impl Iterator<Item = crate::cookie::Cookie> + '_ {
        let url = url::Url::parse(&self.url).ok();
        self.headers
            .get_all(http::header::SET_COOKIE)
            .iter()
            .filter_map(move |value| {
                crate::cookie::parse::parse_set_cookie(value.to_str().ok()?, url.as_ref()?)
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

    pub async fn text(self) -> crate::core::Result<String> {
        self.text_with_charset("utf-8").await
    }

    #[cfg(feature = "charset")]
    pub async fn text_with_charset(self, default_encoding: &str) -> crate::core::Result<String> {
        let label = self.charset_label().map(str::to_owned);
        let bytes = self.bytes().await?;
        let encoding = encoding_rs::Encoding::for_label(
            label.as_deref().unwrap_or(default_encoding).as_bytes(),
        )
        .unwrap_or(encoding_rs::UTF_8);
        Ok(encoding.decode(&bytes).0.into_owned())
    }

    #[cfg(not(feature = "charset"))]
    pub async fn text_with_charset(self, _default_encoding: &str) -> crate::core::Result<String> {
        Ok(String::from_utf8_lossy(&self.bytes().await?).into_owned())
    }

    #[cfg(feature = "charset")]
    fn charset_label(&self) -> Option<&str> {
        let ct = self.header("content-type")?;
        ct.split(';').skip(1).find_map(|param| {
            let (k, v) = param.split_once('=')?;
            k.trim()
                .eq_ignore_ascii_case("charset")
                .then(|| v.trim().trim_matches('"'))
        })
    }

    pub async fn bytes(self) -> crate::core::Result<Bytes> {
        let stream = match self.body {
            ResponseBody::Buffered(b) => return Ok(Bytes::from(b)),
            ResponseBody::Taken => {
                return Err(Error::new(Kind::Body).with_message(
                    "response body stream was taken by `into_stream`; read the bytes from that stream",
                ));
            }
            ResponseBody::Streaming(s) => s,
        };
        let buf = drain_stream_into_vec(stream, self.compression.max_body_size).await?;
        let encoding = self.headers.get(http::header::CONTENT_ENCODING).map(|v| {
            String::from_utf8_lossy(v.as_bytes())
                .trim()
                .to_ascii_lowercase()
        });
        let (buf, _) = decompress_body(buf, encoding.as_deref(), &self.compression)?;
        Ok(Bytes::from(buf))
    }

    pub async fn json<T: serde::de::DeserializeOwned>(self) -> crate::core::Result<T> {
        serde_json::from_slice(&self.bytes().await?).map_err(Error::from_json)
    }

    pub fn into_stream(mut self) -> Result<BodyStream> {
        match std::mem::replace(&mut self.body, ResponseBody::Taken) {
            ResponseBody::Streaming(s) => Ok(s),
            ResponseBody::Buffered(b) => Ok(BodyStream::from_bytes(bytes::Bytes::from(b))),
            ResponseBody::Taken => Err(Error::new(Kind::Body)
                .with_message("response body has already been taken as a stream")),
        }
    }

    pub async fn copy_to<W>(self, writer: &mut W) -> Result<u64>
    where
        W: tokio::io::AsyncWrite + Unpin,
    {
        use futures_util::StreamExt;
        use tokio::io::AsyncWriteExt;

        let mut stream = self.into_stream()?;
        let mut total: u64 = 0;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(Error::from)?;
            writer.write_all(&chunk).await.map_err(Error::from)?;
            total += chunk.len() as u64;
        }
        writer.flush().await.map_err(Error::from)?;
        Ok(total)
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
        if self.status.as_u16() < 400 {
            return None;
        }
        let err = Error::new(Kind::Status).with_status(self.status);
        Some(match self.url.parse::<http::Uri>() {
            Ok(uri) => err.with_url(uri),
            Err(_) => err,
        })
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
            }
        }))
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }

    pub async fn read_until<F>(self, limit: usize, mut done: F) -> Result<Vec<u8>>
    where
        F: FnMut(&[u8], usize) -> bool,
    {
        use futures_util::StreamExt;

        let encoding = self
            .header("content-encoding")
            .map(|v| v.trim().to_ascii_lowercase());
        let mut decoder = Decoder::new(encoding.as_deref(), &self.compression)?;
        let mut stream = self.into_stream()?;
        let mut out = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(Error::from)?;
            let from = out.len();
            match decoder.as_mut() {
                Some(decoder) => decoder.feed(&chunk, &mut out)?,
                None => out.extend_from_slice(&chunk),
            }
            if done(&out, from) || out.len() >= limit {
                return Ok(out);
            }
        }
        if let Some(decoder) = decoder.as_mut() {
            decoder.finish(&mut out)?;
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests;
