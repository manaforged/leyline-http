//! HTTP response types.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use crate::core::body_stream::BodyStream;
use crate::core::error::{Error, Result};

/// Internal body representation. Either already buffered, or a
/// streaming receiver the caller asked for via `RequestBuilder::stream`.
pub(crate) enum ResponseBody {
    /// Fully-materialised bytes. The default.
    Buffered(Vec<u8>),
    /// Streaming delivery. Takes over once the caller opts in.
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

/// An HTTP response with buffered body.
///
/// All fields are accessed through methods so the internal
/// representation can evolve without breaking callers. See the
/// [impl block](#implementations) for the full accessor list.
#[derive(Debug)]
pub struct Response {
    pub(crate) status: u16,
    pub(crate) version: HttpVersion,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) trailers: Vec<(String, String)>,
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
    /// Shared connection-level fingerprints (JA4/JA3/H2/JA4T). `None` for
    /// responses built outside the TLS path (e.g. tests). Cloning this into
    /// the response is one atomic refcount bump — no string copies.
    pub(crate) audit_tls: Option<Arc<crate::audit::AuditTlsCache>>,
    /// Memoised full audit block. Populated on the first `audit()` call so the
    /// per-request JA4H hash + string clones never run for callers that don't
    /// introspect the fingerprint.
    pub(crate) audit_cache: OnceLock<crate::audit::AuditData>,
}

impl Response {
    // ─── Core fields ───────────────────────────────────────────────

    /// HTTP status code.
    pub fn status(&self) -> u16 {
        self.status
    }

    /// HTTP protocol version used for this response.
    pub fn version(&self) -> HttpVersion {
        self.version
    }

    /// Final URL (after redirects).
    pub fn url(&self) -> &str {
        &self.url
    }

    /// URLs visited during the redirect chain, in order. Empty if
    /// there were no redirects.
    pub fn redirect_chain(&self) -> &[String] {
        &self.redirect_chain
    }

    /// Response headers in wire order. Duplicates (e.g. multiple
    /// `Set-Cookie` headers) are preserved.
    ///
    /// Header name casing reflects the wire: HTTP/2 and HTTP/3
    /// responses are lowercase, HTTP/1.1 responses are mixed. Prefer
    /// [`Response::header`] for lookups — it is case-insensitive.
    /// If you iterate this slice yourself, compare names with
    /// `eq_ignore_ascii_case`.
    pub fn headers(&self) -> &[(String, String)] {
        &self.headers
    }

    /// Response trailers in wire order, if the transport exposed them.
    pub fn trailers(&self) -> &[(String, String)] {
        &self.trailers
    }

    /// Iterate cookies collected from `Set-Cookie` headers. Yields
    /// `(name, value)` pairs; for duplicates the last value wins.
    ///
    /// For the raw `Set-Cookie` header strings (with attributes like
    /// `Path=`, `HttpOnly`, etc.) use
    /// `response.header_all("set-cookie")`.
    pub fn cookies(&self) -> impl Iterator<Item = (&str, &str)> {
        self.cookies.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// Look up a single cookie value by name. Returns `None` if the
    /// server did not set a cookie with that name.
    pub fn cookie(&self, name: &str) -> Option<&str> {
        self.cookies.get(name).map(String::as_str)
    }

    /// Negotiated TLS ALPN protocol, if TLS was used and ALPN was
    /// available.
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

    /// Negotiated TLS cipher suite (e.g. `"TLS_AES_128_GCM_SHA256"`), if
    /// TLS was used.
    pub fn tls_cipher(&self) -> Option<&str> {
        self.tls_cipher.as_deref()
    }

    /// Request headers **as they were sent on the wire** — after the
    /// session's preset headers, user-agent, sec-fetch hints,
    /// content-length, and cookie jar have all been merged with any
    /// caller-supplied headers.
    ///
    /// Use this for fingerprint debugging: the order and casing here
    /// is exactly what the peer observed.
    pub fn request_headers(&self) -> &[(String, String)] {
        &self.request_headers
    }

    // ─── Body access ───────────────────────────────────────────────

    /// Response body decoded to a `String`, honoring the `charset` of the
    /// `Content-Type` header (default UTF-8); invalid sequences are replaced
    /// with U+FFFD and a leading BOM overrides the declared charset (WHATWG
    /// behavior). See [`Response::text_utf8`] for a zero-copy borrowing variant
    /// that fails on non-UTF-8, or [`Response::into_text`] to consume `self`.
    ///
    /// With the `charset` feature off this is UTF-8 lossy only.
    ///
    /// Returns an empty string when the body was delivered as a stream
    /// (the caller opted into streaming and has not drained the body).
    pub fn text(&self) -> String {
        self.text_with_charset("utf-8")
    }

    /// Like [`text`](Self::text) but uses `default_encoding` (a WHATWG/IANA
    /// label, e.g. `"utf-8"`, `"windows-1252"`, `"shift_jis"`) when the
    /// response declares no charset. An unrecognized label falls back to UTF-8.
    #[cfg(feature = "charset")]
    pub fn text_with_charset(&self, default_encoding: &str) -> String {
        let label = self.charset_label();
        let encoding =
            encoding_rs::Encoding::for_label(label.unwrap_or(default_encoding).as_bytes())
                .unwrap_or(encoding_rs::UTF_8);
        encoding.decode(self.bytes()).0.into_owned()
    }

    /// UTF-8-lossy fallback when the `charset` feature is disabled.
    #[cfg(not(feature = "charset"))]
    pub fn text_with_charset(&self, _default_encoding: &str) -> String {
        String::from_utf8_lossy(self.bytes()).to_string()
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

    /// Response body as a borrowed UTF-8 string slice. Returns
    /// `Err` if the body contains invalid UTF-8.
    pub fn text_utf8(&self) -> std::result::Result<&str, std::str::Utf8Error> {
        std::str::from_utf8(self.bytes())
    }

    /// Response body as raw bytes. Returns `&[]` for a streaming body
    /// that has not been drained into memory — use
    /// [`Response::into_stream`] instead.
    pub fn bytes(&self) -> &[u8] {
        match &self.body {
            ResponseBody::Buffered(b) => b,
            ResponseBody::Streaming(_) | ResponseBody::Taken => &[],
        }
    }

    /// Take ownership of the response body as raw bytes. Consumes
    /// `self` — use when you need to move the body without a copy.
    ///
    /// Returns `Vec::new()` when the body was delivered as a stream
    /// (opt in with `.stream()` and consume via [`Response::into_stream`]).
    pub fn into_bytes(self) -> Vec<u8> {
        match self.body {
            ResponseBody::Buffered(b) => b,
            ResponseBody::Streaming(_) | ResponseBody::Taken => Vec::new(),
        }
    }

    /// Take ownership of the response body as a `String`, honoring the
    /// `Content-Type` charset (default UTF-8). Consumes `self`. When the
    /// charset is UTF-8 and the body is valid this reuses the existing
    /// allocation; otherwise invalid sequences are replaced with U+FFFD.
    pub fn into_text(self) -> String {
        #[cfg(feature = "charset")]
        let encoding = {
            let label = self.charset_label();
            label
                .and_then(|l| encoding_rs::Encoding::for_label(l.as_bytes()))
                .unwrap_or(encoding_rs::UTF_8)
        };
        #[cfg(feature = "charset")]
        if encoding != encoding_rs::UTF_8 {
            return encoding.decode(&self.into_bytes()).0.into_owned();
        }
        // UTF-8 fast path: reuse the existing allocation when valid.
        match String::from_utf8(self.into_bytes()) {
            Ok(s) => s,
            Err(e) => String::from_utf8_lossy(&e.into_bytes()).into_owned(),
        }
    }

    /// Deserialize the response body as JSON.
    pub fn json<T: serde::de::DeserializeOwned>(&self) -> crate::core::Result<T> {
        Ok(serde_json::from_slice(self.bytes())?)
    }

    /// Take ownership of the streaming response body.
    ///
    /// Available only when the caller opted in with
    /// [`RequestBuilder::stream`](crate::RequestBuilder::stream) *and*
    /// the transport actually delivered the body as a stream. On the
    /// buffered default path — or when the transport fell back to
    /// buffering — this wraps the buffered bytes as a single-chunk
    /// stream so the caller API stays uniform.
    ///
    /// Decompression is NOT applied automatically when streaming is
    /// enabled: the `content-encoding` header is preserved and the
    /// caller is responsible for decompressing the stream.
    pub fn into_stream(mut self) -> Result<BodyStream> {
        match std::mem::replace(&mut self.body, ResponseBody::Taken) {
            ResponseBody::Streaming(s) => Ok(s),
            ResponseBody::Buffered(b) => Ok(BodyStream::from_bytes(bytes::Bytes::from(b))),
            ResponseBody::Taken => Err(Error::Http(
                "response body has already been taken as a stream".into(),
            )),
        }
    }

    /// Stream the response body into `writer`, returning the number of
    /// bytes written. Works on every response (a buffered body is written
    /// in one shot); pair it with [`RequestBuilder::stream`](crate::RequestBuilder::stream)
    /// to avoid holding the whole body in memory on the streaming transports.
    ///
    /// Like [`into_stream`](Self::into_stream), this does NOT decompress —
    /// the bytes are written as received (honouring `content-encoding`).
    ///
    /// ```rust,ignore
    /// let mut file = tokio::fs::File::create("out.bin").await?;
    /// let n = session.get(url).stream().send().await?.copy_to(&mut file).await?;
    /// ```
    pub async fn copy_to<W>(self, writer: &mut W) -> Result<u64>
    where
        W: tokio::io::AsyncWrite + Unpin,
    {
        use futures_util::StreamExt;
        use tokio::io::AsyncWriteExt;

        let mut stream = self.into_stream()?;
        let mut total: u64 = 0;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(Error::Io)?;
            writer.write_all(&chunk).await.map_err(Error::Io)?;
            total += chunk.len() as u64;
        }
        writer.flush().await.map_err(Error::Io)?;
        Ok(total)
    }

    /// Stream the response body to a file at `path`, returning the number of
    /// bytes written. Convenience over [`copy_to`](Self::copy_to) — creates
    /// (or truncates) the file and streams the body into it.
    ///
    /// ```rust,ignore
    /// let n = leyline::get(url).await?.download_to("out.bin").await?;
    /// ```
    pub async fn download_to(self, path: impl AsRef<std::path::Path>) -> Result<u64> {
        let mut file = tokio::fs::File::create(path).await.map_err(Error::Io)?;
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

    // ─── Status helpers ────────────────────────────────────────────

    /// Whether the status code indicates success (2xx).
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// Whether the response was a redirect (3xx).
    pub fn is_redirect(&self) -> bool {
        (300..400).contains(&self.status)
    }

    /// Whether the status is a client error (4xx).
    pub fn is_client_error(&self) -> bool {
        (400..500).contains(&self.status)
    }

    /// Whether the status is a server error (5xx).
    pub fn is_server_error(&self) -> bool {
        (500..600).contains(&self.status)
    }

    /// Turn a 4xx/5xx response into an error.
    ///
    /// Returns `Ok(self)` for 1xx/2xx/3xx, `Err` for 4xx/5xx.
    ///
    /// ```rust,ignore
    /// let resp = session.navigate(url).await?.error_for_status()?;
    /// // If we get here, status is 2xx (or 1xx/3xx).
    /// ```
    pub fn error_for_status(self) -> crate::core::Result<Self> {
        if self.status >= 400 {
            let status = self.status;
            let url = self.url.clone();
            let body = self.into_bytes();
            Err(crate::Error::Status {
                code: status,
                url,
                body,
            })
        } else {
            Ok(self)
        }
    }

    // ─── Audit ──────────────────────────────────────────────────────

    /// Get fingerprint audit data for this response.
    ///
    /// **Returns `None` unless audit was enabled on the session.** It is off
    /// by default (and not enabled by the `Session::chrome()`/etc. shortcuts)
    /// so the hot path pays nothing. Turn it on with
    /// `Session::builder().browser(...).audit(true).build()`.
    ///
    /// Returns JA3, JA4, JA4H, JA4T, and H2 fingerprints computed from
    /// the session's browser profile and the request headers sent.
    ///
    /// Computed lazily and memoised: the connection-level fingerprints
    /// (JA4/JA3/H2/JA4T) are precomputed once per session and shared by
    /// `Arc`; the request-dependent JA4H is hashed on the first call to this
    /// method and cached. Responses that never call `audit()` pay nothing
    /// beyond an atomic refcount bump at construction.
    ///
    /// ```rust,ignore
    /// let resp = session.navigate(url).await?;
    /// if let Some(audit) = resp.audit() {
    ///     println!("JA4:  {}", audit.ja4);
    ///     println!("JA3:  {}", audit.ja3);
    ///     println!("H2:   {}", audit.h2_fingerprint);
    ///     println!("JA4T: {}", audit.ja4t);
    ///     println!("JA4H: {}", audit.ja4h);
    /// }
    /// ```
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

    // ─── Header access ─────────────────────────────────────────────

    /// Get the first value for a response header (case-insensitive).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// Get all values for a response header (case-insensitive).
    pub fn header_all(&self, name: &str) -> Vec<&str> {
        self.headers
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bare_response(audit_tls: Option<Arc<crate::audit::AuditTlsCache>>) -> Response {
        Response {
            status: 200,
            version: HttpVersion::Http2,
            headers: Vec::new(),
            trailers: Vec::new(),
            body: ResponseBody::Buffered(Vec::new()),
            cookies: HashMap::new(),
            url: "https://example.test/".to_string(),
            redirect_chain: Vec::new(),
            request_headers: vec![
                (":method".to_string(), "GET".to_string()),
                ("accept-language".to_string(), "en-US,en;q=0.9".to_string()),
                ("referer".to_string(), "https://example.test/".to_string()),
            ],
            tls_alpn: None,
            tls_peer_certificate: None,
            tls_version: None,
            tls_cipher: None,
            request_method: "GET".to_string(),
            audit_tls,
            audit_cache: OnceLock::new(),
        }
    }

    fn sample_cache() -> Arc<crate::audit::AuditTlsCache> {
        Arc::new(crate::audit::AuditTlsCache {
            ja4: "t13d1516h2_8daaf6152771_d8a2da3f94cd".to_string(),
            ja3: "771,4865-4866,0-23,29-23,0".to_string(),
            h2_fingerprint: "1:65536;2:0;4:6291456;6:262144|15663105|0|m,a,s,p".to_string(),
            ja4t: "64240_2-1-3-1-1-4_1460_8".to_string(),
        })
    }

    #[test]
    fn audit_is_none_without_tls_context() {
        let resp = bare_response(None);
        assert!(resp.audit().is_none());
    }

    #[test]
    fn audit_surfaces_cached_connection_fingerprints() {
        let cache = sample_cache();
        let resp = bare_response(Some(cache.clone()));
        let audit = resp.audit().expect("audit present when tls context set");
        assert_eq!(audit.ja4, cache.ja4);
        assert_eq!(audit.ja3, cache.ja3);
        assert_eq!(audit.h2_fingerprint, cache.h2_fingerprint);
        assert_eq!(audit.ja4t, cache.ja4t);
        // JA4H is request-derived, so it must be non-empty and shaped a_b_c_d.
        assert_eq!(
            audit.ja4h.split('_').count(),
            4,
            "JA4H shape: {}",
            audit.ja4h
        );
    }

    #[test]
    fn audit_memoises_across_calls() {
        let resp = bare_response(Some(sample_cache()));
        let first = resp.audit().unwrap() as *const _;
        let second = resp.audit().unwrap() as *const _;
        // Same allocation on the second call — JA4H is hashed once, not per call.
        assert_eq!(first, second, "audit() must memoise, not recompute");
    }

    #[cfg(feature = "charset")]
    #[test]
    fn text_decodes_declared_charset() {
        let mut resp = bare_response(None);
        resp.headers = vec![(
            "content-type".to_string(),
            "text/html; charset=windows-1252".to_string(),
        )];
        // windows-1252: 0xE9 -> 'é', 0xA9 -> '©'. As raw UTF-8 these bytes are
        // invalid and would become U+FFFD without charset handling.
        resp.body = ResponseBody::Buffered(vec![0xE9, 0xA9]);
        assert_eq!(resp.text(), "é©");
        assert_eq!(resp.into_text(), "é©");
    }

    #[cfg(feature = "charset")]
    #[test]
    fn text_charset_param_is_case_insensitive_and_unquoted() {
        let mut resp = bare_response(None);
        resp.headers = vec![(
            "content-type".to_string(),
            "text/plain; Charset=\"Shift_JIS\"".to_string(),
        )];
        // Shift_JIS 0x82 0xA0 -> 'あ' (U+3042).
        resp.body = ResponseBody::Buffered(vec![0x82, 0xA0]);
        assert_eq!(resp.text(), "あ");
    }

    #[cfg(feature = "charset")]
    #[test]
    fn text_defaults_to_utf8_without_charset() {
        let mut resp = bare_response(None);
        resp.body = ResponseBody::Buffered("héllo".as_bytes().to_vec());
        // No declared charset -> text() uses UTF-8.
        assert_eq!(resp.text(), "héllo");
    }

    #[cfg(feature = "charset")]
    #[test]
    fn declared_charset_overrides_text_with_charset_default() {
        let mut resp = bare_response(None);
        resp.headers = vec![(
            "content-type".to_string(),
            "text/plain; charset=utf-8".to_string(),
        )];
        resp.body = ResponseBody::Buffered("héllo".as_bytes().to_vec());
        // The declared utf-8 wins over the windows-1252 caller default.
        assert_eq!(resp.text_with_charset("windows-1252"), "héllo");
    }
}
