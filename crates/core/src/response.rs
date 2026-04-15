//! HTTP response types.

use std::collections::HashMap;

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
    pub(crate) body: Vec<u8>,
    pub(crate) cookies: HashMap<String, String>,
    pub(crate) url: String,
    pub(crate) redirect_chain: Vec<String>,
    pub(crate) request_headers: Vec<(String, String)>,
    pub(crate) tls_alpn: Option<String>,
    pub(crate) tls_peer_certificate: Option<Vec<u8>>,
    pub(crate) tls_version: Option<String>,
    pub(crate) tls_cipher: Option<String>,
    pub(crate) audit_data: Option<leyline_audit::AuditData>,
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

    /// Response body as a UTF-8 string. Always allocates; invalid
    /// sequences are replaced with U+FFFD. See [`Response::text_utf8`]
    /// for a zero-copy borrowing variant that fails on non-UTF-8, or
    /// [`Response::into_text`] to consume `self` and skip the copy
    /// when the body is already valid UTF-8.
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).to_string()
    }

    /// Response body as a borrowed UTF-8 string slice. Returns
    /// `Err` if the body contains invalid UTF-8.
    pub fn text_utf8(&self) -> std::result::Result<&str, std::str::Utf8Error> {
        std::str::from_utf8(&self.body)
    }

    /// Response body as raw bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.body
    }

    /// Take ownership of the response body as raw bytes. Consumes
    /// `self` — use when you need to move the body without a copy.
    pub fn into_bytes(self) -> Vec<u8> {
        self.body
    }

    /// Take ownership of the response body as a UTF-8 string.
    /// Consumes `self`. If the body is already valid UTF-8 this
    /// reuses the existing allocation; otherwise invalid sequences
    /// are replaced with U+FFFD and a new allocation is made.
    pub fn into_text(self) -> String {
        match String::from_utf8(self.body) {
            Ok(s) => s,
            Err(e) => String::from_utf8_lossy(&e.into_bytes()).into_owned(),
        }
    }

    /// Deserialize the response body as JSON.
    pub fn json<T: serde::de::DeserializeOwned>(&self) -> crate::Result<T> {
        Ok(serde_json::from_slice(&self.body)?)
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
    pub fn error_for_status(self) -> crate::Result<Self> {
        if self.status >= 400 {
            Err(crate::Error::Status {
                code: self.status,
                url: self.url,
                body: self.body,
            })
        } else {
            Ok(self)
        }
    }

    // ─── Audit ──────────────────────────────────────────────────────

    /// Get fingerprint audit data for this response.
    ///
    /// Returns JA3, JA4, JA4H, JA4T, and H2 fingerprints computed from
    /// the session's browser profile and the request headers sent.
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
    pub fn audit(&self) -> Option<&leyline_audit::AuditData> {
        self.audit_data.as_ref()
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
