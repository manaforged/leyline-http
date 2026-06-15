//! HTTP/1.1 `CONNECT` tunnel establishment and response validation.
//!
//! Security invariants enforced by [`validate_connect_response`]:
//!
//! 1. Status MUST be exactly `200` (three-digit token,
//!    whitespace-terminated). `200 OK` / `200 Connection established`
//!    both pass; `2000`, `200x`, `20` fail.
//! 2. A successful 2xx response to CONNECT MUST NOT carry
//!    `Content-Length` or `Transfer-Encoding` per RFC 9110 §9.3.6.
//! 3. There MUST be no bytes after the `\r\n\r\n`; the next octet on
//!    the wire belongs to the TLS handshake and must come from the
//!    origin. A proxy that pre-stuffs bytes after the response is
//!    attempting to inject pre-handshake data into the TLS stream.
//!
//! The status-line check runs first — a non-200 response is a normal
//! upstream failure (auth required, access denied) and deserves an
//! actionable error, not a scary "possible TLS-stream injection"
//! message triggered by the same-read body bytes of a `407`.

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::tls::error::TlsError;
use crate::tls::TlsStream;

use crate::util::{base64_encode, percent_decode};

/// Open a TLS-over-HTTP-CONNECT tunnel through `proxy` and return the
/// wrapped TLS stream. Fingerprint settings come from `connector`.
pub(crate) async fn connect<C: crate::tls::TlsHandshake>(
    connector: &C,
    host: &str,
    port: u16,
    proxy: &url::Url,
    include_alps: bool,
) -> Result<TlsStream, TlsError> {
    let mut tcp_stream = super::connect_to_proxy(proxy, 8080).await?;

    let connect_req = if let Some(password) = proxy.password() {
        let username = percent_decode(proxy.username());
        let password = percent_decode(password);
        let credentials = base64_encode(&format!("{username}:{password}"));
        format!(
            "CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\nProxy-Authorization: Basic {credentials}\r\n\r\n"
        )
    } else {
        format!("CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\n\r\n")
    };

    tcp_stream
        .write_all(connect_req.as_bytes())
        .await
        .map_err(TlsError::TcpConnect)?;

    // Read response until the end-of-headers sentinel `\r\n\r\n`.
    let mut response_buf = Vec::with_capacity(1024);
    let mut tmp = [0u8; 256];
    let end_idx = loop {
        let n = tcp_stream
            .read(&mut tmp)
            .await
            .map_err(TlsError::TcpConnect)?;
        if n == 0 {
            return Err(TlsError::Profile(
                "proxy closed connection before CONNECT response".into(),
            ));
        }
        response_buf.extend_from_slice(&tmp[..n]);
        if response_buf.len() > 8192 {
            return Err(TlsError::Profile("proxy CONNECT response too large".into()));
        }
        if let Some(i) = response_buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
    };

    validate_connect_response(&response_buf, end_idx)?;

    connector
        .do_tls_handshake(tcp_stream, host, include_alps)
        .await
}

/// Validate a proxy CONNECT response header block. `buf` is the bytes
/// read from the proxy; `end_idx` is one past the trailing `\r\n\r\n`
/// terminator. Caller always produces `4 <= end_idx <= buf.len()`, but
/// we defensively reject out-of-contract inputs rather than panic.
///
/// Checks run in this order:
///   a. Bounds — `end_idx` in contract.
///   b. UTF-8 — reject non-UTF-8 header bytes.
///   c. Status line — `HTTP/1.x SP 200 SP reason`. **Non-200 responses
///      exit here**, so the error carries the proxy's real status line
///      instead of a misleading trailing-bytes message.
///   d. Framing headers — reject `Content-Length` / `Transfer-Encoding`.
///   e. Trailing bytes — only meaningful on a successful 200, where
///      the next octet becomes the TLS handshake.
pub(crate) fn validate_connect_response(buf: &[u8], end_idx: usize) -> Result<(), TlsError> {
    if end_idx < 4 || end_idx > buf.len() {
        return Err(TlsError::Profile(format!(
            "proxy CONNECT response validator called with out-of-contract end_idx={end_idx} buf.len={}",
            buf.len()
        )));
    }

    let response = std::str::from_utf8(&buf[..end_idx])
        .map_err(|_| TlsError::Profile("proxy CONNECT response is not UTF-8".into()))?;
    let mut lines = response.split("\r\n");
    let status_line = lines.next().unwrap_or("");

    // (c) Strict status-line parse — check BEFORE the trailing-bytes
    // rule so a 407/403 body arriving in the same TCP read surfaces as
    // "proxy CONNECT failed: HTTP/1.1 407 ..." instead of masquerading
    // as a TLS-stream injection attempt.
    let mut parts = status_line.splitn(3, ' ');
    let version = parts.next().unwrap_or("");
    let code = parts.next().unwrap_or("");
    if !matches!(version, "HTTP/1.1" | "HTTP/1.0") || code != "200" {
        return Err(TlsError::Profile(format!(
            "proxy CONNECT failed: {status_line}"
        )));
    }

    // (d) Framing headers on the 2xx are illegal per RFC 9110 §9.3.6.
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if let Some((name, _)) = line.split_once(':') {
            let name = name.trim();
            if name.eq_ignore_ascii_case("content-length")
                || name.eq_ignore_ascii_case("transfer-encoding")
            {
                return Err(TlsError::Profile(format!(
                    "proxy CONNECT response contains forbidden framing header `{name}` \
                     (RFC 9110 §9.3.6)"
                )));
            }
        }
    }

    // (e) No bytes past the header terminator on a 200 — the next
    // octet belongs to the TLS handshake.
    if end_idx < buf.len() {
        return Err(TlsError::Profile(
            "proxy CONNECT response carries trailing bytes after headers \
             (possible TLS-stream injection)"
                .into(),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_connect_response;

    fn buf(s: &str) -> (Vec<u8>, usize) {
        let b = s.as_bytes().to_vec();
        let end = b.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
        (b, end)
    }

    #[test]
    fn accepts_minimal_200() {
        let (b, e) = buf("HTTP/1.1 200 OK\r\n\r\n");
        assert!(validate_connect_response(&b, e).is_ok());
    }

    #[test]
    fn accepts_http10_200_with_reason() {
        let (b, e) = buf("HTTP/1.0 200 Connection established\r\n\r\n");
        assert!(validate_connect_response(&b, e).is_ok());
    }

    #[test]
    fn accepts_200_with_benign_headers() {
        let (b, e) = buf("HTTP/1.1 200 OK\r\nVia: 1.1 proxy\r\nX-Foo: bar\r\n\r\n");
        assert!(validate_connect_response(&b, e).is_ok());
    }

    #[test]
    fn rejects_200_without_space() {
        let (b, e) = buf("HTTP/1.1 200OK\r\n\r\n");
        let err = validate_connect_response(&b, e).unwrap_err();
        assert!(format!("{err}").contains("proxy CONNECT failed"));
    }

    #[test]
    fn rejects_2000_code() {
        let (b, e) = buf("HTTP/1.1 2000 OK\r\n\r\n");
        let err = validate_connect_response(&b, e).unwrap_err();
        assert!(format!("{err}").contains("proxy CONNECT failed"));
    }

    #[test]
    fn rejects_unsupported_http_version() {
        let (b, e) = buf("HTTP/2.0 200 OK\r\n\r\n");
        let err = validate_connect_response(&b, e).unwrap_err();
        assert!(format!("{err}").contains("proxy CONNECT failed"));
    }

    #[test]
    fn rejects_non_200_status() {
        let (b, e) = buf("HTTP/1.1 407 Proxy Authentication Required\r\n\r\n");
        let err = validate_connect_response(&b, e).unwrap_err();
        assert!(format!("{err}").contains("proxy CONNECT failed"));
    }

    /// Regression: a proxy can return `407` with a `Content-Length: 121`
    /// body and the body often arrives in the same TCP read as the
    /// headers. The old ordering reported "trailing bytes after
    /// headers" — an actionable 407 became a scary TLS-injection
    /// message. Now the status-line check fires first.
    #[test]
    fn non_200_with_trailing_body_reports_status_not_injection() {
        let full = b"HTTP/1.1 407 Proxy Authentication Required\r\n\
                     Content-Length: 9\r\n\r\nforbidden"
            .to_vec();
        let end = full.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
        let err = validate_connect_response(&full, end).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("proxy CONNECT failed") && msg.contains("407"),
            "expected status-line error, got: {msg}"
        );
        assert!(
            !msg.contains("trailing bytes"),
            "trailing-bytes rule must not fire on non-200: {msg}"
        );
    }

    #[test]
    fn rejects_content_length_on_2xx() {
        let (b, e) = buf("HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
        let err = validate_connect_response(&b, e).unwrap_err();
        assert!(format!("{err}").contains("forbidden framing header"));
    }

    #[test]
    fn rejects_transfer_encoding_on_2xx() {
        let (b, e) = buf("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n");
        let err = validate_connect_response(&b, e).unwrap_err();
        assert!(format!("{err}").contains("forbidden framing header"));
    }

    #[test]
    fn rejects_mixed_case_framing_headers() {
        let (b, e) = buf("HTTP/1.1 200 OK\r\ncONTENT-lENGTH: 0\r\n\r\n");
        let err = validate_connect_response(&b, e).unwrap_err();
        assert!(format!("{err}").contains("forbidden framing header"));
    }

    #[test]
    fn rejects_trailing_bytes_after_terminator() {
        let full = b"HTTP/1.1 200 OK\r\n\r\nLEAKED".to_vec();
        let end = full.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
        let err = validate_connect_response(&full, end).unwrap_err();
        assert!(format!("{err}").contains("trailing bytes"));
    }

    #[test]
    fn rejects_non_utf8_body() {
        let mut full = b"HTTP/1.1 200 OK\r\nX-Evil: ".to_vec();
        full.extend_from_slice(&[0xFF, 0xFE, 0x80]);
        full.extend_from_slice(b"\r\n\r\n");
        let end = full.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
        let err = validate_connect_response(&full, end).unwrap_err();
        assert!(format!("{err}").contains("not UTF-8"));
    }

    #[test]
    fn rejects_end_idx_past_buf_len() {
        let b = b"HTTP/1.1 200 OK\r\n\r\n".to_vec();
        let err = validate_connect_response(&b, b.len() + 1).unwrap_err();
        assert!(format!("{err}").contains("out-of-contract"));
    }

    #[test]
    fn rejects_zero_end_idx() {
        let err = validate_connect_response(b"HTTP/1.1 200 OK\r\n\r\n", 0).unwrap_err();
        assert!(format!("{err}").contains("out-of-contract"));
    }

    #[test]
    fn rejects_tiny_end_idx_below_terminator_len() {
        let err = validate_connect_response(b"HTT", 3).unwrap_err();
        assert!(format!("{err}").contains("out-of-contract"));
    }
}
