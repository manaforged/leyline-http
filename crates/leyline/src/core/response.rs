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

/// Wall-clock timing breakdown for the transport hop that produced a
/// [`Response`]. Purely observational — populated from `Instant` reads
/// around the existing connect/send awaits, so it never changes a byte on
/// the wire.
///
/// Currently populated for the **HTTP/2** path only (the default for
/// HTTPS); H1/H3 responses carry [`ResponseTiming::default`] until those
/// paths are instrumented. On a redirect chain the values are **summed
/// across every leg** leyline followed, so this describes the whole call —
/// `total_ms`/`send_ms` add up, `connect_ms` is the total handshake cost of
/// whichever legs opened fresh connections, and `reused` is true only when
/// no leg paid a connect.
///
/// `connect_ms` lumps DNS + TCP + TLS + the H2 preface into one number;
/// splitting those (and a true TTFB/body split, which lives inside the H2
/// driver) is a deliberate phase-2 follow-up.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResponseTiming {
    /// Every leg reused a pooled (warm) connection, so no connect/handshake
    /// cost was paid (`connect_ms` is `None`). An uninstrumented H1/H3
    /// response is distinguishable from a fast warm hop: it has `reused:
    /// false`, `connect_ms: None`, and `total_ms: 0` together.
    pub reused: bool,
    /// DNS + TCP connect + TLS handshake + H2 preface for the leg(s) that
    /// opened a fresh connection, in milliseconds. `None` when every leg
    /// reused. NOTE: on a connection coalesced behind another in-flight
    /// connect this includes the *wait* for that shared handshake, not this
    /// request's own DNS/TCP/TLS — a `coalesced` discriminator is a phase-2
    /// follow-up (TODO), so don't read aggregate `connect_ms` as pure
    /// handshake cost.
    pub connect_ms: Option<u32>,
    /// Request-send → response, in milliseconds, summed across redirect legs.
    /// For the buffered path (the default) this
    /// spans send through the last body byte — `send_request_ex` resolves on
    /// END_STREAM. For `stream_response: true` it is TTFB only (resolves at
    /// the HEADERS frame; the body streams afterward). The internal
    /// TTFB-vs-body split is not yet exposed.
    pub send_ms: u32,
    /// Whole transport exchange (connect, if any, plus send), summed across
    /// redirect legs, in milliseconds. May slightly exceed
    /// `connect_ms + send_ms` on the warm path (it also covers pool checkout)
    /// or when a dead pooled connection was retried before the successful
    /// attempt — that gap is real wall-clock the caller paid.
    pub total_ms: u32,
}

impl ResponseTiming {
    /// Seed for accumulating a redirect-following request across its legs.
    /// `reused` starts `true` (the identity for "no leg paid a fresh
    /// connect") and is flipped to `false` by the first leg that opened a
    /// new connection. Distinct from [`ResponseTiming::default`], whose
    /// `reused: false` is the right resting value for a leg that was never
    /// instrumented (H1/H3).
    pub(crate) fn accumulator() -> Self {
        Self {
            reused: true,
            connect_ms: None,
            send_ms: 0,
            total_ms: 0,
        }
    }

    /// Fold one transport leg into the running total. `total_ms`/`send_ms`
    /// sum (saturating); `connect_ms` sums only the legs that actually
    /// connected (stays `None` if every leg reused a pooled connection);
    /// `reused` stays `true` only while every leg so far reused. This makes
    /// [`Response::timing`] describe the WHOLE request — including any
    /// redirects leyline followed — rather than just the final hop.
    pub(crate) fn add_leg(&mut self, leg: &ResponseTiming) {
        self.total_ms = self.total_ms.saturating_add(leg.total_ms);
        self.send_ms = self.send_ms.saturating_add(leg.send_ms);
        if let Some(c) = leg.connect_ms {
            self.connect_ms = Some(self.connect_ms.unwrap_or(0).saturating_add(c));
        }
        self.reused &= leg.reused;
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
    pub(crate) headers: Vec<(crate::core::HeaderStr, crate::core::HeaderStr)>,
    pub(crate) trailers: Vec<(crate::core::HeaderStr, crate::core::HeaderStr)>,
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
    /// Wall-clock timing breakdown for the request, summed across any
    /// redirect legs. See [`ResponseTiming`]. Default (all-zero) for H1/H3
    /// and test-built responses.
    pub(crate) timing: ResponseTiming,
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

    /// Wall-clock timing breakdown for the transport hop that produced this
    /// response (warm-vs-cold connection, connect/handshake cost, send time).
    /// Populated for HTTP/2; [`ResponseTiming::default`] for H1/H3. See
    /// [`ResponseTiming`].
    pub fn timing(&self) -> &ResponseTiming {
        &self.timing
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

    /// Response headers in wire order, as `(name, value)` pairs.
    /// Duplicates (e.g. multiple `Set-Cookie` headers) are preserved.
    ///
    /// Header name casing reflects the wire: HTTP/2 and HTTP/3
    /// responses are lowercase, HTTP/1.1 responses are mixed. Prefer
    /// [`Response::header`] for lookups — it is case-insensitive.
    /// If you iterate yourself, compare names with `eq_ignore_ascii_case`.
    ///
    /// Values are UTF-8: a non-UTF-8 obs-text byte (rare — e.g. a raw `0xFF`) is
    /// replaced with U+FFFD and is not recoverable through this `&str` API. The
    /// HPACK/QPACK tables keep the original wire bytes; only this view is coerced.
    pub fn headers(&self) -> impl Iterator<Item = (&str, &str)> {
        self.headers.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// Response trailers in wire order, as `(name, value)` pairs, if the
    /// transport exposed them.
    pub fn trailers(&self) -> impl Iterator<Item = (&str, &str)> {
        self.trailers.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// Iterate cookies collected from `Set-Cookie` headers. Yields
    /// `(name, value)` pairs; for duplicates the last value wins. Iteration
    /// order is unspecified (cookies are keyed in a map, not wire-ordered).
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

    /// Request headers as the session assembled them, in send order — preset
    /// headers, user-agent, sec-fetch hints, content-length, and the cookie jar
    /// merged with any caller-supplied headers.
    ///
    /// These are the *application* headers. Transport-level fields synthesized at
    /// serialization are NOT included: the HTTP/2/3 `:method` / `:scheme` /
    /// `:authority` / `:path` pseudo-headers and the HTTP/1 `Host` line. This is
    /// also exactly the set JA4H hashes (which excludes pseudo-headers).
    ///
    /// Populated only when audit is enabled ([`crate::SessionBuilder::audit`]);
    /// otherwise the iterator is empty, since the hot path keeps no copy.
    ///
    /// Yields `(name, value)` as `&str` pairs — representation-independent, like
    /// [`Response::headers`], so the internal storage can evolve without breaking
    /// callers.
    pub fn request_headers(&self) -> impl Iterator<Item = (&str, &str)> {
        self.request_headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
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
        if matches!(self.body, ResponseBody::Streaming(_) | ResponseBody::Taken) {
            return Err(crate::Error::Body(
                "response body was delivered as a stream (opt in with `.stream()`) —                  consume it via `into_stream()` and deserialize the bytes yourself"
                    .into(),
            ));
        }
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
    /// let n = session.request("GET", url).stream().send().await?.copy_to(&mut file).await?;
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
    /// Returns `Ok(self)` for 1xx/2xx/3xx, `Err` for 4xx/5xx and for a
    /// malformed status below 100 (e.g. a response that carried no `:status`).
    ///
    /// ```rust,ignore
    /// let resp = session.navigate(url).await?.error_for_status()?;
    /// // If we get here, status is 2xx (or 1xx/3xx).
    /// ```
    pub fn error_for_status(self) -> crate::core::Result<Self> {
        // Cap the body retained in `Error::Status`. That error is routinely
        // logged and propagated, so stowing a body up to the 100 MB buffer cap
        // would balloon logs and memory. 16 KiB is plenty to debug a 4xx/5xx
        // (error page, JSON error envelope, rate-limit note); the full body is
        // still available from the `Response` before you call this.
        const MAX_ERROR_BODY: usize = 16 * 1024;
        if self.status >= 400 || self.status < 100 {
            let status = self.status;
            // The error value gets logged; strip any URL credentials.
            let url = crate::util::redacted_url(&self.url);
            // Copy only the retained prefix into a right-sized allocation and
            // drop the (potentially huge) full body.
            let full = self.into_bytes();
            let body = full[..full.len().min(MAX_ERROR_BODY)].to_vec();
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
mod tests;
