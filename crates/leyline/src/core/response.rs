//! HTTP response types.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use http::{HeaderMap, HeaderName, HeaderValue, StatusCode};

use crate::core::body_stream::BodyStream;
use crate::core::error::{Error, Kind, Result};
use crate::core::session::decompress::{decompress_and_strip as strip, drain_stream_into_vec};

/// Internal body representation.
pub(crate) enum ResponseBody {
    /// Fully-materialised bytes.
    Buffered(Vec<u8>),
    /// Streaming delivery.
    Streaming(BodyStream),
    /// Streaming body that has already been taken via `into_stream`.
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

/// HTTP protocol version used for the response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum HttpVersion {
    /// HTTP/1.1.
    Http1_1,
    /// HTTP/2.
    Http2,
    /// HTTP/3.
    Http3,
}

impl HttpVersion {
    /// Version token used by JA4H.
    pub(crate) fn ja4h_token(self) -> &'static str {
        match self {
            Self::Http1_1 => "1",
            Self::Http2 => "2",
            Self::Http3 => "3",
        }
    }

    /// Human-readable protocol token.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Http1_1 => "HTTP/1.1",
            Self::Http2 => "HTTP/2",
            Self::Http3 => "HTTP/3",
        }
    }
}

/// Wall-clock timing for a request, summed across redirect legs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ResponseTiming {
    /// Every leg reused a pooled (warm) connection, so no connect/handshake cost was paid (`connect_ms` is `None`).
    pub reused: bool,
    /// DNS + TCP connect + TLS handshake + H2 preface for the leg(s) that opened a fresh connection, in milliseconds.
    pub connect_ms: Option<u32>,
    /// Request-send → response, in milliseconds, summed across redirect legs.
    pub send_ms: u32,
    /// Whole transport exchange (connect, if any, plus send), summed across redirect legs, in milliseconds.
    pub total_ms: u32,
}

impl ResponseTiming {
    /// Seed for accumulating a redirect-following request across its legs.
    pub(crate) fn accumulator() -> Self {
        Self {
            reused: true,
            connect_ms: None,
            send_ms: 0,
            total_ms: 0,
        }
    }

    /// Fold one transport leg into the running total.
    pub(crate) fn add_leg(&mut self, leg: &ResponseTiming) {
        self.total_ms = self.total_ms.saturating_add(leg.total_ms);
        self.send_ms = self.send_ms.saturating_add(leg.send_ms);
        if let Some(c) = leg.connect_ms {
            self.connect_ms = Some(self.connect_ms.unwrap_or(0).saturating_add(c));
        }
        self.reused &= leg.reused;
    }
}

/// An HTTP response. The body is buffered unless the request used `.stream()`.
#[derive(Debug)]
pub struct Response {
    pub(crate) status: StatusCode,
    pub(crate) version: HttpVersion,
    pub(crate) headers: Vec<(HeaderName, HeaderValue)>,
    pub(crate) trailers: Vec<(HeaderName, HeaderValue)>,
    pub(crate) body: ResponseBody,
    pub(crate) cookies: HashMap<String, String>,
    pub(crate) url: String,
    pub(crate) redirect_chain: Vec<String>,
    pub(crate) request_headers: Vec<(String, String)>,
    pub(crate) tls_alpn: Option<String>,
    pub(crate) tls_peer_certificate: Option<Vec<u8>>,
    pub(crate) tls_version: Option<String>,
    pub(crate) tls_cipher: Option<String>,
    /// Request method, kept for lazy JA4H computation in [`Response::audit`].
    pub(crate) request_method: String,
    /// Wall-clock timing breakdown for the request, summed across any redirect legs.
    pub(crate) timing: ResponseTiming,
    /// Shared connection-level fingerprints (JA4/JA3/H2/JA4T).
    pub(crate) audit_tls: Option<Arc<crate::audit::AuditTlsCache>>,
    /// Memoised full audit block.
    pub(crate) audit_cache: OnceLock<crate::audit::AuditData>,
    /// Decompression settings applied when a streamed body is drained.
    pub(crate) compression: crate::core::CompressionConfig,
}

impl Response {
    /// HTTP status code.
    pub fn status(&self) -> StatusCode {
        self.status
    }

    /// HTTP protocol version used for this response.
    pub fn version(&self) -> HttpVersion {
        self.version
    }

    /// Timing summed across redirect legs. A hop that did not record connect/send is zero.
    pub fn timing(&self) -> &ResponseTiming {
        &self.timing
    }

    /// Final URL (after redirects).
    pub fn url(&self) -> &str {
        &self.url
    }

    /// URLs visited during the redirect chain, in order.
    pub fn redirect_chain(&self) -> &[String] {
        &self.redirect_chain
    }

    /// Response headers in wire order, as `(name, value)` pairs.
    pub fn headers(&self) -> impl Iterator<Item = (&HeaderName, &HeaderValue)> {
        self.headers.iter().map(|(k, v)| (k, v))
    }

    /// Response headers copied into a [`HeaderMap`]; wire order and duplicates are preserved by [`headers`](Self::headers), not by the map.
    pub fn header_map(&self) -> HeaderMap {
        let mut map = HeaderMap::with_capacity(self.headers.len());
        for (k, v) in &self.headers {
            map.append(k.clone(), v.clone());
        }
        map
    }

    /// Response trailers in wire order. Buffered HTTP/2 and HTTP/3 responses carry them; streaming responses and HTTP/1.1 yield none.
    pub fn trailers(&self) -> impl Iterator<Item = (&HeaderName, &HeaderValue)> {
        self.trailers.iter().map(|(k, v)| (k, v))
    }

    /// Cookies from `Set-Cookie`. Last value wins per name; order is unspecified. Use [`header_all`](Self::header_all) for the raw headers.
    pub fn cookies(&self) -> impl Iterator<Item = (&str, &str)> {
        self.cookies.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// Look up a single cookie value by name.
    pub fn cookie(&self, name: &str) -> Option<&str> {
        self.cookies.get(name).map(String::as_str)
    }

    /// Negotiated TLS ALPN protocol, if TLS was used and ALPN was available.
    pub fn tls_alpn(&self) -> Option<&str> {
        self.tls_alpn.as_deref()
    }

    /// TLS peer certificate (DER-encoded), if available.
    pub fn tls_peer_certificate(&self) -> Option<&[u8]> {
        self.tls_peer_certificate.as_deref()
    }

    /// Negotiated TLS protocol version (e.g. `"TLS 1.3"`), if TLS was used.
    pub fn tls_version(&self) -> Option<&str> {
        self.tls_version.as_deref()
    }

    /// Negotiated TLS cipher suite (e.g. `"TLS_AES_128_GCM_SHA256"`), if TLS was used.
    pub fn tls_cipher(&self) -> Option<&str> {
        self.tls_cipher.as_deref()
    }

    /// Request headers as the session assembled them, in send order — preset headers, user-agent, sec-fetch hints, content-length, and the cookie jar merged with any caller-supplied headers.
    pub fn request_headers(&self) -> impl Iterator<Item = (&str, &str)> {
        self.request_headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// Response body decoded to a `String`, honoring the `charset` of the `Content-Type` header (default UTF-8); invalid sequences are replaced with U+FFFD and a leading BOM overrides the declared charset (WHATWG behavior). A streamed body is drained first.
    pub async fn text(&mut self) -> crate::core::Result<String> {
        self.text_with_charset("utf-8").await
    }

    /// Like [`text`](Self::text) but uses `default_encoding` (a WHATWG/IANA label, e.g. `"utf-8"`, `"windows-1252"`, `"shift_jis"`) when the response declares no charset.
    #[cfg(feature = "charset")]
    pub async fn text_with_charset(
        &mut self,
        default_encoding: &str,
    ) -> crate::core::Result<String> {
        self.drain().await?;
        let bytes = self.as_bytes().unwrap_or_default();
        let label = self.charset_label();
        let encoding =
            encoding_rs::Encoding::for_label(label.unwrap_or(default_encoding).as_bytes())
                .unwrap_or(encoding_rs::UTF_8);
        Ok(encoding.decode(bytes).0.into_owned())
    }

    /// UTF-8-lossy fallback when the `charset` feature is disabled.
    #[cfg(not(feature = "charset"))]
    pub async fn text_with_charset(
        &mut self,
        _default_encoding: &str,
    ) -> crate::core::Result<String> {
        Ok(String::from_utf8_lossy(self.bytes().await?).to_string())
    }

    /// The `charset` parameter of the `Content-Type` header, if present.
    #[cfg(feature = "charset")]
    fn charset_label(&self) -> Option<&str> {
        let ct = self.content_type()?;
        ct.split(';').skip(1).find_map(|param| {
            let (k, v) = param.split_once('=')?;
            k.trim()
                .eq_ignore_ascii_case("charset")
                .then(|| v.trim().trim_matches('"'))
        })
    }

    /// Response body as a borrowed UTF-8 string slice. A streamed body is drained first.
    pub async fn text_utf8(&mut self) -> crate::core::Result<&str> {
        self.drain().await?;
        std::str::from_utf8(self.as_bytes().unwrap_or_default())
            .map_err(|e| Error::new(Kind::Decode).with_message(e.to_string()))
    }

    /// Response body as raw bytes. A streamed body is drained, decompressed, and kept for later calls.
    pub async fn bytes(&mut self) -> crate::core::Result<&[u8]> {
        self.drain().await?;
        Ok(self.as_bytes().unwrap_or_default())
    }

    /// Already-buffered body bytes, without draining or awaiting; `None` while the body is still a stream or was taken by [`into_stream`](Self::into_stream).
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match &self.body {
            ResponseBody::Buffered(b) => Some(b),
            ResponseBody::Streaming(_) | ResponseBody::Taken => None,
        }
    }

    /// Already-buffered body as UTF-8, without draining or awaiting; `None` while the body is still a stream or was taken by [`into_stream`](Self::into_stream).
    pub fn as_text(&self) -> Option<crate::core::Result<&str>> {
        self.as_bytes().map(|b| {
            std::str::from_utf8(b).map_err(|e| Error::new(Kind::Decode).with_message(e.to_string()))
        })
    }

    /// Buffer a streamed body, decompressing it the way a non-streamed body is decompressed.
    async fn drain(&mut self) -> Result<()> {
        let stream = match std::mem::replace(&mut self.body, ResponseBody::Taken) {
            ResponseBody::Buffered(b) => {
                self.body = ResponseBody::Buffered(b);
                return Ok(());
            }
            ResponseBody::Taken => {
                return Err(Error::new(Kind::Body).with_message(
                    "response body stream was taken by `into_stream`; read the bytes from that stream",
                ));
            }
            ResponseBody::Streaming(s) => s,
        };
        let buf = drain_stream_into_vec(stream).await?;
        let (buf, headers) = strip(buf, std::mem::take(&mut self.headers), &self.compression)?;
        self.headers = headers;
        self.body = ResponseBody::Buffered(buf);
        Ok(())
    }

    /// Take ownership of the response body as raw bytes. A streamed body is drained first.
    pub async fn into_bytes(self) -> crate::core::Result<Vec<u8>> {
        let mut this = self;
        this.drain().await?;
        match this.body {
            ResponseBody::Buffered(b) => Ok(b),
            ResponseBody::Streaming(_) | ResponseBody::Taken => Ok(Vec::new()),
        }
    }

    /// Take ownership of the response body as a `String`, honoring the `Content-Type` charset (default UTF-8). A streamed body is drained first.
    pub async fn into_text(self) -> crate::core::Result<String> {
        #[cfg(feature = "charset")]
        let encoding = {
            let label = self.charset_label();
            label
                .and_then(|l| encoding_rs::Encoding::for_label(l.as_bytes()))
                .unwrap_or(encoding_rs::UTF_8)
        };
        let bytes = self.into_bytes().await?;
        #[cfg(feature = "charset")]
        if encoding != encoding_rs::UTF_8 {
            return Ok(encoding.decode(&bytes).0.into_owned());
        }
        Ok(match String::from_utf8(bytes) {
            Ok(s) => s,
            Err(e) => String::from_utf8_lossy(&e.into_bytes()).into_owned(),
        })
    }

    /// Deserialize the response body as JSON. A streamed body is drained first.
    pub async fn json<T: serde::de::DeserializeOwned>(&mut self) -> crate::core::Result<T> {
        Ok(serde_json::from_slice(self.bytes().await?)?)
    }

    /// Take ownership of the streaming response body.
    pub fn into_stream(mut self) -> Result<BodyStream> {
        match std::mem::replace(&mut self.body, ResponseBody::Taken) {
            ResponseBody::Streaming(s) => Ok(s),
            ResponseBody::Buffered(b) => Ok(BodyStream::from_bytes(bytes::Bytes::from(b))),
            ResponseBody::Taken => Err(Error::new(Kind::Body)
                .with_message("response body has already been taken as a stream")),
        }
    }

    /// Stream the response body into `writer`, returning the number of bytes written.
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

    /// Stream the response body to a file at `path`, returning the number of bytes written.
    pub async fn download_to(self, path: impl AsRef<std::path::Path>) -> Result<u64> {
        let mut file = tokio::fs::File::create(path).await.map_err(Error::from)?;
        self.copy_to(&mut file).await
    }

    /// Content-Length from the response headers, if present.
    pub fn content_length(&self) -> Option<u64> {
        self.header("content-length").and_then(|v| v.parse().ok())
    }

    /// Content-Type from the response headers, if present.
    pub fn content_type(&self) -> Option<&str> {
        self.header("content-type")
    }

    /// Whether the status code indicates success (2xx).
    pub fn is_success(&self) -> bool {
        self.status.is_success()
    }

    /// Whether the response was a redirect (3xx).
    pub fn is_redirect(&self) -> bool {
        self.status.is_redirection()
    }

    /// Whether the status is a client error (4xx).
    pub fn is_client_error(&self) -> bool {
        self.status.is_client_error()
    }

    /// Whether the status is a server error (5xx).
    pub fn is_server_error(&self) -> bool {
        self.status.is_server_error()
    }

    /// Turn a 4xx/5xx response into an error. This call does not await, so it attaches body bytes only when the body is already buffered ([`as_bytes`](Self::as_bytes)); a streamed body gives an error with no body.
    pub fn error_for_status(self) -> crate::core::Result<Self> {
        const MAX_ERROR_BODY: usize = 16 * 1024;
        if self.status.as_u16() >= 400 {
            let status = self.status;
            let url = self.url.clone();
            let full = self.as_bytes().unwrap_or_default();
            let body = full[..full.len().min(MAX_ERROR_BODY)].to_vec();
            let mut err = crate::Error::new(Kind::Status)
                .with_status(status)
                .with_body(body);
            if let Ok(uri) = url.parse::<http::Uri>() {
                err = err.with_url(uri);
            }
            Err(err)
        } else {
            Ok(self)
        }
    }

    /// Get fingerprint audit data for this response.
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

    /// Get the first value for a response header (case-insensitive).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.as_str().eq_ignore_ascii_case(name))
            .and_then(|(_, v)| v.to_str().ok())
    }

    /// All values for a response header (case-insensitive), in wire order.
    pub fn header_all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.headers
            .iter()
            .filter(move |(k, _)| k.as_str().eq_ignore_ascii_case(name))
            .filter_map(|(_, v)| v.to_str().ok())
    }
}

#[cfg(test)]
mod tests;
